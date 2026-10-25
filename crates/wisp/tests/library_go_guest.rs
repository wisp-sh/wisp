#[allow(dead_code)]
#[path = "../src/library_go.rs"]
mod library_go;
#[allow(dead_code)]
#[path = "../src/library_metadata.rs"]
mod library_metadata;
#[allow(dead_code)]
#[path = "../src/library_wasi.rs"]
mod library_wasi;
#[allow(dead_code)]
#[path = "../src/wit_model.rs"]
mod wit_model;

use std::{collections::BTreeMap, fs};

fn component() -> Vec<u8> {
    use wasm_encoder::*;
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types
        .ty()
        .function([ValType::I32, ValType::I32], [ValType::I32]);
    module.section(&types);
    let mut functions = FunctionSection::new();
    functions.function(0);
    module.section(&functions);
    let mut exports = ExportSection::new();
    exports.export("test:published/arithmetic@0.1.0#add", ExportKind::Func, 0);
    module.section(&exports);
    let mut code = CodeSection::new();
    let mut function = Function::new([]);
    for instruction in [
        Instruction::LocalGet(0),
        Instruction::LocalGet(1),
        Instruction::I32Add,
        Instruction::End,
    ] {
        function.instruction(&instruction);
    }
    code.function(&function);
    module.section(&code);
    let mut bytes = module.finish();
    let mut resolve = wit_parser::Resolve::default();
    let package=resolve.push_str("world.wit","package test:published@0.1.0; interface arithmetic { add: func(a:u32,b:u32)->u32; } world library { export arithmetic; }").unwrap();
    let world = resolve.select_world(&[package], Some("library")).unwrap();
    wit_component::embed_component_metadata(
        &mut bytes,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .unwrap();
    wit_component::ComponentEncoder::default()
        .validate(true)
        .module(&bytes)
        .unwrap()
        .encode()
        .unwrap()
}

fn pinned_global_component() -> Vec<u8> {
    use wasm_encoder::*;
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types
        .ty()
        .function([ValType::I32, ValType::I32], [ValType::I32]);
    types.ty().function([], []);
    module.section(&types);
    let mut functions = FunctionSection::new();
    functions.function(0).function(1);
    module.section(&functions);
    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    module.section(&memories);
    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(1024),
    );
    module.section(&globals);
    let mut exports = ExportSection::new();
    exports.export("test:published/arithmetic@0.1.0#add", ExportKind::Func, 0);
    module.section(&exports);
    let mut code = CodeSection::new();
    let mut add = Function::new([]);
    let memory = MemArg {
        offset: 8,
        align: 2,
        memory_index: 0,
    };
    for instruction in [
        Instruction::GlobalGet(0),
        Instruction::LocalGet(0),
        Instruction::LocalGet(1),
        Instruction::I32Add,
        Instruction::I32Store(memory),
        Instruction::GlobalGet(0),
        Instruction::I32Load(memory),
        Instruction::GlobalGet(0),
        Instruction::I32Const(16),
        Instruction::I32Add,
        Instruction::GlobalSet(0),
        Instruction::Return,
        Instruction::Unreachable,
        Instruction::End,
    ] {
        add.instruction(&instruction);
    }
    code.function(&add);
    // Loop-weighted reads make Wago pin the mutable global across the module.
    // This function is never called; the regression itself needs one load.
    let mut hot = Function::new([]);
    hot.instruction(&Instruction::Loop(BlockType::Empty));
    for _ in 0..64 {
        hot.instruction(&Instruction::GlobalGet(0));
        hot.instruction(&Instruction::Drop);
    }
    hot.instruction(&Instruction::End);
    hot.instruction(&Instruction::End);
    code.function(&hot);
    module.section(&code);
    let mut bytes = module.finish();
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve.push_str("world.wit", "package test:published@0.1.0; interface arithmetic { add: func(a:u32,b:u32)->u32; } world library { export arithmetic; }").unwrap();
    let world = resolve.select_world(&[package], Some("library")).unwrap();
    wit_component::embed_component_metadata(
        &mut bytes,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .unwrap();
    wit_component::ComponentEncoder::default()
        .validate(true)
        .module(&bytes)
        .unwrap()
        .encode()
        .unwrap()
}

#[test]
fn portable_runtime_preserves_load_results_across_pinned_global_writes() {
    use std::process::Command;
    let bytes = pinned_global_component();
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("library");
    let module = "example.org/pinned-global";
    library_go::portable::generate(&bytes, &package, module, "0.1.0", &BTreeMap::new()).unwrap();
    assert_eq!(fs::read(package.join("component.wasm")).unwrap(), bytes);
    let native = package.join("component_native.wasm");
    let adapted = fs::read(&native).unwrap();
    assert_ne!(adapted, bytes);
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&adapted)
        .unwrap();
    let archive = library_go::module_archive(&package, module, "0.1.0").unwrap();
    library_go::portable::verify_archive(&bytes, module, "0.1.0", &archive.zip, &BTreeMap::new())
        .unwrap();
    fs::write(&native, &bytes).unwrap();
    let forged = library_go::module_archive(&package, module, "0.1.0").unwrap();
    assert!(
        library_go::portable::verify_archive(
            &bytes,
            module,
            "0.1.0",
            &forged.zip,
            &BTreeMap::new()
        )
        .is_err()
    );
    fs::write(&native, adapted).unwrap();
    fs::write(package.join("runtime_test.go"), r#"package component
import "testing"
func TestPinnedGlobal(t *testing.T) {
    for _, pair := range [][2]uint32{{20,1}, {1,20}, {0,42}, {42,0}, {0xffffffff,2}} {
        if got := ArithmeticAdd(pair[0],pair[1]); got != pair[0]+pair[1] { t.Fatalf("Add(%d,%d) = %d",pair[0],pair[1],got) }
    }
}
"#).unwrap();
    run(Command::new("go")
        .args(["test", "-mod=mod", "."])
        .env("CGO_ENABLED", "0")
        .current_dir(&package));
}
#[test]
fn guest_module_carries_typed_bindings_component_and_canonical_proxy_archive() {
    let component = component();
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("package");
    let notices = BTreeMap::from([("WISP-NOTICES.txt".into(), b"test notice\n".to_vec())]);
    library_go::guest::generate(
        &component,
        &output,
        "example.org/arithmetic",
        "0.1.0",
        &notices,
    )
    .unwrap();
    let bindings = fs::read_to_string(output.join("bindings_wasip1.go")).unwrap();
    assert!(bindings.contains("//go:build wasip1 && wasm"));
    assert!(bindings.contains("var ArithmeticAdd = wisp0.Add"));
    let upstream =
        fs::read_to_string(output.join("test_published_arithmetic/wit_bindings.go")).unwrap();
    assert!(upstream.contains("//go:wasmimport test:published/arithmetic@0.1.0 add"));
    assert!(upstream.contains("Generated by `wit-bindgen` 0.61.1"));
    assert!(!bindings.contains("import \"C\""));
    let archive = library_go::module_archive(&output, "example.org/arithmetic", "0.1.0").unwrap();
    let report = library_go::guest::verify_archive(
        &component,
        "example.org/arithmetic",
        "0.1.0",
        &archive.zip,
        &notices,
    )
    .unwrap();
    assert_eq!(report["platform"], "wasip1_wasm");
    assert_eq!(fs::read(output.join("component.wasm")).unwrap(), component);
    library_go::guest::vendored(&output, "example.org/arithmetic").unwrap();
}
#[test]
fn guest_admission_rejects_rehashed_bindings_and_vendored_substitution() {
    let component = component();
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("package");
    library_go::guest::generate(
        &component,
        &output,
        "example.org/arithmetic",
        "0.1.0",
        &BTreeMap::new(),
    )
    .unwrap();
    let path = output.join("bindings_wasip1.go");
    let source = fs::read_to_string(&path).unwrap().replace(
        "var ArithmeticAdd = wisp0.Add",
        "var ArithmeticAdd = func(a, b uint32) uint32 { return 99 }",
    );
    fs::write(path, source).unwrap();
    let archive = library_go::module_archive(&output, "example.org/arithmetic", "0.1.0").unwrap();
    assert!(
        library_go::guest::verify_archive(
            &component,
            "example.org/arithmetic",
            "0.1.0",
            &archive.zip,
            &BTreeMap::new()
        )
        .is_err()
    );
    assert!(library_go::guest::vendored(&output, "example.org/arithmetic").is_err());
}

fn run(command: &mut std::process::Command) -> String {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn portable_go_import_runs_natively_and_compiles_to_wasm_with_the_same_api() {
    use std::process::Command;
    let component = component();
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("package");
    let module = format!(
        "example.org/wisp-portable-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let module = module.as_str();
    library_go::portable::generate(&component, &package, module, "0.1.0", &BTreeMap::new())
        .unwrap();
    let descriptor: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("wisp-component.json")).unwrap()).unwrap();
    assert_eq!(descriptor["runtime"]["module"], "github.com/wago-org/wago");
    assert_eq!(
        descriptor["runtime"]["version"],
        library_go::portable::RUNTIME_VERSION
    );
    let manifest = fs::read_to_string(package.join("go.mod")).unwrap();
    assert!(manifest.contains(&format!(
        "github.com/wago-org/wago {}",
        library_go::portable::RUNTIME_VERSION
    )));
    assert!(!manifest.contains("replace "));
    assert!(!package.join("worker").exists());
    assert!(!package.join("native").exists());
    assert!(!package.join("component_native.wasm").exists());
    let artifacts = library_go::module_archive(&package, module, "0.1.0").unwrap();
    library_go::portable::verify_archive(
        &component,
        module,
        "0.1.0",
        &artifacts.zip,
        &BTreeMap::new(),
    )
    .unwrap();
    let consumer = temp.path().join("consumer");
    fs::create_dir(&consumer).unwrap();
    let proxy = temp.path().join("proxy");
    let versions = proxy.join(module).join("@v");
    fs::create_dir_all(&versions).unwrap();
    fs::write(versions.join("v0.1.0.zip"), &artifacts.zip).unwrap();
    fs::write(versions.join("v0.1.0.mod"), &artifacts.mod_file).unwrap();
    fs::write(versions.join("v0.1.0.info"), &artifacts.info).unwrap();
    fs::write(
        consumer.join("go.mod"),
        format!("module example.org/consumer\n\ngo 1.27.0\n\nrequire {module} v0.1.0\n"),
    )
    .unwrap();
    fs::write(
        consumer.join("main.go"),
        r#"package main
import arithmetic "@MODULE@"
import "fmt"
func main() {
    if arithmetic.ArithmeticAdd(^uint32(0), 2) != 1 { panic("integer overflow") }
    fmt.Println(arithmetic.ArithmeticAdd(20, 22))
}
"#
        .replace("@MODULE@", module),
    )
    .unwrap();
    assert_eq!(
        run(Command::new("go")
            .args(["run", "-mod=mod", "."])
            .env("CGO_ENABLED", "0")
            .env(
                "GOPROXY",
                format!("file://{},https://proxy.golang.org", proxy.display())
            )
            .env("GOSUMDB", "off")
            .current_dir(&consumer))
        .trim(),
        "42"
    );
    run(Command::new("go")
        .args(["mod", "vendor"])
        .env("GOOS", "wasip1")
        .env("GOARCH", "wasm")
        .env("CGO_ENABLED", "0")
        .current_dir(&consumer));
    library_go::guest::vendored(&consumer.join("vendor").join(module), module).unwrap();
    fs::remove_dir_all(proxy).unwrap();
    assert_eq!(
        run(Command::new("go")
            .args(["run", "-mod=vendor", "."])
            .env("CGO_ENABLED", "0")
            .env("GOPROXY", "off")
            .env("GOSUMDB", "off")
            .current_dir(&consumer))
        .trim(),
        "42"
    );
    let wasm = consumer.join("consumer.wasm");
    run(Command::new("go")
        .args(["build", "-mod=vendor", "-o"])
        .arg(&wasm)
        .arg(".")
        .env("GOOS", "wasip1")
        .env("GOARCH", "wasm")
        .env("CGO_ENABLED", "0")
        .env("GOPROXY", "off")
        .current_dir(&consumer));
    let bytes = fs::read(wasm).unwrap();
    let mut found_import = false;
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let wasmparser::Payload::ImportSection(imports) = payload.unwrap() {
            for import in imports.into_imports() {
                let import = import.unwrap();
                assert!(!import.module.contains("wasmtime"));
                assert!(!import.module.contains("wago"));
                found_import |=
                    import.module == "test:published/arithmetic@0.1.0" && import.name == "add";
            }
        }
    }
    assert!(found_import, "Wasm must call its Component import");
    let path = package.join("bindings_native.go");
    fs::write(
        path,
        "package component\nfunc ArithmeticAdd(a, b uint32) uint32 { return 99 }\n",
    )
    .unwrap();
    let changed = library_go::module_archive(&package, module, "0.1.0").unwrap();
    assert!(
        library_go::portable::verify_archive(
            &component,
            module,
            "0.1.0",
            &changed.zip,
            &BTreeMap::new()
        )
        .is_err()
    );
}

