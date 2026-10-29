use super::*;

#[test]
fn packaged_worker_manifest_matches_cargo_normalization() {
    let metadata = crate::library_metadata::Metadata {
        description: Some("C standard library inside a Wasm-hosted compiler build".into()),
        license: Some("MIT".into()),
        ..Default::default()
    };
    let original = cargo_manifest("demo-c-math", "0.1.0", &metadata, false, false, true).unwrap();
    let expected = packaged_cargo_manifest("demo-c-math", &original).unwrap();
    // Produced by Cargo 1.98.1's package command, not by Wisp's normalizer.
    let actual: toml::Value = toml::from_str(include_str!(
        "../tests/fixtures/cargo-worker-normalized.toml"
    ))
    .unwrap();
    assert_eq!(expected, actual);
    for (dependency, changed) in [("serde_json", "9"), ("tempfile", "4")] {
        let mut forged = actual.clone();
        forged["target"]["cfg(not(target_arch = \"wasm32\"))"]["dependencies"][dependency]["version"] =
            changed.into();
        assert_ne!(expected, forged);
    }
    let mut forged = actual.clone();
    forged["package"]["build"] = true.into();
    assert_ne!(expected, forged);
}

fn manifests() -> Vec<String> {
    let mut manifests = Vec::new();
    for wasi in [false, true] {
        for asynchronous in [false, true] {
            for worker in [false, true] {
                manifests.push(
                    cargo_manifest(
                        "wisp-seed-check",
                        "0.1.0",
                        &crate::library_metadata::Metadata::default(),
                        wasi,
                        asynchronous,
                        worker,
                    )
                    .unwrap(),
                );
            }
        }
    }
    manifests
}

fn dependency(value: &toml::Value) -> toml::Table {
    match value.as_str() {
        Some(version) => toml::Table::from_iter([("version".into(), version.into())]),
        None => value.as_table().unwrap().clone(),
    }
}

#[test]
fn setup_seed_covers_generated_library_dependencies() {
    let seed: toml::Value =
        toml::from_str(include_str!("../assets/setup-cargo-seed/Cargo.toml.txt")).unwrap();
    let lock: toml::Value =
        toml::from_str(include_str!("../assets/setup-cargo-seed/Cargo.lock.txt")).unwrap();
    for manifest in manifests() {
        let manifest: toml::Value = toml::from_str(&manifest).unwrap();
        for target in manifest["target"].as_table().unwrap().values() {
            for (name, required) in target["dependencies"].as_table().unwrap() {
                let required = dependency(required);
                let seeded = dependency(
                    seed["dependencies"]
                        .get(name)
                        .unwrap_or_else(|| panic!("setup seed is missing {name}")),
                );
                assert_eq!(seeded["version"], required["version"], "{name}");
                if required
                    .get("default-features")
                    .and_then(toml::Value::as_bool)
                    != Some(false)
                {
                    assert_ne!(
                        seeded
                            .get("default-features")
                            .and_then(toml::Value::as_bool),
                        Some(false),
                        "{name} needs default features"
                    );
                }
                if let Some(features) = required.get("features") {
                    for feature in features.as_array().unwrap() {
                        assert!(
                            seeded["features"].as_array().unwrap().contains(feature),
                            "setup seed is missing {name}/{feature}"
                        );
                    }
                }
                let version =
                    semver::VersionReq::parse(required["version"].as_str().unwrap()).unwrap();
                assert!(
                    lock["package"].as_array().unwrap().iter().any(|package| {
                        package["name"].as_str() == Some(name)
                            && version
                                .matches(&package["version"].as_str().unwrap().parse().unwrap())
                    }),
                    "setup lock is missing {name}"
                );
            }
        }
    }
}

#[test]
#[ignore = "fetches the locked seed from crates.io into an isolated Cargo home"]
fn setup_seed_packages_generated_libraries_offline() {
    let directory = tempfile::tempdir().unwrap();
    let home = std::env::var_os("WISP_SETUP_TEST_CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| directory.path().join("cargo-home"));
    let seed = directory.path().join("seed");
    write(
        seed.join("Cargo.toml"),
        include_bytes!("../assets/setup-cargo-seed/Cargo.toml.txt"),
    )
    .unwrap();
    write(
        seed.join("Cargo.lock"),
        include_bytes!("../assets/setup-cargo-seed/Cargo.lock.txt"),
    )
    .unwrap();
    write(
        seed.join("src/lib.rs"),
        include_bytes!("../assets/setup-cargo-seed/lib.rs.txt"),
    )
    .unwrap();
    let output = Command::new("cargo")
        .args(["fetch", "--locked"])
        .current_dir(&seed)
        .env("CARGO_HOME", &home)
        .env("CARGO_NET_OFFLINE", "false")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    for (index, manifest) in manifests().iter().enumerate() {
        let library = directory.path().join(format!("library-{index}"));
        write(library.join("Cargo.toml"), manifest).unwrap();
        write(library.join("src/lib.rs"), "// Package resolution only.\n").unwrap();
        let output = Command::new("cargo")
            .args(["package", "--offline", "--no-verify", "--allow-dirty"])
            .current_dir(&library)
            .env("CARGO_HOME", &home)
            .env("CARGO_NET_OFFLINE", "true")
            .env("CARGO_TARGET_DIR", directory.path().join("target"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{manifest}\n{output:?}");
    }
}
