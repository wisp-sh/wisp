use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Write},
    process::Command,
};

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn payload(entries: &[(&str, &[u8], u32, tar::EntryType)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, bytes, mode, kind) in entries {
        let mut header = tar::Header::new_ustar();
        header.set_size(bytes.len() as u64);
        header.set_mode(*mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_entry_type(*kind);
        if kind.is_symlink() || kind.is_hard_link() {
            header.set_link_name("target").unwrap();
        }
        header.set_cksum();
        tar.append_data(&mut header, path, *bytes).unwrap();
    }
    gzip(&tar.into_inner().unwrap())
}
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut gzip = flate2::GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), flate2::Compression::default());
    gzip.write_all(bytes).unwrap();
    gzip.finish().unwrap()
}
fn fixture() -> (Value, Vec<u8>) {
    let wasm = b"\0asm\x0d\0\x01\0";
    let script = b"#!/usr/bin/env python3\nprint('compiler host')\n";
    let payload = payload(&[
        (
            "wisp-buildtools/clang/compiler.wasm",
            wasm,
            0o644,
            tar::EntryType::Regular,
        ),
        (
            "wisp-buildtools/clang/compile.py",
            script,
            0o755,
            tar::EntryType::Regular,
        ),
    ]);
    let record = json!({"schema":4,"kind":"toolchain","identity":"wisp:clang","version":"0.1.0","tool":"clang",
        "upstreamVersion":"llvm-24-d297af3","platform":"linux-x86_64",
        "archive":{"file":"toolchain.tar.gz","size":payload.len(),"sha256":digest(&payload),"inventory":{
            "clang/compiler.wasm":{"size":wasm.len(),"sha256":digest(wasm),"mode":0o644,"format":"wasm-component","role":"compiler"},
            "clang/compile.py":{"size":script.len(),"sha256":digest(script),"mode":0o755,"format":"host-script","role":"adapter"}}},
        "entrypoints":[{"name":"clang","path":"clang/compiler.wasm","format":"wasm-component"}],
        "requirements":[{"name":"wasmtime","kind":"runtime-host","version":"50.0.0","included":false,"note":"Install separately"}],
        "sizes":{"installedBytes":wasm.len()+script.len(),"wasmBytes":wasm.len(),"nativeBytes":0,"supportBytes":script.len()},
        "bootstrapRequirements":["Native LLVM bootstrap compiler"]});
    (record, payload)
}
fn zip(
    manifest: &[u8],
    payload: &[u8],
    extra: bool,
    compression: zip::CompressionMethod,
) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(compression)
        .unix_permissions(0o644);
    zip.start_file("release.json", options).unwrap();
    zip.write_all(manifest).unwrap();
    zip.start_file("toolchain.tar.gz", options).unwrap();
    zip.write_all(payload).unwrap();
    if extra {
        zip.start_file("surprise", options).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
fn bundle(record: &Value, payload: &[u8]) -> Vec<u8> {
    zip(
        &serde_json::to_vec(record).unwrap(),
        payload,
        false,
        zip::CompressionMethod::Stored,
    )
}
fn validate(bytes: &[u8]) -> std::process::Output {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("compiler.wisp.zip");
    fs::write(&path, bytes).unwrap();
    Command::new(env!("CARGO_BIN_EXE_wisp"))
        .args(["toolchain", "validate"])
        .arg(&path)
        .output()
        .unwrap()
}
fn reject(bytes: &[u8], message: &str) {
    let output = validate(bytes);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "invalid package admitted: {message}"
    );
    assert!(
        stderr.contains(message),
        "expected {message:?}, got {stderr:?}"
    );
}
fn update_payload(record: &mut Value, payload: &[u8]) {
    record["archive"]["size"] = json!(payload.len());
    record["archive"]["sha256"] = json!(digest(payload));
}

#[test]
fn admits_bundle_and_reports_measured_sizes_and_external_requirements() {
    let (record, payload) = fixture();
    let output = validate(&bundle(&record, &payload));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["valid"], true);
    assert_eq!(report["identity"], "wisp:clang");
    assert_eq!(report["files"], 2);
    assert_eq!(report["sizes"], record["sizes"]);
    assert_eq!(report["requirements"], record["requirements"]);
}

#[test]
fn portable_toolchains_require_native_hosts_to_be_external() {
    let (mut record, bytes) = fixture();
    record["platform"] = json!("wasm32-wasi");
    let output = validate(&bundle(&record, &bytes));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let native = b"\x7fELFprovided-by-wisp";
    let bytes = payload(&[(
        "wisp-buildtools/clang/wasmtime",
        native,
        0o755,
        tar::EntryType::Regular,
    )]);
    record["archive"]["inventory"] = json!({"clang/wasmtime":{"size":native.len(),"sha256":digest(native),
        "mode":0o755,"format":"native","role":"runtime"}});
    record["entrypoints"] = json!([]);
    record["sizes"] = json!({"installedBytes":native.len(),"wasmBytes":0,"nativeBytes":native.len(),"supportBytes":0});
    update_payload(&mut record, &bytes);
    reject(
        &bundle(&record, &bytes),
        "portable toolchain package cannot include native binaries",
    );
}

