#[allow(dead_code)]
#[path = "../src/inspect.rs"]
mod inspect;
#[allow(dead_code)]
#[path = "../src/release.rs"]
mod release;
#[allow(dead_code)]
#[path = "../src/wit_model.rs"]
mod wit_model;

use wit_model::{AbiAsyncCategory, AbiFunctionKind, AbiModel, AbiType};

fn future_model() -> AbiModel {
    let syntax = tempfile::tempdir().unwrap();
    std::fs::write(
        syntax.path().join("fixture.wit"),
        r#"package example:future-fixture@0.1.0;
interface futures {
  record future-operation { value: future<u32>, driver: u32 }
  variant operation-error { capacity }
  future-value: func(value: u32) -> result<future-operation, operation-error>;
  future-drive: async func(driver: u32) -> u32;
  future-cancel: async func(driver: u32) -> u32;
}
world fixture { export futures; }
"#,
    )
    .unwrap();
    let mut resolve = wit_parser::Resolve::default();
    let (package, _) = resolve.push_dir(syntax.path()).unwrap();
    let world = *resolve.packages[package].worlds.get("fixture").unwrap();
    AbiModel::from_resolve(&resolve, world).unwrap()
}

fn stream_model() -> AbiModel {
    let syntax = tempfile::tempdir().unwrap();
    std::fs::write(
        syntax.path().join("fixture.wit"),
        r#"package example:stream-fixture@0.1.0;
interface streams {
  record stream-operation { value: stream<u32>, driver: u32 }
  variant operation-error { capacity }
  stream-values: func(first: u32, second: u32) -> result<stream-operation, operation-error>;
  stream-drive: async func(driver: u32) -> u32;
  stream-cancel: async func(driver: u32) -> u32;
}
world fixture { export streams; }
"#,
    )
    .unwrap();
    let mut resolve = wit_parser::Resolve::default();
    let (package, _) = resolve.push_dir(syntax.path()).unwrap();
    let world = *resolve.packages[package].worlds.get("fixture").unwrap();
    AbiModel::from_resolve(&resolve, world).unwrap()
}

fn error_context_model() -> AbiModel {
    let syntax = tempfile::tempdir().unwrap();
    std::fs::write(
        syntax.path().join("fixture.wit"),
        r#"package example:errorctx@0.1.0;
interface errors {
  error-value: func() -> error-context;
  debug-message: func(value: error-context) -> string;
}
world fixture { export errors; }
"#,
    )
    .unwrap();
    let mut resolve = wit_parser::Resolve::default();
    let (package, _) = resolve.push_dir(syntax.path()).unwrap();
    let world = *resolve.packages[package].worlds.get("fixture").unwrap();
    AbiModel::from_resolve(&resolve, world).unwrap()
}

fn wasi_network_model() -> AbiModel {
    let syntax = tempfile::tempdir().unwrap();
    std::fs::create_dir(syntax.path().join("deps")).unwrap();
    let dependency_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/library-network/library/wit/deps");
    for entry in std::fs::read_dir(dependency_root).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(
            entry.path(),
            syntax.path().join("deps").join(entry.file_name()),
        )
        .unwrap();
    }
    std::fs::write(
        syntax.path().join("fixture.wit"),
        r#"package example:network-fixture@0.1.0;
world fixture {
  import wasi:http/outgoing-handler@0.2.12;
  import wasi:sockets/instance-network@0.2.12;
  import wasi:sockets/network@0.2.12;
  export probe: func() -> result<list<string>, string>;
}
"#,
    )
    .unwrap();
    let mut resolve = wit_parser::Resolve::default();
    let (package, _) = resolve.push_dir(syntax.path()).unwrap();
    let world = *resolve.packages[package].worlds.get("fixture").unwrap();
    AbiModel::from_resolve(&resolve, world).unwrap()
}

#[test]
fn standard_wasi_http_and_sockets_are_native_only() {
    let model = wasi_network_model();
    let matrix = model.target_support_matrix().unwrap();
    for target in &matrix.targets {
        if target.target == "browser" {
            assert!(!target.supported, "{target:?}");
            assert_eq!(target.reasons.len(), 1, "{target:?}");
            assert_eq!(target.reasons[0].code, "unsupported-browser-wasi-network");
        } else {
            assert!(target.supported, "{target:?}");
            assert!(target.reasons.is_empty(), "{target:?}");
        }
    }
}

#[test]
fn any_standard_wasi_network_import_rejects_the_browser() {
    for retained_package in ["wasi:http@0.2.12", "wasi:sockets@0.2.12"] {
        let mut model = wasi_network_model();
        model
            .imports
            .retain(|interface| interface.package == retained_package);
        let browser = model
            .target_support_matrix()
            .unwrap()
            .targets
            .into_iter()
            .find(|target| target.target == "browser")
            .unwrap();
        assert!(!browser.supported, "{retained_package}: {browser:?}");
        assert_eq!(browser.reasons.len(), 1, "{retained_package}: {browser:?}");
        assert_eq!(
            browser.reasons[0].code, "unsupported-browser-wasi-network",
            "{retained_package}: {browser:?}"
        );
    }
}