fn scalar_component() -> Vec<u8> {
    use wasm_encoder::*;
    let cases = [
        ("bool", "bool", ValType::I32),
        ("u8", "u8", ValType::I32),
        ("u16", "u16", ValType::I32),
        ("u32", "u32", ValType::I32),
        ("u64", "u64", ValType::I64),
        ("s8", "s8", ValType::I32),
        ("s16", "s16", ValType::I32),
        ("s32", "s32", ValType::I32),
        ("s64", "s64", ValType::I64),
        ("f32", "f32", ValType::F32),
        ("f64", "f64", ValType::F64),
        ("alias", "number", ValType::I64),
    ];
    let mut module = Module::new();
    let mut types = TypeSection::new();
    let mut functions = FunctionSection::new();
    let mut exports = ExportSection::new();
    let mut code = CodeSection::new();
    let mut wit =
        String::from("package test:scalars@0.1.0; interface values { type number = u64; ");
    for (index, (name, ty, core)) in cases.iter().enumerate() {
        types.ty().function([*core], [*core]);
        functions.function(index as u32);
        exports.export(
            &format!("test:scalars/values@0.1.0#echo-{name}"),
            ExportKind::Func,
            index as u32,
        );
        let mut f = Function::new([]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::End);
        code.function(&f);
        wit.push_str(&format!("echo-{name}: func(value: {ty}) -> {ty}; "));
    }
    types.ty().function([], [ValType::I32]);
    types.ty().function([], []);
    let index = cases.len() as u32;
    for (offset, (interface, name, body)) in [
        (
            "values",
            "increment",
            vec![
                Instruction::GlobalGet(0),
                Instruction::I32Const(1),
                Instruction::I32Add,
                Instruction::GlobalSet(0),
                Instruction::GlobalGet(0),
            ],
        ),
        ("views", "current", vec![Instruction::GlobalGet(0)]),
        ("values", "fail", vec![Instruction::Unreachable]),
        ("values", "noop", vec![]),
    ]
    .into_iter()
    .enumerate()
    {
        functions.function(index + u32::from(name == "noop"));
        exports.export(
            &format!("test:scalars/{interface}@0.1.0#{name}"),
            ExportKind::Func,
            index + offset as u32,
        );
        let mut f = Function::new([]);
        for instruction in body {
            f.instruction(&instruction);
        }
        f.instruction(&Instruction::End);
        code.function(&f);
    }
    exports.export("root-current", ExportKind::Func, index + 1);
    wit.push_str("increment: func() -> u32; fail: func() -> u32; noop: func(); } interface views { current: func() -> u32; } world library { export values; export views; export root-current: func() -> u32; }");
    module.section(&types);
    module.section(&functions);
    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(0),
    );
    module.section(&globals);
    module.section(&exports);
    module.section(&code);
    let mut bytes = module.finish();
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve.push_str("world.wit", &wit).unwrap();
    let world = resolve.select_world(&[package], Some("library")).unwrap();
    wit_component::embed_component_metadata(
        &mut bytes,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .unwrap();
    wit_component::ComponentEncoder::default()
        .validate(true)
        .module(&bytes)
        .unwrap()
        .encode()
        .unwrap()
}

