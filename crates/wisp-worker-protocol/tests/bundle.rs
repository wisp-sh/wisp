use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use wisp_worker_protocol::bundle;

fn fixture() -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut artifacts = serde_json::Map::new();
    for platform in ["darwin-arm64", "linux-amd64"] {
        let path = format!("worker/{platform}/wisp-worker");
        let bytes = format!("opaque release artifact for {platform}").into_bytes();
        artifacts.insert(
            platform.into(),
            json!({"path":path,"sha256":format!("{:x}",Sha256::digest(&bytes))}),
        );
        files.insert(path, bytes);
    }
    files.insert("worker/manifest.json".into(),serde_json::to_vec_pretty(&json!({
        "schema":1,"protocol":1,"workerVersion":"0.2.0","wasmtimeVersion":wisp_worker_protocol::WASMTIME_VERSION,"artifacts":artifacts
    })).unwrap());
    files
}

#[test]
fn validates_release_artifacts_for_other_hosts_without_local_worker() {
    assert_eq!(
        bundle::validate(&fixture()).unwrap(),
        ["darwin-arm64", "linux-amd64"]
    );
}

#[test]
fn authenticates_foreign_artifacts_against_independent_manifest_pins() {
    let files = fixture();
    let trusted = files["worker/manifest.json"].clone();
    bundle::authenticate_with_manifest(&files, &trusted).unwrap();
    // A release may carry only one platform from a wider trusted runtime set.
    let mut subset = files.clone();
    let mut manifest: serde_json::Value = serde_json::from_slice(&trusted).unwrap();
    manifest["artifacts"]
        .as_object_mut()
        .unwrap()
        .remove("linux-amd64");
    subset.remove("worker/linux-amd64/wisp-worker");
    subset.insert(
        "worker/manifest.json".into(),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    );
    bundle::authenticate_with_manifest(&subset, &trusted).unwrap();
    // Updating both uploaded bytes and their self-hash cannot update trust.
    let bytes = b"replacement executable".to_vec();
    manifest["artifacts"]["darwin-arm64"]["sha256"] =
        json!(format!("{:x}", Sha256::digest(&bytes)));
    subset.insert("worker/darwin-arm64/wisp-worker".into(), bytes);
    subset.insert(
        "worker/manifest.json".into(),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    );
    bundle::validate(&subset).unwrap();
    assert!(
        bundle::authenticate_with_manifest(&subset, &trusted)
            .unwrap_err()
            .to_string()
            .contains("untrusted")
    );
}

#[test]
fn configured_trust_manifest_does_not_resolve_or_execute_a_worker() {
    let files = fixture();
    if std::env::var_os("WISP_BUNDLE_AUTH_CHILD").is_some() {
        bundle::authenticate(&files).unwrap();
        return;
    }
    let path = std::env::temp_dir().join(format!("wisp-worker-trust-{}.json", std::process::id()));
    std::fs::write(&path, &files["worker/manifest.json"]).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "configured_trust_manifest_does_not_resolve_or_execute_a_worker",
        ])
        .env("WISP_BUNDLE_AUTH_CHILD", "1")
        .env("WISP_WORKER", "missing-worker-must-not-be-resolved")
        .env(
            "WISP_WORKER_BUNDLE_DIR",
            "missing-bundle-must-not-be-resolved",
        )
        .env("WISP_WORKER_TRUST_MANIFEST", &path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn rejects_changed_missing_and_unlisted_artifacts() {
    let original = fixture();
    let mut files = original.clone();
    files
        .get_mut("worker/linux-amd64/wisp-worker")
        .unwrap()
        .push(0);
    assert!(
        bundle::validate(&files)
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );
    let mut files = original.clone();
    files.remove("worker/darwin-arm64/wisp-worker");
    assert!(bundle::validate(&files).is_err());
    let mut files = original;
    files.insert("worker/extra".into(), vec![1]);
    assert!(bundle::validate(&files).is_err());
}

#[test]
fn rejects_unsupported_versions_platforms_paths_and_duplicate_keys() {
    for pointer in [
        "/schema",
        "/protocol",
        "/workerVersion",
        "/wasmtimeVersion",
        "/artifacts/linux-amd64/path",
        "/artifacts/linux-amd64/sha256",
    ] {
        let mut files = fixture();
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&files["worker/manifest.json"]).unwrap();
        *manifest.pointer_mut(pointer).unwrap() = json!("invalid");
        files.insert(
            "worker/manifest.json".into(),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        );
        assert!(bundle::validate(&files).is_err(), "{pointer}");
    }
    let mut files = fixture();
    let manifest = String::from_utf8(files["worker/manifest.json"].clone()).unwrap();
    files.insert(
        "worker/manifest.json".into(),
        manifest
            .replacen("\"schema\": 1", "\"schema\": 1, \"schema\": 1", 1)
            .into_bytes(),
    );
    assert!(bundle::validate(&files).is_err());
    let mut files = fixture();
    let manifest = String::from_utf8(files["worker/manifest.json"].clone()).unwrap();
    files.insert(
        "worker/manifest.json".into(),
        manifest
            .replace("linux-amd64", "windows-amd64")
            .into_bytes(),
    );
    assert!(bundle::validate(&files).is_err());
}
