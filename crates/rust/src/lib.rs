use crate::interface::InterfaceGenerator;
use anyhow::{bail, Result};
use heck::*;
use indexmap::IndexSet;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::{self, Write as _};
use std::io::{Read, Write};
use std::mem;
use std::process::{Command, Stdio};
use std::str::FromStr;
use wit_bindgen_core::abi::{Bitcast, WasmType};
use wit_bindgen_core::{
    uwrite, uwriteln, wit_parser::*, Files, InterfaceGenerator as _, Source, Types, WorldGenerator,
};

mod bindgen;
mod interface;

struct InterfaceName {
    /// True when this interface name has been remapped through the use of `with` in the `bindgen!`
    /// macro invocation.
    remapped: bool,

    /// The string name for this interface.
    path: String,
}

#[derive(Default)]
struct RustWasm {
    types: Types,
    src: Source,
    opts: Opts,
    import_modules: Vec<(String, Vec<String>)>,
    export_modules: Vec<(String, Vec<String>)>,
    skip: HashSet<String>,
    interface_names: HashMap<InterfaceId, InterfaceName>,
    /// Each imported and exported interface is stored in this map. Value indicates if last use was import.
    interface_last_seen_as_import: HashMap<InterfaceId, bool>,
    import_funcs_called: bool,
    with_name_counter: usize,
    // Track the with options that were used. Remapped interfaces provided via `with`
    // are required to be used.
    used_with_opts: HashSet<String>,
    world: Option<WorldId>,

    rt_module: IndexSet<RuntimeItem>,
}

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug)]
enum RuntimeItem {
    AllocCrate,
    StringType,
    StdAllocModule,
    VecType,
    StringLift,
    InvalidEnumDiscriminant,
    CharLift,
    BoolLift,
    CabiDealloc,
    RunCtorsOnce,
    AsI32,
    AsI64,
    AsF32,
    AsF64,
    ResourceType,
    RustResource,
}

#[cfg(feature = "clap")]
fn iterate_hashmap_string(s: &str) -> impl Iterator<Item = Result<(&str, &str), String>> {
    s.split(',').map(move |entry| {
        entry.split_once('=').ok_or_else(|| {
            format!("expected string of form `<key>=<value>[,<key>=<value>...]`; got `{s}`")
        })
    })
}

