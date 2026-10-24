//! Direct use of the Go toolchain's WASIp1 compiler/linker package, without a native Go tool.
use crate::build::{digest, read, run_wasm, write};
use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub struct Toolchain {
    pub root: PathBuf,
    pub goroot: PathBuf,
    pub compile: PathBuf,
    pub link: PathBuf,
    pub stdlib: BTreeMap<String, String>,
    pub sha256: String,
    pub version: String,
}
impl Toolchain {
    pub fn load(root: &Path) -> Result<Self> {
        let root = root.canonicalize().context("Go toolchain directory")?;
        let sums = read(&root.join("SHA256SUMS"), 1024 * 1024)?;
        let mut expected = BTreeSet::new();
        for line in std::str::from_utf8(&sums)?.lines() {
            let (hash, name) = line
                .split_once("  ")
                .context("invalid Go SHA256SUMS entry")?;
            crate::release::safe_name(name)?;
            ensure!(
                expected.insert(name.to_owned()),
                "duplicate Go checksum entry"
            );
            let file = root.join(name);
            ensure!(
                file.canonicalize()?.starts_with(&root),
                "Go toolchain path escapes root"
            );
            ensure!(
                digest(&read(&file, 256 * 1024 * 1024)?) == hash,
                "Go toolchain checksum mismatch: {name}"
            );
        }
        fn visit(root: &Path, dir: &Path, actual: &mut BTreeSet<String>) -> Result<()> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                ensure!(!kind.is_symlink(), "symlink in Go toolchain");
                if kind.is_dir() {
                    visit(root, &entry.path(), actual)?;
                } else {
                    ensure!(kind.is_file(), "non-regular Go toolchain file");
                    actual.insert(
                        entry
                            .path()
                            .strip_prefix(root)?
                            .to_str()
                            .context("UTF8 toolchain path")?
                            .replace('\\', "/"),
                    );
                }
            }
            Ok(())
        }
        let mut actual = BTreeSet::new();
        visit(&root, &root, &mut actual)?;
        actual.remove("SHA256SUMS");
        ensure!(
            actual == expected,
            "Go toolchain file set differs from SHA256SUMS"
        );
        let manifest: toml::Value =
            toml::from_str(&fs::read_to_string(root.join("manifest.toml"))?)?;
        ensure!(
            matches!(
                manifest["package"]["version"].as_str(),
                Some("1.27.1" | "1.27.2")
            ) && manifest["host"]["kind"].as_str() == Some("wasip1")
                && manifest["host"]["arch"].as_str() == Some("wasm32"),
            "Go requires a packaged Go 1.27.1 or 1.27.2 WASIp1 toolchain"
        );
        let version = manifest["package"]["version"].as_str().unwrap().to_owned();
        ensure!(
            manifest["capabilities"]["filesystem"].as_bool() == Some(true)
                && manifest["capabilities"]["network"].as_bool() == Some(false),
            "unsupported Go toolchain capabilities"
        );
        let tool = |name: &str| -> Result<PathBuf> {
            let path = manifest["tools"][name]["path"]
                .as_str()
                .context("Go tool path")?;
            ensure!(expected.contains(path), "unlisted Go tool: {path}");
            Ok(root.join(path))
        };
        let goroot_path = manifest["resources"]["goroot"]
            .as_str()
            .context("Go GOROOT")?;
        crate::release::safe_name(goroot_path)?;
        let goroot = root.join(goroot_path).canonicalize()?;
        ensure!(goroot.starts_with(&root), "Go GOROOT escapes package");
        let prefix = format!("{goroot_path}/pkg/wasip1_wasm/");
        let stdlib = expected
            .iter()
            .filter_map(|path| {
                Some((
                    path.strip_prefix(&prefix)?.strip_suffix(".a")?.to_owned(),
                    format!("/goroot/{}", path.strip_prefix(&format!("{goroot_path}/"))?),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        ensure!(
            stdlib.len() as i64
                == manifest["package"]["stdlib_archive_count"]
                    .as_integer()
                    .context("Go standard-library count")?,
            "Go standard-library count mismatch"
        );
        Ok(Self {
            compile: tool("compile")?,
            link: tool("link")?,
            root,
            goroot,
            stdlib,
            sha256: digest(&sums),
            version,
        })
    }

    pub fn receipt(&self) -> serde_json::Value {
        serde_json::json!({"path":self.root,"sha256SumsSha256":self.sha256,"version":self.version})
    }
}
#[derive(Debug)]
struct Package {
    name: String,
    files: Vec<PathBuf>,
    imports: BTreeSet<String>,
}
fn tokens(source: &str) -> Result<Vec<String>> {
    let re = Regex::new(
        r#"(?s)//[^\n]*|/\*.*?\*/|"(?:\\.|[^"\\])*"|`[^`]*`|[A-Za-z_][A-Za-z_0-9]*|[^\s]"#,
    )?;
    Ok(re
        .find_iter(source)
        .map(|m| m.as_str())
        .filter(|s| !s.starts_with("//") && !s.starts_with("/*"))
        .map(str::to_owned)
        .collect())
}
fn inspect(source: &str) -> Result<(String, BTreeSet<String>)> {
    let ts = tokens(source)?;
    ensure!(
        ts.len() >= 2 && ts[0] == "package",
        "missing Go package clause"
    );
    let mut imports = BTreeSet::new();
    let mut i = 2;
    while i < ts.len() {
        if ts[i] != "import" {
            i += 1;
            continue;
        }
        i += 1;
        let group = ts.get(i).is_some_and(|t| t == "(");
        if group {
            i += 1;
        }
        let mut found = false;
        while i < ts.len() {
            if ts[i] == ")" {
                ensure!(group, "invalid Go import");
                i += 1;
                break;
            }
            if ts[i].starts_with(['"', '`']) {
                let path = if ts[i].starts_with('"') {
                    serde_json::from_str::<String>(&ts[i])
                        .context("unsupported Go import string escape")?
                } else {
                    ts[i][1..ts[i].len() - 1].to_owned()
                };
                ensure!(
                    !path.chars().any(char::is_control),
                    "invalid Go import path"
                );
                imports.insert(path);
                found = true;
                i += 1;
                if !group {
                    break;
                }
            } else {
                i += 1;
            }
        }
        ensure!(found, "invalid Go import declaration");
    }
    Ok((ts[1].clone(), imports))
}
fn constraint(expression: &str) -> Result<bool> {
    let token = Regex::new(r"&&|\|\||!|\(|\)|[A-Za-z0-9_.]+")?;
    let ts: Vec<_> = token.find_iter(expression).map(|m| m.as_str()).collect();
    ensure!(
        ts.concat()
            == expression
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>(),
        "invalid Go build constraint"
    );
    fn primary(ts: &[&str], at: &mut usize) -> Result<bool> {
        let item = *ts.get(*at).context("invalid Go build constraint")?;
        *at += 1;
        match item {
            "!" => Ok(!primary(ts, at)?),
            "(" => {
                let value = or(ts, at)?;
                ensure!(ts.get(*at) == Some(&")"), "invalid Go build constraint");
                *at += 1;
                Ok(value)
            }
            "&&" | "||" | ")" => bail!("invalid Go build constraint"),
            "wasip1" | "wasm" | "gc" => Ok(true),
            tag if tag.starts_with("go1.") => Ok(tag[4..].parse::<u32>().is_ok_and(|v| v <= 27)),
            _ => Ok(false),
        }
    }
    fn and(ts: &[&str], at: &mut usize) -> Result<bool> {
        let mut value = primary(ts, at)?;
        while ts.get(*at) == Some(&"&&") {
            *at += 1;
            let next = primary(ts, at)?;
            value &= next;
        }
        Ok(value)
    }
    fn or(ts: &[&str], at: &mut usize) -> Result<bool> {
        let mut value = and(ts, at)?;
        while ts.get(*at) == Some(&"||") {
            *at += 1;
            let next = and(ts, at)?;
            value |= next;
        }
        Ok(value)
    }
    let mut at = 0;
    let value = or(&ts, &mut at)?;
    ensure!(at == ts.len(), "invalid Go build constraint");
    Ok(value)
}
pub(super) fn selected(path: &Path, source: &str) -> Result<bool> {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("Go filename")?;
    if name.starts_with(['.', '_']) || name.ends_with("_test") {
        return Ok(false);
    }
    let parts: Vec<_> = name.split('_').collect();
    let os = [
        "aix",
        "android",
        "darwin",
        "dragonfly",
        "freebsd",
        "hurd",
        "illumos",
        "ios",
        "js",
        "linux",
        "netbsd",
        "openbsd",
        "plan9",
        "solaris",
        "wasip1",
        "windows",
    ];
    let arch = [
        "386", "amd64", "amd64p32", "arm", "arm64", "loong64", "mips", "mipsle", "mips64",
        "mips64le", "ppc64", "ppc64le", "riscv64", "s390x", "wasm",
    ];
    let last = parts[parts.len() - 1];
    if arch.contains(&last) && last != "wasm" {
        return Ok(false);
    }
    if os.contains(&last) && last != "wasip1" {
        return Ok(false);
    }
    if arch.contains(&last) && parts.len() >= 3 {
        let before = parts[parts.len() - 2];
        if os.contains(&before) && before != "wasip1" {
            return Ok(false);
        }
    }
    for line in source.lines() {
        if line.starts_with("package ") {
            break;
        }
        if let Some(expression) = line.strip_prefix("//go:build ") {
            return constraint(expression);
        }
    }
    Ok(true)
}
fn importcfg(mapping: &BTreeMap<String, String>) -> String {
    mapping
        .iter()
        .map(|(name, path)| format!("packagefile {name}={path}\n"))
        .collect()
}

/// Resolve a Wisp Component carried by a normally installed Go dependency.
pub fn published_component(project: &Path, module: &str) -> Result<Option<(PathBuf, String)>> {
    crate::library_go::validate_module_path(module)?;
    let directory = project.join("vendor").join(module);
    if !directory.try_exists()? {
        return Ok(None);
    }
    ensure!(
        directory
            .canonicalize()?
            .starts_with(project.canonicalize()?),
        "vendored module escapes project"
    );
    let descriptor = crate::library_go::guest::vendored(&directory, module)?;
    let version = descriptor["version"]
        .as_str()
        .context("Go module version")?;
    let inventory = fs::read_to_string(project.join("vendor/modules.txt"))?;
    ensure!(
        inventory
            .lines()
            .any(|line| line == format!("# {module} v{version}")),
        "vendored Go module version mismatch"
    );
    let go_mod = fs::read_to_string(project.join("go.mod"))?;
    let requirement = Regex::new(&format!(
        r"(?m)^\s*(?:require\s+)?{}\s+v{}(?:\s|$)",
        regex::escape(module),
        regex::escape(version)
    ))?;
    ensure!(
        requirement.is_match(&go_mod),
        "Go module must be pinned in go.mod require"
    );
    let path = directory.join("component.wasm");
    Ok(Some((
        path,
        descriptor["component"]["sha256"]
            .as_str()
            .context("Go module Component hash")?
            .into(),
    )))
}

fn project_packages(source: &Path) -> Result<(String, BTreeMap<String, Package>, Vec<String>)> {
    let prefix = if source.join("go.mod").is_file() {
        let module = fs::read_to_string(source.join("go.mod"))?;
        Regex::new(r"(?m)^\s*module\s+(\S+)")?
            .captures(&module)
            .context("go.mod module directive")?[1]
            .trim_matches(['"', '`'])
            .to_owned()
    } else {
        "project".into()
    };
    fn discover(
        dir: &Path,
        source: &Path,
        prefix: &str,
        packages: &mut BTreeMap<String, Package>,
    ) -> Result<()> {
        let mut files = Vec::new();
        let mut imports = BTreeSet::new();
        let mut name = None;
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let filename = entry.file_name();
            let filename = filename.to_str().context("UTF8 Go input name")?;
            if filename.starts_with('.')
                || filename.starts_with('_')
                || matches!(
                    filename,
                    "target" | "out" | "dist" | "node_modules" | "testdata"
                )
            {
                continue;
            }
            ensure!(
                !entry.file_type()?.is_symlink(),
                "symlink in Go source input"
            );
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                if path == source.join("vendor") && !prefix.is_empty() {
                    discover(&path, &path, "", packages)?;
                } else {
                    discover(&path, source, prefix, packages)?;
                }
                continue;
            }
            if path.extension().is_none_or(|s| s != "go") {
                continue;
            }
            let text = fs::read_to_string(&path)?;
            if !selected(&path, &text)? {
                continue;
            }
            ensure!(
                !text.lines().any(|l| l.starts_with("//go:embed")),
                "Go embed inputs are not supported by the Go adapter"
            );
            let (found, found_imports) = inspect(&text)?;
            ensure!(
                name.as_ref().is_none_or(|prior| prior == &found),
                "mixed Go packages in {}",
                dir.display()
            );
            name = Some(found);
            imports.extend(found_imports);
            files.push(path);
        }
        if let Some(name) = name {
            files.sort();
            let relative = dir
                .strip_prefix(source)?
                .to_str()
                .context("Go package path")?
                .replace('\\', "/");
            let import = if prefix.is_empty() {
                ensure!(
                    relative
                        .split('/')
                        .next()
                        .is_some_and(|part| part.contains('.')),
                    "invalid vendored Go import path"
                );
                relative
            } else if relative.is_empty() {
                prefix.to_owned()
            } else {
                format!("{prefix}/{relative}")
            };
            packages.insert(
                import,
                Package {
                    name,
                    files,
                    imports,
                },
            );
        }
        Ok(())
    }
    let mut packages = BTreeMap::new();
    discover(source, source, &prefix, &mut packages)?;
    ensure!(
        packages.get(&prefix).is_some_and(|p| p.name == "main"),
        "Go project root must contain package main"
    );
    fn order(
        name: &str,
        packages: &BTreeMap<String, Package>,
        active: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
        result: &mut Vec<String>,
    ) -> Result<()> {
        if done.contains(name) {
            return Ok(());
        }
        ensure!(active.insert(name.into()), "Go import cycle at {name}");
        for import in &packages[name].imports {
            if packages.contains_key(import) {
                order(import, packages, active, done, result)?;
            }
        }
        active.remove(name);
        done.insert(name.into());
        result.push(name.into());
        Ok(())
    }
    let mut ordered = Vec::new();
    order(
        &prefix,
        &packages,
        &mut BTreeSet::new(),
        &mut BTreeSet::new(),
        &mut ordered,
    )?;
    Ok((prefix, packages, ordered))
}