#[test]
fn rejects_unknown_fields_noninteger_sizes_and_duplicate_json_keys() {
    let (record, payload) = fixture();
    let mut changed = record.clone();
    changed["extra"] = json!(true);
    reject(&bundle(&changed, &payload), "toolchain release");
    changed = record.clone();
    changed["sizes"]["installedBytes"] = json!(true);
    reject(&bundle(&changed, &payload), "must be a nonnegative integer");
    let mut manifest = serde_json::to_string(&record).unwrap();
    manifest.insert_str(1, "\"schema\":4,");
    reject(
        &zip(
            manifest.as_bytes(),
            &payload,
            false,
            zip::CompressionMethod::Stored,
        ),
        "duplicate",
    );
}

#[test]
fn rejects_identity_version_entrypoint_and_size_metadata_changes() {
    let (record, payload) = fixture();
    for (pointer, value, message) in [
        ("/identity", json!("wisp:go"), "identity mismatch"),
        ("/version", json!("v0.1.0"), "unexpected character"),
        (
            "/entrypoints/0/path",
            json!("missing"),
            "missing from toolchain inventory",
        ),
        (
            "/entrypoints/0/format",
            json!("wasm-core"),
            "entrypoint format mismatch",
        ),
        ("/sizes/wasmBytes", json!(9), "wasmBytes mismatch"),
        (
            "/archive/inventory/clang~1compiler.wasm/size",
            json!(2_147_483_649u64),
            "individual file size limit",
        ),
    ] {
        let mut changed = record.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        reject(&bundle(&changed, &payload), message);
    }
}

#[test]
fn rejects_zip_compression_extra_entries_prefix_and_trailing_bytes() {
    let (record, payload) = fixture();
    let manifest = serde_json::to_vec(&record).unwrap();
    reject(
        &zip(&manifest, &payload, true, zip::CompressionMethod::Stored),
        "requires two entries",
    );
    reject(
        &zip(&manifest, &payload, false, zip::CompressionMethod::Deflated),
        "unsupported toolchain ZIP encoding",
    );
    let mut changed = bundle(&record, &payload);
    changed.push(0);
    reject(&changed, "termination or comment");
    changed = [vec![0], bundle(&record, &payload)].concat();
    reject(&changed, "central directory framing");
}

#[test]
fn rejects_payload_and_file_hash_changes() {
    let (mut record, payload) = fixture();
    record["archive"]["sha256"] = json!("0".repeat(64));
    reject(&bundle(&record, &payload), "archive SHA-256");
    record["archive"]["sha256"] = json!(digest(&payload));
    record["archive"]["inventory"]["clang/compiler.wasm"]["sha256"] = json!("0".repeat(64));
    reject(&bundle(&record, &payload), "file SHA-256 mismatch");
}

#[test]
fn rejects_zip_crc_changes_even_when_manifest_hashes_match() {
    let (record, payload) = fixture();
    let mut changed = bundle(&record, &payload);
    // Change the payload CRC in both ZIP headers so layout still agrees.
    let central = changed
        .windows(4)
        .rposition(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    let local =
        u32::from_le_bytes(changed[central + 42..central + 46].try_into().unwrap()) as usize;
    changed[central + 16] ^= 1;
    changed[local + 14] ^= 1;
    let output = validate(&changed);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("checksum"));
}

#[test]
fn rejects_tar_mode_format_and_inventory_mismatch() {
    let (record, payload) = fixture();
    for (pointer, value, message) in [
        (
            "/archive/inventory/clang~1compiler.wasm/mode",
            json!(0o755),
            "tar mode mismatch",
        ),
        (
            "/archive/inventory/clang~1compiler.wasm/format",
            json!("wasm-core"),
            "entrypoint format mismatch",
        ),
        (
            "/archive/inventory/clang~1compile.py/format",
            json!("native"),
            "nativeBytes mismatch",
        ),
    ] {
        let mut changed = record.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        reject(&bundle(&changed, &payload), message);
    }
    let mut changed = record.clone();
    changed["archive"]["inventory"]
        .as_object_mut()
        .unwrap()
        .remove("clang/compile.py");
    changed["sizes"]["installedBytes"] = json!(8);
    changed["sizes"]["supportBytes"] = json!(0);
    reject(
        &bundle(&changed, &payload),
        "unexpected toolchain tar member",
    );
    let mut changed = record.clone();
    changed["archive"]["inventory"]["clang/compiler.wasm"]["format"] = json!("wasm-core");
    changed["entrypoints"][0]["format"] = json!("wasm-core");
    reject(&bundle(&changed, &payload), "file format mismatch");
    let mut changed = record.clone();
    changed["archive"]["inventory"]["clang/compile.py"]["format"] = json!("data");
    reject(&bundle(&changed, &payload), "file format mismatch");
}

