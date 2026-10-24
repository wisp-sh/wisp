#[path = "support/source_bindings.rs"]
mod source_bindings;
use source_bindings::{build_bindings, build_go_memory, build_values};

#[allow(dead_code)]
#[path = "../src/generator_tools.rs"]
mod generator_tools;
#[allow(dead_code)]
#[path = "../src/identity.rs"]
mod identity;
#[allow(dead_code)]
#[path = "../src/inspect.rs"]
mod inspect;
#[allow(dead_code)]
#[path = "../src/library.rs"]
mod library;
#[allow(dead_code)]
#[path = "../src/library_async.rs"]
mod library_async;
#[allow(dead_code)]
#[path = "../src/library_error_context.rs"]
mod library_error_context;
#[allow(dead_code)]
#[path = "../src/library_go.rs"]
mod library_go;
#[allow(dead_code)]
#[path = "../src/library_metadata.rs"]
mod library_metadata;
#[allow(dead_code)]
#[path = "../src/library_publish.rs"]
mod library_publish;
#[allow(dead_code)]
#[path = "../src/library_resources.rs"]
mod library_resources;
#[allow(dead_code)]
#[path = "../src/library_wasi.rs"]
mod library_wasi;
#[allow(dead_code)]
#[path = "../src/release.rs"]
mod release;
#[allow(dead_code)]
#[path = "../src/wit_model.rs"]
mod wit_model;

#[test]
fn typed_library_bindings_follow_the_contract_not_fixture_names() {
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve
        .push_str(
            "library.wit",
            r#"
        package shop:prices@1.2.3;
        interface pricing {
            record purchase { count: u32, price: u64 }
            enum failure { overflow, unavailable }
            calculate: func(value: purchase) -> result<u64, failure>;
        }
        world library { export pricing; }
    "#,
        )
        .unwrap();
    let world = resolve.select_world(&[package], Some("library")).unwrap();
    let source = library::rust_source(&resolve, world).unwrap();
    assert!(source.contains("pub mod pricing"));
    assert!(source.contains("pub fn calculate(&mut self, __wisp_arg_0: Purchase)"));
    assert!(source.contains("Result<u64, CallError<Failure>>"));
    assert!(source.contains("#[cfg(target_arch = \"wasm32\")]"));
    assert!(source.contains("shop_prices_pricing()"));
    assert!(source.contains("pub struct Library"));
    assert!(source.contains("pub fn pricing(&self) -> pricing::Client"));
    assert!(!source.contains("arithmetic"));
}