#[test]
fn incomplete_standard_wasi_network_profile_fails_closed_on_native_targets() {
    let mut model = wasi_network_model();
    model
        .imports
        .retain(|interface| interface.name != "instance-network");
    let matrix = model.target_support_matrix().unwrap();
    for target in matrix.targets {
        if target.target == "browser" {
            assert_eq!(target.reasons.len(), 1, "{target:?}");
            assert_eq!(target.reasons[0].code, "unsupported-browser-wasi-network");
        } else {
            assert!(!target.supported, "{target:?}");
            assert_eq!(target.reasons.len(), 1, "{target:?}");
            assert_eq!(
                target.reasons[0].code,
                "unsupported-standard-wasi-network-profile"
            );
        }
    }
}

#[test]
fn standard_wasi_network_rejects_special_lifecycle_binding_plans() {
    let resource_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wit-model-resources");
    let mut resolve = wit_parser::Resolve::default();
    let (package, _) = resolve.push_dir(resource_root).unwrap();
    let world = *resolve.packages[package].worlds.get("fixture").unwrap();
    let resource = AbiModel::from_resolve(&resolve, world).unwrap();
    for mut model in [
        resource,
        future_model(),
        stream_model(),
        error_context_model(),
    ] {
        model.imports = wasi_network_model().imports;
        let matrix = model.target_support_matrix().unwrap();
        for target in matrix.targets {
            assert!(!target.supported, "{target:?}");
            assert!(
                target
                    .reasons
                    .iter()
                    .any(|reason| reason.code == "unsupported-standard-wasi-network-lifecycle"),
                "{target:?}"
            );
        }
    }
}

#[test]
fn adjacent_error_context_shapes_fail_closed() {
    for mutate in [0, 1, 2] {
        let mut model = error_context_model();
        match mutate {
            0 => model.exports[0].functions[0].result = Some(AbiType::String),
            1 => model.exports[0].functions[1].params.clear(),
            2 => {
                let extra = model.exports[0].functions[1].clone();
                model.exports[0].functions.push(extra);
            }
            _ => unreachable!(),
        }
        let error = model.binding_plan().unwrap_err().to_string();
        assert!(error.contains("unsupported"), "{error}");
    }
}

#[test]
fn all_other_stream_shapes_fail_closed() {
    let mut model = stream_model();
    model.exports[0].functions[0].params.pop();
    let matrix = model.target_support_matrix().unwrap();
    assert!(matrix.targets.iter().all(|target| !target.supported));
    assert!(model.binding_plan().is_err());
}

fn catalog_model() -> AbiModel {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wit-model-catalog");
    let mut resolve = wit_parser::Resolve::default();
    let (package, _) = resolve.push_dir(root).expect("catalog WIT parses");
    let world = *resolve.packages[package]
        .worlds
        .get("storefront")
        .expect("storefront world");
    AbiModel::from_resolve(&resolve, world).expect("catalog ABI model")
}