#[cfg(feature = "clap")]
fn parse_exports(s: &str) -> Result<HashMap<ExportKey, String>, String> {
    if s.is_empty() {
        Ok(HashMap::default())
    } else {
        iterate_hashmap_string(s)
            .map(|entry| {
                let (key, value) = entry?;
                Ok((
                    match key {
                        "world" => ExportKey::World,
                        _ => ExportKey::Name(key.to_owned()),
                    },
                    value.to_owned(),
                ))
            })
            .collect()
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExportKey {
    World,
    Name(String),
}

#[cfg(feature = "clap")]
fn parse_with(s: &str) -> Result<HashMap<String, String>, String> {
    if s.is_empty() {
        Ok(HashMap::default())
    } else {
        iterate_hashmap_string(s)
            .map(|entry| {
                let (key, value) = entry?;
                Ok((key.to_owned(), value.to_owned()))
            })
            .collect()
    }
}

#[derive(Default, Debug, Clone)]
#[cfg_attr(feature = "clap", derive(clap::Args))]
pub struct Opts {
    /// Whether or not `rustfmt` is executed to format generated code.
    #[cfg_attr(feature = "clap", arg(long))]
    pub rustfmt: bool,

    /// If true, code generation should qualify any features that depend on
    /// `std` with `cfg(feature = "std")`.
    #[cfg_attr(feature = "clap", arg(long))]
    pub std_feature: bool,

    /// If true, code generation should pass borrowed string arguments as
    /// `&[u8]` instead of `&str`. Strings are still required to be valid
    /// UTF-8, but this avoids the need for Rust code to do its own UTF-8
    /// validation if it doesn't already have a `&str`.
    #[cfg_attr(feature = "clap", arg(long))]
    pub raw_strings: bool,

    /// Names of functions to skip generating bindings for.
    #[cfg_attr(feature = "clap", arg(long))]
    pub skip: Vec<String>,

    /// Names of the concrete types which implement the traits representing any
    /// functions, interfaces, and/or resources exported by the world.
    ///
    /// Example: `--exports world=MyWorld,ns:pkg/iface1=MyIface1,ns:pkg/iface1/resource1=MyResource1`,
    #[cfg_attr(feature = "clap", arg(long, value_parser = parse_exports, default_value = ""))]
    pub exports: HashMap<ExportKey, String>,

    /// If true, generate stub implementations for any exported functions,
    /// interfaces, and/or resources.
    #[cfg_attr(feature = "clap", arg(long))]
    pub stubs: bool,

    /// Optionally prefix any export names with the specified value.
    ///
    /// This is useful to avoid name conflicts when testing.
    #[cfg_attr(feature = "clap", arg(long))]
    pub export_prefix: Option<String>,

    /// Whether to generate owning or borrowing type definitions.
    ///
    /// Valid values include:
    ///
    /// - `owning`: Generated types will be composed entirely of owning fields,
    /// regardless of whether they are used as parameters to imports or not.
    ///
    /// - `borrowing`: Generated types used as parameters to imports will be
    /// "deeply borrowing", i.e. contain references rather than owned values
    /// when applicable.
    ///
    /// - `borrowing-duplicate-if-necessary`: As above, but generating distinct
    /// types for borrowing and owning, if necessary.
    #[cfg_attr(feature = "clap", arg(long, default_value_t = Ownership::Owning))]
    pub ownership: Ownership,

    /// The optional path to the wit-bindgen runtime module to use.
    ///
    /// This defaults to `wit_bindgen::rt`.
    #[cfg_attr(feature = "clap", arg(long))]
    pub runtime_path: Option<String>,

    /// The optional path to the bitflags crate to use.
    ///
    /// This defaults to `wit_bindgen::bitflags`.
    #[cfg_attr(feature = "clap", arg(long))]
    pub bitflags_path: Option<String>,

    /// Additional derive attributes to add to generated types. If using in a CLI, this flag can be
    /// specified multiple times to add multiple attributes.
    ///
    /// These derive attributes will be added to any generated structs or enums
    #[cfg_attr(feature = "clap", arg(long = "additional_derive_attribute", short = 'd', default_values_t = Vec::<String>::new()))]
    pub additional_derive_attributes: Vec<String>,

    /// Remapping of interface names to rust module names.
    #[cfg_attr(feature = "clap", arg(long, value_parser = parse_with, default_value = ""))]
    pub with: HashMap<String, String>,

    /// Add the specified suffix to the name of the custome section containing
    /// the component type.
    #[cfg_attr(feature = "clap", arg(long))]
    pub type_section_suffix: Option<String>,

    /// Apply a workaround required before Rust 1.69 to run wasm ctors only
    /// once.
    #[cfg_attr(feature = "clap", arg(long))]
    pub run_ctors_once_workaround: bool,
}

impl Opts {
    pub fn build(self) -> Box<dyn WorldGenerator> {
        let mut r = RustWasm::new();
        r.skip = self.skip.iter().cloned().collect();
        r.opts = self;
        Box::new(r)
    }
}

impl RustWasm {
    fn new() -> RustWasm {
        RustWasm::default()
    }

    fn interface<'a>(
        &'a mut self,
        identifier: Identifier<'a>,
        wasm_import_module: Option<&'a str>,
        resolve: &'a Resolve,
        in_import: bool,
    ) -> InterfaceGenerator<'a> {
        let mut sizes = SizeAlign::default();
        sizes.fill(resolve);

        InterfaceGenerator {
            identifier,
            wasm_import_module,
            src: Source::default(),
            in_import,
            gen: self,
            sizes,
            resolve,
            return_pointer_area_size: 0,
            return_pointer_area_align: 0,
            needs_runtime_module: false,
        }
    }

    fn emit_modules(&mut self, modules: Vec<(String, Vec<String>)>) {
        #[derive(Default)]
        struct Module {
            submodules: BTreeMap<String, Module>,
            contents: Vec<String>,
        }
        let mut map = Module::default();
        for (module, path) in modules {
            let mut cur = &mut map;
            for name in path[..path.len() - 1].iter() {
                cur = cur
                    .submodules
                    .entry(name.clone())
                    .or_insert(Module::default());
            }
            cur.contents.push(module);
        }
        emit(&mut self.src, map);
        fn emit(me: &mut Source, module: Module) {
            for (name, submodule) in module.submodules {
                uwriteln!(me, "pub mod {name} {{");
                emit(me, submodule);
                uwriteln!(me, "}}");
            }
            for submodule in module.contents {
                uwriteln!(me, "{submodule}");
            }
        }
    }

    fn runtime_path(&self) -> &str {
        self.opts
            .runtime_path
            .as_deref()
            .unwrap_or("wit_bindgen::rt")
    }

    fn bitflags_path(&self) -> &str {
        self.opts
            .bitflags_path
            .as_deref()
            .unwrap_or("wit_bindgen::bitflags")
    }

    fn lookup_export(&self, key: &ExportKey) -> Result<String> {
        if let Some(key) = self.opts.exports.get(key) {
            return Ok(key.clone());
        }
        if self.opts.stubs {
            return Ok("Stub".to_owned());
        }
        let key = match key {
            ExportKey::World => "world".to_owned(),
            ExportKey::Name(name) => format!("\"{name}\""),
        };
        if self.opts.exports.is_empty() {
            bail!(MissingExportsMap { key });
        }
        bail!("expected `exports` map to contain key `{key}`")
    }

    fn name_interface(
        &mut self,
        resolve: &Resolve,
        id: InterfaceId,
        name: &WorldKey,
        is_export: bool,
    ) -> bool {
        let with_name = resolve.name_world_key(name);
        let entry = if let Some(remapped_path) = self.opts.with.get(&with_name) {
            let name = format!("__with_name{}", self.with_name_counter);
            self.used_with_opts.insert(with_name);
            self.with_name_counter += 1;
            uwriteln!(self.src, "use {remapped_path} as {name};");
            InterfaceName {
                remapped: true,
                path: name,
            }
        } else {
            let path = compute_module_path(name, resolve, is_export).join("::");

            InterfaceName {
                remapped: false,
                path,
            }
        };

        let remapped = entry.remapped;
        self.interface_names.insert(id, entry);

        remapped
    }

    fn finish_runtime_module(&mut self) {
        if self.rt_module.is_empty() {
            return;
        }
        self.src.push_str("mod _rt {\n");
        let mut emitted = IndexSet::new();
        while !self.rt_module.is_empty() {
            for item in mem::take(&mut self.rt_module) {
                if emitted.insert(item) {
                    self.emit_runtime_item(item);
                }
            }
        }
        self.src.push_str("}\n");

        if emitted.contains(&RuntimeItem::ResourceType) {
            self.src.push_str("pub use _rt::Resource;\n");
        }
    }

    fn emit_runtime_item(&mut self, item: RuntimeItem) {
        match item {
            RuntimeItem::AllocCrate => {
                uwriteln!(self.src, "extern crate alloc as alloc_crate;");
            }
            RuntimeItem::StdAllocModule => {
                self.rt_module.insert(RuntimeItem::AllocCrate);
                uwriteln!(self.src, "pub use alloc_crate::alloc;");
            }
            RuntimeItem::StringType => {
                self.rt_module.insert(RuntimeItem::AllocCrate);
                uwriteln!(self.src, "pub use alloc_crate::string::String;");
            }
            RuntimeItem::VecType => {
                self.rt_module.insert(RuntimeItem::AllocCrate);
                uwriteln!(self.src, "pub use alloc_crate::vec::Vec;");
            }

            RuntimeItem::CabiDealloc => {
                self.rt_module.insert(RuntimeItem::StdAllocModule);
                self.src.push_str(
                    "\
pub unsafe fn cabi_dealloc(ptr: i32, size: usize, align: usize) {
    if size == 0 {
        return;
    }
    let layout = alloc::Layout::from_size_align_unchecked(size, align);
    alloc::dealloc(ptr as *mut u8, layout);
}
                    ",
                );
            }

            RuntimeItem::StringLift => {
                self.rt_module.insert(RuntimeItem::StringType);
                self.src.push_str(
                    "\
pub unsafe fn string_lift(bytes: Vec<u8>) -> String {
    if cfg!(debug_assertions) {
        String::from_utf8(bytes).unwrap()
    } else {
        String::from_utf8_unchecked(bytes)
    }
}
                    ",
                );
            }

            RuntimeItem::InvalidEnumDiscriminant => {
                self.src.push_str(
                    "\
pub unsafe fn invalid_enum_discriminant<T>() -> T {
    if cfg!(debug_assertions) {
        panic!(\"invalid enum discriminant\")
    } else {
        core::hint::unreachable_unchecked()
    }
}
                    ",
                );
            }

            RuntimeItem::CharLift => {
                self.src.push_str(
                    "\
pub unsafe fn char_lift(val: u32) -> char {
    if cfg!(debug_assertions) {
        core::char::from_u32(val).unwrap()
    } else {
        core::char::from_u32_unchecked(val)
    }
}
                    ",
                );
            }

            RuntimeItem::BoolLift => {
                self.src.push_str(
                    "\
pub unsafe fn bool_lift(val: u8) -> bool {
    if cfg!(debug_assertions) {
        match val {
            0 => false,
            1 => true,
            _ => panic!(\"invalid bool discriminant\"),
        }
    } else {
        core::mem::transmute::<u8, bool>(val)
    }
}
                    ",
                );
            }

            RuntimeItem::RunCtorsOnce => {
                self.src.push_str(
                    r#"
/// Provide a hook for generated export functions to run static
/// constructors at most once. wit-bindgen-rust generates a call to this
/// function at the start of all component export functions. Importantly,
/// it is not called as part of `cabi_realloc`, which is a *core* export
/// func, but may not execute ctors, because the environment ctor in
/// wasi-libc (before rust 1.69.0) calls an import func, which is not
/// permitted by the Component Model when inside realloc.
#[cfg(target_arch = "wasm32")]
pub fn run_ctors_once() {
    static mut RUN: bool = false;
    unsafe {
        if !RUN {
            // This function is synthesized by `wasm-ld` to run all static
            // constructors. wasm-ld will either provide an implementation
            // of this symbol, or synthesize a wrapper around each
            // exported function to (unconditionally) run ctors. By using
            // this function, the linked module is opting into "manually"
            // running ctors.
            extern "C" {
                fn __wasm_call_ctors();
            }
            __wasm_call_ctors();
            RUN = true;
        }
    }
}
                    "#,
                );
            }

            RuntimeItem::AsI32 => {
                self.emit_runtime_as_trait(
                    "i32",
                    &["i32", "u32", "i16", "u16", "i8", "u8", "char", "usize"],
                );
            }

            RuntimeItem::AsI64 => {
                self.emit_runtime_as_trait("i64", &["i64", "u64"]);
            }

            RuntimeItem::AsF32 => {
                self.emit_runtime_as_trait("f32", &["f32"]);
            }

            RuntimeItem::AsF64 => {
                self.emit_runtime_as_trait("f64", &["f64"]);
            }

            RuntimeItem::ResourceType => {
                self.src.push_str(
                    r#"
type RawRep<T> = Option<T>;

use core::fmt;
use core::marker;
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// A type which represents a component model resource, either imported or
/// exported into this component.
///
/// This is a low-level wrapper which handles the lifetime of the resource
/// (namely this has a destructor). The `T` provided defines the component model
/// intrinsics that this wrapper uses.
///
/// One of the chief purposes of this type is to provide `Deref` implementations
/// to access the underlying data when it is owned.
///
/// This type is primarily used in generated code for exported and imported
/// resources.
#[repr(transparent)]
pub struct Resource<T: WasmResource> {
    // NB: This would ideally be `u32` but it is not. The fact that this has
    // interior mutability is not exposed in the API of this type except for the
    // `take_handle` method which is supposed to in theory be private.
    //
    // This represents, almost all the time, a valid handle value. When it's
    // invalid it's stored as `u32::MAX`.
    handle: AtomicU32,
    _marker: marker::PhantomData<T>,
}

/// A trait which all wasm resources implement, namely providing the ability to
/// drop a resource.
///
/// This generally is implemented by generated code, not user-facing code.
pub unsafe trait WasmResource {
    /// Invokes the `[resource-drop]...` intrinsic.
    unsafe fn drop(handle: u32);
}

impl<T: WasmResource> Resource<T> {
    #[doc(hidden)]
    pub unsafe fn from_handle(handle: u32) -> Self {
        debug_assert!(handle != u32::MAX);
        Self {
            handle: AtomicU32::new(handle),
            _marker: marker::PhantomData,
        }
    }

    /// Takes ownership of the handle owned by `resource`.
    ///
    /// Note that this ideally would be `into_handle` taking `Resource<T>` by
    /// ownership. The code generator does not enable that in all situations,
    /// unfortunately, so this is provided instead.
    ///
    /// Also note that `take_handle` is in theory only ever called on values
    /// owned by a generated function. For example a generated function might
    /// take `Resource<T>` as an argument but then call `take_handle` on a
    /// reference to that argument. In that sense the dynamic nature of
    /// `take_handle` should only be exposed internally to generated code, not
    /// to user code.
    #[doc(hidden)]
    pub fn take_handle(resource: &Resource<T>) -> u32 {
        resource.handle.swap(u32::MAX, Relaxed)
    }

    #[doc(hidden)]
    pub fn handle(resource: &Resource<T>) -> u32 {
        resource.handle.load(Relaxed)
    }
}

impl<T: WasmResource> fmt::Debug for Resource<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resource")
            .field("handle", &self.handle)
            .finish()
    }
}