#[test]
fn rejects_noncanonical_tar_ownership_and_timestamps() {
    use std::io::Read;
    let (record, compressed) = fixture();
    let mut original = Vec::new();
    flate2::read::GzDecoder::new(&compressed[..])
        .read_to_end(&mut original)
        .unwrap();
    for (start, bytes) in [
        (108, b"0000001\0".as_slice()),
        (136, b"00000000001\0".as_slice()),
        (265, b"user".as_slice()),
    ] {
        let mut raw = original.clone();
        raw[start..start + bytes.len()].copy_from_slice(bytes);
        raw[148..156].fill(b' ');
        let checksum: u64 = raw[..512].iter().map(|byte| *byte as u64).sum();
        raw[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
        let payload = gzip(&raw);
        let mut changed = record.clone();
        update_payload(&mut changed, &payload);
        reject(
            &bundle(&changed, &payload),
            "noncanonical toolchain tar ownership or timestamp",
        );
    }
}

#[test]
fn rejects_gzip_concatenation_trailing_data_and_truncation() {
    let (record, payload) = fixture();
    for changed_payload in [
        [payload.clone(), gzip(b"")].concat(),
        [payload.clone(), b"trailing".to_vec()].concat(),
        payload[..payload.len() - 4].to_vec(),
    ] {
        let mut changed = record.clone();
        update_payload(&mut changed, &changed_payload);
        let output = validate(&bundle(&changed, &changed_payload));
        assert!(!output.status.success());
    }
}

#[test]
fn rejects_duplicate_members_links_and_missing_files() {
    let (record, _) = fixture();
    let wasm = b"\0asm\x0d\0\x01\0";
    for (kind, expected) in [
        (
            tar::EntryType::Symlink,
            "links, directories, or special files",
        ),
        (tar::EntryType::Link, "links, directories, or special files"),
        (tar::EntryType::Regular, "inventory missing files"),
    ] {
        let payload = payload(&[("wisp-buildtools/clang/compiler.wasm", wasm, 0o644, kind)]);
        let mut changed = record.clone();
        update_payload(&mut changed, &payload);
        reject(&bundle(&changed, &payload), expected);
    }
    let payload = payload(&[
        (
            "wisp-buildtools/clang/compiler.wasm",
            wasm,
            0o644,
            tar::EntryType::Regular,
        ),
        (
            "wisp-buildtools/clang/compiler.wasm",
            wasm,
            0o644,
            tar::EntryType::Regular,
        ),
    ]);
    let mut changed = record;
    update_payload(&mut changed, &payload);
    reject(
        &bundle(&changed, &payload),
        "duplicate toolchain tar member",
    );
}

#[test]
fn rejects_tar_traversal_nonzero_padding_and_nonzero_trailer() {
    use std::io::Read;
    let (record, compressed) = fixture();
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(&compressed[..])
        .read_to_end(&mut raw)
        .unwrap();
    let mut traversal = raw.clone();
    traversal[..100].fill(0);
    let name = b"wisp-buildtools/../escape";
    traversal[..name.len()].copy_from_slice(name);
    traversal[148..156].fill(b' ');
    let checksum: u64 = traversal[..512].iter().map(|byte| *byte as u64).sum();
    traversal[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
    let mut padding = raw.clone();
    padding[512 + 8] = 1;
    let mut trailer = raw.clone();
    *trailer.last_mut().unwrap() = 1;
    for (changed_raw, expected) in [
        (traversal, "unsafe archive path"),
        (padding, "nonzero toolchain tar payload padding"),
        (trailer, "invalid toolchain tar trailing bytes"),
    ] {
        let changed_payload = gzip(&changed_raw);
        let mut changed = record.clone();
        update_payload(&mut changed, &changed_payload);
        reject(&bundle(&changed, &changed_payload), expected);
    }
}

#[test]
fn application_validation_keeps_its_original_bundle_contract() {
    let (record, payload) = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("compiler.wisp.zip");
    fs::write(&path, bundle(&record, &payload)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_wisp"))
        .arg("validate")
        .arg(path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

fn toolchain(args: &[&str], prefix: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_wisp"))
        .arg("toolchain")
        .args(args)
        .arg("--prefix")
        .arg(prefix)
        .output()
        .unwrap()
}
fn successful(output: &std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn save_bundle(directory: &std::path::Path, record: &Value, payload: &[u8]) -> std::path::PathBuf {
    let path = directory.join("fixture.wisp.zip");
    fs::write(&path, bundle(record, payload)).unwrap();
    path
}
fn install_fixture(directory: &std::path::Path) -> (std::path::PathBuf, Value) {
    let prefix = directory.join("installed");
    let (record, payload) = fixture();
    let path = save_bundle(directory, &record, &payload);
    let output = toolchain(&["install", path.to_str().unwrap()], &prefix);
    let report = successful(&output);
    (prefix, report)
}
#[test]
fn installs_version_atomically_without_running_payload_and_reuses_verified_version() {
    let directory = tempfile::tempdir().unwrap();
    let (prefix, report) = install_fixture(directory.path());
    let version = prefix.join("packages/clang/0.1.0");
    assert_eq!(report["selected"], true);
    assert_eq!(report["reused"], false);
    assert_eq!(
        fs::read(version.join("payload/clang/compiler.wasm")).unwrap(),
        b"\0asm\x0d\0\x01\0"
    );
    assert_eq!(fs::read_dir(&prefix).unwrap().count(), 2);
    let again = successful(&toolchain(
        &[
            "install",
            directory.path().join("fixture.wisp.zip").to_str().unwrap(),
        ],
        &prefix,
    ));
    assert_eq!(again["reused"], true);
    let doctor = successful(&toolchain(&["doctor"], &prefix));
    assert_eq!(doctor["valid"], true);
    assert_eq!(doctor["packages"][0]["valid"], true);
    assert_eq!(doctor["hosts"]["available"], false);
}
#[test]
fn installation_admits_the_entire_private_snapshot_before_extraction() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("installed");
    let (mut record, payload) = fixture();
    record["archive"]["inventory"]["clang/compiler.wasm"]["sha256"] = json!("0".repeat(64));
    let path = save_bundle(directory.path(), &record, &payload);
    let output = toolchain(&["install", path.to_str().unwrap()], &prefix);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("file SHA-256 mismatch"));
    assert_eq!(fs::read_dir(prefix).unwrap().count(), 0);
}
#[test]
fn transport_pin_mismatch_leaves_no_payload_or_selection() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("installed");
    let (record, payload) = fixture();
    let path = save_bundle(directory.path(), &record, &payload);
    let output = toolchain(
        &[
            "install",
            path.to_str().unwrap(),
            "--sha256",
            &"0".repeat(64),
        ],
        &prefix,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not match pin"));
    assert_eq!(fs::read_dir(prefix).unwrap().count(), 0);
}
#[test]
fn same_version_cannot_silently_change_its_payload_or_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let (prefix, _) = install_fixture(directory.path());
    let original = fs::read(prefix.join("selection.json")).unwrap();
    let (mut record, payload) = fixture();
    record["upstreamVersion"] = json!("changed-upstream");
    let path = save_bundle(directory.path(), &record, &payload);
    let output = toolchain(&["install", path.to_str().unwrap()], &prefix);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("different digest"));
    assert_eq!(fs::read(prefix.join("selection.json")).unwrap(), original);
    assert_eq!(successful(&toolchain(&["doctor"], &prefix))["valid"], true);
}
#[test]
fn installed_versions_can_be_selected_and_survive_prefix_relocation() {
    let directory = tempfile::tempdir().unwrap();
    let (prefix, _) = install_fixture(directory.path());
    let (mut record, payload) = fixture();
    record["version"] = json!("0.2.0");
    let path = save_bundle(directory.path(), &record, &payload);
    successful(&toolchain(&["install", path.to_str().unwrap()], &prefix));
    let listing = successful(&toolchain(&["list"], &prefix));
    assert_eq!(listing["installed"].as_array().unwrap().len(), 2);
    assert_eq!(
        successful(&toolchain(&["select", "wisp:clang@0.1.0"], &prefix))["version"],
        "0.1.0"
    );
    let moved = directory.path().join("relocated install's");
    fs::rename(&prefix, &moved).unwrap();
    let environment = toolchain(&["env"], &moved);
    assert!(
        environment.status.success(),
        "{}",
        String::from_utf8_lossy(&environment.stderr)
    );
    let text = String::from_utf8(environment.stdout).unwrap();
    assert!(text.starts_with("export WISP_TOOLCHAIN_HOME="));
    assert!(text.contains("0.1.0/payload/clang/compiler.wasm"));
    assert!(!text.contains(prefix.to_str().unwrap()));
    let shell = Command::new("bash")
        .arg("-c")
        .arg(format!("{text}\ntest -f \"$WISP_CLANG\"\n"))
        .output()
        .unwrap();
    assert!(shell.status.success());
    successful(&toolchain(&["doctor"], &moved));
}
#[test]
fn modified_or_added_payload_files_fail_doctor_and_environment_selection() {
    for mutation in ["modified", "extra"] {
        let directory = tempfile::tempdir().unwrap();
        let (prefix, _) = install_fixture(directory.path());
        let payload = prefix.join("packages/clang/0.1.0/payload/clang");
        let path = payload.join(if mutation == "modified" {
            "compiler.wasm"
        } else {
            "extra"
        });
        fs::write(path, b"tampered").unwrap();
        for command in ["doctor", "env"] {
            let output = toolchain(&[command], &prefix);
            assert!(
                !output.status.success(),
                "{command} accepted {mutation} payload"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("checksum or size mismatch") || stderr.contains("file set differs"),
                "{stderr}"
            );
        }
    }
}
#[test]
#[cfg(unix)]
fn doctor_rejects_changed_permissions_and_payload_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for mutation in ["mode", "symlink"] {
        let directory = tempfile::tempdir().unwrap();
        let (prefix, _) = install_fixture(directory.path());
        let payload = prefix.join("packages/clang/0.1.0/payload/clang");
        if mutation == "mode" {
            fs::set_permissions(
                payload.join("compiler.wasm"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        } else {
            symlink(directory.path(), payload.join("escape")).unwrap();
        }
        let output = toolchain(&["doctor"], &prefix);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(if mutation == "mode" {
                "mode mismatch"
            } else {
                "symlink"
            })
        );
    }
}
#[test]
#[cfg(unix)]
fn installation_never_writes_through_an_existing_packages_symlink() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("installed");
    let outside = directory.path().join("outside");
    fs::create_dir_all(&prefix).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, prefix.join("packages")).unwrap();
    let (record, payload) = fixture();
    let path = save_bundle(directory.path(), &record, &payload);
    let output = toolchain(&["install", path.to_str().unwrap()], &prefix);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("symlink"));
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    assert!(!prefix.join("selection.json").exists());
}
#[test]
fn selection_rejects_paths_and_does_not_modify_a_valid_pin_on_failure() {
    let directory = tempfile::tempdir().unwrap();
    let (prefix, _) = install_fixture(directory.path());
    let original = fs::read(prefix.join("selection.json")).unwrap();
    for spec in [
        "wisp:clang@../escape",
        "wisp:clang@99.0.0",
        "other:clang@0.1.0",
    ] {
        assert!(!toolchain(&["select", spec], &prefix).status.success());
        assert_eq!(fs::read(prefix.join("selection.json")).unwrap(), original);
    }
}
#[test]
fn executable_scripts_are_installed_as_bytes_without_being_run() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("installed");
    let marker = directory.path().join("executed");
    let script = format!("#!/bin/sh\ntouch '{}'\n", marker.display());
    let bytes = payload(&[(
        "wisp-buildtools/clang/danger.sh",
        script.as_bytes(),
        0o755,
        tar::EntryType::Regular,
    )]);
    let (mut record, _) = fixture();
    record["archive"]["inventory"] = json!({"clang/danger.sh":{"size":script.len(),"sha256":digest(script.as_bytes()),"mode":0o755,"format":"host-script","role":"adapter"}});
    record["entrypoints"] = json!([]);
    record["sizes"] = json!({"installedBytes":script.len(),"wasmBytes":0,"nativeBytes":0,"supportBytes":script.len()});
    update_payload(&mut record, &bytes);
    let path = save_bundle(directory.path(), &record, &bytes);
    successful(&toolchain(&["install", path.to_str().unwrap()], &prefix));
    assert!(!marker.exists());
    assert_eq!(
        fs::read(prefix.join("packages/clang/0.1.0/payload/clang/danger.sh")).unwrap(),
        script.as_bytes()
    );
}

