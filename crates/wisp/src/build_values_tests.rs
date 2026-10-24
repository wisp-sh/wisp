use super::*;
use crate::build_bindings;
use wit_parser::{
    Resolve, SizeAlign, Type, TypeDefKind, WorldItem,
    abi::{AbiVariant, FlatTypes, WasmType},
};

const CONTRACT: &str = r#"
package test:canonical-values;
interface api {
    record small { byte: u8, wide: u64, signed: s16, real: f32 }
    record owned { flag: bool, text: string, bytes: list<u8>, nested: small, last: f64 }
    record nested { value: owned, status: result<owned, string> }
    small-value: func(value: small) -> small;
    owned-value: func(value: owned) -> owned;
    nested-value: func(value: nested) -> result<nested, list<u8>>;
    no-payload: func(value: result) -> result;
    mixed32: func(value: result<f32, u32>) -> result<f32, u32>;
    mixed64: func(value: result<f32, u64>) -> result<f32, u64>;
    mixed-floats: func(value: result<f32, f64>) -> result<f32, f64>;
    wide-input: func(a: owned, b: owned, c: owned) -> string;
}
world values { export api; }
"#;
#[test]
fn incompatible_interface_record_names_fail_before_generation() {
    let mut resolve = Resolve::default();
    let package = resolve.push_str("collision.wit", "package test:collision; interface first { record item { value:string } run:func(value:item); } interface second { record item { value:u64 } run:func(value:item); } world app { export first; export second; }").unwrap();
    let world = resolve.select_world(&[package], Some("app")).unwrap();
    let error = build_bindings::bindings(&resolve, world)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("different records normalize to WispItem"),
        "{error}"
    );
}

#[test]
fn keyword_fields_in_owned_rust_bindings_are_valid_identifiers() {
    let mut resolve = Resolve::default();
    let package = resolve.push_str("keywords.wit", "package test:keywords; interface api { record item { %type:string, %match:list<u8>, %self:u32 } run:func(value:item)->item; } world app { export api; }").unwrap();
    let world = resolve.select_world(&[package], Some("app")).unwrap();
    let bindings = build_bindings::bindings(&resolve, world).unwrap();
    syn::parse_file(&rust_guest(&bindings).unwrap()).unwrap();
    syn::parse_file(&rust(&bindings, "application").unwrap()).unwrap();
}
fn interface() -> (Resolve, wit_parser::WorldId, wit_parser::InterfaceId) {
    let mut resolve = Resolve::default();
    let package = resolve.push_str("canonical-values.wit", CONTRACT).unwrap();
    let world = resolve.select_world(&[package], Some("values")).unwrap();
    let interface = resolve.worlds[world]
        .exports
        .values()
        .find_map(|item| match item {
            WorldItem::Interface { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    (resolve, world, interface)
}
fn abi(ty: WasmType) -> Abi {
    match ty {
        WasmType::I32 | WasmType::Pointer | WasmType::Length => Abi::I32,
        WasmType::I64 | WasmType::PointerOrI64 => Abi::I64,
        WasmType::F32 => Abi::F32,
        WasmType::F64 => Abi::F64,
    }
}
fn check_type(resolve: &Resolve, sizes: &SizeAlign, scalar: &Scalar, ty: &Type) {
    assert_eq!(
        layout(scalar),
        (sizes.size(ty).size_wasm32(), sizes.align(ty).align_wasm32()),
        "layout for {ty:?}"
    );
    let mut storage = [WasmType::I32; 128];
    let mut expected = FlatTypes::new(&mut storage);
    assert!(resolve.push_flat(ty, &mut expected));
    assert_eq!(
        flat(scalar),
        expected.to_vec().into_iter().map(abi).collect::<Vec<_>>(),
        "flattening for {ty:?}"
    );
    if let ValueKind::Record { fields, .. } = &scalar.kind {
        let Type::Id(id) = ty else {
            panic!("record type ID")
        };
        let TypeDefKind::Record(record) = &resolve.types[*id].kind else {
            panic!("record definition")
        };
        let offsets = record_layout(fields.iter().map(|field| &field.ty)).1;
        assert_eq!(
            offsets,
            sizes
                .field_offsets(record.fields.iter().map(|field| &field.ty))
                .iter()
                .map(|(offset, _)| offset.size_wasm32())
                .collect::<Vec<_>>()
        );
        for (field, expected) in fields.iter().zip(&record.fields) {
            check_type(resolve, sizes, &field.ty, &expected.ty);
        }
    }
}
#[test]
fn values_match_wit_parser_canonical_layout_and_flattening() {
    let (resolve, world, interface) = interface();
    let bindings = build_bindings::bindings(&resolve, world).unwrap();
    let mut sizes = SizeAlign::default();
    sizes.fill(&resolve);
    for binding in &bindings {
        let function = &resolve.interfaces[interface].functions[&binding.name];
        for (scalar, parameter) in binding.params.iter().zip(&function.params) {
            check_type(&resolve, &sizes, scalar, &parameter.ty);
        }
        if let (Some(scalar), Some(ty)) = (&binding.result, &function.result) {
            check_type(&resolve, &sizes, scalar, ty);
        }
    }
}
#[test]
fn indirect_parameters_and_result_pointers_match_wit_parser_signatures() {
    let (resolve, world, interface) = interface();
    let bindings = build_bindings::bindings(&resolve, world).unwrap();
    for binding in &bindings {
        let function = &resolve.interfaces[interface].functions[&binding.name];
        for variant in [AbiVariant::GuestExport, AbiVariant::GuestImport] {
            let signature = resolve.wasm_signature(variant, function);
            let mut params = binding.params.iter().flat_map(flat).collect::<Vec<_>>();
            let indirect = params.len() > 16;
            if indirect {
                params = vec![Abi::I32];
            }
            let mut results = binding.result.as_ref().map(flat).unwrap_or_default();
            let retptr = results.len() > 1;
            if retptr {
                results.clear();
                if variant == AbiVariant::GuestExport {
                    results.push(Abi::I32);
                } else {
                    params.push(Abi::I32);
                }
            }
            assert_eq!(
                params,
                signature.params.into_iter().map(abi).collect::<Vec<_>>(),
                "{} {variant:?}",
                binding.name
            );
            assert_eq!(
                results,
                signature.results.into_iter().map(abi).collect::<Vec<_>>(),
                "{} {variant:?}",
                binding.name
            );
            assert_eq!(indirect, signature.indirect_params);
            assert_eq!(retptr, signature.retptr);
        }
    }
}
#[test]
fn generated_owned_results_have_post_return_cleanup_and_joined_tags_have_abi_width() {
    let (resolve, world, _) = interface();
    let bindings = build_bindings::bindings(&resolve, world).unwrap();
    let rust = rust(&bindings, "fixture").unwrap();
    let go = go(&bindings);
    for binding in &bindings {
        if binding.result.as_ref().is_some_and(|ty| flat(ty).len() > 1) {
            let name = format!("cabi_post_{}", export_name(binding));
            assert!(rust.contains(&name));
            assert!(go.contains(&name));
        }
    }
    let result = result(Some(string()), Some(bytes()));
    for lang in [Lang::Rust, Lang::Go] {
        let mut generator = Gen::new(lang);
        generator.flatten_memory(&result, "ptr");
        let assignment = generator
            .code
            .lines()
            .find(|line| line.starts_with("v0 ="))
            .unwrap();
        assert!(
            assignment.contains(if lang == Lang::Rust {
                "as u32"
            } else {
                "uint32("
            }),
            "{assignment}"
        );
    }
}