impl<T: WasmResource> Drop for Resource<T> {
    fn drop(&mut self) {
        unsafe {
            match self.handle.load(Relaxed) {
                // If this handle was "taken" then don't do anything in the
                // destructor.
                u32::MAX => {}

                // ... but otherwise do actually destroy it with the imported
                // component model intrinsic as defined through `T`.
                other => T::drop(other),
            }
        }
    }
}
                    "#,
                );
            }
            RuntimeItem::RustResource => {
                self.rt_module.insert(RuntimeItem::ResourceType);
                self.rt_module.insert(RuntimeItem::AllocCrate);
                self.src.push_str(
                    r#"
use alloc_crate::boxed::Box;
use core::ops::{Deref, DerefMut};

/// A trait which extends [`WasmResource`] used for Rust-defined resources, or
/// those exported from this component.
///
/// This generally is implemented by generated code, not user-facing code.
pub unsafe trait RustResource: WasmResource {
    /// Invokes the `[resource-new]...` intrinsic.
    unsafe fn new(rep: usize) -> u32;
    /// Invokes the `[resource-rep]...` intrinsic.
    unsafe fn rep(handle: u32) -> usize;
}

impl<T: RustResource> Resource<T> {
    /// Creates a new Rust-defined resource from the underlying representation
    /// `T`.
    ///
    /// This will move `T` onto the heap to create a single pointer to represent
    /// it which is then wrapped up in a component model resource.
    pub fn new(val: T) -> Resource<T> {
        let rep = Box::into_raw(Box::new(Some(val))) as usize;
        unsafe {
            let handle = T::new(rep);
            Resource::from_handle(handle)
        }
    }

    #[doc(hidden)]
    pub unsafe fn dtor(rep: usize) {
        let _ = Box::from_raw(rep as *mut RawRep<T>);
    }

    /// Takes back ownership of the object, dropping the resource handle.
    pub fn into_inner(resource: Self) -> T {
        unsafe {
            let rep = T::rep(resource.handle.load(Relaxed));
            RawRep::take(&mut *(rep as *mut RawRep<T>)).unwrap()
        }
    }

    #[doc(hidden)]
    pub unsafe fn lift_borrow<'a>(rep: usize) -> &'a T {
        RawRep::as_ref(&*(rep as *const RawRep<T>)).unwrap()
    }
}

impl<T: RustResource> Deref for Resource<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe {
            let rep = T::rep(self.handle.load(Relaxed));
            RawRep::as_ref(&*(rep as *const RawRep<T>)).unwrap()
        }
    }
}