#[test]
#[cfg(unix)]
fn host_receipt_claims_cannot_replace_actual_capability_checks() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let host = directory.path().join("hosts/wasmtime-50.0.0");
    fs::create_dir_all(&host).unwrap();
    let binary = b"#!/bin/sh\nprintf 'wasmtime 50.0.0\\n'\n";
    fs::write(host.join("wasmtime"), binary).unwrap();
    fs::set_permissions(host.join("wasmtime"), fs::Permissions::from_mode(0o755)).unwrap();
    let source: Value = serde_json::from_str(include_str!(
        "../../../tools/shared-hosts/wasmtime.lock.json"
    ))
    .unwrap();
    let receipt = json!({"schema":1,"owner":"wisp","version":"50.0.0","source":source,
        "patchSha256":digest(include_bytes!("../../../tools/shared-hosts/component-preload.patch")),
        "binarySha256":digest(binary),"smoke":{"externalModuleImport":"wisp-test-provider","value":42,"missingProviderRejected":true}});
    fs::write(
        host.join("receipt.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let report = successful(&toolchain(&["hosts"], directory.path()));
    assert_eq!(report["available"], false);
    assert_eq!(report["hosts"][0]["available"], false);
    assert!(
        report["hosts"][0]["error"]
            .as_str()
            .unwrap()
            .contains("capability smoke failed")
    );
}

#[test]
fn adapter_setup_rejects_changed_cached_bytes_without_installing_support() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("installed");
    let cache = directory.path().join("cache");
    fs::create_dir_all(cache.join("downloads")).unwrap();
    fs::write(
        cache.join("downloads/73b01f85e42e99c890efbbf619cd765e04e5809da36d976b87fc64abd87f5812"),
        b"changed adapter",
    )
    .unwrap();
    let output = toolchain(
        &[
            "setup-hosts",
            "--host",
            "adapter",
            "--cache",
            cache.to_str().unwrap(),
        ],
        &prefix,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("adapter cache checksum"));
    assert!(!prefix.join("hosts/preview1-50.0.0").exists());
    assert!(!prefix.join(".install-lock").exists());
}
#[test]
fn incompatible_compiler_bundled_adapter_requires_pinned_wisp_support() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("installed");
    let wasm = b"\0asm\x01\0\0\0";
    let bytes = payload(&[(
        "wisp-buildtools/wasm-tools/adapter.wasm",
        wasm,
        0o644,
        tar::EntryType::Regular,
    )]);
    let (mut record, _) = fixture();
    record["identity"] = json!("wisp:wasm-tools");
    record["tool"] = json!("wasm-tools");
    record["archive"]["inventory"] = json!({"wasm-tools/adapter.wasm":{"size":wasm.len(),"sha256":digest(wasm),"mode":0o644,"format":"wasm-core","role":"runtime"}});
    record["entrypoints"] = json!([{"name":"preview1-reactor-adapter","path":"wasm-tools/adapter.wasm","format":"wasm-core"}]);
    record["sizes"] = json!({"installedBytes":wasm.len(),"wasmBytes":wasm.len(),"nativeBytes":0,"supportBytes":0});
    update_payload(&mut record, &bytes);
    let bundle = save_bundle(directory.path(), &record, &bytes);
    successful(&toolchain(&["install", bundle.to_str().unwrap()], &prefix));
    let output = toolchain(&["env"], &prefix);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("setup-hosts --host adapter"));
}
#[test]
fn doctor_reports_modified_pinned_adapter_and_environment_rejects_it() {
    let directory = tempfile::tempdir().unwrap();
    let (prefix, _) = install_fixture(directory.path());
    let adapter = prefix.join("hosts/preview1-50.0.0");
    fs::create_dir_all(&adapter).unwrap();
    fs::write(
        adapter.join("wasi_snapshot_preview1.reactor.wasm"),
        b"changed adapter",
    )
    .unwrap();
    let report = successful(&toolchain(&["doctor"], &prefix));
    assert_eq!(report["adapter"]["available"], false);
    assert!(
        report["adapter"]["error"]
            .as_str()
            .unwrap()
            .contains("checksum mismatch")
    );
    assert!(!toolchain(&["env"], &prefix).status.success());
}
#[test]
#[ignore = "downloads the pinned official preview1 adapter"]
fn pinned_adapter_download_is_reused_offline_and_selected_without_a_compiler_bundle() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("installed");
    let cache = directory.path().join("cache");
    let args = [
        "setup-hosts",
        "--host",
        "adapter",
        "--cache",
        cache.to_str().unwrap(),
    ];
    let report = successful(&toolchain(&args, &prefix));
    assert_eq!(
        report["adapter"]["sha256"],
        "73b01f85e42e99c890efbbf619cd765e04e5809da36d976b87fc64abd87f5812"
    );
    let environment = toolchain(&["env"], &prefix);
    assert!(environment.status.success());
    assert!(
        String::from_utf8_lossy(&environment.stdout)
            .contains("hosts/preview1-50.0.0/wasi_snapshot_preview1.reactor.wasm")
    );
    fs::remove_dir_all(prefix.join("hosts/preview1-50.0.0")).unwrap();
    let offline = Command::new(env!("CARGO_BIN_EXE_wisp"))
        .arg("toolchain")
        .args(args)
        .arg("--prefix")
        .arg(&prefix)
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .env("NO_PROXY", "")
        .output()
        .unwrap();
    assert_eq!(successful(&offline)["adapter"]["available"], true);
}

