use serde_json::json;
use std::{fs, path::Path};
use tempfile::TempDir;
use wisp_worker_protocol::{Request, client::Client};

fn client() -> Client {
    Client::spawn(Path::new(env!("CARGO_BIN_EXE_wisp-worker")), None).unwrap()
}
fn artifact(temp: &TempDir, name: &str, source: &str) -> String {
    let path = temp.path().join(name);
    fs::write(&path, wat::parse_str(source).unwrap()).unwrap();
    path.to_str().unwrap().to_owned()
}

#[test]
fn fresh_stores_and_traps_do_not_contaminate_later_jobs() {
    let temp = TempDir::new().unwrap();
    let module = artifact(
        &temp,
        "state.wasm",
        r#"(module
        (global $counter (mut i32) (i32.const 0))
        (func (export "next") (result i32)
          global.get $counter i32.const 1 i32.add global.set $counter global.get $counter)
        (func (export "fail") unreachable))"#,
    );
    let mut worker = client();
    for _ in 0..2 {
        let reply = worker
            .request(Request {
                module: module.clone(),
                function: "next".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(reply.ok, "{:?}", reply.error);
        assert_eq!(reply.results, vec![json!(1)]);
    }
    assert!(
        !worker
            .request(Request {
                module: module.clone(),
                function: "fail".into(),
                ..Default::default()
            })
            .unwrap()
            .ok
    );
    assert!(
        worker
            .request(Request {
                module,
                function: "next".into(),
                ..Default::default()
            })
            .unwrap()
            .ok
    );
}

#[test]
fn component_calls_keep_state_and_traps_invalidate_the_instance() {
    let temp = TempDir::new().unwrap();
    let module = artifact(
        &temp,
        "component.wasm",
        r#"(component
      (core module $m
        (global $counter (mut i32) (i32.const 0))
        (func (export "next") (result i32)
          global.get $counter i32.const 1 i32.add global.set $counter global.get $counter)
        (func (export "fail") unreachable))
      (core instance $i (instantiate $m))
      (func (export "next") (result u32) (canon lift (core func $i "next")))
      (func (export "fail") (canon lift (core func $i "fail"))))"#,
    );
    let mut worker = client();
    let open = worker
        .request(Request {
            operation: "open".into(),
            module,
            ..Default::default()
        })
        .unwrap();
    assert!(open.ok, "{:?}", open.error);
    for expected in 1..=2 {
        let reply = worker
            .request(Request {
                operation: "call".into(),
                instance: open.instance,
                function: "next".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(reply.ok, "{:?}", reply.error);
        assert_eq!(reply.results, vec![json!(expected)]);
    }
    let trap = worker
        .request(Request {
            operation: "call".into(),
            instance: open.instance,
            function: "fail".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(!trap.ok);
    let again = worker
        .request(Request {
            operation: "call".into(),
            instance: open.instance,
            function: "next".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(!again.ok);
}

#[test]
fn timeout_kills_the_worker_and_invalidates_the_client() {
    let temp = TempDir::new().unwrap();
    let module = artifact(
        &temp,
        "loop.wasm",
        "(module (func (export \"_start\") (loop $forever br $forever)))",
    );
    let mut worker = client();
    let mut request = Request {
        module,
        ..Default::default()
    };
    request.limits.timeout_ms = 25;
    // Compilation is also bounded by the parent deadline. Either epoch
    // interruption or a killed worker must stop an infinite guest.
    if let Ok(reply) = worker.request(request) {
        assert!(!reply.ok);
    }
}

#[test]
fn wasi_has_no_ambient_filesystem() {
    let temp = TempDir::new().unwrap();
    let module = artifact(
        &temp,
        "caps.wasm",
        r#"(module
      (import "wasi_snapshot_preview1" "fd_prestat_get" (func $prestat (param i32 i32) (result i32)))
      (memory (export "memory") 1)
      (func (export "probe") (result i32) i32.const 3 i32.const 0 call $prestat))"#,
    );
    let reply = client()
        .request(Request {
            module,
            function: "probe".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(reply.results, vec![json!(8)]);
}

#[test]
fn component_imports_shared_core_modules_without_cli_patches() {
    let temp = TempDir::new().unwrap();
    let shared = artifact(
        &temp,
        "math.wasm",
        r#"(module
      (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))"#,
    );
    let module = artifact(
        &temp,
        "consumer.wasm",
        r#"(component
      (import "math" (core module $math
        (type $add (func (param i32 i32) (result i32)))
        (export "add" (func (type $add)))))
      (core instance $i (instantiate $math))
      (func (export "add") (param "a" u32) (param "b" u32) (result u32)
        (canon lift (core func $i "add"))))"#,
    );
    let reply = client()
        .request(Request {
            module,
            function: "add".into(),
            params: vec![json!(20), json!(22)],
            modules: [("math".into(), shared)].into(),
            ..Default::default()
        })
        .unwrap();
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(reply.results, vec![json!(42)]);
}

#[test]
fn guest_output_cannot_inject_protocol_frames() {
    use base64::Engine as _;
    let temp = TempDir::new().unwrap();
    let module = artifact(
        &temp,
        "output.wasm",
        r#"(module
      (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
      (memory (export "memory") 1)
      (data (i32.const 0) "\10\00\00\00\09\00\00\00")
      (data (i32.const 16) "{\22id\22:7}\n")
      (func (export "emit") (result i32)
        i32.const 1 i32.const 0 i32.const 1 i32.const 32 call $write))"#,
    );
    let mut worker = client();
    let reply = worker
        .request(Request {
            module,
            function: "emit".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(reply.stdout)
            .unwrap(),
        b"{\"id\":7}\n"
    );
    assert!(
        worker
            .request(Request {
                operation: "ping".into(),
                ..Default::default()
            })
            .unwrap()
            .ok
    );
}

#[test]
fn async_transfers_are_rejected_before_guest_execution_even_when_nested() {
    let temp = TempDir::new().unwrap();
    let mut worker = client();
    for kind in ["future", "stream"] {
        let module = artifact(
            &temp,
            &format!("{kind}.wasm"),
            &format!(
                r#"(component
              (type $async ({kind} u32))
              (type $optional (option $async))
              (core module $m
                (memory (export "memory") 1)
                (func (export "take") (param i32 i32) unreachable)
                (func (export "make") (result i32) unreachable))
              (core instance $i (instantiate $m))
              (func (export "take") (param "value" $optional)
                (canon lift (core func $i "take")))
              (func (export "make") (result $optional)
                (canon lift (core func $i "make") (memory (core memory $i "memory")))))"#
            ),
        );
        for (function, params) in [("take", vec![json!({"none": null})]), ("make", vec![])] {
            let reply = worker
                .request(Request {
                    module: module.clone(),
                    function: function.into(),
                    params,
                    ..Default::default()
                })
                .unwrap();
            assert!(!reply.ok);
            assert_eq!(
                reply.error.as_deref(),
                Some(format!("{kind} transfer is not supported by the worker protocol").as_str())
            );
            assert!(reply.results.is_empty());
        }
    }
    assert!(
        worker
            .request(Request {
                operation: "ping".into(),
                ..Default::default()
            })
            .unwrap()
            .ok
    );
}

#[test]
fn precompiled_modules_components_and_preloads_use_the_workers_engine() {
    use std::process::Command;
    let temp = TempDir::new().unwrap();
    let host = env!("CARGO_BIN_EXE_wisp-worker");
    let provider = artifact(
        &temp,
        "provider.wasm",
        r#"(module (func (export "answer") (result i32) i32.const 42))"#,
    );
    let component = artifact(
        &temp,
        "consumer.wasm",
        r#"(component
        (core type $provider (module (export "answer" (func (result i32)))))
        (import "provider" (core module $provider (type $provider)))
        (core instance $instance (instantiate $provider))
        (alias core export $instance "answer" (core func $answer))
        (func (export "answer") (result s32) (canon lift (core func $answer))))"#,
    );
    let compile = |input: &str, name: &str| {
        let output = temp.path().join(name);
        let result = Command::new(host)
            .env("RAYON_NUM_THREADS", "1")
            .env("TOKIO_WORKER_THREADS", "1")
            .args(["precompile", input, "-o"])
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        output.to_str().unwrap().to_owned()
    };
    let native_provider = compile(&provider, "provider.cwasm");
    let native_component = compile(&component, "consumer.cwasm");
    let rejected = Command::new(host)
        .args(["run", "--invoke", "answer", &native_provider])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("--allow-precompiled"));
    let accepted = Command::new(host)
        .args([
            "run",
            "--allow-precompiled",
            "--invoke",
            "answer",
            &native_provider,
        ])
        .output()
        .unwrap();
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&accepted.stdout).trim(), "42");
    let rejected = client()
        .request(Request {
            module: native_component.clone(),
            function: "answer".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(!rejected.ok);
    assert!(rejected.error.unwrap().contains("--allow-precompiled"));
    let mut worker = Client::spawn_with_precompiled(Path::new(host), None).unwrap();
    let response = worker
        .request(Request {
            module: native_component.clone(),
            function: "answer".into(),
            modules: [("provider".into(), native_provider)].into(),
            ..Default::default()
        })
        .unwrap();
    assert!(response.ok, "{:?}", response.error);
    assert_eq!(response.results, vec![json!(42)]);
    let response = worker
        .request(Request {
            module: native_component,
            sha256: Some("0".repeat(64)),
            function: "answer".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(!response.ok);
    assert!(response.error.unwrap().contains("checksum mismatch"));
}
