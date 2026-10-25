mod build;
mod build_go;
mod build_llvm;
mod build_rust;
mod compiler_hosts;
mod dependencies;
mod generator_tools;
mod identity;
mod inspect;
mod library;
mod library_async;
mod library_error_context;
mod library_go;
mod library_metadata;
mod library_publish;
mod library_resources;
mod library_wasi;
mod limits;
mod release;
mod toolchain_install;
mod toolchain_package;
mod wit_model;
mod worker;
use anyhow::{Context, Result, bail};
use std::path::Path;
const USAGE: &str = "usage: wisp run [OPTIONS] COMPONENT [ARGS...] | wisp build [PROJECT_DIR|CONFIG.toml] [--out OUTPUT] | wisp inspect [--ownership] COMPONENT | wisp toolchain <validate|install|select|list|env|doctor|hosts|setup-hosts> ... | wisp library pack PACKAGE_DIR --out OUTPUT | wisp library validate OUTPUT | wisp library prepare DIR --config CONFIG --out PLAN | wisp library publish PLAN --state STATE | wisp deps <lock|compose> ...";

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") if args.len() == 1 => {
            println!("wisp {}", env!("CARGO_PKG_VERSION"))
        }
        Some("--help" | "-h") if args.len() == 1 => {
            println!("Wisp {}\n\n{USAGE}", env!("CARGO_PKG_VERSION"));
        }
        Some("run") => std::process::exit(worker::run(&args[1..])?),
        Some("build") => println!(
            "{}",
            serde_json::to_string_pretty(&build::run_cli(&args[1..])?)?
        ),
        Some("deps") => println!(
            "{}",
            serde_json::to_string_pretty(&dependencies::run_cli(&args[1..])?)?
        ),
        Some("library") => println!(
            "{}",
            serde_json::to_string_pretty(&library::run_cli(&args[1..])?)?
        ),
        Some("toolchain") => {
            let report = toolchain_package::run_cli(&args[1..])?;
            if args.get(1).is_some_and(|command| command == "env") {
                print!("{}", report.as_str().context("toolchain environment")?);
            } else {
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
        }
        Some("inspect") if args.len() == 3 && args[1] == "--ownership" => println!(
            "{}",
            serde_json::to_string_pretty(&inspect::ownership(Path::new(&args[2]))?)?
        ),
        Some("inspect") if args.len() == 2 => println!(
            "{}",
            serde_json::to_string_pretty(&inspect::inspect(Path::new(&args[1]))?)?
        ),
        _ => bail!(USAGE),
    }
    Ok(())
}
fn main() {
    if let Err(e) = run().context("wisp") {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
