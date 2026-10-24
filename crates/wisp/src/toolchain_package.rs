//! Streaming admission for compiler bundles, separate from application releases.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

const MAX_PACKAGE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_EXPANDED: u64 = 4 * 1024 * 1024 * 1024;
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;
const MAX_FILES: usize = 50_000;
const FORMATS: &[&str] = &[
    "wasm-component",
    "wasm-core",
    "wasm-archive",
    "native",
    "host-script",
    "data",
];
const ROLES: &[&str] = &[
    "compiler",
    "stdlib",
    "headers",
    "runtime",
    "adapter",
    "licenses",
    "metadata",
    "sdk",
    "documentation",
];

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// Check framing without scanning or loading the payload. ZIP64 is unnecessary
/// below this format's 2 GiB total limit and is deliberately unsupported.
fn zip_layout(input: &mut File) -> Result<()> {
    let length = input.metadata()?.len();
    ensure!(
        (22..=MAX_PACKAGE).contains(&length),
        "toolchain package size limit"
    );
    input.seek(SeekFrom::End(-22))?;
    let mut end = [0; 22];
    input.read_exact(&mut end)?;
    ensure!(
        end[..4] == *b"PK\x05\x06" && u16_at(&end, 20) == 0,
        "toolchain ZIP termination or comment"
    );
    ensure!(
        u16_at(&end, 4) == 0
            && u16_at(&end, 6) == 0
            && u16_at(&end, 8) == 2
            && u16_at(&end, 10) == 2,
        "toolchain ZIP requires two entries on one disk"
    );
    let central_start = u32_at(&end, 16) as u64;
    let central_size = u32_at(&end, 12) as u64;
    ensure!(
        central_start + central_size == length - 22,
        "toolchain ZIP central directory framing"
    );
    let mut central_at = central_start;
    let mut regions = Vec::new();
    let mut names = BTreeSet::new();
    for expected_name in ["release.json", "toolchain.tar.gz"] {
        ensure!(
            central_at + 46 <= length - 22,
            "truncated toolchain ZIP central header"
        );
        input.seek(SeekFrom::Start(central_at))?;
        let mut central = [0; 46];
        input.read_exact(&mut central)?;
        ensure!(
            central[..4] == *b"PK\x01\x02",
            "invalid toolchain ZIP central header"
        );
        let flags = u16_at(&central, 8);
        let name_length = u16_at(&central, 28) as usize;
        ensure!(
            flags == 0 && u16_at(&central, 10) == 0 && u16_at(&central, 6) <= 20,
            "unsupported toolchain ZIP encoding"
        );
        ensure!(
            u16_at(&central, 30) == 0 && u16_at(&central, 32) == 0 && u16_at(&central, 34) == 0,
            "toolchain ZIP extra fields, comments, or multiple disks forbidden"
        );
        ensure!(
            (1..=32).contains(&name_length) && central_at + 46 + name_length as u64 <= length - 22,
            "toolchain ZIP name length"
        );
        let size = u32_at(&central, 24) as u64;
        ensure!(
            u32_at(&central, 20) as u64 == size,
            "toolchain ZIP must use stored entries"
        );
        let mut name = vec![0; name_length];
        input.read_exact(&mut name)?;
        let name = std::str::from_utf8(&name)?.to_owned();
        ensure!(
            name == expected_name && names.insert(name.clone()),
            "unexpected or duplicate toolchain ZIP entry"
        );
        ensure!(
            size <= if name == "release.json" {
                MAX_MANIFEST
            } else {
                MAX_PACKAGE
            },
            "toolchain ZIP member size limit"
        );
        let mode = u32_at(&central, 38) >> 16;
        ensure!(
            (mode == 0 || mode & 0o170000 == 0o100000 || mode & 0o170000 == 0)
                && u32_at(&central, 38) & 0x10 == 0,
            "toolchain ZIP links, directories, or special files forbidden"
        );
        let local_at = u32_at(&central, 42) as u64;
        ensure!(
            local_at + 30 + name_length as u64 + size <= central_start,
            "toolchain ZIP member bounds"
        );
        input.seek(SeekFrom::Start(local_at))?;
        let mut local = [0; 30];
        input.read_exact(&mut local)?;
        ensure!(
            local[..4] == *b"PK\x03\x04"
                && u16_at(&local, 4) == u16_at(&central, 6)
                && u16_at(&local, 6) == flags
                && local[8..26] == central[10..28]
                && u16_at(&local, 26) as usize == name_length
                && u16_at(&local, 28) == 0,
            "toolchain ZIP local and central headers differ"
        );
        let mut local_name = vec![0; name_length];
        input.read_exact(&mut local_name)?;
        ensure!(
            local_name == name.as_bytes(),
            "toolchain ZIP local name mismatch"
        );
        regions.push((local_at, local_at + 30 + name_length as u64 + size));
        central_at += 46 + name_length as u64;
    }
    ensure!(
        central_at == central_start + central_size
            && regions[0].0 == 0
            && regions[0].1 == regions[1].0
            && regions[1].1 == central_start,
        "toolchain ZIP records must exactly cover the container"
    );
    Ok(())
}

