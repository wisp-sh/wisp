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

#[test]
fn go_jq_world_export_generates_direct_client_evaluate() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = std::fs::read(root.join("fixtures/jq/component.wasm")).unwrap();
    let wit_parser::decoding::DecodedWasm::Component(resolve, world) =
        wit_parser::decoding::decode(&bytes).unwrap()
    else {
        panic!("not a component")
    };
    let sources = library_go::sources(&resolve, world, &"a".repeat(64)).unwrap();
    assert!(
        sources.go.contains("func(view *Client) Evaluate("),
        "{}",
        sources.go
    );
    assert!(
        sources
            .rust
            .contains("client.bindings.call_evaluate(&mut client.store,"),
        "{}",
        sources.rust
    );
    assert!(sources.header.contains("_evaluate("));
}

#[test]
fn go_jq_world_uses_capability_free_native_host() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = std::fs::read(root.join("fixtures/jq/component.wasm")).unwrap();
    let wit_parser::decoding::DecodedWasm::Component(resolve, world) =
        wit_parser::decoding::decode(&bytes).unwrap()
    else {
        panic!("not a component")
    };
    let sources = library_go::sources(&resolve, world, &"a".repeat(64)).unwrap();
    assert!(sources.rust.contains("pure_wasi_linker(&mut linker)"));
    assert!(!sources.rust.contains("inherit_network()"));
}

#[test]
fn go_direct_export_preserves_named_interface_accessors() {
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve.push_str("mixed.wit", "package shop:test@0.1.0; interface stats { answer: func() -> u32; } world calculator { export stats; export evaluate: func(input: string) -> string; }").unwrap();
    let world = resolve
        .select_world(&[package], Some("calculator"))
        .unwrap();
    let sources = library_go::sources(&resolve, world, &"a".repeat(64)).unwrap();
    assert!(sources.go.contains("func(client *Client) Stats()*Stats"));
    assert!(sources.go.contains("func(view *Stats) Answer("));
    assert!(sources.go.contains("func(view *Client) Evaluate("));
    assert!(
        sources
            .rust
            .contains("client.bindings.shop_test_stats().func_answer()")
    );
}