/// Vendored packages selected by the Wasm import graph. Native-only runtime
/// dependencies must not enter the Go toolchain's source tree.
pub(super) fn vendor_roots(source: &Path) -> Result<BTreeSet<PathBuf>> {
    let (_, packages, ordered) = project_packages(source)?;
    let vendor = source.join("vendor");
    Ok(ordered
        .iter()
        .filter_map(|name| packages[name].files.first())
        .filter(|path| path.starts_with(&vendor))
        .filter_map(|path| path.parent().map(Path::to_owned))
        .collect())
}

/// Find Wisp modules on the same reachable Go import graph used for compilation.
pub(super) fn component_modules(source: &Path) -> Result<BTreeSet<String>> {
    let (_, packages, ordered) = project_packages(source)?;
    let vendor = source.join("vendor");
    let mut modules = BTreeSet::new();
    for name in ordered {
        let package = &packages[&name];
        for import in &package.imports {
            ensure!(
                packages.contains_key(import)
                    || !import.split('/').next().is_some_and(|p| p.contains('.')),
                "Go package {import} is not installed; run go mod vendor before building"
            );
        }
        let Some(file) = package.files.first() else {
            continue;
        };
        if !file.starts_with(&vendor) {
            continue;
        }
        let mut directory = file.parent().context("Go package directory")?;
        while directory != vendor {
            if directory.join("wisp-component.json").try_exists()? {
                modules.insert(
                    directory
                        .strip_prefix(&vendor)?
                        .to_str()
                        .context("Go module path")?
                        .replace('\\', "/"),
                );
                break;
            }
            directory = directory.parent().context("vendored module root")?;
        }
    }
    Ok(modules)
}

