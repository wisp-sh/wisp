//! Author-supplied metadata and notices, preserved by regeneration admission.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Authored {
    description: Option<String>,
    license: Option<String>,
    repository: Option<String>,
    homepage: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
    #[serde(default)]
    notice_files: Vec<PathBuf>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub description: Option<String>,
    pub license: Option<String>,
    pub repository: Option<String>,
    pub homepage: Option<String>,
    pub authors: Vec<String>,
    pub notices: BTreeMap<String, String>,
}

impl Authored {
    pub fn load(self, directory: &Path) -> Result<Metadata> {
        let directory = directory.canonicalize()?;
        ensure!(self.notice_files.len() <= 16, "too many notice files");
        let mut notices = BTreeMap::new();
        for file in self.notice_files {
            ensure!(
                file.components()
                    .all(|part| matches!(part, Component::Normal(_))),
                "unsafe notice path"
            );
            let path = directory.join(&file).canonicalize()?;
            ensure!(path.starts_with(&directory), "notice file escapes package");
            ensure!(
                fs::metadata(&path)?.len() <= 256 * 1024,
                "notice file size limit"
            );
            let name = file
                .file_name()
                .and_then(|s| s.to_str())
                .context("notice filename")?;
            ensure!(
                notices
                    .insert(name.into(), fs::read_to_string(path)?)
                    .is_none(),
                "duplicate notice filename"
            );
        }
        let metadata = Metadata {
            description: self.description,
            license: self.license,
            repository: self.repository,
            homepage: self.homepage,
            authors: self.authors,
            notices,
        };
        metadata.validate()?;
        Ok(metadata)
    }
}

impl Metadata {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.authors.len() <= 64 && self.notices.len() <= 16,
            "metadata entry count limit"
        );
        for text in self
            .description
            .iter()
            .chain(self.license.iter())
            .chain(self.repository.iter())
            .chain(self.homepage.iter())
            .chain(self.authors.iter())
        {
            ensure!(
                !text.is_empty() && text.len() <= 4096 && !text.chars().any(char::is_control),
                "invalid package metadata text"
            );
        }
        let mut total = 0;
        for (name, notice) in &self.notices {
            ensure!(
                !name.is_empty()
                    && name.len() <= 128
                    && name != "."
                    && name != ".."
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
                "unsafe notice filename"
            );
            ensure!(
                notice.len() <= 256 * 1024 && !notice.contains('\0'),
                "invalid notice content"
            );
            total += notice.len();
        }
        ensure!(total <= 1024 * 1024, "total notice size limit");
        Ok(())
    }

    pub fn cargo_fields(&self) -> Result<String> {
        let mut fields = toml::map::Map::new();
        for (name, value) in [
            (
                "description",
                self.description
                    .as_deref()
                    .or(Some("Wisp Component library")),
            ),
            ("license", self.license.as_deref()),
            ("repository", self.repository.as_deref()),
            ("homepage", self.homepage.as_deref()),
        ] {
            if let Some(value) = value {
                fields.insert(name.into(), value.into());
            }
        }
        if !self.authors.is_empty() {
            fields.insert("authors".into(), toml::Value::try_from(&self.authors)?);
        }
        Ok(toml::to_string(&fields)?)
    }

    pub fn node_fields(&self, manifest: &mut serde_json::Value) {
        let fields = manifest.as_object_mut().expect("generated manifest object");
        for (name, value) in [
            ("description", &self.description),
            ("license", &self.license),
            ("repository", &self.repository),
            ("homepage", &self.homepage),
        ] {
            if let Some(value) = value {
                fields.insert(name.into(), value.clone().into());
            }
        }
        if !self.authors.is_empty() {
            fields.insert("contributors".into(), serde_json::json!(self.authors));
        }
    }

    pub fn go_files(&self) -> BTreeMap<String, Vec<u8>> {
        let mut files = self.files(false);
        files.insert("WISP-NOTICES.txt".into(), b"This Go module contains a native Wasmtime runtime compiled from the pinned build-shim/Cargo.lock. Upstream dependency notices accompany the generated module. Author-supplied Component notices are under notices/. Wisp does not infer a Component license.\n".to_vec());
        files
    }

    pub fn files(&self, node: bool) -> BTreeMap<String, Vec<u8>> {
        let mut files = self
            .notices
            .iter()
            .map(|(name, text)| (format!("notices/{name}"), text.as_bytes().to_vec()))
            .collect::<BTreeMap<_, _>>();
        let notice = if node {
            "JavaScript support code was generated by JCO 1.33.0, including jco-transpile 0.13.0. Its Apache-2.0 WITH LLVM-exception license is included in WISP-JCO-LICENSE.txt. Author-supplied Component notices are under notices/. Package metadata describes the author's supplied license; Wisp does not infer a license for the Component.\n"
        } else {
            "Native builds depend on Wasmtime 50.0.0; Component builds depend on wit-bindgen 0.61.1. Cargo retrieves their source packages and license notices during dependency installation. Author-supplied Component notices are under notices/. Package metadata describes the author's supplied license; Wisp does not infer a license for the Component.\n"
        };
        files.insert("WISP-NOTICES.txt".into(), notice.as_bytes().to_vec());
        if node {
            files.insert(
                "WISP-JCO-LICENSE.txt".into(),
                include_bytes!("../assets/jco-LICENSE").to_vec(),
            );
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_preserves_notices_and_quotes_without_descriptor_injection() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("LICENSE"), "Author license text\n").unwrap();
        let authored: Authored = toml::from_str(
            "description = 'Text with \"quotes\"'\nlicense = 'MIT'\nnotice-files = ['LICENSE']",
        )
        .unwrap();
        let metadata = authored.load(temp.path()).unwrap();
        let fields: toml::Value = toml::from_str(&metadata.cargo_fields().unwrap()).unwrap();
        assert_eq!(fields["description"].as_str(), Some("Text with \"quotes\""));
        assert_eq!(
            metadata.files(false)["notices/LICENSE"],
            b"Author license text\n"
        );
        let mut unsafe_metadata = metadata;
        unsafe_metadata
            .notices
            .insert("../injected".into(), "bad".into());
        assert!(unsafe_metadata.validate().is_err());
    }
}
