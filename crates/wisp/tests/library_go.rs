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

use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, process::Command};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn success(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn generated_go_executes_nested_types_typed_errors_and_shared_interfaces() {
    let root = root();
    let temp = tempfile::tempdir().unwrap();
    let build = root.join("out/library-go-fixture-build");
    success(
        Command::new("cargo")
            .env("RUSTUP_TOOLCHAIN", "1.98.1")
            .args([
                "build",
                "--locked",
                "--offline",
                "--target",
                "wasm32-unknown-unknown",
                "--manifest-path",
            ])
            .arg(root.join("tests/library-chain/general/library/Cargo.toml"))
            .arg("--target-dir")
            .arg(&build),
    );
    let component = temp.path().join("catalog.wasm");
    success(
        Command::new(root.join(".tools/component-library-chain/wasm-tools"))
            .args(["component", "new"])
            .arg(build.join("wasm32-unknown-unknown/debug/catalog_library.wasm"))
            .arg("-o")
            .arg(&component),
    );
    let package = temp.path().join("package");
    let bytes = fs::read(component).unwrap();
    let notices = std::collections::BTreeMap::from([
        (
            "WISP-NOTICES.txt".to_owned(),
            b"Native library notices are included in WISP-GO-LICENSES.json.\n".to_vec(),
        ),
        (
            "notices/AUTHOR.txt".to_owned(),
            b"Fixture notice for admission tests.\n".to_vec(),
        ),
    ]);
    let report = library_go::generate_with_notices(
        &bytes,
        &package,
        "component.local/catalog",
        "0.1.0",
        &notices,
    )
    .unwrap();
    let reproduced = library_go::generate(
        &bytes,
        &temp.path().join("reproduced"),
        "component.local/catalog",
        "0.1.0",
    )
    .unwrap();
    assert_eq!(
        report["nativeSha256"], reproduced["nativeSha256"],
        "native archive differs between fresh build directories"
    );
    library_go::verify_output_with_notices(
        &bytes,
        "component.local/catalog",
        "0.1.0",
        &package,
        &notices,
    )
    .unwrap();
    let archive = library_go::module_archive(&package, "component.local/catalog", "0.1.0").unwrap();
    let repeated =
        library_go::module_archive(&package, "component.local/catalog", "0.1.0").unwrap();
    assert_eq!(
        archive.zip, repeated.zip,
        "Go module archive must be reproducible"
    );
    library_go::verify_archive_with_notices(
        &bytes,
        "component.local/catalog",
        "0.1.0",
        &archive.zip,
        &notices,
    )
    .unwrap();
    let bridge = package.join("bridge.go");
    let original = fs::read(&bridge).unwrap();
    fs::write(&bridge, b"package component\nfunc Untrusted(){}\n").unwrap();
    assert!(
        library_go::verify_output_with_notices(
            &bytes,
            "component.local/catalog",
            "0.1.0",
            &package,
            &notices
        )
        .unwrap_err()
        .to_string()
        .contains("source mismatch")
    );
    fs::write(bridge, original).unwrap();
    let native = package.join(format!(
        "native/{}/libwisp_go.a",
        report["platform"].as_str().unwrap()
    ));
    fs::write(&native, b"arbitrary forged native code").unwrap();
    let mut forged = report.clone();
    forged["nativeSha256"] = json!(format!(
        "{:x}",
        Sha256::digest(b"arbitrary forged native code")
    ));
    fs::write(
        package.join("go-library.json"),
        serde_json::to_vec_pretty(&forged).unwrap(),
    )
    .unwrap();
    assert!(
        library_go::verify_output_with_notices(
            &bytes,
            "component.local/catalog",
            "0.1.0",
            &package,
            &notices
        )
        .unwrap_err()
        .to_string()
        .contains("regenerated native archive mismatch")
    );
    fs::copy(
        temp.path().join("reproduced").join(format!(
            "native/{}/libwisp_go.a",
            report["platform"].as_str().unwrap()
        )),
        &native,
    )
    .unwrap();
    fs::write(
        package.join("go-library.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    let evidence = root.join("evidence").join(format!(
        "library-go-{}",
        report["platform"].as_str().unwrap()
    ));
    fs::create_dir_all(&evidence).unwrap();
    fs::write(
        evidence.join("package.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    fs::write(evidence.join("module.json"), serde_json::to_vec_pretty(&json!({"module":"component.local/catalog","version":"0.1.0","archiveSha256":format!("{:x}",Sha256::digest(&archive.zip)),"archiveBytes":archive.zip.len(),"nativeRebuildIdentical":true,"forgedNativeAndReportRejected":true,"sourceSubstitutionRejected":true})).unwrap()).unwrap();
    let proxy = temp.path().join("proxy/component.local/catalog/@v");
    fs::create_dir_all(&proxy).unwrap();
    fs::write(proxy.join("v0.1.0.zip"), &archive.zip).unwrap();
    fs::write(proxy.join("v0.1.0.mod"), &archive.mod_file).unwrap();
    fs::write(proxy.join("v0.1.0.info"), &archive.info).unwrap();
    fs::write(proxy.join("list"), "v0.1.0\n").unwrap();
    let consumer = temp.path().join("consumer");
    fs::create_dir(&consumer).unwrap();
    fs::write(
        consumer.join("go.mod"),
        "module consumer.local/check\n\ngo 1.27.0\n\nrequire component.local/catalog v0.1.0\n",
    )
    .unwrap();
    fs::write(
        consumer.join("catalog_test.go"),
        include_str!("../../../tests/library-chain/general/go_test.go"),
    )
    .unwrap();
    let configure = |command: &mut Command| {
        command
            .current_dir(&consumer)
            .env("GOSUMDB", "off")
            .env("GOTOOLCHAIN", "local")
            .env("GOMODCACHE", temp.path().join("modcache"))
            .env("GOCACHE", temp.path().join("gocache"));
    };
    let mut download = Command::new("go");
    configure(&mut download);
    download
        .env(
            "GOPROXY",
            format!("file://{}", temp.path().join("proxy").display()),
        )
        .args(["mod", "download", "all"]);
    success(&mut download);
    fs::remove_dir_all(temp.path().join("proxy")).unwrap();
    fs::remove_dir_all(&package).unwrap();
    let mut test = Command::new("go");
    configure(&mut test);
    test.env("GOPROXY", "off")
        .args(["test", "-count=1", "-v", "./..."]);
    success(&mut test);
    fs::write(
        evidence.join("consumer.txt"),
        "ordinary GOPROXY install, isolated caches, source/proxy removed, GOPROXY=off: PASS\n",
    )
    .unwrap();
}

#[test]
fn module_archive_rejects_path_confusion_symlinks_and_entry_floods() {
    use std::io::{Cursor, Write};
    for (path, expected) in [
        (
            "component.local/catalog@v0.1.0/../../outside",
            "unsafe or duplicate path",
        ),
        (
            "component.local/catalog@v0.1.0/a\\b",
            "unsafe or duplicate path",
        ),
        (
            "component.local/catalog@v0.1.0/C:outside",
            "unsafe or duplicate path",
        ),
        ("other.local/catalog@v0.1.0/go.mod", "prefix mismatch"),
    ] {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"invalid").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let error = library_go::verify_archive(&[], "component.local/catalog", "0.1.0", &bytes)
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error:#}");
    }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.add_symlink(
        "component.local/catalog@v0.1.0/link",
        "outside",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    assert!(
        library_go::verify_archive(&[], "component.local/catalog", "0.1.0", &bytes)
            .unwrap_err()
            .to_string()
            .contains("non-regular")
    );
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..129 {
        zip.start_file(
            format!("component.local/catalog@v0.1.0/file-{index}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    assert!(
        library_go::verify_archive(&[], "component.local/catalog", "0.1.0", &bytes)
            .unwrap_err()
            .to_string()
            .contains("entry count")
    );
}

#[test]
fn module_identity_requires_canonical_versions_and_major_suffixes() {
    for (module, version) in [
        ("example.org/thing", "0.1.0"),
        ("example.org/thing", "1.2.3-rc.1"),
        ("example.org/thing/v2", "2.0.0"),
    ] {
        library_go::validate_module(module, version).unwrap();
    }
    for (module, version) in [
        ("example.org/thing", "2.0.0"),
        ("example.org/thing/v2", "1.0.0"),
        ("example.org/Thing", "1.0.0"),
        ("example.org/../thing", "1.0.0"),
        ("example.org/thing", "1.0.0+build"),
    ] {
        assert!(library_go::validate_module(module, version).is_err());
    }
}
