use crate::Request;
use anyhow::{Context, Result, bail};
// A small compatibility frontend lets existing tool packages transition without
// depending on the Wasmtime CLI. Execution always goes through the library above.
pub fn parse_run(args: &[String]) -> Result<Request> {
    parse_run_with_cache(args, std::env::var_os("XDG_CACHE_HOME").is_some())
}

/// Parse for a launcher that configures the worker's cache before engine creation.
/// Cache configuration cannot be changed by an individual protocol request.
pub fn parse_run_with_cache(args: &[String], cache_enabled: bool) -> Result<Request> {
    let mut request = Request::default();
    let mut inherit_env = false;
    let mut restrict_sockets = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let mut value = || -> Result<&str> {
            i += 1;
            args.get(i)
                .map(String::as_str)
                .context("missing option value")
        };
        match arg.as_str() {
            "--dir" => {
                let mount = value()?;
                let (host, guest) = mount.split_once("::").unwrap_or((mount, mount));
                request.mounts.push(crate::Mount {
                    host: host.into(),
                    guest: guest.into(),
                    read_only: false,
                });
            }
            "--env" => {
                let (k, v) = value()?.split_once('=').context("expected NAME=VALUE")?;
                request.env.insert(k.into(), v.into());
            }
            "--preload" => {
                let (k, v) = value()?.split_once('=').context("expected NAME=PATH")?;
                request.modules.insert(k.into(), v.into());
            }
            "--invoke" => {
                request.function = value()?.into();
            }
            "--timeout-ms" => {
                request.limits.timeout_ms = value()?.parse()?;
            }
            "--memory-bytes" => {
                request.limits.memory_bytes = value()?.parse()?;
            }
            _ if ["-C", "-O", "-W", "-S"]
                .iter()
                .any(|prefix| arg.starts_with(prefix)) =>
            {
                let (group, attached) = arg.split_at(2);
                let option = if attached.is_empty() {
                    value()?
                } else {
                    attached
                };
                match (group, option) {
                    ("-C", "cache=y") if cache_enabled => {}
                    ("-C", "cache=y") => bail!("worker cache must be configured before startup"),
                    ("-S", "p3=y") => request.wasi_p3 = true,
                    ("-S", "p3=n") => request.wasi_p3 = false,
                    ("-S", "inherit-network=y") => request.network = true,
                    ("-S", "inherit-network=n") => request.network = false,
                    ("-S", "inherit-env=y") => inherit_env = true,
                    ("-S", "inherit-env=n") => inherit_env = false,
                    ("-S", "tcp=n" | "udp=n") => restrict_sockets = true,
                    ("-W", option) if option.starts_with("timeout=") => {
                        request.limits.timeout_ms = option[8..]
                            .parse::<u64>()?
                            .checked_mul(1000)
                            .context("worker timeout exceeds limit")?;
                    }
                    // These values match the worker's fixed engine configuration.
                    // Other values need protocol support, even when Wasmtime's
                    // CLI accepts them. In particular, async-stack-size is absent.
                    ("-O", "opt-level=0")
                    | ("-C", "parallel-compilation=n")
                    | (
                        "-W",
                        "exceptions=y" | "threads=y" | "memory64=y" | "max-wasm-stack=33554432",
                    )
                    | ("-S", "http=n") => {}
                    _ => bail!("unsupported worker execution option {group} {option}"),
                }
            }
            "--" => {
                i += 1;
                break;
            }
            _ if arg.starts_with('-') => bail!("unsupported worker option {arg}"),
            _ => break,
        }
        i += 1;
    }
    if request.network && restrict_sockets {
        bail!("worker protocol cannot combine inherited networking with socket restrictions");
    }
    if inherit_env {
        let explicit = std::mem::take(&mut request.env);
        request.env.extend(std::env::vars());
        request.env.extend(explicit);
    }
    request.module = args
        .get(i)
        .context("missing Wasm module or Component")?
        .clone();
    request.args = args[i + 1..].to_vec();
    if !request.function.is_empty() {
        // Core-module CLI calls use positional numeric parameters. Structured
        // Component calls use the JSON protocol instead.
        request.params = request
            .args
            .iter()
            .map(|a| serde_json::from_str(a).context("invalid core argument"))
            .collect::<Result<_>>()?;
        request.args.clear();
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Request> {
        parse_run_with_cache(
            &args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>(),
            true,
        )
    }

    #[test]
    fn supported_compact_and_separate_flags_produce_the_same_request() {
        let compact = parse(&[
            "-Ccache=y",
            "-Sp3=y",
            "-Oopt-level=0",
            "-Wtimeout=2",
            "compiler.wasm",
            "--version",
        ])
        .unwrap();
        let separate = parse(&[
            "-C",
            "cache=y",
            "-S",
            "p3=y",
            "-O",
            "opt-level=0",
            "-W",
            "timeout=2",
            "compiler.wasm",
            "--version",
        ])
        .unwrap();
        assert_eq!(
            serde_json::to_value(&compact).unwrap(),
            serde_json::to_value(&separate).unwrap()
        );
        assert!(compact.wasi_p3);
        assert_eq!(compact.limits.timeout_ms, 2000);
        assert_eq!(compact.args, ["--version"]);
    }

    #[test]
    fn rejects_settings_the_worker_protocol_cannot_honor() {
        for args in [
            vec![
                "-Ccache=y",
                "-Sp3=y",
                "-Wasync-stack-size=33554432",
                "rustc.wasm",
            ],
            vec!["-W", "async-stack-size=33554432", "rustc.wasm"],
            vec!["-Ccache=n", "compiler.wasm"],
            vec!["-Oopt-level=2", "compiler.wasm"],
            vec!["-Wmax-wasm-stack=1024", "compiler.wasm"],
            vec!["-Cp3=y", "compiler.wasm"],
        ] {
            assert!(
                parse(&args)
                    .unwrap_err()
                    .to_string()
                    .contains("unsupported worker execution option"),
                "{args:?}"
            );
        }
    }

    #[test]
    fn rejects_network_restrictions_that_would_be_ignored() {
        for flags in [
            ["-Sinherit-network=y", "-Stcp=n"],
            ["-Sudp=n", "-Sinherit-network=y"],
        ] {
            assert!(parse(&[flags[0], flags[1], "compiler.wasm"]).is_err());
        }
        assert!(
            !parse(&[
                "-Sinherit-network=n",
                "-Stcp=n",
                "-Sudp=n",
                "-Shttp=n",
                "compiler.wasm"
            ])
            .unwrap()
            .network
        );
    }

    #[test]
    fn reports_missing_values_and_timeout_overflow() {
        assert!(parse(&["-S"]).is_err());
        assert!(parse(&["-Wtimeout=18446744073709551615", "compiler.wasm"]).is_err());
    }

    #[test]
    fn cache_flags_require_cache_configuration_at_worker_startup() {
        let args = vec!["-Ccache=y".into(), "compiler.wasm".into()];
        assert!(parse_run_with_cache(&args, true).is_ok());
        assert!(
            parse_run_with_cache(&args, false)
                .unwrap_err()
                .to_string()
                .contains("cache must be configured before startup")
        );
    }
}