impl<T: RustResource> DerefMut for Resource<T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe {
            let rep = T::rep(self.handle.load(Relaxed));
            RawRep::as_mut(&mut *(rep as *mut RawRep<T>)).unwrap()
        }
    }
}
                    "#,
                );
            }
        }
    }

    // This is a workaround for in the bindings sometimes we've got `&i32` and
    // sometimes we've got `i32` but that should all be able to be worked with
    // as `i32`, so these helper functions are used to boil away the
    // indirection.
    fn emit_runtime_as_trait(&mut self, ty: &str, to_convert: &[&str]) {
        let upcase = ty.to_uppercase();
        self.src.push_str(&format!(
            r#"
pub fn as_{ty}<T: As{upcase}>(t: T) -> {ty} {{
    t.as_{ty}()
}}

pub trait As{upcase} {{
    fn as_{ty}(self) -> {ty};
}}

impl<'a, T: Copy + As{upcase}> As{upcase} for &'a T {{
    fn as_{ty}(self) -> {ty} {{
        (*self).as_{ty}()
    }}
}}
            "#
        ));

        for to_convert in to_convert {
            self.src.push_str(&format!(
                r#"
impl As{upcase} for {to_convert} {{
    #[inline]
    fn as_{ty}(self) -> {ty} {{
        self as {ty}
    }}
}}
                "#
            ));
        }
    }
}