#[test]
fn second_wit_fixture_builds_a_contract_independent_model() {
    let model = catalog_model();
    assert_eq!(model.package, "example:catalog@0.1.0");
    assert_eq!(model.world, "storefront");
    assert!(model.imports.is_empty());
    assert_eq!(model.exports.len(), 1);

    let interface = &model.exports[0];
    assert_eq!(interface.package, "example:catalog@0.1.0");
    assert_eq!(interface.name, "storefront-world");
    assert_eq!(
        interface.types,
        vec![
            wit_model::AbiNamedType {
                name: "sku".into(),
                ty: AbiType::String
            },
            wit_model::AbiNamedType {
                name: "locale".into(),
                ty: AbiType::String
            },
            wit_model::AbiNamedType {
                name: "quantity".into(),
                ty: AbiType::U32
            },
            wit_model::AbiNamedType {
                name: "availability".into(),
                ty: AbiType::Bool
            },
            wit_model::AbiNamedType {
                name: "adjustment".into(),
                ty: AbiType::S32
            },
            wit_model::AbiNamedType {
                name: "priority".into(),
                ty: AbiType::U8
            },
            wit_model::AbiNamedType {
                name: "rank".into(),
                ty: AbiType::U16
            },
            wit_model::AbiNamedType {
                name: "temperature".into(),
                ty: AbiType::S8
            },
            wit_model::AbiNamedType {
                name: "pressure".into(),
                ty: AbiType::S16
            },
            wit_model::AbiNamedType {
                name: "symbol".into(),
                ty: AbiType::Char
            },
            wit_model::AbiNamedType {
                name: "metric".into(),
                ty: AbiType::F32
            },
            wit_model::AbiNamedType {
                name: "measurement".into(),
                ty: AbiType::F64
            },
            wit_model::AbiNamedType {
                name: "serial".into(),
                ty: AbiType::U64
            },
            wit_model::AbiNamedType {
                name: "signed-serial".into(),
                ty: AbiType::S64
            },
            wit_model::AbiNamedType {
                name: "search-request".into(),
                ty: AbiType::Record(vec![
                    ("term".into(), AbiType::String),
                    ("limit".into(), AbiType::U32),
                    ("include-disabled".into(), AbiType::Bool),
                ])
            },
            wit_model::AbiNamedType {
                name: "delivery-speed".into(),
                ty: AbiType::Enum(vec![
                    "standard".into(),
                    "next-day".into(),
                    "same-day".into(),
                ])
            },
            wit_model::AbiNamedType {
                name: "access-modes".into(),
                ty: AbiType::Flags(vec!["read-only".into(), "write".into(), "execute".into(),])
            },
            wit_model::AbiNamedType {
                name: "lookup-key".into(),
                ty: AbiType::Variant(vec![
                    ("all".into(), None),
                    ("sku".into(), Some(Box::new(AbiType::String))),
                    ("quantity".into(), Some(Box::new(AbiType::U32))),
                    ("enabled".into(), Some(Box::new(AbiType::Bool))),
                ])
            },
            wit_model::AbiNamedType {
                name: "optional-sku".into(),
                ty: AbiType::Option(Box::new(AbiType::String))
            },
            wit_model::AbiNamedType {
                name: "optional-quantity".into(),
                ty: AbiType::Option(Box::new(AbiType::U32))
            },
            wit_model::AbiNamedType {
                name: "lookup-result".into(),
                ty: AbiType::Result {
                    ok: Some(Box::new(AbiType::String)),
                    err: Some(Box::new(AbiType::U32)),
                }
            },
            wit_model::AbiNamedType {
                name: "status-result".into(),
                ty: AbiType::Result {
                    ok: None,
                    err: Some(Box::new(AbiType::String)),
                }
            },
            wit_model::AbiNamedType {
                name: "quantity-result".into(),
                ty: AbiType::Result {
                    ok: Some(Box::new(AbiType::U32)),
                    err: None,
                }
            },
            wit_model::AbiNamedType {
                name: "strings".into(),
                ty: AbiType::List(Box::new(AbiType::String))
            },
            wit_model::AbiNamedType {
                name: "u8s".into(),
                ty: AbiType::List(Box::new(AbiType::U8))
            },
            wit_model::AbiNamedType {
                name: "u16s".into(),
                ty: AbiType::List(Box::new(AbiType::U16))
            },
            wit_model::AbiNamedType {
                name: "u32s".into(),
                ty: AbiType::List(Box::new(AbiType::U32))
            },
            wit_model::AbiNamedType {
                name: "u64s".into(),
                ty: AbiType::List(Box::new(AbiType::U64))
            },
            wit_model::AbiNamedType {
                name: "s8s".into(),
                ty: AbiType::List(Box::new(AbiType::S8))
            },
            wit_model::AbiNamedType {
                name: "s16s".into(),
                ty: AbiType::List(Box::new(AbiType::S16))
            },
            wit_model::AbiNamedType {
                name: "s32s".into(),
                ty: AbiType::List(Box::new(AbiType::S32))
            },
            wit_model::AbiNamedType {
                name: "s64s".into(),
                ty: AbiType::List(Box::new(AbiType::S64))
            },
            wit_model::AbiNamedType {
                name: "f32s".into(),
                ty: AbiType::List(Box::new(AbiType::F32))
            },
            wit_model::AbiNamedType {
                name: "f64s".into(),
                ty: AbiType::List(Box::new(AbiType::F64))
            },
            wit_model::AbiNamedType {
                name: "bools".into(),
                ty: AbiType::List(Box::new(AbiType::Bool))
            },
            wit_model::AbiNamedType {
                name: "chars".into(),
                ty: AbiType::List(Box::new(AbiType::Char))
            },
            wit_model::AbiNamedType {
                name: "product-key".into(),
                ty: AbiType::Tuple(vec![
                    AbiType::String,
                    AbiType::U32,
                    AbiType::Bool,
                    AbiType::Char,
                    AbiType::U64,
                ])
            },
        ]
    );
    assert_eq!(interface.functions.len(), 37);

    let lookup = &interface.functions[0];
    assert_eq!(lookup.name, "lookup");
    assert_eq!(lookup.kind, AbiFunctionKind::Freestanding);
    assert_eq!(lookup.asynchronous, AbiAsyncCategory::Sync);
    assert_eq!(
        lookup.params,
        vec![
            ("sku".into(), AbiType::Named("sku".into())),
            ("locale".into(), AbiType::Named("locale".into())),
        ]
    );
    assert_eq!(
        lookup.result,
        Some(AbiType::Result {
            ok: Some(Box::new(AbiType::List(Box::new(AbiType::String)))),
            err: Some(Box::new(AbiType::String)),
        })
    );
    assert_eq!(interface.functions[1].name, "list-categories");
    assert_eq!(interface.functions[2].name, "stock-labels");
    assert_eq!(
        interface.functions[2].params,
        vec![("count".into(), AbiType::Named("quantity".into()))]
    );
    assert_eq!(interface.functions[3].name, "availability-label");
    assert_eq!(
        interface.functions[3].params,
        vec![("available".into(), AbiType::Named("availability".into()))]
    );
    assert_eq!(interface.functions[4].name, "adjustment-label");
    assert_eq!(
        interface.functions[4].params,
        vec![("value".into(), AbiType::Named("adjustment".into()))]
    );
    assert_eq!(interface.functions[5].name, "priority-label");
    assert_eq!(
        interface.functions[5].params,
        vec![("value".into(), AbiType::Named("priority".into()))]
    );
    assert_eq!(interface.functions[6].name, "rank-label");
    assert_eq!(
        interface.functions[6].params,
        vec![("value".into(), AbiType::Named("rank".into()))]
    );
    assert_eq!(interface.functions[7].name, "temperature-label");
    assert_eq!(
        interface.functions[7].params,
        vec![("value".into(), AbiType::Named("temperature".into()))]
    );
    assert_eq!(interface.functions[8].name, "pressure-label");
    assert_eq!(
        interface.functions[8].params,
        vec![("value".into(), AbiType::Named("pressure".into()))]
    );
    assert_eq!(interface.functions[9].name, "character-label");
    assert_eq!(
        interface.functions[9].params,
        vec![("value".into(), AbiType::Named("symbol".into()))]
    );
    assert_eq!(interface.functions[10].name, "metric-label");
    assert_eq!(
        interface.functions[10].params,
        vec![("value".into(), AbiType::Named("metric".into()))]
    );
    assert_eq!(interface.functions[11].name, "measurement-label");
    assert_eq!(
        interface.functions[11].params,
        vec![("value".into(), AbiType::Named("measurement".into()))]
    );
    assert_eq!(interface.functions[12].name, "serial-label");
    assert_eq!(
        interface.functions[12].params,
        vec![("value".into(), AbiType::Named("serial".into()))]
    );
    assert_eq!(interface.functions[13].name, "signed-serial-label");
    assert_eq!(
        interface.functions[13].params,
        vec![("value".into(), AbiType::Named("signed-serial".into()))]
    );
    assert_eq!(interface.functions[15].name, "delivery-label");
    assert_eq!(
        interface.functions[15].params,
        vec![("speed".into(), AbiType::Named("delivery-speed".into()))]
    );
    assert_eq!(interface.functions[16].name, "access-label");
    assert_eq!(
        interface.functions[16].params,
        vec![("modes".into(), AbiType::Named("access-modes".into()))]
    );
    assert_eq!(interface.functions[17].name, "lookup-by");
    assert_eq!(
        interface.functions[17].params,
        vec![("key".into(), AbiType::Named("lookup-key".into()))]
    );
    assert_eq!(interface.functions[18].name, "optional-sku-label");
    assert_eq!(
        interface.functions[18].params,
        vec![("value".into(), AbiType::Named("optional-sku".into()))]
    );
    assert_eq!(interface.functions[19].name, "optional-quantity-label");
    assert_eq!(
        interface.functions[19].params,
        vec![("value".into(), AbiType::Named("optional-quantity".into()))]
    );
    assert_eq!(interface.functions[36].name, "accept-product-key");
    assert_eq!(
        interface.functions[36].params,
        vec![("value".into(), AbiType::Named("product-key".into()))]
    );
}

