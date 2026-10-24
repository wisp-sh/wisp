//! One mapping from logical Component package identity to language package names.
use anyhow::{Context, Result, ensure};

pub fn parse(value: &str) -> Result<(&str, &str)> {
    let (namespace, name) = value
        .split_once(':')
        .context("package identity must be namespace:name")?;
    for part in [namespace, name] {
        ensure!(
            !part.is_empty()
                && part.len() <= 64
                && part.split('-').all(|word| {
                    !word.is_empty()
                        && word.as_bytes()[0].is_ascii_lowercase()
                        && word
                            .bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                }),
            "invalid package identity: {value}"
        );
    }
    Ok((namespace, name))
}

pub fn package_names(identity: &str) -> Result<(String, String)> {
    let (namespace, name) = parse(identity)?;
    Ok((
        format!("wisp-{namespace}--{name}"),
        format!("@{namespace}/{name}"),
    ))
}
