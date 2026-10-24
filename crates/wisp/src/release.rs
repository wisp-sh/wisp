//! Strict JSON parsing and bounded archive reading shared by library packaging.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
};
pub type Files = BTreeMap<String, Vec<u8>>;
pub fn strict_json(data: &[u8]) -> Result<Value> {
    struct Strict(Value);
    impl<'de> serde::Deserialize<'de> for Strict {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Strict;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    write!(f, "strict JSON")
                }
                fn visit_bool<E: serde::de::Error>(
                    self,
                    v: bool,
                ) -> std::result::Result<Strict, E> {
                    Ok(Strict(json!(v)))
                }
                fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Strict, E> {
                    Ok(Strict(json!(v)))
                }
                fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Strict, E> {
                    Ok(Strict(json!(v)))
                }
                fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Strict, E> {
                    Ok(Strict(json!(v)))
                }
                fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Strict, E> {
                    Ok(Strict(json!(v)))
                }
                fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut a: A,
                ) -> std::result::Result<Strict, A::Error> {
                    let mut v = Vec::new();
                    while let Some(Strict(x)) = a.next_element()? {
                        v.push(x)
                    }
                    Ok(Strict(Value::Array(v)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut a: A,
                ) -> std::result::Result<Strict, A::Error> {
                    let mut v = serde_json::Map::new();
                    while let Some((k, Strict(x))) = a.next_entry::<String, Strict>()? {
                        if v.insert(k, x).is_some() {
                            return Err(serde::de::Error::custom("duplicate JSON key"));
                        }
                    }
                    Ok(Strict(Value::Object(v)))
                }
            }
            d.deserialize_any(Visitor)
        }
    }
    Ok(serde_json::from_slice::<Strict>(data)?.0)
}
pub fn exact(v: &Value, keys: &[&str], label: &str) -> Result<()> {
    let actual: BTreeSet<_> = v
        .as_object()
        .with_context(|| format!("invalid {label} object"))?
        .keys()
        .map(String::as_str)
        .collect();
    ensure!(
        actual == keys.iter().copied().collect(),
        "invalid {label} fields"
    );
    Ok(())
}
pub fn safe_name(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty()
            && s.len() <= 240
            && s.bytes().all(|b| (32..=126).contains(&b) && b != b'\\')
            && s.split('/').all(|p| !p.is_empty() && p != "." && p != ".."),
        "unsafe archive path"
    );
    Ok(())
}
fn octal(bytes: &[u8]) -> Result<usize> {
    let digits_start = bytes
        .iter()
        .position(|byte| *byte != b' ')
        .unwrap_or(bytes.len());
    let digits_end = bytes[digits_start..]
        .iter()
        .position(|byte| !(b'0'..=b'7').contains(byte))
        .map_or(bytes.len(), |offset| digits_start + offset);
    ensure!(digits_start < digits_end, "invalid tar numeric field");
    let suffix = &bytes[digits_end..];
    let terminator = suffix.iter().position(|byte| *byte == 0);
    ensure!(
        suffix
            .iter()
            .enumerate()
            .all(|(index, byte)| *byte == b' ' || Some(index) == terminator)
            && terminator.is_none_or(|index| !suffix[index + 1..].contains(&0)),
        "invalid tar numeric field"
    );
    Ok(usize::from_str_radix(
        std::str::from_utf8(&bytes[digits_start..digits_end])?,
        8,
    )?)
}
fn text(bytes: &[u8]) -> Result<&str> {
    Ok(std::str::from_utf8(
        bytes.split(|b| *b == 0).next().unwrap(),
    )?)
}
pub fn archive_files(data: &[u8]) -> Result<Files> {
    Ok(archive_files_limited(data, 256 * 1024 * 1024)?.0)
}