/// If the package `id` is the only package with its namespace/name combo
/// then pass through the name unmodified. If, however, there are multiple
/// versions of this package then the package module is going to get version
/// information.
fn name_package_module(resolve: &Resolve, id: PackageId) -> String {
    let pkg = &resolve.packages[id];
    let versions_with_same_name = resolve
        .packages
        .iter()
        .filter_map(|(_, p)| {
            if p.name.namespace == pkg.name.namespace && p.name.name == pkg.name.name {
                Some(&p.name.version)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let base = pkg.name.name.to_snake_case();
    if versions_with_same_name.len() == 1 {
        return base;
    }

    let version = match &pkg.name.version {
        Some(version) => version,
        // If this package didn't have a version then don't mangle its name
        // and other packages with the same name but with versions present
        // will have their names mangled.
        None => return base,
    };

    // Here there's multiple packages with the same name that differ only in
    // version, so the version needs to be mangled into the Rust module name
    // that we're generating. This in theory could look at all of
    // `versions_with_same_name` and produce a minimal diff, e.g. for 0.1.0
    // and 0.2.0 this could generate "foo1" and "foo2", but for now
    // a simpler path is chosen to generate "foo0_1_0" and "foo0_2_0".
    let version = version
        .to_string()
        .replace('.', "_")
        .replace('-', "_")
        .replace('+', "_")
        .to_snake_case();
    format!("{base}{version}")
}

impl WorldGenerator for RustWasm {
    fn preprocess(&mut self, resolve: &Resolve, world: WorldId) {
        wit_bindgen_core::generated_preamble(&mut self.src, env!("CARGO_PKG_VERSION"));

        // Render some generator options to assist with debugging and/or to help
        // recreate it if the original generation command is lost.
        uwriteln!(self.src, "// Options used:");
        if self.opts.std_feature {
            uwriteln!(self.src, "//   * std_feature");
        }
        if self.opts.raw_strings {
            uwriteln!(self.src, "//   * raw_strings");
        }
        if !self.opts.skip.is_empty() {
            uwriteln!(self.src, "//   * skip: {:?}", self.opts.skip);
        }
        if !matches!(self.opts.ownership, Ownership::Owning) {
            uwriteln!(self.src, "//   * ownership: {:?}", self.opts.ownership);
        }
        if !self.opts.additional_derive_attributes.is_empty() {
            uwriteln!(
                self.src,
                "//   * additional derives {:?}",
                self.opts.additional_derive_attributes
            );
        }
        if !self.opts.with.is_empty() {
            let mut with = self.opts.with.iter().collect::<Vec<_>>();
            with.sort();
            uwriteln!(self.src, "//   * with {with:?}");
        }
        self.types.analyze(resolve);
        self.world = Some(world);
    }

    fn import_interface(
        &mut self,
        resolve: &Resolve,
        name: &WorldKey,
        id: InterfaceId,
        _files: &mut Files,
    ) {
        self.interface_last_seen_as_import.insert(id, true);
        let wasm_import_module = resolve.name_world_key(name);
        let mut gen = self.interface(
            Identifier::Interface(id, name),
            Some(&wasm_import_module),
            resolve,
            true,
        );
        let (snake, module_path) = gen.start_append_submodule(name);
        if gen.gen.name_interface(resolve, id, name, false) {
            return;
        }
        gen.types(id);

        gen.generate_imports(resolve.interfaces[id].functions.values());

        gen.finish_append_submodule(&snake, module_path);
    }

    fn import_funcs(
        &mut self,
        resolve: &Resolve,
        world: WorldId,
        funcs: &[(&str, &Function)],
        _files: &mut Files,
    ) {
        self.import_funcs_called = true;

        let mut gen = self.interface(Identifier::World(world), Some("$root"), resolve, true);

        gen.generate_imports(funcs.iter().map(|(_, func)| *func));

        let src = gen.finish();
        self.src.push_str(&src);
    }

    fn export_interface(
        &mut self,
        resolve: &Resolve,
        name: &WorldKey,
        id: InterfaceId,
        _files: &mut Files,
    ) -> Result<()> {
        self.interface_last_seen_as_import.insert(id, false);
        let mut gen = self.interface(Identifier::Interface(id, name), None, resolve, false);
        let (snake, module_path) = gen.start_append_submodule(name);
        if gen.gen.name_interface(resolve, id, name, true) {
            return Ok(());
        }
        gen.types(id);
        gen.generate_exports(resolve.interfaces[id].functions.values())?;
        gen.finish_append_submodule(&snake, module_path);

        if self.opts.stubs {
            let (pkg, name) = match name {
                WorldKey::Name(name) => (None, name),
                WorldKey::Interface(id) => {
                    let interface = &resolve.interfaces[*id];
                    (
                        Some(interface.package.unwrap()),
                        interface.name.as_ref().unwrap(),
                    )
                }
            };
            for (resource, funcs) in group_by_resource(resolve.interfaces[id].functions.values()) {
                let world_id = self.world.unwrap();
                let mut gen = self.interface(Identifier::World(world_id), None, resolve, false);
                let pkg = pkg.map(|pid| {
                    let namespace = resolve.packages[pid].name.namespace.clone();
                    let package_module = name_package_module(resolve, pid);
                    (namespace, package_module)
                });
                gen.generate_stub(resource, pkg, name, true, &funcs);
                let stub = gen.finish();
                self.src.push_str(&stub);
            }
        }
        Ok(())
    }

    fn export_funcs(
        &mut self,
        resolve: &Resolve,
        world: WorldId,
        funcs: &[(&str, &Function)],
        _files: &mut Files,
    ) -> Result<()> {
        let mut gen = self.interface(Identifier::World(world), None, resolve, false);
        gen.generate_exports(funcs.iter().map(|f| f.1))?;
        let src = gen.finish();
        self.src.push_str(&src);

        if self.opts.stubs {
            for (resource, funcs) in group_by_resource(funcs.iter().map(|f| f.1)) {
                let mut gen = self.interface(Identifier::World(world), None, resolve, false);
                let world = &resolve.worlds[world];
                gen.generate_stub(resource, None, &world.name, false, &funcs);
                let stub = gen.finish();
                self.src.push_str(&stub);
            }
        }
        Ok(())
    }

    fn import_types(
        &mut self,
        resolve: &Resolve,
        world: WorldId,
        types: &[(&str, TypeId)],
        _files: &mut Files,
    ) {
        let mut gen = self.interface(Identifier::World(world), Some("$root"), resolve, true);
        for (name, ty) in types {
            gen.define_type(name, *ty);
        }
        let src = gen.finish();
        self.src.push_str(&src);
    }

    fn finish_imports(&mut self, resolve: &Resolve, world: WorldId, files: &mut Files) {
        if !self.import_funcs_called {
            // We call `import_funcs` even if the world doesn't import any
            // functions since one of the side effects of that method is to
            // generate `struct`s for any imported resources.
            self.import_funcs(resolve, world, &[], files);
        }
    }

    fn finish(&mut self, resolve: &Resolve, world: WorldId, files: &mut Files) -> Result<()> {
        let name = &resolve.worlds[world].name;

        let imports = mem::take(&mut self.import_modules);
        self.emit_modules(imports);
        let exports = mem::take(&mut self.export_modules);
        self.emit_modules(exports);

        self.finish_runtime_module();

        self.src.push_str("\n#[cfg(target_arch = \"wasm32\")]\n");

        // The custom section name here must start with "component-type" but
        // otherwise is attempted to be unique here to ensure that this doesn't get
        // concatenated to other custom sections by LLD by accident since LLD will
        // concatenate custom sections of the same name.
        let suffix = self.opts.type_section_suffix.as_deref().unwrap_or("");
        self.src.push_str(&format!(
            "#[link_section = \"component-type:{name}{suffix}\"]\n"
        ));

        let mut producers = wasm_metadata::Producers::empty();
        producers.add(
            "processed-by",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        );

        let component_type = wit_component::metadata::encode(
            resolve,
            world,
            wit_component::StringEncoding::UTF8,
            Some(&producers),
        )
        .unwrap();

        self.src.push_str("#[doc(hidden)]\n");
        self.src.push_str(&format!(
            "pub static __WIT_BINDGEN_COMPONENT_TYPE: [u8; {}] = *b\"\\\n",
            component_type.len()
        ));
        let mut line_length = 0;
        let s = self.src.as_mut_string();
        for byte in component_type.iter() {
            if line_length >= 80 {
                s.push_str("\\\n");
                line_length = 0;
            }
            match byte {
                b'\\' => {
                    s.push_str("\\\\");
                    line_length += 2;
                }
                b'"' => {
                    s.push_str("\\\"");
                    line_length += 2;
                }
                b if b.is_ascii_alphanumeric() || b.is_ascii_punctuation() => {
                    s.push(char::from(*byte));
                    line_length += 1;
                }
                0 => {
                    s.push_str("\\0");
                    line_length += 2;
                }
                _ => {
                    uwrite!(s, "\\x{:02x}", byte);
                    line_length += 4;
                }
            }
        }

        self.src.push_str("\";\n");

        let rt = self.runtime_path().to_string();
        uwriteln!(
            self.src,
            "
                #[inline(never)]
                #[doc(hidden)]
                #[cfg(target_arch = \"wasm32\")]
                pub fn __link_section() {{
                    {rt}::maybe_link_cabi_realloc();
                }}
            ",
        );

        if self.opts.stubs {
            self.src.push_str("\n#[derive(Debug)]\npub struct Stub;\n");
        }

        let mut src = mem::take(&mut self.src);
        if self.opts.rustfmt {
            let mut child = Command::new("rustfmt")
                .arg("--edition=2018")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .expect("failed to spawn `rustfmt`");
            child
                .stdin
                .take()
                .unwrap()
                .write_all(src.as_bytes())
                .unwrap();
            src.as_mut_string().truncate(0);
            child
                .stdout
                .take()
                .unwrap()
                .read_to_string(src.as_mut_string())
                .unwrap();
            let status = child.wait().unwrap();
            assert!(status.success());
        }

        let module_name = name.to_snake_case();
        files.push(&format!("{module_name}.rs"), src.as_bytes());

        let remapping_keys = self.opts.with.keys().cloned().collect::<HashSet<String>>();

        let mut unused_keys = remapping_keys
            .difference(&self.used_with_opts)
            .collect::<Vec<&String>>();

        unused_keys.sort();

        if !unused_keys.is_empty() {
            bail!("unused remappings provided via `with`: {unused_keys:?}");
        }

        Ok(())
    }
}

fn compute_module_path(name: &WorldKey, resolve: &Resolve, is_export: bool) -> Vec<String> {
    let mut path = Vec::new();
    if is_export {
        path.push("exports".to_string());
    }
    match name {
        WorldKey::Name(name) => {
            path.push(name.to_snake_case());
        }
        WorldKey::Interface(id) => {
            let iface = &resolve.interfaces[*id];
            let pkg = iface.package.unwrap();
            let pkgname = resolve.packages[pkg].name.clone();
            path.push(pkgname.namespace.to_snake_case());
            path.push(name_package_module(resolve, pkg));
            path.push(iface.name.as_ref().unwrap().to_snake_case());
        }
    }
    path
}

enum Identifier<'a> {
    World(WorldId),
    Interface(InterfaceId, &'a WorldKey),
}

fn group_by_resource<'a>(
    funcs: impl Iterator<Item = &'a Function>,
) -> BTreeMap<Option<TypeId>, Vec<&'a Function>> {
    let mut by_resource = BTreeMap::<_, Vec<_>>::new();
    for func in funcs {
        match &func.kind {
            FunctionKind::Freestanding => by_resource.entry(None).or_default().push(func),
            FunctionKind::Method(ty) | FunctionKind::Static(ty) | FunctionKind::Constructor(ty) => {
                by_resource.entry(Some(*ty)).or_default().push(func);
            }
        }
    }
    by_resource
}

#[derive(Default, Debug, Clone, Copy)]
pub enum Ownership {
    /// Generated types will be composed entirely of owning fields, regardless
    /// of whether they are used as parameters to imports or not.
    #[default]
    Owning,

    /// Generated types used as parameters to imports will be "deeply
    /// borrowing", i.e. contain references rather than owned values when
    /// applicable.
    Borrowing {
        /// Whether or not to generate "duplicate" type definitions for a single
        /// WIT type if necessary, for example if it's used as both an import
        /// and an export, or if it's used both as a parameter to an import and
        /// a return value from an import.
        duplicate_if_necessary: bool,
    },
}

impl FromStr for Ownership {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "owning" => Ok(Self::Owning),
            "borrowing" => Ok(Self::Borrowing {
                duplicate_if_necessary: false,
            }),
            "borrowing-duplicate-if-necessary" => Ok(Self::Borrowing {
                duplicate_if_necessary: true,
            }),
            _ => Err(format!(
                "unrecognized ownership: `{s}`; \
                 expected `owning`, `borrowing`, or `borrowing-duplicate-if-necessary`"
            )),
        }
    }
}