fn text<'a>(value: &'a Value, maximum: usize, label: &str) -> Result<&'a str> {
    let value = value
        .as_str()
        .with_context(|| format!("{label} must be a string"))?;
    ensure!(
        value.len() <= maximum && !value.chars().any(char::is_control),
        "invalid {label}"
    );
    Ok(value)
}
fn sha(value: &Value) -> Result<&str> {
    let value = text(value, 64, "SHA-256")?;
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid SHA-256"
    );
    Ok(value)
}
fn integer(value: &Value, label: &str) -> Result<u64> {
    value
        .as_u64()
        .with_context(|| format!("{label} must be a nonnegative integer"))
}

fn metadata(record: &Value) -> Result<BTreeMap<String, &Value>> {
    use crate::release::exact;
    exact(
        record,
        &[
            "schema",
            "kind",
            "identity",
            "version",
            "tool",
            "upstreamVersion",
            "platform",
            "archive",
            "entrypoints",
            "requirements",
            "sizes",
            "bootstrapRequirements",
        ],
        "toolchain release",
    )?;
    ensure!(
        record["schema"].as_u64() == Some(4) && record["kind"] == "toolchain",
        "unsupported toolchain package schema or kind"
    );
    let identity = text(&record["identity"], 133, "toolchain identity")?;
    let (namespace, name) = crate::identity::parse(identity)?;
    ensure!(
        namespace == "wisp" && text(&record["tool"], 64, "tool name")? == name,
        "toolchain identity mismatch"
    );
    let version = text(&record["version"], 128, "package version")?;
    ensure!(
        semver::Version::parse(version)?.to_string() == version,
        "noncanonical toolchain package version"
    );
    ensure!(
        !text(&record["upstreamVersion"], 256, "upstream version")?.is_empty(),
        "empty upstream version"
    );
    ensure!(
        record["platform"] == "linux-x86_64" || record["platform"] == "wasm32-wasi",
        "unsupported toolchain platform"
    );
    let archive = &record["archive"];
    exact(
        archive,
        &["file", "size", "sha256", "inventory"],
        "toolchain archive",
    )?;
    ensure!(
        archive["file"] == "toolchain.tar.gz"
            && integer(&archive["size"], "archive size")? <= MAX_PACKAGE,
        "invalid toolchain archive"
    );
    sha(&archive["sha256"])?;
    let inventory = archive["inventory"]
        .as_object()
        .context("toolchain inventory must be an object")?;
    ensure!(
        !inventory.is_empty() && inventory.len() <= MAX_FILES,
        "toolchain inventory file count limit"
    );
    let mut files = BTreeMap::new();
    let mut sizes = [0u64; 4];
    for (path, file) in inventory {
        crate::release::safe_name(path)?;
        crate::release::safe_name(&format!("wisp-buildtools/{path}"))?;
        exact(
            file,
            &["size", "sha256", "mode", "format", "role"],
            "toolchain inventory file",
        )?;
        let size = integer(&file["size"], "file size")?;
        ensure!(size <= MAX_PACKAGE, "toolchain individual file size limit");
        sha(&file["sha256"])?;
        ensure!(
            matches!(file["mode"].as_u64(), Some(0o644 | 0o755)),
            "invalid toolchain file mode"
        );
        let format = text(&file["format"], 32, "file format")?;
        ensure!(
            FORMATS.contains(&format),
            "unsupported toolchain file format"
        );
        ensure!(
            ROLES.contains(&text(&file["role"], 32, "file role")?),
            "unsupported toolchain file role"
        );
        sizes[0] = sizes[0]
            .checked_add(size)
            .context("installed size overflow")?;
        let category = match format {
            "wasm-component" | "wasm-core" | "wasm-archive" => 1,
            "native" => 2,
            _ => 3,
        };
        sizes[category] = sizes[category]
            .checked_add(size)
            .context("format size overflow")?;
        files.insert(path.clone(), file);
    }
    ensure!(sizes[0] <= MAX_EXPANDED, "toolchain installed size limit");
    ensure!(
        record["platform"] != "wasm32-wasi" || sizes[2] == 0,
        "portable toolchain package cannot include native binaries"
    );
    for path in files.keys() {
        let mut parent = path.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            ensure!(
                !files.contains_key(prefix),
                "toolchain inventory file/directory collision"
            );
            parent = prefix;
        }
    }
    exact(
        &record["sizes"],
        &["installedBytes", "wasmBytes", "nativeBytes", "supportBytes"],
        "toolchain sizes",
    )?;
    for (key, expected) in ["installedBytes", "wasmBytes", "nativeBytes", "supportBytes"]
        .into_iter()
        .zip(sizes)
    {
        ensure!(
            integer(&record["sizes"][key], key)? == expected,
            "toolchain {key} mismatch"
        );
    }
    let entrypoints = record["entrypoints"]
        .as_array()
        .context("entrypoints must be an array")?;
    ensure!(entrypoints.len() <= 64, "toolchain entrypoints limit");
    let mut names = BTreeSet::new();
    for entry in entrypoints {
        exact(entry, &["name", "path", "format"], "toolchain entrypoint")?;
        let name = text(&entry["name"], 128, "entrypoint name")?;
        ensure!(
            !name.is_empty() && names.insert(name),
            "empty or duplicate toolchain entrypoint name"
        );
        let path = text(&entry["path"], 240, "entrypoint path")?;
        let file = files
            .get(path)
            .context("entrypoint missing from toolchain inventory")?;
        ensure!(
            entry["format"] == file["format"],
            "toolchain entrypoint format mismatch"
        );
    }
    let requirements = record["requirements"]
        .as_array()
        .context("requirements must be an array")?;
    ensure!(requirements.len() <= 64, "toolchain requirements limit");
    let mut names = BTreeSet::new();
    for requirement in requirements {
        exact(
            requirement,
            &["name", "kind", "version", "included", "note"],
            "toolchain requirement",
        )?;
        let name = text(&requirement["name"], 128, "requirement name")?;
        ensure!(
            !name.is_empty() && names.insert(name),
            "empty or duplicate toolchain requirement name"
        );
        ensure!(
            [
                "runtime-host",
                "orchestrator",
                "shared-tool",
                "system-library"
            ]
            .contains(&text(&requirement["kind"], 32, "requirement kind")?),
            "unsupported requirement kind"
        );
        text(&requirement["version"], 256, "requirement version")?;
        text(&requirement["note"], 4096, "requirement note")?;
        ensure!(
            requirement["included"].is_boolean(),
            "requirement included must be boolean"
        );
    }
    let bootstrap = record["bootstrapRequirements"]
        .as_array()
        .context("bootstrap requirements must be an array")?;
    ensure!(bootstrap.len() <= 64, "bootstrap requirements limit");
    for requirement in bootstrap {
        text(requirement, 4096, "bootstrap requirement")?;
    }
    Ok(files)
}