#[test]
fn canonical_contract_binds_model_support_and_domain_separated_fingerprint() {
    use sha2::{Digest, Sha256};

    let model = error_context_model();
    let contract = model.canonical_contract().expect("canonical contract");
    assert_eq!(contract["schema"], 2);
    assert_eq!(contract["package"], model.package);
    assert_eq!(contract["world"], model.world);
    assert_eq!(contract["imports"], serde_json::json!([]));
    assert_eq!(
        contract["exports"],
        model.canonical_json().unwrap()["exports"]
    );
    assert_eq!(
        contract["targetSupport"],
        serde_json::to_value(model.target_support_matrix().unwrap()).unwrap()
    );

    let bytes = model.canonical_contract_bytes().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        contract
    );
    let mut input = b"wisp-wit-contract\0".to_vec();
    input.extend_from_slice(b"2\0");
    input.extend_from_slice(&bytes);
    assert_eq!(
        model.contract_fingerprint().unwrap(),
        format!("sha256:{:x}", Sha256::digest(input))
    );

    let mut reordered = model.clone();
    reordered.exports[0].functions.swap(0, 1);
    assert_ne!(
        reordered.contract_fingerprint().unwrap(),
        model.contract_fingerprint().unwrap(),
        "semantic declaration order must remain fingerprinted"
    );
}

#[test]
fn support_matrix_is_deterministic_for_every_package_target() {
    let matrix = catalog_model()
        .target_support_matrix()
        .expect("support matrix");
    assert_eq!(
        serde_json::to_value(matrix).unwrap(),
        serde_json::json!({
            "schema": 1,
            "targets": [
                {"target":"browser","supported":true,"reasons":[]},
                {"target":"go","supported":true,"reasons":[]},
                {"target":"node","supported":true,"reasons":[]},
                {"target":"rust","supported":true,"reasons":[]}
            ]
        })
    );
}

#[test]
fn catalog_model_preserves_named_scalar_result_parameters() {
    let model = catalog_model();
    let interface = &model.exports[0];
    for (type_name, ok, err, function_name) in [
        (
            "lookup-result",
            Some(AbiType::String),
            Some(AbiType::U32),
            "result-label",
        ),
        ("status-result", None, Some(AbiType::String), "status-label"),
        (
            "quantity-result",
            Some(AbiType::U32),
            None,
            "quantity-result-label",
        ),
    ] {
        assert!(interface.types.iter().any(|ty| {
            ty.name == type_name
                && ty.ty
                    == AbiType::Result {
                        ok: ok.clone().map(Box::new),
                        err: err.clone().map(Box::new),
                    }
        }));
        assert!(interface.functions.iter().any(|function| {
            function.name == function_name
                && function.params == vec![("value".into(), AbiType::Named(type_name.into()))]
        }));
    }
}

