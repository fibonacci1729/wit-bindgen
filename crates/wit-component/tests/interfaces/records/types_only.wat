(component
  (type (;0;) (tuple char u32))
  (type (;1;) (func (param "x" 0)))
  (type (;2;) (func (result 0)))
  (type (;3;) (record))
  (type (;4;) (func (param "x" 3)))
  (type (;5;) (func (result 3)))
  (type (;6;) (record (field "a" u32) (field "b" u32)))
  (type (;7;) (func (param "x" 6)))
  (type (;8;) (func (result 6)))
  (type (;9;) (record (field "a" bool) (field "b" bool) (field "c" bool) (field "d" bool) (field "e" bool) (field "f" bool) (field "g" bool) (field "h" bool) (field "i" bool)))
  (type (;10;) (func (param "x" 9)))
  (type (;11;) (func (result 9)))
  (type (;12;) (record (field "a" 6) (field "b" u32) (field "c" 3) (field "d" string) (field "e" 9)))
  (type (;13;) (func (param "x" 12)))
  (type (;14;) (func (result 12)))
  (type (;15;) s32)
  (type (;16;) (tuple 15))
  (type (;17;) (func (param "e" 16) (result s32)))
  (type (;18;) (flags "a" "b" "c" "d" "e" "f" "g" "h" "i"))
  (type (;19;) (record (field "a" 6) (field "b" u32) (field "c" 3) (field "d" string) (field "e" 18)))
  (type (;20;) (func (param "x" 18)))
  (type (;21;) (func (result 18)))
  (type (;22;) (func (param "x" 19)))
  (type (;23;) (func (result 19)))
  (type (;24;) 
    (instance
      (alias outer 1 3 (type (;0;)))
      (export "empty"  (type (eq 0)))
      (alias outer 1 6 (type (;1;)))
      (export "scalars"  (type (eq 1)))
      (alias outer 1 18 (type (;2;)))
      (export "really-flags"  (type (eq 2)))
      (alias outer 1 19 (type (;3;)))
      (export "aggregates"  (type (eq 3)))
      (alias outer 1 15 (type (;4;)))
      (export "int-typedef"  (type (eq 4)))
      (alias outer 1 16 (type (;5;)))
      (export "tuple-typedef2"  (type (eq 5)))
      (alias outer 1 1 (type (;6;)))
      (export "tuple-arg" (func (type 6)))
      (alias outer 1 2 (type (;7;)))
      (export "tuple-result" (func (type 7)))
      (alias outer 1 4 (type (;8;)))
      (export "empty-arg" (func (type 8)))
      (alias outer 1 5 (type (;9;)))
      (export "empty-result" (func (type 9)))
      (alias outer 1 7 (type (;10;)))
      (export "scalar-arg" (func (type 10)))
      (alias outer 1 8 (type (;11;)))
      (export "scalar-result" (func (type 11)))
      (alias outer 1 20 (type (;12;)))
      (export "flags-arg" (func (type 12)))
      (alias outer 1 21 (type (;13;)))
      (export "flags-result" (func (type 13)))
      (alias outer 1 22 (type (;14;)))
      (export "aggregate-arg" (func (type 14)))
      (alias outer 1 23 (type (;15;)))
      (export "aggregate-result" (func (type 15)))
      (alias outer 1 17 (type (;16;)))
      (export "typedef-inout" (func (type 16)))
    )
  )
  (type (;25;) (tuple s32))
  (type (;26;) 
    (instance
      (alias outer 1 3 (type (;0;)))
      (export "empty"  (type (eq 0)))
      (alias outer 1 6 (type (;1;)))
      (export "scalars"  (type (eq 1)))
      (alias outer 1 9 (type (;2;)))
      (export "really-flags"  (type (eq 2)))
      (alias outer 1 12 (type (;3;)))
      (export "aggregates"  (type (eq 3)))
      (alias outer 1 25 (type (;4;)))
      (export "tuple-typedef"  (type (eq 4)))
      (alias outer 1 15 (type (;5;)))
      (export "int-typedef"  (type (eq 5)))
      (alias outer 1 16 (type (;6;)))
      (export "tuple-typedef2"  (type (eq 6)))
      (alias outer 1 1 (type (;7;)))
      (export "tuple-arg" (func (type 7)))
      (alias outer 1 2 (type (;8;)))
      (export "tuple-result" (func (type 8)))
      (alias outer 1 4 (type (;9;)))
      (export "empty-arg" (func (type 9)))
      (alias outer 1 5 (type (;10;)))
      (export "empty-result" (func (type 10)))
      (alias outer 1 7 (type (;11;)))
      (export "scalar-arg" (func (type 11)))
      (alias outer 1 8 (type (;12;)))
      (export "scalar-result" (func (type 12)))
      (alias outer 1 10 (type (;13;)))
      (export "flags-arg" (func (type 13)))
      (alias outer 1 11 (type (;14;)))
      (export "flags-result" (func (type 14)))
      (alias outer 1 13 (type (;15;)))
      (export "aggregate-arg" (func (type 15)))
      (alias outer 1 14 (type (;16;)))
      (export "aggregate-result" (func (type 16)))
      (alias outer 1 17 (type (;17;)))
      (export "typedef-inout" (func (type 17)))
    )
  )
  (import "records" (instance (;0;) (type 24)))
  (export "records" (type 26))
)