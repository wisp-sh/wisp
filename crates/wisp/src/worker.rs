use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use wisp_worker_protocol::{Request, Response, client::Client};

pub fn resolve() -> Result<PathBuf> {
    discover()?.context("install wisp-worker beside wisp or set WISP_WORKER")
}

// Absence permits a compatible CLI fallback; a broken explicit selection does not.
pub(crate) fn discover() -> Result<Option<PathBuf>> {
    if let Some(path) = std::env::var_os("WISP_WORKER") {
        return PathBuf::from(path)
            .canonicalize()
            .map(Some)
            .context("WISP_WORKER");
    }
    let name = if cfg!(windows) {
        "wisp-worker.exe"
    } else {
        "wisp-worker"
    };
    let sibling = std::env::current_exe()?.with_file_name(name);
    if sibling.is_file() {
        return Ok(Some(sibling.canonicalize()?));
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let path = dir.join(name);
        if path.is_file() {
            return Ok(Some(path.canonicalize()?));
        }
    }
    Ok(None)
}

// Reuse engines and compiled artifacts throughout a build. Each run still gets
// a fresh Store, selected environment, and explicit filesystem capabilities.
pub fn execute(path: &Path, cache: &Path, request: Request) -> Result<Response> {
    static CLIENTS: OnceLock<Mutex<HashMap<(PathBuf, PathBuf), Client>>> = OnceLock::new();
    let mut clients = CLIENTS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| anyhow::anyhow!("worker client lock poisoned"))?;
    let key = (path.to_owned(), cache.to_owned());
    if !clients.contains_key(&key) {
        clients.insert(key.clone(), Client::spawn(path, Some(cache))?);
    }
    let response = clients.get_mut(&key).unwrap().request(request);
    if response.is_err() {
        clients.remove(&key);
    }
    response.context("Wisp worker execution")
}

pub fn run(args: &[String]) -> Result<i32> {
    use std::io::Write;
    let request = wisp_worker_protocol::command::parse_run_with_cache(args, true)?;
    let cache = std::env::temp_dir().join("wisp-worker-cache");
    std::fs::create_dir_all(&cache)?;
    let mut client = Client::spawn(&resolve()?, Some(&cache))?;
    let response = client.request(request)?;
    std::io::stdout().write_all(&STANDARD.decode(&response.stdout)?)?;
    std::io::stderr().write_all(&STANDARD.decode(&response.stderr)?)?;
    for value in &response.results {
        println!("{value}");
    }
    if let Some(error) = &response.error {
        eprintln!("wisp: {error}");
    }
    ensure!(
        response.exit_code != 0 || response.ok,
        "worker failed without an exit status"
    );
    Ok(response.exit_code)
}