#[test]
fn result_parameters_anonymous_nested_and_composite_payloads_fail_closed() {
    let mut anonymous = catalog_model();
    anonymous.exports[0].functions[0].params[0].1 = AbiType::Result {
        ok: Some(Box::new(AbiType::String)),
        err: Some(Box::new(AbiType::U32)),
    };
    let error = anonymous.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-parameter-type]"), "{error}");

    for payload in [
        AbiType::Option(Box::new(AbiType::String)),
        AbiType::Record(vec![("value".into(), AbiType::String)]),
        AbiType::List(Box::new(AbiType::String)),
    ] {
        let mut nested = catalog_model();
        nested.exports[0].types.push(wit_model::AbiNamedType {
            name: "bad-result".into(),
            ty: AbiType::Result {
                ok: Some(Box::new(payload)),
                err: Some(Box::new(AbiType::String)),
            },
        });
        nested.exports[0].functions[0].params[0].1 = AbiType::Named("bad-result".into());
        let error = nested.binding_plan().unwrap_err().to_string();
        assert!(
            error.contains("[unsupported-result-payload-type]"),
            "{error}"
        );
        assert!(error.contains(".type.bad-result.ok"), "{error}");
    }
}

#[test]
fn list_parameters_anonymous_nested_and_composite_elements_fail_closed() {
    let mut anonymous = catalog_model();
    anonymous.exports[0].functions[0].params[0].1 = AbiType::List(Box::new(AbiType::U32));
    let error = anonymous.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-parameter-type]"), "{error}");

    for element in [
        AbiType::List(Box::new(AbiType::U32)),
        AbiType::Option(Box::new(AbiType::String)),
        AbiType::Record(vec![("value".into(), AbiType::String)]),
    ] {
        let mut nested = catalog_model();
        nested.exports[0].types.push(wit_model::AbiNamedType {
            name: "bad-list".into(),
            ty: AbiType::List(Box::new(element)),
        });
        nested.exports[0].functions[0].params[0].1 = AbiType::Named("bad-list".into());
        let error = nested.binding_plan().unwrap_err().to_string();
        assert!(error.contains("[unsupported-list-element-type]"), "{error}");
        assert!(error.contains(".type.bad-list.element"), "{error}");
    }

    let mut result = catalog_model();
    result.exports[0].functions[0].result = Some(AbiType::List(Box::new(AbiType::U32)));
    let error = result.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-result-type]"), "{error}");
}

#[test]
fn tuple_parameters_empty_anonymous_nested_and_composite_elements_fail_closed() {
    let syntax = tempfile::tempdir().unwrap();
    std::fs::write(
        syntax.path().join("world.wit"),
        "package example:empty-tuple@0.1.0;\nworld fixture { type empty = tuple<>; export take: func(value: empty) -> result<list<string>, string>; }\n",
    )
    .unwrap();
    let mut resolve = wit_parser::Resolve::default();
    let (package, _) = resolve.push_dir(syntax.path()).unwrap();
    let world = *resolve.packages[package].worlds.get("fixture").unwrap();
    let parsed_empty = AbiModel::from_resolve(&resolve, world).unwrap();
    let reasons = &parsed_empty.target_support_matrix().unwrap().targets[0].reasons;
    assert!(reasons.iter().any(|reason| reason.code == "empty-tuple"));

    let mut empty = catalog_model();
    empty.exports[0].types.push(wit_model::AbiNamedType {
        name: "empty-tuple".into(),
        ty: AbiType::Tuple(Vec::new()),
    });
    empty.exports[0].functions[0].params[0].1 = AbiType::Named("empty-tuple".into());
    let error = empty.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[empty-tuple]"), "{error}");

    let mut anonymous = catalog_model();
    anonymous.exports[0].functions[0].params[0].1 =
        AbiType::Tuple(vec![AbiType::String, AbiType::U32]);
    let error = anonymous.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-parameter-type]"), "{error}");

    for element in [
        AbiType::List(Box::new(AbiType::U32)),
        AbiType::Option(Box::new(AbiType::String)),
        AbiType::Record(vec![("value".into(), AbiType::String)]),
        AbiType::Tuple(vec![AbiType::U32]),
    ] {
        let mut nested = catalog_model();
        nested.exports[0].types.push(wit_model::AbiNamedType {
            name: "bad-tuple".into(),
            ty: AbiType::Tuple(vec![AbiType::String, element]),
        });
        nested.exports[0].functions[0].params[0].1 = AbiType::Named("bad-tuple".into());
        let error = nested.binding_plan().unwrap_err().to_string();
        assert!(
            error.contains("[unsupported-tuple-element-type]"),
            "{error}"
        );
        assert!(error.contains(".type.bad-tuple.element.1"), "{error}");
    }

    let mut result = catalog_model();
    result.exports[0].functions[0].result =
        Some(AbiType::Tuple(vec![AbiType::String, AbiType::U32]));
    let error = result.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-result-type]"), "{error}");
}

