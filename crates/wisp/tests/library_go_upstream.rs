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
use wit_parser::{Resolve, WorldId};

const WIT: &str = include_str!("fixtures/go-upstream-guest/world.wit");

fn world(wit: &str) -> (Resolve, WorldId) {
    let mut resolve = Resolve::default();
    let package = resolve.push_str("world.wit", wit).unwrap();
    let world = resolve.select_world(&[package], Some("library")).unwrap();
    (resolve, world)
}

// An executable Component type fixture. Its functions trap, so capability and
// package-authentication tests exercise no guest compiler or external runtime.
fn component() -> Vec<u8> {
    use wasm_encoder::*;
    use wit_parser::{
        WorldItem,
        abi::{AbiVariant, WasmType},
    };
    let (resolve, world) = world(WIT);
    let core = |ty| match ty {
        WasmType::I32 | WasmType::Pointer | WasmType::Length => ValType::I32,
        WasmType::I64 | WasmType::PointerOrI64 => ValType::I64,
        WasmType::F32 => ValType::F32,
        WasmType::F64 => ValType::F64,
    };
    let mut types = TypeSection::new();
    let mut functions = FunctionSection::new();
    let mut exports = ExportSection::new();
    let mut code = CodeSection::new();
    let mut count = 0;
    for (key, item) in &resolve.worlds[world].exports {
        let WorldItem::Interface { id, .. } = item else {
            continue;
        };
        for f in resolve.interfaces[*id].functions.values() {
            let signature = resolve.wasm_signature(AbiVariant::GuestExport, f);
            types.ty().function(
                signature.params.into_iter().map(core),
                signature.results.into_iter().map(core),
            );
            functions.function(count);
            exports.export(
                &format!("{}#{}", resolve.name_world_key(key), f.name),
                ExportKind::Func,
                count,
            );
            let mut f = Function::new([]);
            f.instruction(&Instruction::Unreachable)
                .instruction(&Instruction::End);
            code.function(&f);
            count += 1;
        }
    }
    types.ty().function([ValType::I32; 4], [ValType::I32]);
    functions.function(count);
    exports.export("cabi_realloc", ExportKind::Func, count);
    exports.export("memory", ExportKind::Memory, 0);
    let mut realloc = Function::new([]);
    realloc
        .instruction(&Instruction::Unreachable)
        .instruction(&Instruction::End);
    code.function(&realloc);
    let mut memory = MemorySection::new();
    memory.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    let mut module = Module::new();
    module
        .section(&types)
        .section(&functions)
        .section(&memory)
        .section(&exports)
        .section(&code);
    let mut bytes = module.finish();
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
fn rich_guest_capability_does_not_claim_native_wago_support() {
    let bytes = component();
    assert!(library_go::guest::supports(&bytes));
    assert!(!library_go::portable::supports(&bytes));
}

#[test]
fn upstream_generates_rich_imports_and_root_facade() {
    let (resolve, world) = world(WIT);
    let files = library_go::guest::import_files(&resolve, world, "example.org/rich-go").unwrap();
    assert_eq!(
        files["test_rich_go_values/wit_bindings.go"],
        include_bytes!("fixtures/go-upstream-guest/test_rich_go_values/wit_bindings.go").as_slice()
    );
    let root = std::str::from_utf8(&files["bindings_wasip1.go"]).unwrap();
    assert!(root.contains("var ValuesEcho = wisp0.Echo"));
    assert!(root.contains("var ValuesChoices = wisp0.Choices"));
    assert!(root.contains("type ValuesPayload = wisp0.Payload"));
    assert!(root.contains("const ValuesModeCareful = wisp0.ModeCareful"));
    assert!(!root.contains("go:wasmimport"));
    assert!(!files.keys().any(|name| name.starts_with("wit_exports/")));
}

#[test]
fn rich_guest_archive_authenticates_nested_upstream_sources() {
    let bytes = component();
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("package");
    library_go::guest::generate(
        &bytes,
        &output,
        "example.org/rich-go",
        "0.1.0",
        &BTreeMap::new(),
    )
    .unwrap();
    let archive = library_go::module_archive(&output, "example.org/rich-go", "0.1.0").unwrap();
    library_go::guest::verify_archive(
        &bytes,
        "example.org/rich-go",
        "0.1.0",
        &archive.zip,
        &BTreeMap::new(),
    )
    .unwrap();
    library_go::guest::vendored(&output, "example.org/rich-go").unwrap();
    let path = output.join("test_rich_go_values/wit_bindings.go");
    fs::write(
        path,
        "//go:build wasip1 && wasm\npackage test_rich_go_values\n",
    )
    .unwrap();
    assert!(library_go::guest::vendored(&output, "example.org/rich-go").is_err());
    let forged = library_go::module_archive(&output, "example.org/rich-go", "0.1.0").unwrap();
    assert!(
        library_go::guest::verify_archive(
            &bytes,
            "example.org/rich-go",
            "0.1.0",
            &forged.zip,
            &BTreeMap::new()
        )
        .is_err()
    );
}

#[test]
fn resource_and_async_signatures_fail_before_upstream_codegen() {
    for wit in [
        "package test:unsupported; interface api { resource item; get: func() -> item; } world library { export api; }",
        "package test:unsupported; interface api { get: async func() -> u32; } world library { export api; }",
        "package test:unsupported; interface api { get: func() -> future<u32>; } world library { export api; }",
    ] {
        let (resolve, world) = world(wit);
        assert!(
            library_go::guest::import_files(&resolve, world, "example.org/unsupported").is_err()
        );
    }
}