pub(crate) fn validate_metadata(record: &Value) -> Result<()> {
    metadata(record)?;
    Ok(())
}

struct Hashed<R> {
    input: R,
    sha: Sha256,
    count: u64,
}
impl<R: Read> Read for Hashed<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.input.read(buffer)?;
        self.sha.update(&buffer[..count]);
        self.count += count as u64;
        Ok(count)
    }
}

fn octal(bytes: &[u8]) -> Result<u64> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0 || *byte == b' ')
        .unwrap_or(bytes.len());
    ensure!(
        end > 0
            && bytes[..end].iter().all(|byte| (b'0'..=b'7').contains(byte))
            && bytes[end..].iter().all(|byte| *byte == 0 || *byte == b' '),
        "invalid toolchain tar numeric field"
    );
    Ok(u64::from_str_radix(std::str::from_utf8(&bytes[..end])?, 8)?)
}
fn tar_text(bytes: &[u8]) -> Result<&str> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    ensure!(
        bytes[end..].iter().all(|byte| *byte == 0),
        "invalid toolchain tar text padding"
    );
    Ok(std::str::from_utf8(&bytes[..end])?)
}
fn format_matches(path: &str, prefix: &[u8], format: &str) -> bool {
    let detected = if prefix.starts_with(b"\0asm\x0d\0\x01\0") {
        "wasm-component"
    } else if prefix.starts_with(b"\0asm\x01\0\0\0") {
        "wasm-core"
    } else if prefix.starts_with(b"!<arch>\n") {
        "wasm-archive"
    } else if prefix.starts_with(b"\x7fELF") || prefix.starts_with(b"MZ") {
        "native"
    } else if path.ends_with(".py") || path.ends_with(".sh") || prefix.starts_with(b"#!") {
        "host-script"
    } else {
        "data"
    };
    detected == format
}

