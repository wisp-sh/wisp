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
fn installed_go_error_context_library_runs_offline() {
    use std::{fs, process::Command};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();
    let tools = root.join(".tools/component-library-chain/wasm-tools");
    let core = temp.path().join("core.wasm");
    let component = temp.path().join("component.wasm");
    let embedded = temp.path().join("embedded.wasm");
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
    run(Command::new(&tools)
        .arg("parse")
        .arg(root.join("tests/library-error-context/core.wat"))
        .arg("-o")
        .arg(&core));
    run(Command::new(&tools)
        .args(["component", "embed"])
        .arg(root.join("tests/library-error-context/wit"))
        .arg(&core)
        .arg("-o")
        .arg(&embedded));
    run(Command::new(&tools)
        .args(["component", "new"])
        .arg(&embedded)
        .arg("-o")
        .arg(&component));
    let bytes = fs::read(component).unwrap();
    let package = temp.path().join("package");
    let report =
        library_go::generate(&bytes, &package, "component.local/context-values", "0.1.0").unwrap();
    let archive =
        library_go::module_archive(&package, "component.local/context-values", "0.1.0").unwrap();
    let proxy = temp.path().join("proxy/component.local/context-values/@v");
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
    fs::write(consumer.join("go.mod"),"module consumer.local/context-values\n\ngo 1.27.0\n\nrequire component.local/context-values v0.1.0\n").unwrap();
    fs::write(
        consumer.join("context_test.go"),
        include_str!("../../../tests/library-error-context/go_test.go"),
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
        "library-go-error-context-{}",
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
