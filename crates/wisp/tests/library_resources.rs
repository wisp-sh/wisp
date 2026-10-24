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
fn resources_follow_resolve_and_share_library_ownership() {
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve
        .push_str(
            "resources.wit",
            r#"
        package shop:widgets@2.3.4;
        interface widgets {
            resource widget {
                constructor(value: u32);
                value: func() -> u32;
                combine: static func(left: borrow<widget>, right: own<widget>) -> own<widget>;
            }
            make: func(value: u32) -> own<widget>;
        }
        world library { export widgets; }
    "#,
        )
        .unwrap();
    let id = resolve.packages[package].interfaces["widgets"];
    let definitions = library_resources::definitions(
        &resolve,
        id,
        "shop::widgets::widgets",
        "shop_widgets_widgets",
    )
    .unwrap();
    assert!(definitions.contains("pub struct Widget"));
    assert!(definitions.contains("pub fn close(&mut self)"));
    assert!(definitions.contains("impl Drop for Widget"));
    assert!(definitions.contains("pub fn combine(__wisp_client: &mut Client, __wisp_arg_0: &'_ super::widgets::Widget, mut __wisp_arg_1: super::widgets::Widget)"));
    assert!(definitions.contains("same_instance"));
    assert!(definitions.contains("resource_drop"));
    assert!(!definitions.contains("counter"));
    let make = &resolve.interfaces[id].functions["make"];
    let method = library_resources::client_method(
        &resolve,
        make,
        "shop::widgets::widgets",
        "shop_widgets_widgets",
    )
    .unwrap()
    .unwrap();
    assert!(method.contains("Result<super::widgets::Widget, CallError<std::convert::Infallible>>"));
}

#[test]
fn shared_type_dag_is_rejected_before_expansion() {
    let mut wit =
        String::from("package shop:wide@1.0.0; interface values { record level0 { value: u32 } ");
    for index in 1..14 {
        wit.push_str(&format!(
            "record level{index} {{ left: level{}, right: level{} }} ",
            index - 1,
            index - 1
        ));
    }
    wit.push_str("read: func(value: level13) -> level13; } world library { export values; }");
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve.push_str("wide.wit", &wit).unwrap();
    let world = resolve.packages[package].worlds["library"];
    let error = library::validate_type_expansion(&resolve, world)
        .unwrap_err()
        .to_string();
    assert!(error.contains("expanded value limit"), "{error}");
}

#[test]
fn deep_type_chain_is_rejected_before_recursive_generation() {
    let mut wit =
        String::from("package shop:deep@1.0.0; interface values { record level0 { value: u32 } ");
    for index in 1..72 {
        wit.push_str(&format!(
            "record level{index} {{ previous: level{} }} ",
            index - 1
        ));
    }
    wit.push_str("read: func(value: level71) -> level71; } world library { export values; }");
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve.push_str("deep.wit", &wit).unwrap();
    let world = resolve.packages[package].worlds["library"];
    let error = library::validate_type_expansion(&resolve, world)
        .unwrap_err()
        .to_string();
    assert!(error.contains("depth limit"), "{error}");
}