#[test]
fn named_options_admit_every_supported_scalar_payload_for_every_target() {
    let mut model = catalog_model();
    let interface = &mut model.exports[0];
    let scalars = [
        ("bool", AbiType::Bool, "bool"),
        ("u8", AbiType::U8, "u8"),
        ("u16", AbiType::U16, "u16"),
        ("u32", AbiType::U32, "u32"),
        ("u64", AbiType::U64, "u64"),
        ("s8", AbiType::S8, "s8"),
        ("s16", AbiType::S16, "s16"),
        ("s32", AbiType::S32, "s32"),
        ("s64", AbiType::S64, "s64"),
        ("f32", AbiType::F32, "f32"),
        ("f64", AbiType::F64, "f64"),
        ("char", AbiType::Char, "char"),
        ("string", AbiType::String, "string"),
    ];
    let result = interface.functions[1].result.clone();
    for (name, payload, _) in &scalars {
        let option_name = format!("optional-{name}");
        interface.types.push(wit_model::AbiNamedType {
            name: option_name.clone(),
            ty: AbiType::Option(Box::new(payload.clone())),
        });
        interface.functions.push(wit_model::AbiFunction {
            name: format!("accept-optional-{name}"),
            item_name: format!("accept-optional-{name}"),
            kind: AbiFunctionKind::Freestanding,
            params: vec![("value".into(), AbiType::Named(option_name))],
            result: result.clone(),
            owner_resource: None,
            asynchronous: AbiAsyncCategory::Sync,
        });
    }

    let matrix = model.target_support_matrix().unwrap();
    assert_eq!(matrix.targets.len(), 4);
    assert!(
        matrix
            .targets
            .iter()
            .all(|target| target.supported && target.reasons.is_empty())
    );
    let plan = model.binding_plan().unwrap();
    let shape = plan.executable_export_shape();
    for (name, _, wire) in scalars {
        let function_name = format!("accept-optional-{name}");
        let function = shape
            .as_array()
            .unwrap()
            .iter()
            .find(|function| function["name"] == function_name)
            .unwrap();
        assert_eq!(function["params"][0], serde_json::json!({"option": wire}));
    }
}

#[test]
fn option_results_anonymous_nested_and_composite_payloads_fail_closed() {
    let mut result = catalog_model();
    result.exports[0].functions[0].result = Some(AbiType::Option(Box::new(AbiType::String)));
    let error = result.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-result-type]"), "{error}");

    let mut anonymous = catalog_model();
    anonymous.exports[0].functions[0].params[0].1 = AbiType::Option(Box::new(AbiType::String));
    let error = anonymous.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-parameter-type]"), "{error}");

    for payload in [
        AbiType::Option(Box::new(AbiType::String)),
        AbiType::Record(vec![("value".into(), AbiType::String)]),
        AbiType::List(Box::new(AbiType::String)),
    ] {
        let mut nested = catalog_model();
        nested.exports[0].types.push(wit_model::AbiNamedType {
            name: "bad-option".into(),
            ty: AbiType::Option(Box::new(payload)),
        });
        nested.exports[0].functions[0].params[0].1 = AbiType::Named("bad-option".into());
        let error = nested.binding_plan().unwrap_err().to_string();
        assert!(
            error.contains("[unsupported-option-payload-type]"),
            "{error}"
        );
        assert!(error.contains(".type.bad-option.payload"), "{error}");
    }
}

#[test]
fn variant_results_anonymous_nested_composite_and_case_name_fail_closed() {
    let mut result = catalog_model();
    result.exports[0].functions[0].result = Some(AbiType::Named("lookup-key".into()));
    let error = result.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-result-type]"), "{error}");

    let mut anonymous = catalog_model();
    anonymous.exports[0].functions[0].params[0].1 = AbiType::Variant(vec![("all".into(), None)]);
    let error = anonymous.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-parameter-type]"), "{error}");

    for payload in [
        AbiType::Record(vec![("value".into(), AbiType::String)]),
        AbiType::Variant(vec![("nested".into(), None)]),
        AbiType::List(Box::new(AbiType::String)),
    ] {
        let mut nested = catalog_model();
        nested.exports[0].types[17].ty =
            AbiType::Variant(vec![("bad".into(), Some(Box::new(payload)))]);
        let error = nested.binding_plan().unwrap_err().to_string();
        assert!(
            error.contains("[unsupported-variant-payload-type]"),
            "{error}"
        );
        assert!(error.contains(".type.lookup-key.case.bad"), "{error}");
    }

    let mut collision = catalog_model();
    collision.exports[0].types[17].ty =
        AbiType::Variant(vec![("by-sku".into(), None), ("by_sku".into(), None)]);
    let error = collision.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[target-name-collision]"), "{error}");

    let mut reserved = catalog_model();
    reserved.exports[0].types[17].ty = AbiType::Variant(vec![("self".into(), None)]);
    let error = reserved.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[target-name-reserved]"), "{error}");
    assert!(error.contains("target=rust"), "{error}");

    let mut reserved_type = catalog_model();
    reserved_type.exports[0].types[17].name = "self".into();
    reserved_type.exports[0].functions[17].params[0].1 = AbiType::Named("self".into());
    let error = reserved_type.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[target-name-reserved]"), "{error}");
    assert!(error.contains(".type.self"), "{error}");

    let mut empty = catalog_model();
    empty.exports[0].types[17].ty = AbiType::Variant(Vec::new());
    let error = empty.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[empty-variant]"), "{error}");
    assert!(error.contains(".type.lookup-key"), "{error}");
}