pub fn compile(
    runtime: &crate::compiler_hosts::ExecutionHost,
    tools: &Toolchain,
    source: &Path,
    work: &Path,
    cache: &Path,
    commands: &mut Vec<serde_json::Value>,
) -> Result<Vec<u8>> {
    let (prefix, packages, ordered) = project_packages(source)?;
    let mut archives = tools.stdlib.clone();
    let mut main = String::new();
    for (index, name) in ordered.iter().enumerate() {
        let package = &packages[name];
        let mut mapping = BTreeMap::new();
        for import in &package.imports {
            if import == "unsafe" {
                continue;
            }
            mapping.insert(import.clone(), archives.get(import).with_context(||format!("Go package {name} imports unavailable package {import}; install module dependencies with go mod vendor; cgo is not supported"))?.clone());
        }
        let cfg = format!("importcfg-{index}");
        write(work.join(&cfg), importcfg(&mapping))?;
        let archive = format!("/build/package-{index}.a");
        let mut args = vec![
            "-p".into(),
            if name == &prefix {
                "main".into()
            } else {
                name.clone()
            },
            "-pack".into(),
            "-trimpath".into(),
            "/src".into(),
            "-importcfg".into(),
            format!("/build/{cfg}"),
            "-o".into(),
            archive.clone(),
        ];
        for path in &package.files {
            args.push(format!(
                "/src/{}",
                path.strip_prefix(source)?
                    .to_str()
                    .context("Go source path")?
                    .replace('\\', "/")
            ));
        }
        run_wasm(
            runtime,
            &tools.compile,
            false,
            &args,
            &[
                (source, "/src"),
                (work, "/build"),
                (&tools.goroot, "/goroot"),
            ],
            &[
                ("PWD", "/build"),
                ("TMPDIR", "/build"),
                ("GOROOT", "/goroot"),
                ("GOOS", "wasip1"),
                ("GOARCH", "wasm"),
            ],
            cache,
            &[],
            commands,
        )?;
        if name == &prefix {
            main = archive;
        } else {
            archives.insert(name.clone(), archive);
        }
    }
    write(work.join("link.importcfg"), importcfg(&archives))?;
    run_wasm(
        runtime,
        &tools.link,
        false,
        &[
            "-buildmode=c-shared".into(),
            "-linkmode=internal".into(),
            "-importcfg".into(),
            "/build/link.importcfg".into(),
            "-o".into(),
            "/build/core.wasm".into(),
            main,
        ],
        &[(work, "/build"), (&tools.goroot, "/goroot")],
        &[
            ("PWD", "/build"),
            ("TMPDIR", "/build"),
            ("GOROOT", "/goroot"),
            ("GOOS", "wasip1"),
            ("GOARCH", "wasm"),
        ],
        cache,
        &[],
        commands,
    )?;
    read(&work.join("core.wasm"), 16 * 1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_report_the_verified_go_package_version() {
        for version in ["1.27.1", "1.27.2"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            fs::create_dir(root.join("go")).unwrap();
            let manifest = format!(
                r#"
[package]
version = "{version}"
stdlib_archive_count = 0
[host]
kind = "wasip1"
arch = "wasm32"
[capabilities]
filesystem = true
network = false
[tools.compile]
path = "compile.wasm"
[tools.link]
path = "link.wasm"
[resources]
goroot = "go"
"#
            );
            let files = [
                ("manifest.toml", manifest.as_bytes()),
                ("compile.wasm", b"compiler".as_slice()),
                ("link.wasm", b"linker".as_slice()),
            ];
            let mut sums = String::new();
            for (name, bytes) in files {
                fs::write(root.join(name), bytes).unwrap();
                sums.push_str(&format!("{}  {name}\n", digest(bytes)));
            }
            fs::write(root.join("SHA256SUMS"), sums).unwrap();
            let toolchain = Toolchain::load(root).unwrap();
            assert_eq!(toolchain.receipt()["version"], version);
            assert_eq!(toolchain.receipt()["sha256SumsSha256"], toolchain.sha256);
        }
    }

    #[test]
    fn go_imports_ignore_comments_and_literals() {
        let (name,imports) = inspect("package main\n// import \"bad\"\nimport ( alias \"fmt\"; `strings` )\nconst text = `import \"bad\"`\n").unwrap();
        assert_eq!(name, "main");
        assert_eq!(imports, BTreeSet::from(["fmt".into(), "strings".into()]));
    }
    #[test]
    fn target_selection_uses_suffixes_and_constraints() {
        for (name, source, expected) in [
            ("main_linux_wasm.go", "package main", false),
            ("main_wasip1_wasm.go", "package main", true),
            ("main_test.go", "package main", false),
            (
                "main.go",
                "//go:build (wasip1 && wasm) && !cgo && go1.27\npackage main",
                true,
            ),
            ("main.go", "//go:build linux || go1.28\npackage main", false),
        ] {
            assert_eq!(
                selected(Path::new(name), source).unwrap(),
                expected,
                "{name}"
            );
        }
        assert!(constraint("wasip1 &&").is_err());
    }
}