#[test]
fn library_package_names_are_unambiguous() {
    assert_eq!(
        library::package_names("shop:prices").unwrap(),
        ("wisp-shop--prices".into(), "@shop/prices".into())
    );
    assert_ne!(
        library::package_names("a-b:c").unwrap(),
        library::package_names("a:b-c").unwrap()
    );
    for invalid in [
        "shop",
        "shop:../prices",
        "shop:prices:extra",
        "a_b:c",
        "a--b:c",
    ] {
        assert!(library::package_names(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn nested_library_values_are_supported_in_parameters_and_results() {
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve
        .push_str(
            "nested.wit",
            r#"
        package shop:catalog@1.0.0;
        interface catalog {
            flags permissions { read, write }
            record entry { title: string, aliases: list<string>, access: permissions }
            variant selection { missing, found(entry) }
            type choices = list<selection>;
            type failure = tuple<u32, string>;
            lookup: func(query: option<string>, prior: choices) -> result<option<entry>, failure>;
            echo: func(value: tuple<string, option<entry>>) -> tuple<string, option<entry>>;
        }
        world library { export catalog; }
    "#,
        )
        .unwrap();
    let world = resolve.select_world(&[package], Some("library")).unwrap();
    let source = library::rust_source(&resolve, world).unwrap();
    assert!(source.contains("pub fn lookup"));
    assert!(source.contains("Result<Option<Entry>, CallError<Failure>>"));
    assert!(source.contains("pub fn echo"));
}

#[test]
fn jq_native_host_uses_denied_clocks_instead_of_host_time() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = std::fs::read(root.join("fixtures/jq/component.wasm")).unwrap();
    let wit_parser::decoding::DecodedWasm::Component(resolve, world) =
        wit_parser::decoding::decode(&bytes).unwrap()
    else {
        panic!("not a component")
    };
    let source = library::rust_source(&resolve, world).unwrap();
    assert!(source.contains("WISP_DENIED: wall clock"));
    assert!(source.contains("WISP_DENIED: monotonic clock"));
    assert!(source.contains("pure_wasi_linker(&mut linker)?"));
    assert!(!source.contains("inherit_network()"));
}

#[test]
fn decoded_jq_top_level_export_matches_authored_function_contract() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = std::fs::read(root.join("fixtures/jq/component.wasm")).unwrap();
    let wit_parser::decoding::DecodedWasm::Component(decoded, world) =
        wit_parser::decoding::decode(&bytes).unwrap()
    else {
        panic!("not a component")
    };
    let mut authored = wit_parser::Resolve::default();
    let package = authored
        .push_str(
            "jq.wit",
            &std::fs::read_to_string(root.join("fixtures/jq/wit/jq.wit")).unwrap(),
        )
        .unwrap();
    let authored_world = authored
        .select_world(&[package], Some("jq-component"))
        .unwrap();
    let actual = wit_model::AbiModel::from_resolve(&decoded, world).unwrap();
    let expected = wit_model::AbiModel::from_resolve(&authored, authored_world).unwrap();
    assert!(library::exports_match(&actual, &expected));
    let mut wrong = expected.clone();
    wrong.exports[0].functions[0].name = "different".into();
    assert!(!library::exports_match(&actual, &wrong));
    // Guest imports are a separate host-policy gate; isolate export generation here.
    let mut exports_only = decoded.clone();
    exports_only.worlds[world].imports.clear();
    let source = library::rust_source(&exports_only, world).unwrap();
    assert!(source.contains("super::guest::evaluate("));
    assert!(source.contains("bindings.call_evaluate(store, &__wisp_arg_0, &__wisp_arg_1)"));
}

#[test]
fn direct_export_generated_package_compiles_for_native() {
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve.push_str("direct.wit", "package shop:prices@1.2.3; world calculator { export calculate: func(value: string) -> result<list<string>, string>; }").unwrap();
    let world = resolve
        .select_world(&[package], Some("calculator"))
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::create_dir_all(dir.path().join("wit")).unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        library::rust_source(&resolve, world).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.path().join("component.wasm"), b"\0asm\x0d\0\x01\0").unwrap();
    std::fs::write(dir.path().join("wit/host.wit"), "package shop:prices@1.2.3; world calculator { export calculate: func(value: string) -> result<list<string>, string>; }").unwrap();
    std::fs::write(dir.path().join("wit/guest.wit"), "package shop:prices@1.2.3; world wisp-imports { import calculate: func(value: string) -> result<list<string>, string>; }").unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = 'wisp-generated-direct-check'\nversion = '0.1.0'\nedition = '2024'\n[workspace]\n[target.'cfg(not(target_arch = \"wasm32\"))'.dependencies]\nwasmtime = { version = '=50.0.0', default-features = false, features = ['component-model', 'cranelift', 'runtime', 'std'] }\n[target.'cfg(target_arch = \"wasm32\")'.dependencies]\nwit-bindgen = '=0.61.1'\n").unwrap();
    let mut command = std::process::Command::new("cargo");
    let output = command
        .arg("check")
        .arg("--offline")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn direct_keyword_export_generates_valid_rust_calls() {
    for (name, method) in [
        ("match", "r#match"),
        ("type", "r#type"),
        ("async", "r#async"),
        ("gen", "r#gen"),
        ("self", "self_"),
        ("super", "super_"),
        ("crate", "crate_"),
    ] {
        let mut resolve = wit_parser::Resolve::default();
        let package = resolve
        .push_str(
            "keywords.wit",
            &format!("package shop:regex; world library {{ export %{name}: func(value: string) -> bool; }}"),
        )
        .unwrap();
        let world = resolve.select_world(&[package], Some("library")).unwrap();
        let source = library::rust_source(&resolve, world).unwrap();
        syn::parse_file(&source).expect("generated Rust must parse for keyword exports");
        assert!(source.contains(&format!("pub fn {method}(&mut self, __wisp_arg_0: String)")));
        assert!(source.contains(&format!("super::guest::{method}(&__wisp_arg_0)")));
        assert!(source.contains(&format!("bindings.call_{name}(store, &__wisp_arg_0)")));
    }
}

#[test]
fn direct_freestanding_export_generates_native_and_guest_calls() {
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve.push_str("direct.wit", "package shop:prices@1.2.3; world calculator { export calculate: func(value: u32) -> result<u32, string>; }").unwrap();
    let world = resolve
        .select_world(&[package], Some("calculator"))
        .unwrap();
    let source = library::rust_source(&resolve, world).unwrap();
    assert!(source.contains("pub fn calculate(&mut self, __wisp_arg_0: u32)"));
    assert!(source.contains("super::guest::calculate(__wisp_arg_0)"));
    assert!(source.contains("bindings.call_calculate(store, __wisp_arg_0)"));
}

#[test]
fn bundled_worker_rust_package_runs_without_an_installed_runtime() {
    use std::{fs, process::Command};
    use wasm_encoder::*;
    // Keep this fixture import-free. Source-built Rust fixtures carry WASI and
    // deliberately retain the established native host until worker parity.
    let mut module = Module::new();
    let mut types = TypeSection::new();
    types
        .ty()
        .function([ValType::I32, ValType::I32], [ValType::I32]);
    types.ty().function([ValType::I32], [ValType::I32]);
    types.ty().function([ValType::I64], [ValType::I64]);
    module.section(&types);
    let mut functions = FunctionSection::new();
    functions.function(0).function(1).function(2);
    module.section(&functions);
    let mut exports = ExportSection::new();
    for (index, name) in ["add", "negate", "widen"].iter().enumerate() {
        exports.export(
            &format!("test:worker/api@0.1.0#{name}"),
            ExportKind::Func,
            index as u32,
        );
    }
    module.section(&exports);
    let mut code = CodeSection::new();
    for instructions in [
        vec![
            Instruction::LocalGet(0),
            Instruction::LocalGet(1),
            Instruction::I32Add,
        ],
        vec![Instruction::LocalGet(0), Instruction::I32Eqz],
        vec![
            Instruction::LocalGet(0),
            Instruction::I64Const(1),
            Instruction::I64Add,
        ],
    ] {
        let mut function = Function::new([]);
        for instruction in instructions {
            function.instruction(&instruction);
        }
        function.instruction(&Instruction::End);
        code.function(&function);
    }
    module.section(&code);
    let mut bytes = module.finish();
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve.push_str("worker.wit", "package test:worker@0.1.0; interface api { add: func(a:u32,b:u32)->u32; negate: func(value:bool)->bool; widen: func(value:u64)->u64; } world library { export api; }").unwrap();
    let world = resolve.select_world(&[package], Some("library")).unwrap();
    wit_component::embed_component_metadata(
        &mut bytes,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .unwrap();
    let component = wit_component::ComponentEncoder::default()
        .validate(true)
        .module(&bytes)
        .unwrap()
        .encode()
        .unwrap();
    let wit_parser::decoding::DecodedWasm::Component(resolve, world) =
        wit_parser::decoding::decode(&component).unwrap()
    else {
        panic!("Component fixture")
    };
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    let source = library::rust_source(&resolve, world).unwrap();
    fs::write(temp.path().join("src/main.rs"), format!(
        "{source}\nfn main() {{let library=Library::new().unwrap();let mut api=library.api();assert_eq!(api.add(20,22).unwrap(),42);assert!(!api.negate(true).unwrap());assert_eq!(api.widen(41).unwrap(),42);println!(\"42\");}}"
    )).unwrap();
    fs::write(temp.path().join("component.wasm"), component).unwrap();
    for (path, bytes) in wisp_worker_protocol::bundle::files().unwrap() {
        let path = temp.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name="bundled-worker-rust-test"
version="0.1.0"
edition="2024"
[workspace]
[dependencies]
serde={version="1",features=["derive"]}
serde_json="1"
tempfile="3"
[profile.dev]
debug=0
"#,
    )
    .unwrap();
    let target = temp.path().join("target");
    let build = Command::new("cargo")
        .args(["build", "--offline", "-j1", "--target-dir"])
        .arg(&target)
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(target.join("debug/bundled-worker-rust-test"))
        .env_clear()
        .env("TMPDIR", temp.path())
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "42");
}
