(module
  (import "[export]example:resources/resources@0.1.0" "[resource-new]counter"
    (func $resource-new (param i32) (result i32)))
  (import "[export]example:resources/resources@0.1.0" "[resource-rep]counter"
    (func $resource-rep (param i32) (result i32)))
  (import "[export]example:resources/resources@0.1.0" "[resource-drop]counter"
    (func $resource-drop (param i32)))
  (memory (export "memory") 1)
  (global $drops (mut i32) (i32.const 0))
  (global $malformed (mut i32) (i32.const 0))
  (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32) (i32.const 1024))
  (func (export "example:resources/resources@0.1.0#[constructor]counter") (param i32) (result i32)
    (global.set $malformed (i32.eq (local.get 0) (i32.const -1)))
    (call $resource-new (local.get 0)))
  (func (export "example:resources/resources@0.1.0#[method]counter.get") (param i32) (result i32)
    (i32.store (i32.const 0) (i32.const 0))
    (if (global.get $malformed)
      (then
        (i32.store (i32.const 4) (i32.const 65532))
        (i32.store (i32.const 8) (i32.const 1)))
      (else
        (i32.store (i32.const 4) (i32.const 64))
        (i32.store (i32.const 8) (i32.const 0))))
    (i32.const 0))
  (func (export "example:resources/resources@0.1.0#[static]counter.merge") (param i32 i32) (result i32)
    (global.set $drops (i32.add (global.get $drops) (i32.const 1)))
    (i32.store (i32.const 16) (i32.const 0))
    (i32.store (i32.const 20) (i32.const 64))
    (i32.store (i32.const 24) (i32.const 0))
    (i32.const 16))
  (func (export "example:resources/resources@0.1.0#[dtor]counter") (param i32)
    (global.set $drops (i32.add (global.get $drops) (i32.const 1))))
)