#[test]
fn portable_runtime_preserves_scalar_values_state_concurrency_and_traps() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("library");
    library_go::portable::generate(
        &scalar_component(),
        &package,
        "example.org/scalars",
        "0.1.0",
        &BTreeMap::new(),
    )
    .unwrap();
    let source = r#"package component
import ("math"; "sync"; "testing"; "fmt"; "strings")
func TestScalars(t *testing.T) {
    if !ValuesEchoBool(true) || ValuesEchoBool(false) { t.Fatal("bool") }
    if ValuesEchoU8(255) != 255 || ValuesEchoU16(65535) != 65535 { t.Fatal("narrow unsigned") }
    if ValuesEchoU32(^uint32(0)) != ^uint32(0) || ValuesEchoU64(^uint64(0)) != ^uint64(0) { t.Fatal("unsigned") }
    if ValuesEchoS8(-128) != -128 || ValuesEchoS16(-32768) != -32768 { t.Fatal("narrow signed") }
    if ValuesEchoS32(-2147483648) != -2147483648 || ValuesEchoS64(-9223372036854775808) != -9223372036854775808 { t.Fatal("signed") }
    if ValuesEchoAlias(^uint64(0)) != ^uint64(0) { t.Fatal("alias") }
    if ValuesEchoF32(-1.25) != -1.25 || ValuesEchoF64(-1.25) != -1.25 { t.Fatal("floats") }
    if !math.IsNaN(float64(ValuesEchoF32(float32(math.NaN())))) || !math.IsNaN(ValuesEchoF64(math.NaN())) { t.Fatal("NaN") }
    if !math.IsInf(ValuesEchoF64(math.Inf(-1)), -1) || !math.Signbit(ValuesEchoF64(math.Copysign(0, -1))) { t.Fatal("float bits") }
    ValuesNoop()
    var workers sync.WaitGroup
    for range 8 { workers.Add(1); go func() { defer workers.Done(); for range 100 { ValuesIncrement() } }() }
    workers.Wait()
    if ViewsCurrent() != 800 || RootCurrent() != 800 { t.Fatal("shared instance") }
    trap := func(call func()) { t.Helper(); defer func() { if value := recover(); value == nil || !strings.Contains(fmt.Sprint(value), "unreachable") { t.Errorf("unexpected trap: %v", value) } }(); call() }
    trap(func() { ValuesFail() })
    trap(func() { ViewsCurrent() })
}
"#;
    fs::write(package.join("runtime_test.go"), source).unwrap();
    run(Command::new("go")
        .args(["test", "-mod=mod", "-race", "."])
        .current_dir(&package));
}

