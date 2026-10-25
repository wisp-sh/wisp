use std::{fs, path::Path, process::Command};
fn fixture(config: &str, wit: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("wit")).unwrap();
    fs::write(root.path().join("wisp.toml"), config).unwrap();
    fs::write(root.path().join("wit/world.wit"), wit).unwrap();
    fs::write(
        root.path().join("main.cpp"),
        "unsigned unused() { return 0; }\n",
    )
    .unwrap();
    root
}
fn reject(project: &Path, expected: &str) {
    let out = project.join("release");
    let result = Command::new(env!("CARGO_BIN_EXE_wisp"))
        .arg("build")
        .arg(project)
        .arg("--out")
        .arg(&out)
        .env("WISP_WASMTIME", project.join("missing-execution-host"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains(expected),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        !out.exists(),
        "failed builds must not publish partial output"
    );
}
const CONFIG: &str = "[package]\nname='test:build'\nversion='0.1.0'\n[build]\nroot='main'\n[build.units.main]\ncompiler='clang'\nworld='library'\nsources=['main.cpp']\n";
const WIT: &str = "package test:build@0.1.0; world library { export answer: func() -> u32; }";
#[test]
fn adapter_accepts_a_wit_file_and_top_level_world_before_tool_resolution() {
    let root = fixture(
        "[package]\nname='test:build'\nversion='0.1.0'\n[build]\ncompiler='zig'\nworld='library'\nwit='wit/world.wit'\nsources=['main.zig']\n",
        WIT,
    );
    fs::write(
        root.path().join("main.zig"),
        "export fn answer() u32 { return 42; }\n",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_wisp"))
        .args(["build", root.path().to_str().unwrap(), "--component-only"])
        .env_remove("WISP_ZIG_TOOLCHAIN")
        .env("WISP_TOOLCHAIN_HOME", root.path().join("toolchains"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("install the zig toolchain"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!root.path().join("dist").exists());
}
#[test]
fn accepts_wit_lists_before_resolving_the_execution_host() {
    let root = fixture(
        CONFIG,
        "package test:build@0.1.0; world library { export answer: func() -> list<u16>; }",
    );
    reject(root.path(), "set WISP_WASMTIME");
}
#[test]
fn rejects_build_cycles_before_running_compilers() {
    let config = format!(
        "{CONFIG}\n[build.units.main.dependencies]\nhelper='helper'\n[build.units.helper]\ncompiler='clang'\nworld='library'\nsources=['main.cpp']\n[build.units.helper.dependencies]\nmain='main'\n"
    );
    let root = fixture(&config, WIT);
    reject(root.path(), "build dependency cycle");
}
#[test]
fn refuses_undeclared_component_dependencies() {
    let root = fixture(
        CONFIG,
        "package test:build@0.1.0; interface arithmetic { add: func(a:u32,b:u32)->u32; } world library { import arithmetic; export answer: func()->u32; }",
    );
    reject(root.path(), "has no dependency for WIT import");
}
#[test]
fn preserves_existing_output_directory() {
    let root = fixture(CONFIG, WIT);
    let out = root.path().join("release");
    fs::create_dir(&out).unwrap();
    fs::write(out.join("keep"), "unchanged").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_wisp"))
        .arg("build")
        .arg(root.path())
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("output already exists"));
    assert_eq!(fs::read_to_string(out.join("keep")).unwrap(), "unchanged");
}
#[test]
fn rejects_unknown_config_and_wit_identity_mismatch() {
    let root = fixture(
        &CONFIG.replace("compiler='clang'", "compiler='clang'\ncommand='sh'"),
        WIT,
    );
    reject(root.path(), "unknown field");
    let root = fixture(CONFIG, &WIT.replace("test:build", "test:other"));
    reject(root.path(), "build WIT identity mismatch");
}

#[test]
fn rust_requires_a_wisp_execution_host_before_loading_its_package() {
    let root = fixture(
        "[package]\nname='test:build'\nversion='0.1.0'\n[build]\ncompiler='rust'\nproject='rust'\n[build.tools]\nrust='compiler'\n",
        WIT,
    );
    fs::create_dir(root.path().join("rust")).unwrap();
    fs::write(
        root.path().join("rust/Cargo.toml"),
        "[package]\nname='application'\nversion='0.1.0'\n[lib]\npath='lib.rs'\n",
    )
    .unwrap();
    fs::write(
        root.path().join("rust/lib.rs"),
        "pub fn answer() -> u32 { 42 }\n",
    )
    .unwrap();
    // Even a former runtime-shaped path in the compiler package is irrelevant.
    let former = root.path().join("compiler/build/bootstrap/runtime");
    fs::create_dir_all(&former).unwrap();
    fs::write(
        root.path().join("compiler/toolchain.json"),
        r#"{"schemaVersion":1,"compilerHost":"wasm32-wasip3","applicationTarget":"wasm32-wasip1"}"#,
    )
    .unwrap();
    fs::write(
        former.join("wasmtime"),
        b"must not execute a compiler-owned host",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_wisp"))
        .arg("build")
        .arg(root.path())
        .arg("--component-only")
        .env_remove("WISP_RUST")
        .env(
            "WISP_RUST_WASMTIME",
            root.path().join("missing-execution-host"),
        )
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("set WISP_RUST_WASMTIME"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!root.path().join("dist").exists());
}

#[test]
fn rejects_source_escape_and_output_cache_overlap_before_running_compilers() {
    let root = fixture(&CONFIG.replace("main.cpp", "../outside.cpp"), WIT);
    reject(root.path(), "cannot contain '..'");
    let root = fixture(CONFIG, WIT);
    let out = root.path().join("release");
    let result = Command::new(env!("CARGO_BIN_EXE_wisp"))
        .arg("build")
        .arg(root.path())
        .arg("--out")
        .arg(&out)
        .arg("--cache")
        .arg(out.join("cache"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("must be separate directories"));
    assert!(!out.exists());
}

fn component_bytes() -> Vec<u8> {
    use wasm_encoder::{
        CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction, Module,
        TypeSection, ValType,
    };
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([], [ValType::I32]);
    module.section(&types);
    let mut functions = FunctionSection::new();
    functions.function(0);
    module.section(&functions);
    let mut exports = ExportSection::new();
    exports.export("answer", ExportKind::Func, 0);
    module.section(&exports);
    let mut code = CodeSection::new();
    let mut function = Function::new([]);
    function.instruction(&Instruction::I32Const(42));
    function.instruction(&Instruction::End);
    code.function(&function);
    module.section(&code);
    let mut bytes = module.finish();
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve.push_str("world.wit", WIT).unwrap();
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
fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
fn prebuilt_config(bytes: &[u8]) -> String {
    CONFIG
        .replace(
            "compiler='clang'",
            &format!(
                "component={{path='dependency.wasm',sha256='{}'}}",
                sha(bytes)
            ),
        )
        .replace("sources=['main.cpp']\n", "")
}
#[test]
fn imports_prebuilt_component_without_loading_a_compiler() {
    let bytes = component_bytes();
    let root = fixture(&prebuilt_config(&bytes), WIT);
    fs::write(root.path().join("dependency.wasm"), &bytes).unwrap();
    let out = root.path().join("release");
    let result = Command::new(env!("CARGO_BIN_EXE_wisp"))
        .arg("build")
        .arg(root.path())
        .arg("--component-only")
        .arg("--out")
        .arg(&out)
        .env("WISP_WASMTIME", "/compiler-must-not-be-loaded/wasmtime")
        .env("WISP_CLANG", "/compiler-must-not-be-loaded/clang")
        .env("WISP_GO_TOOLCHAIN", "/compiler-must-not-be-loaded/go")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read(out.join("component.wasm")).unwrap(), bytes);
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("build/build.json")).unwrap()).unwrap();
    assert_eq!(receipt["units"]["main"]["compiler"], "component");
    assert_eq!(receipt["contractMode"], "wit");
    assert_eq!(receipt["units"]["main"]["commands"], serde_json::json!([]));
    assert_eq!(
        receipt["units"]["main"]["inputs"]["dependency.wasm"],
        sha(&bytes)
    );
    assert!(receipt["tools"].get("wasmtime").is_none());
}
#[test]
fn rejects_prebuilt_hash_and_invalid_binary_before_running_compilers() {
    let bytes = component_bytes();
    let config = prebuilt_config(&bytes);
    let root = fixture(&config, WIT);
    fs::write(root.path().join("dependency.wasm"), b"different bytes").unwrap();
    reject(root.path(), "prebuilt Component sha256 mismatch");
    let root = fixture(&config.replace(&sha(&bytes), "bad"), WIT);
    reject(root.path(), "invalid prebuilt Component sha256");
    let core = wasm_encoder::Module::new().finish();
    let root = fixture(&prebuilt_config(&core), WIT);
    fs::write(root.path().join("dependency.wasm"), core).unwrap();
    reject(root.path(), "wrap Core Wasm with WIT first");
    let invalid = b"\0asm\x0d\0\x01\0\xff";
    let root = fixture(&prebuilt_config(invalid), WIT);
    fs::write(root.path().join("dependency.wasm"), invalid).unwrap();
    reject(root.path(), "invalid prebuilt Component binary");
}
#[test]
fn rejects_prebuilt_contract_mismatches_before_running_compilers() {
    let bytes = component_bytes();
    let root = fixture(&prebuilt_config(&bytes), &WIT.replace("u32", "u64"));
    fs::write(root.path().join("dependency.wasm"), &bytes).unwrap();
    reject(root.path(), "prebuilt Component export contract differs");
    let config = format!(
        "{}\n[build.units.main.dependencies]\nextra='zhelper'\n[build.units.zhelper]\nworld='library'\ncomponent={{path='dependency.wasm',sha256='{}'}}\n",
        prebuilt_config(&bytes),
        sha(&bytes)
    );
    let wit = WIT.replace("world library {", "world library { import extra: func();");
    let root = fixture(&config, &wit);
    fs::write(root.path().join("dependency.wasm"), &bytes).unwrap();
    reject(root.path(), "prebuilt Component import contract differs");
}
#[test]
fn rejects_conflicting_prebuilt_config_and_path_escape() {
    let bytes = component_bytes();
    let config = prebuilt_config(&bytes);
    let root = fixture(
        &config.replace("world='library'", "world='library'\ncompiler='clang'"),
        WIT,
    );
    reject(root.path(), "exactly one of compiler or component");
    let root = fixture(
        &config.replace("world='library'", "world='library'\nsources=['main.cpp']"),
        WIT,
    );
    reject(root.path(), "prebuilt Component unit main cannot set");
    let root = fixture(
        &config.replace("dependency.wasm", "../dependency.wasm"),
        WIT,
    );
    reject(root.path(), "cannot contain '..'");
}

#[test]
fn checked_in_go_bindings_do_not_require_an_exact_generated_copy() {
    let config = CONFIG
        .replace("compiler='clang'", "compiler='go'")
        .replace("sources=['main.cpp']", "project='go'");
    let root = fixture(&config, WIT);
    fs::create_dir(root.path().join("go")).unwrap();
    fs::write(
        root.path().join("go/go.mod"),
        "module example.org/application\n\ngo 1.27.0\n",
    )
    .unwrap();
    fs::write(
        root.path().join("go/main.go"),
        "package main\nfunc main() {}\n",
    )
    .unwrap();
    fs::write(
        root.path().join("go/wisp_bindings.go"),
        "package main\n// stale bindings\n",
    )
    .unwrap();
    reject(root.path(), "set WISP_WASMTIME");
    assert_eq!(
        fs::read_to_string(root.path().join("go/wisp_bindings.go")).unwrap(),
        "package main\n// stale bindings\n"
    );
}

#[test]
fn rejects_unsupported_inferred_rust_api_before_loading_a_compiler() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname='application'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    fs::write(
        root.path().join("src/lib.rs"),
        "pub fn greeting(value: &str) -> String { value.into() }\n",
    )
    .unwrap();
    fs::write(
        root.path().join("wisp.toml"),
        "[package]\nname='test:rust'\nversion='0.1.0'\n[build]\ncompiler='rust'\n",
    )
    .unwrap();
    reject(root.path(), "unsupported public Rust type");
}

#[test]
fn infers_cpp_collections_options_and_enums_without_a_wit_file() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("wisp.toml"), "[package]\nname='test:inferred'\nversion='0.1.0'\n[build]\ncompiler='clang'\nsources=['main.cpp']\n").unwrap();
    fs::write(root.path().join("main.cpp"), "#include <vector>\n#include <optional>\n#include <string>\nenum class Mode { Fast = 7, Careful = 42 };\nstd::vector<std::optional<Mode>> choices(std::vector<std::optional<Mode>> value) { return value; }\nstd::vector<std::string> labels(std::vector<std::string> value) { return value; }\n").unwrap();
    reject(root.path(), "set WISP_WASMTIME");
}

#[test]
fn infers_go_collections_options_and_enums_without_a_wit_file() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("wisp.toml"),
        "[package]\nname='test:inferred'\nversion='0.1.0'\n[build]\ncompiler='go'\nproject='.'\n",
    )
    .unwrap();
    fs::write(
        root.path().join("go.mod"),
        "module example.org/inferred\n\ngo 1.27.0\n",
    )
    .unwrap();
    fs::write(root.path().join("main.go"), "package main\ntype Mode uint32\nconst (\n Fast Mode = iota\n Careful\n)\nfunc Choices(value []*Mode) []*Mode { return value }\nfunc Labels(value []string) []string { return value }\nfunc main() {}\n").unwrap();
    reject(root.path(), "set WISP_WASMTIME");
}