fn tar_inventory<R: Read>(input: &mut R, files: &BTreeMap<String, &Value>) -> Result<()> {
    let mut expanded = 0u64;
    let mut seen = BTreeSet::new();
    let mut buffer = [0; 65536];
    loop {
        let mut header = [0; 512];
        input
            .read_exact(&mut header)
            .context("truncated toolchain tar header or missing termination")?;
        expanded += 512;
        ensure!(
            expanded <= MAX_EXPANDED,
            "toolchain expanded archive size limit"
        );
        if header.iter().all(|byte| *byte == 0) {
            let mut trailing = 0u64;
            loop {
                let count = input.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                expanded += count as u64;
                trailing += count as u64;
                ensure!(
                    expanded <= MAX_EXPANDED && buffer[..count].iter().all(|byte| *byte == 0),
                    "invalid toolchain tar trailing bytes or size"
                );
            }
            ensure!(
                trailing >= 512 && expanded % 512 == 0,
                "invalid toolchain tar termination"
            );
            ensure!(
                seen.len() == files.len(),
                "toolchain tar inventory missing files"
            );
            return Ok(());
        }
        let checksum: u64 = header
            .iter()
            .enumerate()
            .map(|(at, byte)| {
                if (148..156).contains(&at) {
                    32
                } else {
                    *byte as u64
                }
            })
            .sum();
        ensure!(
            octal(&header[148..156])? == checksum,
            "toolchain tar checksum mismatch"
        );
        ensure!(
            header[257..263] == *b"ustar\0" && header[263..265] == *b"00",
            "toolchain tar must use USTAR"
        );
        ensure!(
            [0, b'0'].contains(&header[156]) && header[157..257].iter().all(|byte| *byte == 0),
            "toolchain tar links, directories, or special files forbidden"
        );
        ensure!(
            octal(&header[108..116])? == 0
                && octal(&header[116..124])? == 0
                && octal(&header[136..148])? == 0
                && header[265..329].iter().all(|byte| *byte == 0),
            "noncanonical toolchain tar ownership or timestamp"
        );
        let name = tar_text(&header[..100])?;
        let prefix = tar_text(&header[345..500])?;
        let path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        crate::release::safe_name(&path)?;
        let path = path
            .strip_prefix("wisp-buildtools/")
            .context("toolchain tar package root mismatch")?;
        let expected = files.get(path).context("unexpected toolchain tar member")?;
        ensure!(
            seen.len() < MAX_FILES && seen.insert(path.to_owned()),
            "duplicate toolchain tar member"
        );
        let size = octal(&header[124..136])?;
        ensure!(
            size <= MAX_PACKAGE && size == integer(&expected["size"], "file size")?,
            "toolchain tar member size mismatch"
        );
        ensure!(
            octal(&header[100..108])? == integer(&expected["mode"], "file mode")?,
            "toolchain tar mode mismatch"
        );
        let padded = size.checked_add(511).context("tar size overflow")? / 512 * 512;
        ensure!(
            expanded
                .checked_add(padded)
                .is_some_and(|value| value <= MAX_EXPANDED),
            "toolchain expanded archive size limit"
        );
        let mut remaining = size;
        let mut digest = Sha256::new();
        let mut first = Vec::new();
        while remaining != 0 {
            let count = remaining.min(buffer.len() as u64) as usize;
            input
                .read_exact(&mut buffer[..count])
                .context("truncated toolchain tar payload")?;
            if first.len() < 8 {
                first.extend_from_slice(&buffer[..count.min(8 - first.len())]);
            }
            digest.update(&buffer[..count]);
            remaining -= count as u64;
        }
        ensure!(
            format!("{:x}", digest.finalize()) == sha(&expected["sha256"])?,
            "toolchain file SHA-256 mismatch: {path}"
        );
        ensure!(
            format_matches(path, &first, expected["format"].as_str().unwrap()),
            "toolchain file format mismatch: {path}"
        );
        let padding = (padded - size) as usize;
        input.read_exact(&mut buffer[..padding])?;
        ensure!(
            buffer[..padding].iter().all(|byte| *byte == 0),
            "nonzero toolchain tar payload padding"
        );
        expanded += padded;
    }
}