fn install_entrypoint_fixture(
    directory: &std::path::Path,
    prefix: &std::path::Path,
    tool: &str,
    entry: &str,
    path: &str,
) {
    let script = b"#!/usr/bin/env python3\nraise SystemExit('fixture must never execute')\n";
    let format = if path.ends_with(".py") {
        "host-script"
    } else {
        "wasm-core"
    };
    let bytes: &[u8] = if format == "host-script" {
        script
    } else {
        b"\0asm\x01\0\0\0"
    };
    let archive = payload(&[(
        &format!("wisp-buildtools/{path}"),
        bytes,
        0o644,
        tar::EntryType::Regular,
    )]);
    let (mut record, _) = fixture();
    record["identity"] = json!(format!("wisp:{tool}"));
    record["tool"] = json!(tool);
    record["archive"]["inventory"] = json!({path: {"size":bytes.len(),"sha256":digest(bytes),
        "mode":0o644,"format":format,"role":if format == "host-script" {"adapter"} else {"compiler"}}});
    record["entrypoints"] = json!([{"name":entry,"path":path,"format":format}]);
    record["sizes"] = json!({"installedBytes":bytes.len(),"wasmBytes":if format == "host-script" {0} else {bytes.len()},
        "nativeBytes":0,"supportBytes":if format == "host-script" {bytes.len()} else {0}});
    update_payload(&mut record, &archive);
    let bundle = save_bundle(directory, &record, &archive);
    successful(&toolchain(&["install", bundle.to_str().unwrap()], prefix));
}