impl fmt::Display for Ownership {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Ownership::Owning => "owning",
            Ownership::Borrowing {
                duplicate_if_necessary: false,
            } => "borrowing",
            Ownership::Borrowing {
                duplicate_if_necessary: true,
            } => "borrowing-duplicate-if-necessary",
        })
    }
}

#[derive(Default)]
struct FnSig {
    async_: bool,
    unsafe_: bool,
    private: bool,
    use_item_name: bool,
    generics: Option<String>,
    self_arg: Option<String>,
    self_is_first_param: bool,
}

pub fn to_rust_ident(name: &str) -> String {
    match name {
        // Escape Rust keywords.
        // Source: https://doc.rust-lang.org/reference/keywords.html
        "as" => "as_".into(),
        "break" => "break_".into(),
        "const" => "const_".into(),
        "continue" => "continue_".into(),
        "crate" => "crate_".into(),
        "else" => "else_".into(),
        "enum" => "enum_".into(),
        "extern" => "extern_".into(),
        "false" => "false_".into(),
        "fn" => "fn_".into(),
        "for" => "for_".into(),
        "if" => "if_".into(),
        "impl" => "impl_".into(),
        "in" => "in_".into(),
        "let" => "let_".into(),
        "loop" => "loop_".into(),
        "match" => "match_".into(),
        "mod" => "mod_".into(),
        "move" => "move_".into(),
        "mut" => "mut_".into(),
        "pub" => "pub_".into(),
        "ref" => "ref_".into(),
        "return" => "return_".into(),
        "self" => "self_".into(),
        "static" => "static_".into(),
        "struct" => "struct_".into(),
        "super" => "super_".into(),
        "trait" => "trait_".into(),
        "true" => "true_".into(),
        "type" => "type_".into(),
        "unsafe" => "unsafe_".into(),
        "use" => "use_".into(),
        "where" => "where_".into(),
        "while" => "while_".into(),
        "async" => "async_".into(),
        "await" => "await_".into(),
        "dyn" => "dyn_".into(),
        "abstract" => "abstract_".into(),
        "become" => "become_".into(),
        "box" => "box_".into(),
        "do" => "do_".into(),
        "final" => "final_".into(),
        "macro" => "macro_".into(),
        "override" => "override_".into(),
        "priv" => "priv_".into(),
        "typeof" => "typeof_".into(),
        "unsized" => "unsized_".into(),
        "virtual" => "virtual_".into(),
        "yield" => "yield_".into(),
        "try" => "try_".into(),
        s => s.to_snake_case(),
    }
}