#[test]
fn flags_results_anonymous_flags_and_name_collisions_remain_fail_closed() {
    let mut result = catalog_model();
    result.exports[0].functions[0].result = Some(AbiType::Named("access-modes".into()));
    let error = result.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-result-type]"), "{error}");

    let mut anonymous = catalog_model();
    anonymous.exports[0].functions[0].params[0].1 = AbiType::Flags(vec!["read-only".into()]);
    let error = anonymous.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-parameter-type]"), "{error}");

    let mut collision = catalog_model();
    collision.exports[0].types[16].ty =
        AbiType::Flags(vec!["read-only".into(), "read_only".into()]);
    let error = collision.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[target-name-collision]"), "{error}");
    assert!(
        error.contains(".type.access-modes.flag.read_only"),
        "{error}"
    );

    let mut reserved = catalog_model();
    reserved.exports[0].types[16].ty = AbiType::Flags(vec!["self".into()]);
    let error = reserved.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[target-name-reserved]"), "{error}");
    assert!(error.contains("target=rust"), "{error}");
}

#[test]
fn enum_results_anonymous_enums_and_case_name_collisions_remain_fail_closed() {
    let mut result = catalog_model();
    result.exports[0].functions[0].result = Some(AbiType::Named("delivery-speed".into()));
    let error = result.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-result-type]"), "{error}");

    let mut anonymous = catalog_model();
    anonymous.exports[0].functions[0].params[0].1 = AbiType::Enum(vec!["standard".into()]);
    let error = anonymous.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-parameter-type]"), "{error}");

    let mut collision = catalog_model();
    collision.exports[0].types[15].ty = AbiType::Enum(vec!["next-day".into(), "next_day".into()]);
    let error = collision.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[target-name-collision]"), "{error}");
    assert!(error.contains("target=go"), "{error}");
    assert!(!error.contains("target=browser"), "{error}");
    assert!(
        error.contains(".type.delivery-speed.case.next_day"),
        "{error}"
    );

    let mut reserved = catalog_model();
    reserved.exports[0].types[15].ty = AbiType::Enum(vec!["self".into()]);
    let error = reserved.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[target-name-reserved]"), "{error}");
    assert!(error.contains("target=rust"), "{error}");
    assert!(error.contains(".type.delivery-speed.case.self"), "{error}");

    let mut reserved_type = catalog_model();
    reserved_type.exports[0].types[15].name = "self".into();
    reserved_type.exports[0].functions[15].params[0].1 = AbiType::Named("self".into());
    let error = reserved_type.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[target-name-reserved]"), "{error}");
    assert!(error.contains("target=rust"), "{error}");
    assert!(error.contains(".type.self"), "{error}");

    let mut empty = catalog_model();
    empty.exports[0].types[15].ty = AbiType::Enum(Vec::new());
    let error = empty.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[empty-enum]"), "{error}");
    assert!(error.contains(".type.delivery-speed"), "{error}");
}

#[test]
fn nested_records_and_record_results_remain_fail_closed() {
    let mut nested = catalog_model();
    nested.exports[0].types[2].ty = AbiType::Record(vec![(
        "nested".into(),
        AbiType::Record(vec![("value".into(), AbiType::String)]),
    )]);
    let error = nested.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-record-field-type]"), "{error}");
    assert!(error.contains(".type.quantity.field.nested"), "{error}");

    let mut result = catalog_model();
    result.exports[0].functions[0].result = Some(AbiType::Record(vec![
        ("term".into(), AbiType::String),
        ("limit".into(), AbiType::U32),
        ("include-disabled".into(), AbiType::Bool),
    ]));
    let error = result.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-result-type]"), "{error}");
}

#[test]
fn unsupported_shape_reports_stable_code_target_and_fully_qualified_wit_identity() {
    let mut model = catalog_model();
    model.exports[0].functions[0].params[0].1 = AbiType::Record(vec![]);
    let error = model.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-parameter-type]"), "{error}");
    assert!(error.contains("target=browser"), "{error}");
    assert!(
        error.contains("wit=example:catalog@0.1.0/storefront/storefront-world.lookup.param.sku"),
        "{error}"
    );
}

#[test]
fn error_context_aliases_remain_fail_closed_with_stable_reason_codes() {
    let mut model = catalog_model();
    model.exports[0].types[0].ty = AbiType::ErrorContext;
    let error = model.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[unsupported-exported-type]"), "{error}");
    assert!(error.contains("target=browser"), "{error}");
    assert!(
        error.contains("wit=example:catalog@0.1.0/storefront/storefront-world.type.sku"),
        "{error}"
    );
    assert!(error.contains("[unsupported-parameter-type]"), "{error}");
}

