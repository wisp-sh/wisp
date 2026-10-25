//! Verified shared core modules supplied to compiler Components at instantiation.
use crate::build::{Tool, digest, read};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub(super) struct CoreModule {
    pub name: String,
    pub tool: Tool,
}
pub(super) struct SharedLlvm {
    root: PathBuf,
    manifest_sha256: String,
    pub modules: Vec<CoreModule>,
}

impl SharedLlvm {
    pub fn load(root: &Path) -> Result<Self> {
        let root = root.canonicalize().context("shared LLVM package root")?;
        ensure!(
            root.is_dir(),
            "WISP_LLVM must select the shared LLVM package directory"
        );
        let bytes = read(&root.join("manifest.json"), 1024 * 1024)?;
        let manifest: Value = crate::release::strict_json(&bytes)?;
        ensure!(
            manifest["schema"] == 1 && manifest["profile"] == "llvm24-wasip3",
            "unsupported shared LLVM package profile"
        );
        let entries = manifest["coreModules"]
            .as_array()
            .context("shared LLVM core modules")?;
        ensure!(
            !entries.is_empty() && entries.len() <= 16,
            "shared LLVM module count limit"
        );
        let mut names = BTreeSet::new();
        let mut paths = BTreeSet::new();
        let mut modules = Vec::new();
        for entry in entries {
            let name = entry["name"]
                .as_str()
                .context("shared LLVM module import name")?;
            ensure!(
                !name.is_empty()
                    && name.len() <= 64
                    && name.bytes().all(|byte| byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || byte == b'-')
                    && names.insert(name.to_owned()),
                "invalid or duplicate shared LLVM module name"
            );
            let relative = entry["path"].as_str().context("shared LLVM module path")?;
            crate::release::safe_name(relative)?;
            ensure!(
                paths.insert(relative.to_owned()),
                "duplicate shared LLVM module path"
            );
            let path = root.join(relative).canonicalize()?;
            ensure!(
                path.starts_with(&root),
                "shared LLVM module escapes its package"
            );
            let contents = read(&path, 256 * 1024 * 1024)?;
            ensure!(
                contents.starts_with(b"\0asm\x01\0\0\0"),
                "shared LLVM dependency must be a core Wasm module"
            );
            let sha256 = digest(&contents);
            ensure!(
                entry["sha256"].as_str() == Some(sha256.as_str())
                    && entry["sizeBytes"].as_u64() == Some(contents.len() as u64),
                "shared LLVM module checksum or size mismatch: {name}"
            );
            modules.push(CoreModule {
                name: name.to_owned(),
                tool: Tool { path, sha256 },
            });
        }
        Ok(Self {
            root,
            manifest_sha256: digest(&bytes),
            modules,
        })
    }
    pub fn receipt(&self) -> Value {
        json!({"path":self.root,"manifestSha256":self.manifest_sha256,"profile":"llvm24-wasip3",
            "linkage":"shared-core-module-imports","coreModules":self.modules.iter().map(|module|
                json!({"name":module.name,"path":module.tool.path,"sha256":module.tool.sha256})).collect::<Vec<_>>()})
    }
    pub fn validate_unchanged(&self) -> Result<()> {
        ensure!(
            digest(&read(&self.root.join("manifest.json"), 1024 * 1024)?) == self.manifest_sha256,
            "shared LLVM manifest changed during build"
        );
        for module in &self.modules {
            ensure!(
                digest(&read(&module.tool.path, 256 * 1024 * 1024)?) == module.tool.sha256,
                "shared LLVM module changed during build: {}",
                module.name
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verifies_shared_module_bytes_before_exposing_them_to_a_compiler() {
        let root = tempfile::tempdir().unwrap();
        let module = b"\0asm\x01\0\0\0";
        std::fs::write(root.path().join("llvm.wasm"), module).unwrap();
        std::fs::write(
            root.path().join("manifest.json"),
            serde_json::to_vec(&json!({
            "schema":1,"profile":"llvm24-wasip3","coreModules":[{"name":"wisp-llvm24",
                "path":"llvm.wasm","sha256":digest(module),"sizeBytes":module.len()}]}))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(SharedLlvm::load(root.path()).unwrap().modules.len(), 1);
        std::fs::write(root.path().join("llvm.wasm"), b"\0asm\x01\0\0\0changed").unwrap();
        let error = SharedLlvm::load(root.path()).err().unwrap();
        assert!(error.to_string().contains("checksum or size mismatch"));
    }
}