#[test]
fn installed_go_jq_evaluates_real_component() {
    use std::{fs, process::Command};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = fs::read(root.join("fixtures/jq/component.wasm")).unwrap();
    let scratch = root.join("out");
    fs::create_dir_all(&scratch).unwrap();
    let temp = tempfile::tempdir_in(&scratch).unwrap();
    let package = temp.path().join("package");
    library_go::generate(&bytes, &package, "component.local/jq", "0.1.0").unwrap();
    let archive = library_go::module_archive(&package, "component.local/jq", "0.1.0").unwrap();
    let proxy = temp.path().join("proxy/component.local/jq/@v");
    fs::create_dir_all(&proxy).unwrap();
    for (name, value) in [
        ("v0.1.0.zip", archive.zip.as_slice()),
        ("v0.1.0.mod", archive.mod_file.as_slice()),
        ("v0.1.0.info", archive.info.as_slice()),
        ("list", b"v0.1.0\n".as_slice()),
    ] {
        fs::write(proxy.join(name), value).unwrap();
    }
    let consumer = temp.path().join("consumer");
    fs::create_dir(&consumer).unwrap();
    fs::write(
        consumer.join("go.mod"),
        "module consumer.local/jq\n\ngo 1.27.0\n\nrequire component.local/jq v0.1.0\n",
    )
    .unwrap();
    fs::write(consumer.join("jq_test.go"), r#"package jqtest
import (
    "errors"
    "testing"
    component "component.local/jq"
)
func TestEvaluate(t *testing.T) {
    client, err := component.NewClient(); if err != nil { t.Fatal(err) }; defer client.Close()
    got, err := client.Evaluate(".items[] | . * 2", `{"items":[3,5]}`)
    if err != nil { t.Fatal(err) }
    if len(got) != 2 || got[0] != "6" || got[1] != "10" { t.Fatalf("unexpected jq values: %#v",got) }
    _, err = client.Evaluate("invalid [", `{}`)
    if err == nil { t.Fatal("invalid filter was accepted") }
    var guest *component.CallError[string]
    if !errors.As(err, &guest) || guest.Guest == nil { t.Fatalf("expected guest error, got %T: %v",err,err) }
    got, err = client.Evaluate(".", `{"ok":true}`)
    if err != nil || len(got) != 1 || got[0] != `{"ok":true}` { t.Fatalf("recovery: %#v, %v",got,err) }
    _, err = client.Evaluate("now", `null`)
    var denied *component.CallError[string]
    if !errors.As(err, &denied) || denied.Runtime == "" { t.Fatalf("clock was not denied: %v",err) }
    _, err = client.Evaluate(".", `42`)
    if err == nil { t.Fatal("trapped client was not poisoned") }
    fresh, err := component.NewClient(); if err != nil { t.Fatal(err) }; defer fresh.Close()
    got, err = fresh.Evaluate(".", `42`)
    if err != nil || len(got) != 1 || got[0] != "42" { t.Fatalf("fresh client: %#v, %v",got,err) }
}
"#).unwrap();
    let run = |cmd: &mut Command| {
        let result = cmd
            .current_dir(&consumer)
            .env("GOTOOLCHAIN", "local")
            .env("GOSUMDB", "off")
            .env("GOMODCACHE", temp.path().join("modcache"))
            .env("GOCACHE", temp.path().join("gocache"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{cmd:?}\n{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    };
    run(Command::new("go").args(["mod", "download", "all"]).env(
        "GOPROXY",
        format!("file://{}", temp.path().join("proxy").display()),
    ));
    fs::remove_dir_all(temp.path().join("proxy")).unwrap();
    fs::remove_dir_all(package).unwrap();
    run(Command::new("go")
        .args(["test", "-count=1", "-v", "./..."])
        .env("GOPROXY", "off"));
}

#[test]
fn go_resource_source_uses_typed_handles_and_ownership_receipts() {
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve
        .push_str(
            "resources.wit",
            include_str!("../../../tests/library-resources/library/wit/world.wit"),
        )
        .unwrap();
    let world = resolve.select_world(&[package], Some("library")).unwrap();
    let sources = library_go::sources(&resolve, world, &"a".repeat(64)).unwrap();
    assert!(sources.rust.contains("commit_resources"));
    assert!(sources.go.contains("NewCounter"));
    assert!(sources.go.contains("CounterCombine"));
    assert!(sources.values.contains("CountersCounter"));
}

#[test]
fn go_rejects_shared_type_dag_before_source_expansion() {
    let mut wit = String::from(
        "package test:wide@0.1.0; interface values { record node0 { left: string, right: string }\n",
    );
    for index in 1..15 {
        wit.push_str(&format!(
            "record node{index} {{ left: node{}, right: node{} }}\n",
            index - 1,
            index - 1
        ));
    }
    wit.push_str("echo: func(value: node14) -> node14; } world library { export values; }");
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve.push_str("wide.wit", &wit).unwrap();
    let world = resolve.select_world(&[package], Some("library")).unwrap();
    let error = library_go::sources(&resolve, world, &"a".repeat(64))
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("expanded value limit"), "{error}");
}

#[test]
fn installed_go_resource_library_executes_after_source_and_proxy_removal() {
    use std::{fs, process::Command};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scratch = root.join("out");
    fs::create_dir_all(&scratch).unwrap();
    let temp = tempfile::tempdir_in(&scratch).unwrap();
    let run = |command: &mut Command| {
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{command:?}\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    // Keep the fixture build in task-owned workspace scratch, which is removed
    // before the separate native archive build. Avoid duplicate debug caches.
    let build = temp.path().join("fixture-build");
    run(Command::new("cargo")
        .env("RUSTUP_TOOLCHAIN", "1.98.1")
        .env("CARGO_BUILD_JOBS", "1")
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_PROFILE_DEV_DEBUG", "0")
        .args([
            "build",
            "--locked",
            "--offline",
            "--target",
            "wasm32-unknown-unknown",
            "--manifest-path",
        ])
        .arg(root.join("tests/library-resources/library/Cargo.toml"))
        .arg("--target-dir")
        .arg(&build));
    let component = temp.path().join("component.wasm");
    run(
        Command::new(root.join(".tools/component-library-chain/wasm-tools"))
            .args(["component", "new"])
            .arg(build.join("wasm32-unknown-unknown/debug/wisp_resource_library.wasm"))
            .arg("-o")
            .arg(&component),
    );
    let bytes = fs::read(&component).unwrap();
    fs::remove_dir_all(&build).unwrap();
    let package = temp.path().join("package");
    let report =
        library_go::generate(&bytes, &package, "component.local/resources", "0.1.0").unwrap();
    let archive =
        library_go::module_archive(&package, "component.local/resources", "0.1.0").unwrap();
    let proxy = temp.path().join("proxy/component.local/resources/@v");
    fs::create_dir_all(&proxy).unwrap();
    for (name, bytes) in [
        ("v0.1.0.zip", archive.zip.as_slice()),
        ("v0.1.0.mod", archive.mod_file.as_slice()),
        ("v0.1.0.info", archive.info.as_slice()),
        ("list", b"v0.1.0\n".as_slice()),
    ] {
        fs::write(proxy.join(name), bytes).unwrap();
    }
    let consumer = temp.path().join("consumer");
    fs::create_dir(&consumer).unwrap();
    fs::write(consumer.join("go.mod"),"module consumer.local/resources\n\ngo 1.27.0\n\nrequire component.local/resources v0.1.0\n").unwrap();
    fs::write(
        consumer.join("resource_test.go"),
        include_str!("../../../tests/library-resources/go_test.go"),
    )
    .unwrap();
    let command = |args: &[&str], proxy: &str| {
        let mut command = Command::new("go");
        command
            .current_dir(&consumer)
            .env("GOTOOLCHAIN", "local")
            .env("GOSUMDB", "off")
            .env("GOPROXY", proxy)
            .env("GOMODCACHE", temp.path().join("modcache"))
            .env("GOCACHE", temp.path().join("gocache"))
            .args(args);
        command
    };
    run(&mut command(
        &["mod", "download", "all"],
        &format!("file://{}", temp.path().join("proxy").display()),
    ));
    fs::remove_dir_all(temp.path().join("proxy")).unwrap();
    fs::remove_dir_all(&package).unwrap();
    let output = run(&mut command(&["test", "-count=1", "-v", "./..."], "off"));
    let evidence = root.join("evidence").join(format!(
        "library-go-resources-{}",
        report["platform"].as_str().unwrap()
    ));
    fs::create_dir_all(&evidence).unwrap();
    fs::write(
        evidence.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    fs::write(
        evidence.join("consumer.log"),
        [output.stdout, output.stderr].concat(),
    )
    .unwrap();
}