fn to_upper_camel_case(name: &str) -> String {
    match name {
        // The name "Guest" is reserved for traits generated by exported
        // interfaces, so remap types defined in wit to something else.
        "guest" => "Guest_".to_string(),
        s => s.to_upper_camel_case(),
    }
}

fn wasm_type(ty: WasmType) -> &'static str {
    match ty {
        WasmType::I32 => "i32",
        WasmType::I64 => "i64",
        WasmType::F32 => "f32",
        WasmType::F64 => "f64",
    }
}

fn int_repr(repr: Int) -> &'static str {
    match repr {
        Int::U8 => "u8",
        Int::U16 => "u16",
        Int::U32 => "u32",
        Int::U64 => "u64",
    }
}

fn bitcast(casts: &[Bitcast], operands: &[String], results: &mut Vec<String>) {
    for (cast, operand) in casts.iter().zip(operands) {
        results.push(match cast {
            Bitcast::None => operand.clone(),
            Bitcast::I32ToI64 => format!("i64::from({})", operand),
            Bitcast::F32ToI32 => format!("({}).to_bits() as i32", operand),
            Bitcast::F64ToI64 => format!("({}).to_bits() as i64", operand),
            Bitcast::I64ToI32 => format!("{} as i32", operand),
            Bitcast::I32ToF32 => format!("f32::from_bits({} as u32)", operand),
            Bitcast::I64ToF64 => format!("f64::from_bits({} as u64)", operand),
            Bitcast::F32ToI64 => format!("i64::from(({}).to_bits())", operand),
            Bitcast::I64ToF32 => format!("f32::from_bits({} as u32)", operand),
        });
    }
}