#[test]
fn go_alias_and_function_name_collision_is_rejected_before_emission() {
    let mut model = catalog_model();
    model.exports[0].types[0].name = "lookup".into();
    model.exports[0].functions[0].params[0].1 = AbiType::Named("lookup".into());
    let error = model.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[target-name-collision]"), "{error}");
    assert!(error.contains("target=go"), "{error}");
    assert!(error.contains(".type.lookup"), "{error}");
    assert!(error.contains(".lookup"), "{error}");
}

#[test]
fn target_name_collision_reports_stable_code_and_both_wit_identities() {
    let mut model = catalog_model();
    model.exports[0].functions[0].name = "look-up".into();
    model.exports[0].functions[0].item_name = "look-up".into();
    model.exports[0].functions[1].name = "look_up".into();
    model.exports[0].functions[1].item_name = "look_up".into();
    let error = model.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[target-name-collision]"), "{error}");
    assert!(error.contains("target=node"), "{error}");
    assert!(
        error.contains("example:catalog@0.1.0/storefront/storefront-world.look-up"),
        "{error}"
    );
    assert!(
        error.contains("example:catalog@0.1.0/storefront/storefront-world.look_up"),
        "{error}"
    );
}

#[test]
fn support_matrix_rejects_go_reserved_parameter_names() {
    let mut model = catalog_model();
    model.exports[0].functions[0].params[0].0 = "type".into();
    let error = model.binding_plan().unwrap_err().to_string();
    assert!(error.contains("[target-name-reserved]"), "{error}");
    assert!(error.contains("target=go"), "{error}");
    assert!(
        error.contains("wit=example:catalog@0.1.0/storefront/storefront-world.lookup.param.type"),
        "{error}"
    );
}

#[test]
fn emitter_rejects_strict_mode_javascript_reserved_exports() {
    let mut model = catalog_model();
    model.exports[0].functions[0].name = "private".into();
    model.exports[0].functions[0].item_name = "private".into();
    let error = model.binding_plan().unwrap_err().to_string();
    assert!(error.contains("reserved JavaScript export"), "{error}");
}

#[test]
fn scalar_results_remain_fail_closed_with_stable_diagnostics() {
    for result in [
        AbiType::Bool,
        AbiType::S8,
        AbiType::S16,
        AbiType::S32,
        AbiType::U8,
        AbiType::U16,
        AbiType::U64,
        AbiType::S64,
        AbiType::Char,
        AbiType::F32,
        AbiType::F64,
    ] {
        let mut model = catalog_model();
        model.exports[0].functions[3].result = Some(result);
        let error = model.binding_plan().unwrap_err().to_string();
        assert!(error.contains("[unsupported-result-type]"), "{error}");
        assert!(error.contains("target=browser"), "{error}");
        assert!(
            error.contains(
                "wit=example:catalog@0.1.0/storefront/storefront-world.availability-label.result"
            ),
            "{error}"
        );
    }
}

#[test]
fn component_inspection_exposes_the_canonical_model_to_packaging() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report = inspect::inspect(&root.join("tests/parcel.wasm")).expect("inspect component");
    assert_eq!(report["witContract"]["schema"], 1);
    assert_eq!(report["canonicalContract"]["schema"], 2);
    assert_eq!(
        report["canonicalContract"]["targetSupport"]["targets"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    let model: AbiModel = serde_json::from_value(report["witContract"].clone()).unwrap();
    assert_eq!(
        report["canonicalContract"],
        model.canonical_contract().unwrap()
    );
    assert_eq!(
        report["contractFingerprint"],
        model.contract_fingerprint().unwrap()
    );
    assert_eq!(report["bindingInventory"]["schema"], 1);
    assert_eq!(report["bindingInventory"]["functions"][0]["name"], "parcel");
    assert_eq!(
        report["bindingInventory"]["functions"][0]["interface"],
        "root-world"
    );
    assert_eq!(
        report["witContract"]["exports"][0]["functions"][0]["params"][0][0],
        "text"
    );
}

#[test]
fn world_type_aliases_are_not_reported_as_runtime_imports() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = root.join("tests/fixtures/wit-model-catalog");
    let temp = tempfile::tempdir().unwrap();
    let component = temp.path().join("component.wasm");
    let status = std::process::Command::new(fixture.join("build.sh"))
        .arg(&component)
        .status()
        .unwrap();
    assert!(status.success());
    let report = inspect::inspect(&component).unwrap();
    assert_eq!(report["imports"], serde_json::json!([]));
    assert!(report.get("importContract").is_none());
}

#[test]
fn canonical_contract_json_is_stable_and_contains_no_arena_ids() {
    let value = catalog_model().canonical_json().expect("canonical JSON");
    let bytes = serde_json::to_vec(&value).expect("serialize canonical model");
    assert_eq!(value["schema"], 1);
    assert_eq!(value["package"], "example:catalog@0.1.0");
    assert_eq!(value["world"], "storefront");
    assert_eq!(value["exports"][0]["functions"][0]["name"], "lookup");
    assert!(!String::from_utf8(bytes).unwrap().contains("typeId"));
}
