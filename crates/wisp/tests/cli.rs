#[allow(dead_code)]
#[path = "../src/wit_model.rs"]
mod wit_model;

use serde_json::Value;
use std::{path::PathBuf, process::Command};
#[test]
fn inspect_ownership_is_separate_from_exact_graph_digest() {
    let output = Command::new(env!("CARGO_BIN_EXE_wisp"))
        .args(["inspect", "--ownership"])
        .arg(root().join("fixtures/jq/component.wasm"))
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let graph: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(graph["root"], "root:component");
    let nodes = graph["packages"].as_array().unwrap();
    assert_eq!(nodes.len(), 5);
    let io = nodes
        .iter()
        .find(|p| p["identity"] == "wasi:io@0.2.12")
        .unwrap();
    assert_eq!(io["interfaces"], serde_json::json!(["error", "streams"]));
    assert!(
        graph["worldImports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["package"] == "wasi:filesystem@0.2.12")
    );
}
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[test]
fn inspect_real_jq_matches_existing_wire_contract_without_python_or_wasm_tools() {
    let output = Command::new(env!("CARGO_BIN_EXE_wisp"))
        .arg("inspect")
        .arg(root().join("fixtures/jq/component.wasm"))
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let policy: Value = serde_json::from_slice(
        &std::fs::read(root().join("crates/wisp/assets/pure-profile.lock.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["sha256"],
        "3f4f95f882d2f7e574e1a3dcd01f690a8f72e928fe8c7d151892daf6433b173e"
    );
    assert_eq!(report["imports"], policy["imports"]);
    assert_eq!(report["importContract"], policy["importContract"]);
    assert_eq!(
        report["exports"],
        serde_json::json!([{"name":"evaluate","params":["string","string"],"result":{"result":{"ok":{"list":"string"},"err":"string"}}}])
    );
}
