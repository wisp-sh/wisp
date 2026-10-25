#[allow(dead_code)]
#[path = "../src/dependencies.rs"]
mod dependencies;
#[allow(dead_code)]
#[path = "../src/identity.rs"]
mod identity;
#[allow(dead_code)]
#[path = "../src/inspect.rs"]
mod inspect;
#[allow(dead_code)]
#[path = "../src/limits.rs"]
mod limits;
#[allow(dead_code)]
#[path = "../src/release.rs"]
mod release;
#[allow(dead_code)]
#[path = "../src/wit_model.rs"]
mod wit_model;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn tool(name: &str) -> PathBuf {
    root().join(".tools/component-library-chain").join(name)
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Fixture {
    temp: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        Self {
            temp: tempfile::tempdir().unwrap(),
        }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }
    fn package(
        &self,
        directory: &Path,
        identity: &str,
        version: &str,
        imports: &[(&str, &str, &str)],
        value: u32,
    ) -> PathBuf {
        fs::create_dir_all(directory).unwrap();
        let export = format!("{identity}/api@1.0.0");
        let mut wat = String::from("(component\n");
        for (index, (interface, _, _)) in imports.iter().enumerate() {
            wat.push_str(&format!("(import {interface:?} (instance $i{index} (export \"value\" (func (result u32)))))\n"));
        }
        if imports.is_empty() {
            wat.push_str(&format!("(core module $m (func (export \"value\") (result i32) i32.const {value}))\n(core instance $m (instantiate $m))\n(func $value (result u32) (canon lift (core func $m \"value\")))\n"));
        } else {
            wat.push_str("(alias export $i0 \"value\" (func $value))\n");
        }
        wat.push_str(&format!("(instance $api (export \"value\" (func $value)))\n(export {export:?} (instance $api)))\n"));
        let source = directory.join("component.wat");
        let component = directory.join("component.wasm");
        fs::write(&source, wat).unwrap();
        let result = Command::new(tool("wasm-tools"))
            .args(["parse"])
            .arg(&source)
            .arg("-o")
            .arg(&component)
            .output()
            .expect("provision Component chain tools first");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let requirements = imports
            .iter()
            .map(|(interface, package, version)| {
                (
                    (*interface).to_owned(),
                    json!({"package": package, "version": version}),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let manifest = directory.join("component.json");
        fs::write(&manifest, serde_json::to_vec_pretty(&json!({
            "schema":1,"kind":"component-dependency-package","identity":identity,"version":version,
            "component":{"path":"component.wasm","sha256":sha(&fs::read(component).unwrap())},
            "dependencies":requirements,"hostImports":[],
        })).unwrap()).unwrap();
        manifest
    }
    fn registry(&self, identity: &str, version: &str, imports: &[(&str, &str, &str)], value: u32) {
        let (namespace, name) = identity.split_once(':').unwrap();
        self.package(
            &self
                .path("registry")
                .join(namespace)
                .join(name)
                .join(version),
            identity,
            version,
            imports,
            value,
        );
    }
    fn chain(&self) -> PathBuf {
        self.registry("demo:a", "1.0.0", &[], 21);
        self.registry(
            "demo:b",
            "1.0.0",
            &[("demo:a/api@1.0.0", "demo:a", "^1.0")],
            0,
        );
        self.package(
            &self.path("root"),
            "demo:c",
            "1.0.0",
            &[
                ("demo:b/api@1.0.0", "demo:b", "^1.0"),
                ("demo:a/api@1.0.0", "demo:a", "^1.0"),
            ],
            0,
        )
    }
    fn lock(&self, manifest: &Path, update: Option<&str>) -> anyhow::Result<dependencies::Lock> {
        dependencies::resolve_lock(
            manifest,
            &self.path("registry"),
            &self.path("cache"),
            &self.path("component.lock.json"),
            update,
        )
    }
    fn execute(&self, directory: &str) -> String {
        let result = Command::new(tool("wasmtime"))
            .args(["run", "--invoke", "value()"])
            .arg(self.path(directory).join("component.wasm"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap().trim().into()
    }
    fn rewrite_component(&self, directory: &Path, rewrite: impl FnOnce(String) -> String) {
        let wat = directory.join("component.wat");
        fs::write(&wat, rewrite(fs::read_to_string(&wat).unwrap())).unwrap();
        let component = directory.join("component.wasm");
        let status = Command::new(tool("wasm-tools"))
            .arg("parse")
            .arg(wat)
            .arg("-o")
            .arg(&component)
            .status()
            .unwrap();
        assert!(status.success());
        let manifest = directory.join("component.json");
        let mut record: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        record["component"]["sha256"] = sha(&fs::read(component).unwrap()).into();
        fs::write(manifest, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    }
}

#[test]
fn transitive_shared_components_compose_and_execute_from_cache() {
    let fixture = Fixture::new();
    let manifest = fixture.chain();
    let lock = fixture.lock(&manifest, None).unwrap();
    assert_eq!(lock.nodes.len(), 3);
    fs::remove_dir_all(fixture.path("registry")).unwrap();
    dependencies::compose_lock(
        &fixture.path("component.lock.json"),
        &fixture.path("cache"),
        &fixture.path("composed"),
    )
    .unwrap();
    assert_eq!(fixture.execute("composed"), "21");
}

#[test]
fn a_lock_retains_bytes_until_a_targeted_update() {
    let fixture = Fixture::new();
    let manifest = fixture.chain();
    let first = fixture.lock(&manifest, None).unwrap();
    let locked_bytes = fs::read(fixture.path("component.lock.json")).unwrap();
    fixture.registry("demo:a", "1.1.0", &[], 34);
    let retained = fixture.lock(&manifest, None).unwrap();
    assert_eq!(first, retained);
    assert_eq!(
        fs::read(fixture.path("component.lock.json")).unwrap(),
        locked_bytes
    );
    let updated = fixture.lock(&manifest, Some("demo:a")).unwrap();
    assert!(updated.nodes.contains_key("demo:a@1.1.0"));
    assert!(updated.nodes.contains_key("demo:b@1.0.0"));
    assert!(!updated.nodes.contains_key("demo:a@1.0.0"));
    dependencies::compose_lock(
        &fixture.path("component.lock.json"),
        &fixture.path("cache"),
        &fixture.path("updated"),
    )
    .unwrap();
    assert_eq!(fixture.execute("updated"), "34");
}

#[test]
fn replacement_bytes_under_a_locked_version_are_rejected() {
    let fixture = Fixture::new();
    let manifest = fixture.chain();
    fixture.lock(&manifest, None).unwrap();
    fixture.registry("demo:a", "1.0.0", &[], 99);
    let error = fixture.lock(&manifest, None).unwrap_err();
    assert!(
        error.to_string().contains("locked identity changed"),
        "{error:#}"
    );
}

#[test]
fn conflicting_shared_requirements_and_cycles_fail_before_lock_output() {
    let fixture = Fixture::new();
    fixture.registry("demo:a", "1.0.0", &[], 1);
    fixture.registry("demo:a", "2.0.0", &[], 2);
    fixture.registry(
        "demo:b",
        "1.0.0",
        &[("demo:a/api@1.0.0", "demo:a", "^2.0")],
        0,
    );
    let manifest = fixture.package(
        &fixture.path("root"),
        "demo:c",
        "1.0.0",
        &[
            ("demo:a/api@1.0.0", "demo:a", "^1.0"),
            ("demo:b/api@1.0.0", "demo:b", "^1.0"),
        ],
        0,
    );
    assert!(
        fixture
            .lock(&manifest, None)
            .unwrap_err()
            .to_string()
            .contains("incompatible")
    );
    assert!(!fixture.path("component.lock.json").exists());
    fixture.registry(
        "demo:a",
        "1.0.0",
        &[("demo:b/api@1.0.0", "demo:b", "^1.0")],
        1,
    );
    fixture.registry(
        "demo:b",
        "1.0.0",
        &[("demo:a/api@1.0.0", "demo:a", "^1.0")],
        0,
    );
    assert!(
        fixture
            .lock(&manifest, None)
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
    assert!(!fixture.path("component.lock.json").exists());
}

#[test]
fn tampered_cached_component_is_rejected_before_composition_output() {
    let fixture = Fixture::new();
    let manifest = fixture.chain();
    let lock = fixture.lock(&manifest, None).unwrap();
    let node = &lock.nodes["demo:a@1.0.0"];
    fs::write(
        fixture
            .path("cache/components")
            .join(format!("{}.wasm", node.sha256)),
        b"substituted",
    )
    .unwrap();
    let error = dependencies::compose_lock(
        &fixture.path("component.lock.json"),
        &fixture.path("cache"),
        &fixture.path("composed"),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("cache checksum mismatch"),
        "{error:#}"
    );
    assert!(!fixture.path("composed").exists());
}

#[test]
fn missing_packages_and_incompatible_wit_are_rejected_before_lock_output() {
    let fixture = Fixture::new();
    let manifest = fixture.package(
        &fixture.path("root"),
        "demo:b",
        "1.0.0",
        &[("demo:a/api@1.0.0", "demo:a", "^1.0")],
        0,
    );
    assert!(
        fixture
            .lock(&manifest, None)
            .unwrap_err()
            .to_string()
            .contains("missing Component package")
    );
    fixture.registry("demo:a", "1.0.0", &[], 1);
    fixture.rewrite_component(&fixture.path("registry/demo/a/1.0.0"), |wat| {
        wat.replace("result u32", "result u64")
            .replace("result i32", "result i64")
            .replace("i32.const", "i64.const")
    });
    let error = fixture.lock(&manifest, None).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("interface compatibility failed before execution"),
        "{error:#}"
    );
    assert!(!fixture.path("component.lock.json").exists());
}

#[test]
fn two_versions_require_distinct_exact_interface_identities() {
    let fixture = Fixture::new();
    fixture.registry("demo:a", "1.0.0", &[], 21);
    fixture.registry("demo:a", "2.0.0", &[], 34);
    fixture.rewrite_component(&fixture.path("registry/demo/a/2.0.0"), |wat| {
        wat.replace("demo:a/api@1.0.0", "demo:a/api@2.0.0")
    });
    let manifest = fixture.package(
        &fixture.path("root"),
        "demo:c",
        "1.0.0",
        &[
            ("demo:a/api@1.0.0", "demo:a", "=1.0.0"),
            ("demo:a/api@2.0.0", "demo:a", "=2.0.0"),
        ],
        0,
    );
    let lock = fixture.lock(&manifest, None).unwrap();
    assert_eq!(lock.nodes.len(), 3);
    assert!(lock.nodes.contains_key("demo:a@1.0.0"));
    assert!(lock.nodes.contains_key("demo:a@2.0.0"));
    dependencies::compose_lock(
        &fixture.path("component.lock.json"),
        &fixture.path("cache"),
        &fixture.path("composed"),
    )
    .unwrap();
    assert_eq!(fixture.execute("composed"), "21");
}

#[test]
fn declared_host_imports_remain_at_the_outer_boundary() {
    let fixture = Fixture::new();
    fixture.registry(
        "demo:a",
        "1.0.0",
        &[("host:clock/ticks@1.0.0", "host:clock", "^1.0")],
        0,
    );
    let provider = fixture.path("registry/demo/a/1.0.0/component.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&provider).unwrap()).unwrap();
    record["dependencies"] = json!({});
    record["hostImports"] = json!(["host:clock/ticks@1.0.0"]);
    fs::write(provider, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let manifest = fixture.package(
        &fixture.path("root"),
        "demo:b",
        "1.0.0",
        &[("demo:a/api@1.0.0", "demo:a", "^1.0")],
        0,
    );
    fixture.lock(&manifest, None).unwrap();
    let report = dependencies::compose_lock(
        &fixture.path("component.lock.json"),
        &fixture.path("cache"),
        &fixture.path("composed"),
    )
    .unwrap();
    assert_eq!(report["hostImports"], json!(["host:clock/ticks@1.0.0"]));
}

#[test]
fn reinstall_restores_exact_locked_bytes_and_composition_is_reproducible() {
    let fixture = Fixture::new();
    let manifest = fixture.chain();
    let first = fixture.lock(&manifest, None).unwrap();
    let report_a = dependencies::compose_lock(
        &fixture.path("component.lock.json"),
        &fixture.path("cache"),
        &fixture.path("first"),
    )
    .unwrap();
    fs::remove_dir_all(fixture.path("cache")).unwrap();
    assert_eq!(fixture.lock(&manifest, None).unwrap(), first);
    let report_b = dependencies::compose_lock(
        &fixture.path("component.lock.json"),
        &fixture.path("cache"),
        &fixture.path("second"),
    )
    .unwrap();
    assert_eq!(report_a["componentSha256"], report_b["componentSha256"]);
    assert_eq!(
        fs::read(fixture.path("first/component.wasm")).unwrap(),
        fs::read(fixture.path("second/component.wasm")).unwrap()
    );
}

#[test]
fn invalid_locked_edges_and_existing_output_are_preserved_on_rejection() {
    let fixture = Fixture::new();
    let manifest = fixture.chain();
    fixture.lock(&manifest, None).unwrap();
    fs::create_dir(fixture.path("existing")).unwrap();
    fs::write(fixture.path("existing/keep"), "keep").unwrap();
    assert!(
        dependencies::compose_lock(
            &fixture.path("component.lock.json"),
            &fixture.path("cache"),
            &fixture.path("existing")
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(fixture.path("existing/keep")).unwrap(),
        "keep"
    );
    let file = fixture.path("component.lock.json");
    let mut lock: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    lock["nodes"]["demo:b@1.0.0"]["dependencies"]["demo:a/api@1.0.0"] = "demo:b@1.0.0".into();
    fs::write(&file, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
    assert!(
        dependencies::compose_lock(&file, &fixture.path("cache"), &fixture.path("invalid"))
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
    assert!(!fixture.path("invalid").exists());
}

#[test]
fn incompatible_latest_version_backtracks_without_loosening_requirements() {
    let fixture = Fixture::new();
    fixture.registry("demo:a", "1.0.0", &[], 21);
    fixture.registry("demo:a", "2.0.0", &[], 34);
    fixture.registry(
        "demo:b",
        "1.0.0",
        &[("demo:a/api@1.0.0", "demo:a", "^1.0")],
        0,
    );
    fixture.registry(
        "demo:b",
        "1.1.0",
        &[("demo:a/api@1.0.0", "demo:a", "^2.0")],
        0,
    );
    let manifest = fixture.package(
        &fixture.path("root"),
        "demo:c",
        "1.0.0",
        &[
            ("demo:a/api@1.0.0", "demo:a", "^1.0"),
            ("demo:b/api@1.0.0", "demo:b", "^1.0"),
        ],
        0,
    );
    let lock = fixture.lock(&manifest, None).unwrap();
    assert!(lock.nodes.contains_key("demo:b@1.0.0"));
    assert!(lock.nodes.contains_key("demo:a@1.0.0"));
}

#[test]
fn deps_cli_resolves_and_executes_the_transitive_chain() {
    let fixture = Fixture::new();
    let manifest = fixture.chain();
    let binary = env!("CARGO_BIN_EXE_wisp");
    let result = Command::new(binary)
        .args(["deps", "lock"])
        .arg(manifest)
        .arg("--registry")
        .arg(fixture.path("registry"))
        .arg("--cache")
        .arg(fixture.path("cache"))
        .arg("--out")
        .arg(fixture.path("component.lock.json"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let lock: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(lock["nodes"].as_object().unwrap().len(), 3);
    let result = Command::new(binary)
        .args(["deps", "compose"])
        .arg(fixture.path("component.lock.json"))
        .arg("--cache")
        .arg(fixture.path("cache"))
        .arg("--out")
        .arg(fixture.path("cli-composed"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fixture.execute("cli-composed"), "21");
}

#[test]
fn cache_symlinks_and_duplicate_manifest_keys_fail_closed() {
    let fixture = Fixture::new();
    let manifest = fixture.chain();
    fs::create_dir_all(fixture.path("cache")).unwrap();
    fs::create_dir(fixture.path("outside")).unwrap();
    std::os::unix::fs::symlink(fixture.path("outside"), fixture.path("cache/components")).unwrap();
    assert!(
        fixture
            .lock(&manifest, None)
            .unwrap_err()
            .to_string()
            .contains("symlinked")
    );
    assert_eq!(fs::read_dir(fixture.path("outside")).unwrap().count(), 0);
    fs::remove_file(fixture.path("cache/components")).unwrap();
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(&manifest, text.replacen('{', "{\"schema\":1,", 1)).unwrap();
    assert!(
        fixture
            .lock(&manifest, None)
            .unwrap_err()
            .to_string()
            .contains("duplicate JSON key")
    );
    assert!(!fixture.path("component.lock.json").exists());
}

#[test]
fn shared_type_only_provider_interfaces_are_preserved() {
    let fixture = Fixture::new();
    fixture.registry("demo:a", "1.0.0", &[], 0);
    let replace = |directory: &Path, bytes: &[u8]| {
        fs::write(directory.join("component.wasm"), bytes).unwrap();
        let manifest = directory.join("component.json");
        let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        value["component"]["sha256"] = json!(sha(bytes));
        fs::write(manifest, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    };
    replace(
        &fixture.path("registry/demo/a/1.0.0"),
        include_bytes!("fixtures/shared-type-provider/provider.wasm"),
    );
    let manifest = fixture.package(
        &fixture.path("root"),
        "demo:b",
        "1.0.0",
        &[
            ("demo:a/types@1.0.0", "demo:a", "^1.0"),
            ("demo:a/api@1.0.0", "demo:a", "^1.0"),
        ],
        0,
    );
    replace(
        &fixture.path("root"),
        include_bytes!("fixtures/shared-type-provider/consumer.wasm"),
    );
    fixture.lock(&manifest, None).unwrap();
    let report = dependencies::compose_lock(
        &fixture.path("component.lock.json"),
        &fixture.path("cache"),
        &fixture.path("composed"),
    )
    .unwrap();
    assert_eq!(
        report["preservedTypeExports"],
        json!({"demo:a/types@1.0.0":"demo:a@1.0.0"})
    );
    let output = Command::new(tool("wasmtime"))
        .args(["run", "--invoke", "echo({value: 21})"])
        .arg(fixture.path("composed/component.wasm"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "{value: 21}"
    );
}
