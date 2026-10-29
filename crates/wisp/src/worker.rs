use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    collections::HashMap,
    ffi::OsString,
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

fn run_cache_dir(get: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf> {
    let selected = if let Some(path) = get("WISP_WORKER_CACHE") {
        PathBuf::from(path)
    } else if let Some(path) = get("XDG_CACHE_HOME").filter(|path| !path.is_empty()) {
        PathBuf::from(path).join("wisp/worker")
    } else if cfg!(windows) {
        PathBuf::from(
            get("LOCALAPPDATA")
                .filter(|path| !path.is_empty())
                .context("set WISP_WORKER_CACHE, XDG_CACHE_HOME or LOCALAPPDATA")?,
        )
        .join("wisp/worker")
    } else {
        PathBuf::from(
            get("HOME")
                .filter(|path| !path.is_empty())
                .context("set WISP_WORKER_CACHE, XDG_CACHE_HOME or HOME")?,
        )
        .join(".cache/wisp/worker")
    };
    ensure!(
        selected.is_absolute(),
        "worker cache directory must be an absolute path"
    );
    Ok(selected)
}

pub fn run(args: &[String]) -> Result<i32> {
    use std::io::Write;
    let request = wisp_worker_protocol::command::parse_run_with_cache(args, true)?;
    let cache = run_cache_dir(|name| std::env::var_os(name))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn select(values: &[(&str, &Path)]) -> Result<PathBuf> {
        run_cache_dir(|name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.as_os_str().to_owned())
        })
    }

    #[test]
    fn run_cache_respects_explicit_override_then_xdg() {
        let root = std::env::current_dir().unwrap();
        let explicit = root.join("worker-cache");
        let xdg = root.join("user-cache");
        assert_eq!(
            select(&[
                ("WISP_WORKER_CACHE", &explicit),
                ("XDG_CACHE_HOME", &xdg),
                ("HOME", &root)
            ])
            .unwrap(),
            explicit
        );
        assert_eq!(
            select(&[("XDG_CACHE_HOME", &xdg), ("HOME", &root)]).unwrap(),
            xdg.join("wisp/worker")
        );
    }

    #[test]
    fn run_cache_uses_user_storage_and_ignores_temporary_directories() {
        let root = std::env::current_dir().unwrap();
        let home_key = if cfg!(windows) {
            "LOCALAPPDATA"
        } else {
            "HOME"
        };
        let suffix = if cfg!(windows) {
            "wisp/worker"
        } else {
            ".cache/wisp/worker"
        };
        assert_eq!(
            select(&[
                (home_key, &root),
                ("XDG_CACHE_HOME", Path::new("")),
                ("TMPDIR", &root.join("temporary"))
            ])
            .unwrap(),
            root.join(suffix)
        );
        assert!(select(&[("TMPDIR", &root)]).is_err());
    }

    #[test]
    fn run_cache_requires_an_absolute_nonempty_location() {
        for name in [
            "WISP_WORKER_CACHE",
            "XDG_CACHE_HOME",
            if cfg!(windows) {
                "LOCALAPPDATA"
            } else {
                "HOME"
            },
        ] {
            assert!(select(&[(name, Path::new("relative"))]).is_err(), "{name}");
            assert!(select(&[(name, Path::new(""))]).is_err(), "{name}");
        }
        let root = std::env::current_dir().unwrap();
        assert!(
            select(&[
                ("WISP_WORKER_CACHE", Path::new("")),
                ("XDG_CACHE_HOME", &root)
            ])
            .is_err()
        );
    }
}