fn archive_files_limited(data: &[u8], expanded_limit: usize) -> Result<(Files, usize)> {
    let mut decoder = flate2::bufread::GzDecoder::new(Cursor::new(data));
    let mut raw = Vec::new();
    decoder
        .by_ref()
        .take(expanded_limit as u64 + 1)
        .read_to_end(&mut raw)?;
    ensure!(raw.len() <= expanded_limit, "archive expanded size limit");
    ensure!(
        decoder.into_inner().position() == data.len() as u64,
        "truncated or concatenated gzip"
    );
    let mut out = Files::new();
    let mut seen = BTreeSet::<String>::new();
    let mut offset = 0;
    let mut pax = false;
    while offset + 512 <= raw.len() {
        let h = &raw[offset..offset + 512];
        if h.iter().all(|b| *b == 0) {
            ensure!(
                !pax && raw.len() - offset >= 1024
                    && raw.len() % 512 == 0
                    && raw[offset..].iter().all(|b| *b == 0),
                "invalid tar termination"
            );
            return Ok((out, raw.len()));
        }
        let checksum: usize = h
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    *b as usize
                }
            })
            .sum();
        ensure!(octal(&h[148..156])? == checksum, "invalid tar checksum");
        let size = octal(&h[124..136])?;
        ensure!(size <= 256 * 1024 * 1024, "archive member size limit");
        let end = offset + 512 + size.div_ceil(512) * 512;
        ensure!(end <= raw.len(), "truncated tar payload");
        let payload = &raw[offset + 512..offset + 512 + size];
        if h[156] == b'x' {
            ensure!(
                !pax && size <= 16384,
                "consecutive or oversized PAX metadata"
            );
            pax = true;
            let mut pos = 0;
            while pos < size {
                let space = payload[pos..]
                    .iter()
                    .position(|b| *b == b' ')
                    .context("invalid PAX record")?
                    + pos;
                let length: usize = std::str::from_utf8(&payload[pos..space])?.parse()?;
                ensure!(
                    length > space - pos + 1 && length <= size - pos,
                    "invalid PAX length"
                );
                let entry = &payload[space + 1..pos + length];
                ensure!(entry.last() == Some(&b'\n'), "invalid PAX record");
                let key = entry.split(|b| *b == b'=').next().unwrap();
                ensure!(
                    [b"mtime".as_slice(), b"atime", b"ctime"].contains(&key)
                        && entry.contains(&b'='),
                    "unsupported PAX metadata"
                );
                pos += length;
            }
        } else {
            ensure!(
                [0, b'0', b'5'].contains(&h[156]),
                "unsupported tar member type"
            );
            pax = false;
            let mut name = text(&h[..100])?.to_owned();
            let prefix = text(&h[345..500])?;
            if !prefix.is_empty() {
                name = format!("{prefix}/{name}");
            }
            if h[156] == b'5' {
                name = name.trim_end_matches('/').to_owned();
            }
            safe_name(&name)?;
            ensure!(
                seen.len() < 4096 && seen.insert(name.clone()),
                "duplicate archive member or members limit"
            );
            ensure!(
                !out.keys().any(|n| name.starts_with(&format!("{n}/")))
                    && !(h[156] != b'5' && seen.iter().any(|n| n.starts_with(&format!("{name}/")))),
                "file/directory collision"
            );
            if h[156] != b'5' {
                out.insert(name, payload.to_vec());
            }
        }
        offset = end;
    }
    bail!("missing tar termination")
}
#[cfg(test)]
pub fn tar(files: &Files) -> Result<Vec<u8>> {
    use std::io::Write;
    let mut raw = Vec::new();
    for (name, data) in files {
        safe_name(name)?;
        let mut h = [0u8; 512];
        if name.len() <= 100 {
            h[..name.len()].copy_from_slice(name.as_bytes())
        } else {
            let (prefix, base) = name.rsplit_once('/').context("tar name too long")?;
            ensure!(
                prefix.len() <= 155 && base.len() <= 100,
                "tar path too long"
            );
            h[..base.len()].copy_from_slice(base.as_bytes());
            h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
        }
        h[100..108].copy_from_slice(b"0000644\0");
        h[108..116].copy_from_slice(b"0000000\0");
        h[116..124].copy_from_slice(b"0000000\0");
        h[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        h[136..148].copy_from_slice(b"00000000000\0");
        h[148..156].fill(b' ');
        h[156] = b'0';
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        let sum: usize = h.iter().map(|b| *b as usize).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        raw.extend(h);
        raw.extend(data);
        raw.resize(raw.len().div_ceil(512) * 512, 0);
    }
    raw.extend([0; 1024]);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&raw)?;
    Ok(gz.finish()?)
}
#[cfg(test)]
mod tests {
    use super::octal;

    #[test]
    fn tar_octal_accepts_canonical_terminator_layouts() {
        for field in [
            b"00000000665\0".as_slice(),
            b"0000000665 \0",
            b"   665     ",
        ] {
            assert_eq!(octal(field).unwrap(), 0o665);
        }
    }
}
