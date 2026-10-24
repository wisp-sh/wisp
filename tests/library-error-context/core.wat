(module
  (type $new (func (param i32 i32) (result i32)))
  (type $debug (func (param i32 i32)))
  (type $drop (func (param i32)))
  (import "$root" "[error-context-new-utf8]" (func $new (type $new)))
  (import "$root" "[error-context-debug-message-utf8]" (func $debug (type $debug)))
  (import "$root" "[error-context-drop]" (func $drop (type $drop)))
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 32))
  (data (i32.const 0) "fixture error")
  (func $realloc (export "cabi_realloc") (param $old i32) (param $old_size i32) (param $align i32) (param $new_size i32) (result i32)
    (local $p i32)
    global.get $heap
    local.get $align
    i32.const 1
    i32.sub
    i32.add
    local.get $align
    i32.const 1
    i32.sub
    i32.const -1
    i32.xor
    i32.and
    local.tee $p
    local.get $new_size
    i32.add
    global.set $heap
    local.get $p)
  (func (export "wisp:context-values/errors@0.1.0#create") (result i32)
    i32.const 0 i32.const 13 call $new)
  (func (export "wisp:context-values/errors@0.1.0#describe") (param $value i32) (result i32)
    (local $retptr i32)
    i32.const 0 i32.const 0 i32.const 4 i32.const 8 call $realloc local.set $retptr
    local.get $value local.get $retptr call $debug
    local.get $value call $drop
    local.get $retptr))
