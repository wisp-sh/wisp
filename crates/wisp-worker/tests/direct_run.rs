use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use wisp_worker_protocol::{Request, client::Client};

const WORKER: &str = env!("CARGO_BIN_EXE_wisp-worker");

fn sleeper(directory: &TempDir, nanos: u64) -> PathBuf {
    let path = directory.path().join("sleep.wasm");
    let source = format!(
        r#"(module
        (import "wasi_snapshot_preview1" "poll_oneoff"
            (func $poll (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (func (export "ready") (result i32) i32.const 42)
        (func (export "_start")
            i32.const 16 i32.const 1 i32.store
            i32.const 24 i64.const {nanos} i64.store
            i32.const 0 i32.const 64 i32.const 1 i32.const 128 call $poll drop))"#
    );
    fs::write(&path, wat::parse_str(source).unwrap()).unwrap();
    path
}

fn run(args: &[&str], module: &Path) -> Output {
    let mut child = Command::new(WORKER)
        .arg("run")
        .args(args)
        .arg(module)
        .env_remove("XDG_CACHE_HOME")
        .env("TOKIO_WORKER_THREADS", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Bound the regression itself: the old direct runner never interrupted a
    // suspended host call or a blocking artifact open at the requested deadline.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "direct worker exceeded watchdog: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn direct_run_times_out_a_suspended_wasi_timer() {
    let directory = TempDir::new().unwrap();
    let module = sleeper(&directory, 30_000_000_000);
    let output = run(&["--timeout-ms", "200"], &module);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("worker response:"),
        "{output:?}"
    );
}

#[test]
fn direct_run_preserves_successful_calls_and_short_host_sleeps() {
    let directory = TempDir::new().unwrap();
    let module = sleeper(&directory, 1_000_000);
    let output = run(&["--timeout-ms", "3000", "--invoke", "ready"], &module);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"42\n");
    let output = run(&["--timeout-ms", "3000"], &module);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn direct_deadline_also_covers_blocking_artifact_reads() {
    let directory = TempDir::new().unwrap();
    let fifo = directory.path().join("blocked.wasm");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    // Like compilation, File::open does not yield to an async timeout. No large
    // compiler workload is needed to verify the process deadline covers it.
    let output = run(&["--timeout-ms", "200"], &fifo);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("worker response:"),
        "{output:?}"
    );
}

#[test]
fn host_call_timeout_terminates_the_client_and_all_its_sessions() {
    let directory = TempDir::new().unwrap();
    let module = sleeper(&directory, 30_000_000_000)
        .to_str()
        .unwrap()
        .to_owned();
    let component = directory.path().join("instance.wasm");
    fs::write(
        &component,
        wat::parse_str(
            r#"(component
        (core module $m (func (export "ready") (result i32) i32.const 42))
        (core instance $i (instantiate $m))
        (func (export "ready") (result u32) (canon lift (core func $i "ready"))))"#,
        )
        .unwrap(),
    )
    .unwrap();
    let mut client = Client::spawn(Path::new(WORKER), None).unwrap();
    let open = client
        .request(Request {
            operation: "open".into(),
            module: component.to_str().unwrap().to_owned(),
            ..Default::default()
        })
        .unwrap();
    assert!(open.ok, "{open:?}");
    // Compile before starting the short deadline so it tests the sleeping host
    // future, independently of the speed of compilation on a CI runner.
    assert!(
        client
            .request(Request {
                module: module.clone(),
                function: "ready".into(),
                ..Default::default()
            })
            .unwrap()
            .ok
    );
    let mut request = Request {
        module: module.clone(),
        ..Default::default()
    };
    request.limits.timeout_ms = 200;
    assert_eq!(
        client.request(request).unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
    let call = Request {
        operation: "call".into(),
        instance: open.instance,
        function: "ready".into(),
        ..Default::default()
    };
    assert_eq!(
        client.request(call.clone()).unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    let mut replacement = Client::spawn(Path::new(WORKER), None).unwrap();
    assert!(!replacement.request(call).unwrap().ok);
    assert!(
        replacement
            .request(Request {
                module,
                function: "ready".into(),
                ..Default::default()
            })
            .unwrap()
            .ok
    );
}