#[test]
fn portable_runtime_executes_compiler_generated_components_without_cgo() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (name, fixture) in [
        (
            "cpp",
            "examples/go-import-wasm/go/vendor/example.org/cpp-arithmetic/component.wasm",
        ),
        (
            "rust",
            "examples/cpp-import-rust/deps/arithmetic/component.wasm",
        ),
        (
            "go",
            "examples/cpp-import-go/deps/arithmetic/component.wasm",
        ),
    ] {
        let package = temp.path().join(name);
        library_go::portable::generate(
            &fs::read(root.join(fixture)).unwrap(),
            &package,
            &format!("example.org/{name}-compiled"),
            "0.1.0",
            &BTreeMap::new(),
        )
        .unwrap();
        fs::write(package.join("runtime_test.go"), r#"package component
import "testing"
func TestCompiled(t *testing.T) {
    for _, pair := range [][2]uint32{{20,1}, {1,20}, {0,42}, {42,0}, {0xffffffff,2}} {
        if got := Add(pair[0],pair[1]); got != pair[0]+pair[1] { t.Fatalf("Add(%d,%d) = %d",pair[0],pair[1],got) }
    }
}
"#).unwrap();
        run(Command::new("go")
            .args(["test", "-mod=mod", "."])
            .env("CGO_ENABLED", "0")
            .current_dir(&package));
    }
}
