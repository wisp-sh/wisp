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
fn installed_go_network_library_runs_offline() {
    use std::{fs, process::Command};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();
    let build = root.join("out/library-go-network-fixture-build");
    let build_result = Command::new("cargo")
        .env("RUSTUP_TOOLCHAIN", "1.98.1")
        .args([
            "build",
            "--locked",
            "--offline",
            "--target",
            "wasm32-unknown-unknown",
            "--manifest-path",
        ])
        .arg(root.join("tests/library-network/library/Cargo.toml"))
        .arg("--target-dir")
        .arg(&build)
        .output()
        .unwrap();
    assert!(
        build_result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&build_result.stdout),
        String::from_utf8_lossy(&build_result.stderr)
    );
    let component_path = temp.path().join("component.wasm");
    let assembled = Command::new(root.join(".tools/component-library-chain/wasm-tools"))
        .args(["component", "new"])
        .arg(build.join("wasm32-unknown-unknown/debug/wisp_network_fixture.wasm"))
        .arg("-o")
        .arg(&component_path)
        .output()
        .unwrap();
    assert!(
        assembled.status.success(),
        "{}{}",
        String::from_utf8_lossy(&assembled.stdout),
        String::from_utf8_lossy(&assembled.stderr)
    );
    let component = fs::read(&component_path).unwrap();
    let package = temp.path().join("package");
    let report =
        library_go::generate(&component, &package, "component.local/network", "0.1.0").unwrap();
    let archive = library_go::module_archive(&package, "component.local/network", "0.1.0").unwrap();
    let proxy = temp.path().join("proxy/component.local/network/@v");
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
    fs::write(
        consumer.join("go.mod"),
        "module consumer.local/network\n\ngo 1.27.0\n\nrequire component.local/network v0.1.0\n",
    )
    .unwrap();
    fs::write(
        consumer.join("network_test.go"),
        include_str!("../../../tests/library-network/go_test.go"),
    )
    .unwrap();
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
        "library-go-network-{}",
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