#[test]
fn exports_selected_language_and_support_roots_after_prefix_relocation() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("installed");
    for tool in [
        "zig",
        "assemblyscript",
        "nim",
        "javascript",
        "python",
        "grain",
        "bend",
        "swift",
        "java",
        "kotlin",
        "teavm",
    ] {
        install_entrypoint_fixture(
            directory.path(),
            &prefix,
            tool,
            "build",
            &format!("{tool}/compile.py"),
        );
    }
    install_entrypoint_fixture(directory.path(), &prefix, "jvm", "jvm", "jvm/dist/jvm.wasm");
    install_entrypoint_fixture(
        directory.path(),
        &prefix,
        "llvm-swift",
        "swift-llvm",
        "llvm/dist/swift-shared/swift-frontend-llvm.so.wasm",
    );
    install_entrypoint_fixture(
        directory.path(),
        &prefix,
        "llvm",
        "llvm24",
        "llvm/dist/shared/lib/llvm24.so.wasm",
    );
    let moved = directory.path().join("moved compiler's prefix");
    fs::rename(&prefix, &moved).unwrap();
    let output = toolchain(&["env"], &moved);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exports = String::from_utf8(output.stdout).unwrap();
    let quoted_root = moved.to_str().unwrap().replace('\'', "'\\''");
    for (environment, tool, relative) in [
        ("WISP_ZIG_TOOLCHAIN", "zig", "zig"),
        (
            "WISP_ASSEMBLYSCRIPT_TOOLCHAIN",
            "assemblyscript",
            "assemblyscript",
        ),
        ("WISP_NIM_TOOLCHAIN", "nim", "nim"),
        ("WISP_JAVASCRIPT_TOOLCHAIN", "javascript", "javascript"),
        ("WISP_TYPESCRIPT_TOOLCHAIN", "javascript", "javascript"),
        ("WISP_PYTHON_TOOLCHAIN", "python", "python"),
        ("WISP_GRAIN_TOOLCHAIN", "grain", "grain"),
        ("WISP_BEND_TOOLCHAIN", "bend", "bend"),
        ("WISP_SWIFT_TOOLCHAIN", "swift", "swift"),
        ("WISP_JAVA_TOOLCHAIN", "java", "java"),
        ("WISP_KOTLIN_TOOLCHAIN", "kotlin", "kotlin"),
        ("WISP_TEAVM_TOOLCHAIN", "teavm", "teavm"),
        ("WISP_JVM_TOOLCHAIN", "jvm", "jvm"),
        ("WISP_LLVM_SWIFT", "llvm-swift", "llvm/dist/swift-shared"),
        ("WISP_LLVM_TOOLCHAIN", "llvm", "llvm"),
        ("WISP_LLVM", "llvm", "llvm/dist/shared"),
    ] {
        assert!(
            exports.contains(&format!(
                "export {environment}='{quoted_root}/packages/{tool}/0.1.0/payload/{relative}'\n"
            )),
            "{environment}: {exports}"
        );
    }
    fs::write(
        moved.join("packages/java/0.1.0/payload/java/compile.py"),
        b"changed",
    )
    .unwrap();
    let output = toolchain(&["env"], &moved);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("checksum or size mismatch"));
}