pub(crate) fn admit(path: &Path) -> Result<Value> {
    let mut input = File::open(path)?;
    zip_layout(&mut input)?;
    let mut archive = zip::ZipArchive::new(input)?;
    let mut manifest = Vec::new();
    archive
        .by_name("release.json")?
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut manifest)?;
    ensure!(
        manifest.len() as u64 <= MAX_MANIFEST,
        "toolchain manifest size limit"
    );
    let record = crate::release::strict_json(&manifest)?;
    let files = metadata(&record)?;
    let payload = archive.by_name("toolchain.tar.gz")?;
    ensure!(
        payload.size() == integer(&record["archive"]["size"], "archive size")?,
        "toolchain archive size mismatch"
    );
    let hashed = Hashed {
        input: payload,
        sha: Sha256::new(),
        count: 0,
    };
    let mut decoder = flate2::bufread::GzDecoder::new(BufReader::new(hashed));
    tar_inventory(&mut decoder, &files)?;
    let mut buffered = decoder.into_inner();
    ensure!(
        buffered.fill_buf()?.is_empty(),
        "trailing or concatenated toolchain gzip"
    );
    let hashed = buffered.into_inner();
    ensure!(
        hashed.count == integer(&record["archive"]["size"], "archive size")?
            && format!("{:x}", hashed.sha.finalize()) == sha(&record["archive"]["sha256"])?,
        "toolchain archive SHA-256 or size mismatch"
    );
    Ok(record)
}

pub fn validate(path: &Path) -> Result<Value> {
    let record = admit(path)?;
    let files = record["archive"]["inventory"].as_object().unwrap();
    Ok(
        json!({"valid":true,"schema":4,"kind":"toolchain","identity":record["identity"],"version":record["version"],"tool":record["tool"],"upstreamVersion":record["upstreamVersion"],"platform":record["platform"],"archive":{ "size":record["archive"]["size"],"sha256":record["archive"]["sha256"] },"files":files.len(),"sizes":record["sizes"],"entrypoints":record["entrypoints"],"requirements":record["requirements"]}),
    )
}

pub fn run_cli(args: &[String]) -> Result<Value> {
    match args {
        [command, path] if command == "validate" => validate(Path::new(path)),
        [command] if command == "hosts" => Ok(crate::compiler_hosts::status()),
        _ => crate::toolchain_install::run_cli(args),
    }
}
