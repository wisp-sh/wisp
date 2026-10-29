mod runtime;
mod values;
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::io::{BufRead, Read, Write};
use wisp_worker_protocol::{MAX_FRAME, Request, VERSION, client::Client, command::parse_run};

async fn serve(allow_precompiled: bool) -> Result<()> {
    let mut worker = runtime::Worker::with_precompiled(allow_precompiled)?;
    let input = std::io::stdin();
    let mut input = input.lock();
    loop {
        let mut frame = Vec::new();
        let n = (&mut input)
            .take((MAX_FRAME + 1) as u64)
            .read_until(b'\n', &mut frame)?;
        if n == 0 {
            break;
        }
        ensure!(
            n <= MAX_FRAME && frame.last() == Some(&b'\n'),
            "worker request frame exceeds limit"
        );
        let request: Request = serde_json::from_slice(&frame).context("invalid worker request")?;
        let response = worker.execute(request).await;
        serde_json::to_writer(std::io::stdout().lock(), &response)?;
        std::io::stdout().write_all(b"\n")?;
        std::io::stdout().flush()?;
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result: Result<i32> = async {
        match args.first().map(String::as_str) {
            Some("--version") => {
                println!(
                    "wisp-worker {} wasmtime {} protocol {VERSION}",
                    env!("CARGO_PKG_VERSION"),
                    wisp_worker_protocol::WASMTIME_VERSION
                );
                Ok(0)
            }
            Some("serve") => {
                ensure!(args.len() == 1 || args[1..] == ["--allow-precompiled"], "usage: wisp-worker serve [--allow-precompiled]");
                serve(args.len() == 2).await?;
                Ok(0)
            }
            Some("precompile") => {
                ensure!(args.len() == 4 && args[2] == "-o", "usage: wisp-worker precompile MODULE -o OUTPUT");
                let bytes = runtime::Worker::new()?.precompile(&args[1])?;
                let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&args[3])?;
                output.write_all(&bytes)?;
                Ok(0)
            }
            Some("run") => {
                let allow_precompiled = args.get(1).is_some_and(|arg| arg == "--allow-precompiled");
                let request = parse_run(&args[if allow_precompiled { 2 } else { 1 }..])?;
                // Supervise a separate process, just like `wisp run`. Epochs do
                // not interrupt suspended host futures, and an async timeout
                // cannot preempt synchronous compilation. Client kills and reaps
                // the worker on timeout, dropping every guest store and session.
                let executable = std::env::current_exe()?;
                let cache = std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from);
                let mut client = if allow_precompiled {
                    Client::spawn_with_precompiled(&executable, cache.as_deref())?
                } else {
                    Client::spawn(&executable, cache.as_deref())?
                };
                let response = client.request(request)?;
                std::io::stdout().write_all(&STANDARD.decode(response.stdout)?)?;
                std::io::stderr().write_all(&STANDARD.decode(response.stderr)?)?;
                for value in response.results {
                    println!("{value}");
                }
                if let Some(error) = response.error {
                    eprintln!("wisp-worker: {error}");
                }
                Ok(response.exit_code)
            }
            _ => bail!(
                "usage: wisp-worker serve [--allow-precompiled] | run [--allow-precompiled] [--dir HOST::GUEST] MODULE [ARGS...] | precompile MODULE -o OUTPUT | --version"
            ),
        }
    }
    .await;
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("wisp-worker: {error:#}");
            std::process::exit(1);
        }
    }
}