enum RustFlagsRepr {
    U8,
    U16,
    U32,
    U64,
    U128,
}

impl RustFlagsRepr {
    fn new(f: &Flags) -> RustFlagsRepr {
        match f.repr() {
            FlagsRepr::U8 => RustFlagsRepr::U8,
            FlagsRepr::U16 => RustFlagsRepr::U16,
            FlagsRepr::U32(1) => RustFlagsRepr::U32,
            FlagsRepr::U32(2) => RustFlagsRepr::U64,
            FlagsRepr::U32(3 | 4) => RustFlagsRepr::U128,
            FlagsRepr::U32(n) => panic!("unsupported number of flags: {}", n * 32),
        }
    }
}

impl fmt::Display for RustFlagsRepr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RustFlagsRepr::U8 => "u8".fmt(f),
            RustFlagsRepr::U16 => "u16".fmt(f),
            RustFlagsRepr::U32 => "u32".fmt(f),
            RustFlagsRepr::U64 => "u64".fmt(f),
            RustFlagsRepr::U128 => "u128".fmt(f),
        }
    }
}

#[derive(Debug)]
pub struct MissingExportsMap {
    key: String,
}

impl fmt::Display for MissingExportsMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "no `exports` map provided in configuration - provide an `exports` map a key `{key}`",
            key = self.key,
        )
    }
}

impl std::error::Error for MissingExportsMap {}