#[test]
fn installed_rust_host_matches_the_selected_compiler_manifest() {
    for (compiler_host, version, source, patch, smoke) in [(
        "wasm32-wasip3",
        "50.0.0",
        include_bytes!("../../../tools/shared-hosts/wasmtime.lock.json").as_slice(),
        include_bytes!("../../../tools/shared-hosts/component-preload.patch").as_slice(),
        json!({"externalModuleImport":"wisp-test-provider","value":42,"missingProviderRejected":true}),
    )] {
        let directory = tempfile::tempdir().unwrap();
        let prefix = directory.path().join("installed");
        let wasm = b"\0asm\x0d\0\x01\0";
        let metadata = serde_json::to_vec(&json!({"compilerHost":compiler_host})).unwrap();
        let archive = payload(&[
            (
                "wisp-buildtools/rust/dist/compiler/build/bootstrap/rustc.wasm",
                wasm,
                0o644,
                tar::EntryType::Regular,
            ),
            (
                "wisp-buildtools/rust/dist/compiler/toolchain.json",
                &metadata,
                0o644,
                tar::EntryType::Regular,
            ),
        ]);
        let (mut record, _) = fixture();
        record["identity"] = json!("wisp:rust");
        record["tool"] = json!("rust");
        record["archive"]["inventory"] = json!({
            "rust/dist/compiler/build/bootstrap/rustc.wasm":{"size":wasm.len(),"sha256":digest(wasm),"mode":0o644,"format":"wasm-component","role":"compiler"},
            "rust/dist/compiler/toolchain.json":{"size":metadata.len(),"sha256":digest(&metadata),"mode":0o644,"format":"data","role":"metadata"}
        });
        record["entrypoints"] = json!([{"name":"rustc","path":"rust/dist/compiler/build/bootstrap/rustc.wasm","format":"wasm-component"}]);
        record["sizes"] = json!({"installedBytes":wasm.len()+metadata.len(),"wasmBytes":wasm.len(),"nativeBytes":0,"supportBytes":metadata.len()});
        update_payload(&mut record, &archive);
        let bundle = save_bundle(directory.path(), &record, &archive);
        successful(&toolchain(&["install", bundle.to_str().unwrap()], &prefix));
        let host = prefix.join(format!("hosts/wasmtime-{version}"));
        fs::create_dir_all(&host).unwrap();
        let binary = b"host fixture, never executed by toolchain env";
        fs::write(host.join("wasmtime"), binary).unwrap();
        fs::write(host.join("receipt.json"), serde_json::to_vec(&json!({
            "schema":1,"owner":"wisp","version":version,"source":serde_json::from_slice::<Value>(source).unwrap(),
            "patchSha256":digest(patch),"binarySha256":digest(binary),"smoke":smoke
        })).unwrap()).unwrap();
        let environment = toolchain(&["env"], &prefix);
        assert!(
            environment.status.success(),
            "{}",
            String::from_utf8_lossy(&environment.stderr)
        );
        let exports = String::from_utf8(environment.stdout).unwrap();
        assert!(
            exports.contains(&format!(
                "export WISP_RUST_WASMTIME='{}/wasmtime'",
                host.display()
            )),
            "{compiler_host}: {exports}"
        );
        // Reject a compiler override for an unsupported host before execution.
        let project = directory.path().join("project");
        fs::create_dir_all(project.join("compiler")).unwrap();
        fs::write(
            project.join("compiler/toolchain.json"),
            br#"{"compilerHost":"wasm32-wasip1-threads"}"#,
        )
        .unwrap();
        fs::write(
            project.join("Cargo.toml"),
            "[package]\nname='test-override'\nversion='0.1.0'\n[lib]\npath='lib.rs'\n",
        )
        .unwrap();
        fs::write(project.join("lib.rs"), "pub fn answer() -> u32 { 42 }\n").unwrap();
        for environment_override in [false, true] {
            let config = if environment_override {
                ""
            } else {
                "rust='compiler'\n"
            };
            fs::write(project.join("wisp.toml"), format!("[package]\nname='test:override'\nversion='0.1.0'\n[build]\ncompiler='rust'\nproject='.'\n[build.tools]\n{config}")).unwrap();
            let mut command = Command::new(env!("CARGO_BIN_EXE_wisp"));
            command
                .args(["build", project.to_str().unwrap(), "--component-only"])
                .env("WISP_TOOLCHAIN_HOME", &prefix)
                .env_remove("WISP_RUST")
                .env_remove("WISP_RUST_WASMTIME")
                .env_remove("WISP_WASMTIME");
            if environment_override {
                command.env("WISP_RUST", project.join("compiler"));
            }
            let result = command.output().unwrap();
            assert!(!result.status.success());
            assert!(
                String::from_utf8_lossy(&result.stderr)
                    .contains("Rust compiler host must be wasm32-wasip3"),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}
