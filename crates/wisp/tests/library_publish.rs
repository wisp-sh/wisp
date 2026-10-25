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
#[allow(dead_code)]
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

use serde_json::{Value, json};

fn config() -> Value {
    json!({"cargo":{"index":"http://127.0.0.1:8765/cargo/","api":"http://127.0.0.1:8765","package":"wisp-wisp--math","tokenEnv":null},
        "npm":{"registry":"http://127.0.0.1:8765/npm/","package":"@wisp/math","access":"public","tokenEnv":null},
        "component":{"registry":"http://127.0.0.1:8765","repository":"wisp/math","tokenEnv":null}})
}

#[test]
fn publication_configuration_requires_explicit_safe_destinations() {
    library_publish::validate_config(&config()).unwrap();
    for url in [
        "http://registry.example",
        "https://secret@registry.example",
        "file:///tmp/registry",
        "https://registry.example/?token=secret",
    ] {
        let mut invalid = config();
        invalid["npm"]["registry"] = url.into();
        assert!(library_publish::validate_config(&invalid).is_err(), "{url}");
    }
    let mut invalid = config();
    invalid["component"]["repository"] = "wisp/../private".into();
    assert!(library_publish::validate_config(&invalid).is_err());
    invalid = config();
    invalid["npm"]["tokenEnv"] = "plain-secret-token".into();
    assert!(library_publish::validate_config(&invalid).is_err());
    invalid = config();
    invalid["npm"]["token"] = "must-not-be-persisted".into();
    assert!(library_publish::validate_config(&invalid).is_err());
}

#[test]
fn cargo_upload_preserves_archive_and_target_dependency_identity() {
    let manifest: toml::Value = toml::from_str(
        r#"
        [package]
        name = "wisp-wisp--math"
        version = "1.2.3"
        description = "Component math"
        [target.'cfg(not(target_arch = "wasm32"))'.dependencies]
        wasmtime = { version = "=50.0.0", default-features = false, features = ["component-model"] }
        [target.'cfg(target_arch = "wasm32")'.dependencies]
        wit-bindgen = "=0.61.1"
    "#,
    )
    .unwrap();
    let archive = b"the reviewed crate bytes";
    let body = library_publish::cargo_upload_body(&manifest, archive).unwrap();
    let json_size = u32::from_le_bytes(body[..4].try_into().unwrap()) as usize;
    let metadata: Value = serde_json::from_slice(&body[4..4 + json_size]).unwrap();
    let offset = 4 + json_size;
    assert_eq!(
        u32::from_le_bytes(body[offset..offset + 4].try_into().unwrap()) as usize,
        archive.len()
    );
    assert_eq!(&body[offset + 4..], archive);
    assert_eq!(metadata["name"], "wisp-wisp--math");
    assert_eq!(metadata["vers"], "1.2.3");
    let deps = metadata["deps"].as_array().unwrap();
    assert_eq!(deps.len(), 2);
    assert!(
        deps.iter()
            .all(|d| d["registry"] == "https://github.com/rust-lang/crates.io-index")
    );
    let native = deps.iter().find(|d| d["name"] == "wasmtime").unwrap();
    assert_eq!(native["target"], "cfg(not(target_arch = \"wasm32\"))");
    assert_eq!(native["default_features"], false);
}

#[test]
fn npm_upload_contains_exact_tarball_with_both_integrity_algorithms() {
    use base64::Engine;
    use sha2::{Digest, Sha512};
    let manifest =
        json!({"name":"@wisp/math","version":"1.2.3","type":"module","main":"./index.js"});
    let bytes = b"the reviewed npm archive";
    let body = library_publish::npm_publish_document(
        &manifest,
        bytes,
        "http://127.0.0.1:8765/npm/",
        "public",
    )
    .unwrap();
    let attachment = body["_attachments"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(attachment["data"].as_str().unwrap())
            .unwrap(),
        bytes
    );
    assert_eq!(attachment["length"], bytes.len());
    let version = &body["versions"]["1.2.3"];
    assert_eq!(
        version["dist"]["integrity"],
        format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(Sha512::digest(bytes))
        )
    );
    assert_eq!(version["dist"]["shasum"].as_str().unwrap().len(), 40);
    assert_eq!(version["main"], "./index.js");
}
