//! Packaged language adapters orchestrate Wasm compilers and provide their own ABI.
use super::*;

pub(super) fn validate(compiler: Compiler, unit: &Unit) -> Result<()> {
    ensure!(
        !unit.sources.is_empty(),
        "{} requires sources",
        compiler.name()
    );
    ensure!(
        unit.includes.is_empty(),
        "{} uses project for local imports; includes is only supported by C/C++",
        compiler.name()
    );
    let multiple = matches!(
        compiler,
        Compiler::Swift | Compiler::Java | Compiler::Kotlin
    );
    ensure!(
        multiple || unit.sources.len() == 1,
        "{} requires exactly one source entrypoint",
        compiler.name()
    );
    if matches!(compiler, Compiler::Java | Compiler::Kotlin) {
        ensure!(
            unit.entry
                .as_ref()
                .is_some_and(|entry| !entry.trim().is_empty()),
            "{} requires entry naming the main class",
            compiler.name()
        );
    } else if !matches!(compiler, Compiler::Python) {
        ensure!(
            unit.entry.is_none(),
            "entry is supported by Java, Kotlin and Python"
        );
    }
    if let Some(entry) = &unit.entry {
        ensure!(
            !entry.is_empty() && entry.len() <= 256 && !entry.chars().any(char::is_control),
            "invalid compiler entry"
        );
    }
    if unit.optimization.is_some() {
        ensure!(
            matches!(
                compiler,
                Compiler::Zig | Compiler::Assemblyscript | Compiler::Grain | Compiler::Goose
            ),
            "{} does not support optimization overrides",
            compiler.name()
        );
    }
    Ok(())
}

pub(super) struct Toolchain {
    compiler: Compiler,
    script: Tool,
    python: Tool,
    runtime: crate::compiler_hosts::ExecutionHost,
    environment: BTreeMap<String, PathBuf>,
}

fn selected(root: &Path, configured: &Option<PathBuf>, variable: &str) -> Result<Option<PathBuf>> {
    let path = match std::env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| configured.clone())
    {
        Some(path) => Some(path),
        None => crate::toolchain_install::resolve(variable)?,
    };
    path.map(|path| {
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        path.canonicalize()
            .with_context(|| format!("{variable}: {}", path.display()))
    })
    .transpose()
}

fn executable(
    root: &Path,
    configured: &Option<PathBuf>,
    variable: &str,
    fallback: &str,
) -> Result<Tool> {
    let explicit = std::env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| configured.clone());
    let path = match explicit {
        Some(path) if path.is_absolute() => path,
        Some(path) if path.components().count() > 1 => root.join(path),
        value => {
            let name = value.unwrap_or_else(|| fallback.into());
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|dir| dir.join(&name))
                .find(|path| path.is_file())
                .with_context(|| format!("configure {variable} or install {}", name.display()))?
        }
    }
    .canonicalize()?;
    Ok(Tool {
        sha256: digest(&read(&path, 256 * 1024 * 1024)?),
        path,
    })
}

impl Toolchain {
    pub(super) fn infer_python(&self, source: &Path) -> Result<String> {
        let script = self.script.path.with_file_name("infer.py");
        let result = Command::new(&self.python.path)
            .arg(&script)
            .arg(source)
            .arg("--declarations")
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .envs(&self.environment)
            .stdin(Stdio::null())
            .output()
            .context("running packaged guest Python contract inference")?;
        ensure!(
            result.status.success(),
            "Python contract inference failed: {}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        ensure!(
            result.stdout.len() <= MAX_SOURCE as usize,
            "Python inferred contract exceeds size limit"
        );
        Ok(String::from_utf8(result.stdout)?)
    }

    pub(super) fn load(root: &Path, tools: &Tools, compiler: Compiler) -> Result<Self> {
        let configured = match compiler {
            Compiler::Zig => &tools.zig,
            Compiler::Assemblyscript => &tools.assemblyscript,
            Compiler::Nim => &tools.nim,
            Compiler::Javascript => &tools.javascript,
            Compiler::Typescript => {
                if tools.typescript.is_some() {
                    &tools.typescript
                } else {
                    &tools.javascript
                }
            }
            Compiler::Python => &tools.python,
            Compiler::Grain => &tools.grain,
            Compiler::Bend => &tools.bend,
            Compiler::Goose => &tools.goose,
            Compiler::Swift => &tools.swift,
            Compiler::Java => &tools.java,
            Compiler::Kotlin => &tools.kotlin,
            _ => bail!("compiler does not use a packaged adapter"),
        };
        let variable = if matches!(compiler, Compiler::Typescript)
            && tools.typescript.is_none()
            && std::env::var_os("WISP_TYPESCRIPT_TOOLCHAIN").is_none()
        {
            "WISP_JAVASCRIPT_TOOLCHAIN".to_owned()
        } else {
            format!("WISP_{}_TOOLCHAIN", compiler.name().to_uppercase())
        };
        let path = selected(root, configured, &variable)?.with_context(|| {
            format!(
                "install the {} toolchain or set {variable}",
                compiler.name()
            )
        })?;
        let script = if path.is_dir() {
            path.join("compile.py")
        } else {
            path
        };
        let script = Tool {
            sha256: digest(&read(&script, MAX_SOURCE)?),
            path: script,
        };
        let python = executable(root, &tools.python_host, "WISP_PYTHON_HOST", "python3")?;
        let runtime = crate::compiler_hosts::select(
            root,
            &[("WISP_WASMTIME", &tools.wasmtime)],
            crate::compiler_hosts::COMMAND_VERSION,
            if matches!(compiler, Compiler::Goose) {
                crate::compiler_hosts::Contract::WorkerProtocol
            } else {
                crate::compiler_hosts::Contract::WasmtimeCli
            },
        )?;
        let mut environment = BTreeMap::new();
        environment.insert("WISP_WASMTIME".into(), runtime.path.clone());
        for (variable, configured) in [
            ("WISP_WASMTIME_PYTHON", &tools.wasmtime_python),
            ("WISP_GOOSE_FRONTEND_HOST", &tools.goose_frontend_host),
            ("WISP_CLANG", &tools.clang),
            ("WISP_CLANG_RESOURCE_DIR", &tools.clang_resource_dir),
            ("WISP_LLVM", &tools.llvm),
            ("WISP_LLVM_TOOLCHAIN", &tools.llvm_toolchain),
            ("WISP_LLVM_SWIFT", &tools.llvm_swift),
            ("WISP_WASM_TOOLS", &tools.wasm_tools),
            ("WISP_WASM_TOOLS_TOOLCHAIN", &tools.wasm_tools_toolchain),
            ("WISP_PREVIEW1_ADAPTER", &tools.preview1_adapter),
            ("WISP_JAVASCRIPT_TOOLCHAIN", &tools.javascript),
            ("WISP_JVM_TOOLCHAIN", &tools.jvm),
            ("WISP_TEAVM_TOOLCHAIN", &tools.teavm),
        ] {
            let needed = match variable {
                "WISP_GOOSE_FRONTEND_HOST" => matches!(compiler, Compiler::Goose),
                "WISP_WASMTIME_PYTHON" => {
                    matches!(
                        compiler,
                        Compiler::Assemblyscript | Compiler::Swift | Compiler::Bend
                    )
                }
                "WISP_JAVASCRIPT_TOOLCHAIN" => matches!(compiler, Compiler::Bend),
                "WISP_CLANG" => matches!(compiler, Compiler::Zig),
                "WISP_CLANG_RESOURCE_DIR" => matches!(
                    compiler,
                    Compiler::Zig
                        | Compiler::Nim
                        | Compiler::Goose
                        | Compiler::Java
                        | Compiler::Kotlin
                ),
                "WISP_LLVM" | "WISP_LLVM_TOOLCHAIN" => matches!(
                    compiler,
                    Compiler::Zig
                        | Compiler::Nim
                        | Compiler::Goose
                        | Compiler::Java
                        | Compiler::Kotlin
                ),
                "WISP_LLVM_SWIFT" => matches!(compiler, Compiler::Swift),
                "WISP_WASM_TOOLS" | "WISP_WASM_TOOLS_TOOLCHAIN" => matches!(
                    compiler,
                    Compiler::Javascript
                        | Compiler::Typescript
                        | Compiler::Python
                        | Compiler::Grain
                        | Compiler::Bend
                        | Compiler::Swift
                        | Compiler::Java
                        | Compiler::Kotlin
                ),
                "WISP_PREVIEW1_ADAPTER" => matches!(
                    compiler,
                    Compiler::Nim
                        | Compiler::Goose
                        | Compiler::Grain
                        | Compiler::Swift
                        | Compiler::Java
                        | Compiler::Kotlin
                ),
                "WISP_JVM_TOOLCHAIN" | "WISP_TEAVM_TOOLCHAIN" => {
                    matches!(compiler, Compiler::Java | Compiler::Kotlin)
                }
                _ => false,
            };
            if !needed {
                continue;
            }
            if let Some(path) = selected(root, configured, variable)? {
                environment.insert(variable.into(), path);
            }
        }
        // Explicit package paths also work without a selected installation.
        for (variable, source, levels) in [
            ("WISP_LLVM_TOOLCHAIN", "WISP_LLVM", 2),
            ("WISP_WASM_TOOLS_TOOLCHAIN", "WISP_WASM_TOOLS", 2),
        ] {
            if !environment.contains_key(variable) {
                if let Some(path) = environment
                    .get(source)
                    .and_then(|path| path.ancestors().nth(levels))
                    .map(Path::to_path_buf)
                {
                    environment.insert(variable.into(), path);
                }
            }
        }
        Ok(Self {
            compiler,
            script,
            python,
            runtime,
            environment,
        })
    }

    pub(super) fn receipt(&self) -> Value {
        json!({"adapter":{"path":self.script.path,"sha256":self.script.sha256},"pythonHost":{"path":self.python.path,"sha256":self.python.sha256},"wasmtime":{"path":self.runtime.path,"sha256":self.runtime.sha256},"dependencies":self.environment})
    }

    pub(super) fn notices(&self) -> Result<BTreeMap<String, String>> {
        let mut notices = BTreeMap::new();
        if matches!(self.compiler, Compiler::Goose) {
            let path = self
                .script
                .path
                .parent()
                .context("Goose package directory")?
                .join("dist/licenses/Goose-Apache-2.0.txt");
            notices.insert(
                "Goose-Apache-2.0.txt".into(),
                String::from_utf8(read(&path, MAX_SOURCE)?)?,
            );
        }
        Ok(notices)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn compile(
        &self,
        root: &Path,
        unit: &Unit,
        source: &Path,
        scratch: &Path,
        cache: &Path,
        resolve: &Resolve,
        world: WorldId,
        python_inferred: bool,
        files: &mut BTreeMap<String, String>,
        total: &mut u64,
        commands: &mut Vec<Value>,
    ) -> Result<Vec<u8>> {
        let project = unit
            .project
            .as_ref()
            .map(|path| input(root, path))
            .transpose()?;
        let mut sources = Vec::new();
        let mut directories = BTreeSet::new();
        if let Some(project) = &project {
            ensure!(project.is_dir(), "adapter project must be a directory");
            directories.insert(project.clone());
        }
        for path in &unit.sources {
            let absolute = input(root, path)?;
            if let Some(project) = &project {
                ensure!(
                    absolute.starts_with(project),
                    "source {} is outside project",
                    path.display()
                );
            }
            if absolute.is_dir() {
                ensure!(
                    matches!(self.compiler, Compiler::Java | Compiler::Kotlin),
                    "source entrypoint must be a file"
                );
                directories.insert(absolute.clone());
            } else {
                ensure!(
                    source_extension(self.compiler, &absolute),
                    "unsupported {} source extension: {}",
                    self.compiler.name(),
                    path.display()
                );
                copy_file(root, &absolute, source, files, total)?;
                directories.insert(absolute.parent().context("source directory")?.to_path_buf());
            }
            sources.push(source.join(absolute.strip_prefix(root)?));
        }
        for directory in directories {
            snapshot(root, &directory, source, self.compiler, files, total)?;
        }
        let module_root = match &project {
            Some(project) => source.join(project.strip_prefix(root)?),
            None => common_parent(&sources)?,
        };
        let wit = scratch.join("world.wit");
        write(&wit, world_text(resolve, world)?)?;
        let output = scratch.join("output.wasm");
        let mut argv = vec![self.script.path.to_string_lossy().into_owned()];
        argv.extend(
            sources
                .iter()
                .map(|path| path.to_string_lossy().into_owned()),
        );
        argv.extend(["--output".into(), output.to_string_lossy().into_owned()]);
        match self.compiler {
            Compiler::Javascript | Compiler::Typescript => {
                argv.extend([
                    "--module-root".into(),
                    module_root.to_string_lossy().into_owned(),
                ]);
                wit_args(&mut argv, &wit, resolve, world, false);
            }
            Compiler::Bend => {
                argv.extend([
                    "--project-root".into(),
                    module_root.to_string_lossy().into_owned(),
                ]);
                wit_args(&mut argv, &wit, resolve, world, false);
            }
            Compiler::Zig | Compiler::Nim | Compiler::Grain | Compiler::Goose => {
                argv.extend([
                    "--project-root".into(),
                    module_root.to_string_lossy().into_owned(),
                ]);
                match self.compiler {
                    Compiler::Zig => {
                        argv.push(format!("-{}", unit.optimization.as_deref().unwrap_or("O0")))
                    }
                    Compiler::Nim => {}
                    Compiler::Goose => {
                        argv.push(format!("-{}", unit.optimization.as_deref().unwrap_or("O1")));
                    }
                    Compiler::Grain => {
                        wit_args(&mut argv, &wit, resolve, world, true);
                        argv.extend([
                            "--cache-dir".into(),
                            cache.join("grain").to_string_lossy().into_owned(),
                        ]);
                        if unit.optimization.as_deref() == Some("O1") {
                            argv.push("--release".into());
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Compiler::Assemblyscript => argv.extend([
                "--optimize".into(),
                if unit.optimization.as_deref() == Some("O1") {
                    "1"
                } else {
                    "0"
                }
                .into(),
            ]),
            Compiler::Python => {
                if !legacy_python_contract(resolve, world) {
                    if python_inferred {
                        argv.push("--inferred".into());
                    }
                    argv.extend([
                        "--cache-dir".into(),
                        cache.join("python").to_string_lossy().into_owned(),
                    ]);
                    ensure!(
                        unit.entry.is_none(),
                        "entry is only supported by the legacy scalar Python adapter"
                    );
                    wit_args(&mut argv, &wit, resolve, world, false);
                    if project.is_some() {
                        argv.extend([
                            "--package-root".into(),
                            module_root.to_string_lossy().into_owned(),
                        ]);
                    }
                }
                if let Some(entry) = &unit.entry {
                    argv.extend(["--entry".into(), entry.clone()]);
                }
            }
            Compiler::Swift | Compiler::Java | Compiler::Kotlin => {
                wit_args(&mut argv, &wit, resolve, world, true);
                if let Some(entry) = &unit.entry {
                    argv.extend(["--main".into(), entry.clone()]);
                }
            }
            _ => unreachable!(),
        }
        let mut command = Command::new(&self.python.path);
        command
            .args(&argv)
            .current_dir(source)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", cache)
            .env("XDG_CACHE_HOME", cache)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("TMPDIR", scratch)
            .envs(&self.environment)
            .stdin(Stdio::null());
        if let Some(path) = std::env::var_os("WISP_NATIVE_TOOL_ATTEMPTS") {
            command.env("WISP_NATIVE_TOOL_ATTEMPTS", path);
        }
        let result = command
            .output()
            .with_context(|| format!("running {} packaged adapter", self.compiler.name()))?;
        commands.push(json!({"program":self.python.path,"arguments":argv,"status":result.status.code(),"stdout":String::from_utf8_lossy(&result.stdout),"stderr":String::from_utf8_lossy(&result.stderr)}));
        ensure!(
            result.status.success(),
            "{} adapter failed: {}{}",
            self.compiler.name(),
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        ensure!(
            digest(&read(&self.script.path, MAX_SOURCE)?) == self.script.sha256,
            "compiler adapter changed while building"
        );
        let mut bytes = read(&output, MAX_COMPONENT)?;
        if bytes.starts_with(b"\0asm\x01\0\0\0") {
            wit_component::embed_component_metadata(
                &mut bytes,
                resolve,
                world,
                wit_component::StringEncoding::UTF8,
            )?;
            let mut encoder = wit_component::ComponentEncoder::default()
                .validate(true)
                .module(&bytes)?;
            if matches!(self.compiler, Compiler::Nim | Compiler::Goose) {
                let adapter = self.environment.get("WISP_PREVIEW1_ADAPTER").context(
                    "WASI reactor requires WISP_PREVIEW1_ADAPTER or an installed reactor adapter",
                )?;
                let bytes = read(adapter, MAX_COMPONENT)?;
                ensure!(
                    digest(&bytes) == crate::toolchain_install::ADAPTER_SHA256,
                    "WASI reactor requires Wisp's pinned preview1 reactor adapter"
                );
                encoder = encoder.adapter("wasi_snapshot_preview1", &bytes)?;
            }
            bytes = encoder.encode().with_context(|| format!("assembling {} Component; source exports must implement the selected WIT canonical ABI", self.compiler.name()))?;
        }
        validate_component(&bytes, resolve, world).with_context(|| {
            format!(
                "{} adapter output differs from selected WIT",
                self.compiler.name()
            )
        })?;
        let receipt = PathBuf::from(format!("{}.build.json", output.display()));
        if receipt.is_file() {
            let bytes = read(&receipt, 16 * 1024 * 1024)?;
            let value: Value = serde_json::from_slice(&bytes).context("adapter build receipt")?;
            commands.push(json!({"adapterReceipt":value,"sha256":digest(&bytes)}));
        }
        Ok(bytes)
    }
}

fn legacy_python_contract(resolve: &Resolve, world: WorldId) -> bool {
    let exports = &resolve.worlds[world].exports;
    if exports.len() != 1 {
        return false;
    }
    let Some(WorldItem::Interface { id, .. }) = exports.values().next() else {
        return false;
    };
    let interface = &resolve.interfaces[*id];
    interface.name.as_deref() == Some("api")
        && interface.package.is_some_and(|package| {
            resolve.packages[package].name.to_string() == "wisp:python@0.1.0"
        })
}

fn wit_args(
    args: &mut Vec<String>,
    wit: &Path,
    resolve: &Resolve,
    world: WorldId,
    component: bool,
) {
    if component {
        args.push("--component".into());
    }
    args.extend([
        "--wit".into(),
        wit.to_string_lossy().into_owned(),
        "--world".into(),
        resolve.worlds[world].name.clone(),
    ]);
}

fn common_parent(paths: &[PathBuf]) -> Result<PathBuf> {
    let mut parent = paths[0].parent().context("source parent")?.to_path_buf();
    while !paths.iter().all(|path| path.starts_with(&parent)) {
        parent = parent
            .parent()
            .context("common source directory")?
            .to_path_buf();
    }
    Ok(parent)
}

fn source_extension(compiler: Compiler, path: &Path) -> bool {
    let extension = path.extension().and_then(|value| value.to_str());
    match compiler {
        Compiler::Zig => extension == Some("zig"),
        Compiler::Assemblyscript => extension == Some("ts"),
        Compiler::Nim => extension == Some("nim"),
        Compiler::Javascript | Compiler::Typescript => {
            matches!(extension, Some("js" | "mjs" | "jsx" | "ts" | "mts" | "tsx"))
        }
        Compiler::Python => extension == Some("py"),
        Compiler::Grain => extension == Some("gr"),
        Compiler::Bend => extension == Some("bend"),
        Compiler::Goose => extension == Some("goose"),
        Compiler::Swift => extension == Some("swift"),
        Compiler::Java => extension == Some("java"),
        Compiler::Kotlin => extension == Some("kt"),
        _ => false,
    }
}

fn snapshot(
    root: &Path,
    directory: &Path,
    destination: &Path,
    compiler: Compiler,
    files: &mut BTreeMap<String, String>,
    total: &mut u64,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_str().context("UTF8 source filename")?;
        let javascript = matches!(compiler, Compiler::Javascript | Compiler::Typescript);
        let in_dependency = directory
            .strip_prefix(root)?
            .components()
            .any(|part| part.as_os_str() == "node_modules");
        if name.starts_with('.')
            || name == "node_modules" && !javascript
            || matches!(name, "target" | "dist" | "build" | "work") && !in_dependency
            || matches!(name, "__pycache__" | "zig-out")
        {
            continue;
        }
        let path = entry.path();
        let metadata = entry.file_type()?;
        ensure!(
            !metadata.is_symlink(),
            "symlink in source input: {}",
            path.display()
        );
        if metadata.is_dir() {
            snapshot(root, &path, destination, compiler, files, total)?;
        } else {
            ensure!(
                metadata.is_file(),
                "source input must be a regular file: {}",
                path.display()
            );
            // Compilers can read non-source assets, such as Zig @embedFile data.
            // copy_file enforces the same aggregate budget and records their hashes.
            copy_file(root, &path, destination, files, total)?;
        }
    }
    Ok(())
}

pub(super) fn validate_component(bytes: &[u8], resolve: &Resolve, world: WorldId) -> Result<()> {
    ensure!(
        bytes.starts_with(b"\0asm\x0d\0\x01\0"),
        "compiler must emit Core Wasm or a WebAssembly Component"
    );
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(bytes)?;
    let wit_parser::decoding::DecodedWasm::Component(actual, actual_world) =
        wit_parser::decoding::decode(bytes)?
    else {
        bail!("expected executable Component")
    };
    crate::library::validate_type_expansion(&actual, actual_world)?;
    let mut actual = crate::wit_model::AbiModel::from_resolve(&actual, actual_world)?;
    let mut expected = crate::wit_model::AbiModel::from_resolve(resolve, world)?;
    ensure!(
        crate::library::exports_match(&actual, &expected),
        "Component exports differ from selected WIT world"
    );
    actual.exports = std::mem::take(&mut actual.imports);
    expected.exports = std::mem::take(&mut expected.imports);
    actual.exports.retain(|interface| {
        !interface.package.starts_with("wasi:")
            || expected.exports.iter().any(|declared| {
                declared.package == interface.package && declared.name == interface.name
            })
    });
    ensure!(
        crate::library::exports_match(&actual, &expected),
        "Component imports differ from selected WIT world"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(compiler: Compiler, path: &str) -> Unit {
        Unit {
            compiler: Some(compiler),
            component: None,
            world: "math".into(),
            sources: vec![path.into()],
            includes: vec![],
            project: None,
            entry: matches!(compiler, Compiler::Java | Compiler::Kotlin).then(|| "Math".into()),
            optimization: None,
            dependencies: BTreeMap::new(),
        }
    }

    fn add_component() -> (Vec<u8>, Resolve, WorldId) {
        use wasm_encoder::{
            CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction, Module,
            TypeSection, ValType,
        };
        let mut resolve = Resolve::default();
        let package = resolve.push_str("test.wit", "package test:adapter@0.1.0; world math { export add: func(a: s32, b: s32) -> s32; }").unwrap();
        let world = resolve.select_world(&[package], Some("math")).unwrap();
        let mut module = Module::new();
        let mut types = TypeSection::new();
        types
            .ty()
            .function([ValType::I32, ValType::I32], [ValType::I32]);
        module.section(&types);
        let mut functions = FunctionSection::new();
        functions.function(0);
        module.section(&functions);
        let mut exports = ExportSection::new();
        exports.export("add", ExportKind::Func, 0);
        module.section(&exports);
        let mut function = Function::new([]);
        function
            .instruction(&Instruction::LocalGet(0))
            .instruction(&Instruction::LocalGet(1))
            .instruction(&Instruction::I32Add)
            .instruction(&Instruction::End);
        let mut code = CodeSection::new();
        code.function(&function);
        module.section(&code);
        let mut bytes = module.finish();
        wit_component::embed_component_metadata(
            &mut bytes,
            &resolve,
            world,
            wit_component::StringEncoding::UTF8,
        )
        .unwrap();
        let component = wit_component::ComponentEncoder::default()
            .validate(true)
            .module(&bytes)
            .unwrap()
            .encode()
            .unwrap();
        (component, resolve, world)
    }

    #[test]
    fn distinguishes_legacy_python_api_from_authored_contracts() {
        let mut resolve = Resolve::default();
        let package = resolve.push_str("legacy.wit", "package wisp:python@0.1.0; interface api { add: func(a: s32, b: s32) -> s32; } world legacy { export api; }").unwrap();
        let world = resolve.select_world(&[package], Some("legacy")).unwrap();
        assert!(legacy_python_contract(&resolve, world));
        let package = resolve.push_str("library.wit", "package demo:python@0.1.0; world library { export greet: func(name: string) -> string; }").unwrap();
        let world = resolve.select_world(&[package], Some("library")).unwrap();
        assert!(!legacy_python_contract(&resolve, world));
    }

    #[test]
    fn admits_large_runtime_components_and_checks_the_upper_bound_before_reading() {
        use std::borrow::Cow;
        use wasm_encoder::Encode;

        let (mut component, resolve, world) = add_component();
        component.push(0);
        wasm_encoder::CustomSection {
            name: Cow::Borrowed("runtime-snapshot"),
            data: Cow::Owned(vec![0; 17 * 1024 * 1024]),
        }
        .encode(&mut component);
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime.wasm");
        fs::write(&path, &component).unwrap();
        let artifact = super::super::ComponentFile {
            path: "runtime.wasm".into(),
            sha256: super::super::digest(&component),
        };
        assert_eq!(
            super::super::prebuilt_component(root.path(), &artifact, &resolve, world).unwrap(),
            component
        );
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(crate::limits::MAX_COMPONENT_BYTES + 1)
            .unwrap();
        assert!(
            super::super::prebuilt_component(root.path(), &artifact, &resolve, world)
                .unwrap_err()
                .to_string()
                .contains("size limit")
        );
    }

    #[test]
    fn validates_adapter_entrypoints_and_options() {
        assert!(validate(Compiler::Java, &unit(Compiler::Java, "Math.java")).is_ok());
        let mut java = unit(Compiler::Java, "Math.java");
        java.entry = None;
        assert!(
            validate(Compiler::Java, &java)
                .unwrap_err()
                .to_string()
                .contains("entry")
        );
        let mut python = unit(Compiler::Python, "main.py");
        python.optimization = Some("O1".into());
        assert!(validate(Compiler::Python, &python).is_err());
        let mut zig = unit(Compiler::Zig, "main.zig");
        zig.sources.push("other.zig".into());
        assert!(validate(Compiler::Zig, &zig).is_err());
    }

    #[test]
    fn adapter_output_must_match_authored_contract() {
        let (bytes, mut resolve, world) = add_component();
        validate_component(&bytes, &resolve, world).unwrap();
        let package = resolve
            .push_str(
                "other.wit",
                "package test:other; world other { export sub: func(a: s32, b: s32) -> s32; }",
            )
            .unwrap();
        let other = resolve.select_world(&[package], Some("other")).unwrap();
        assert!(
            validate_component(&bytes, &resolve, other)
                .unwrap_err()
                .to_string()
                .contains("exports differ")
        );
    }

    #[test]
    fn stages_sources_and_runs_each_packaged_adapter() {
        let python = executable(
            Path::new("/"),
            &None,
            "WISP_ADAPTER_TEST_PYTHON_HOST",
            "python3",
        )
        .unwrap();
        let (component, resolve, world) = add_component();
        for (compiler, filename) in [
            (Compiler::Zig, "main.zig"),
            (Compiler::Assemblyscript, "main.ts"),
            (Compiler::Nim, "main.nim"),
            (Compiler::Javascript, "main.js"),
            (Compiler::Typescript, "main.ts"),
            (Compiler::Python, "main.py"),
            (Compiler::Grain, "main.gr"),
            (Compiler::Goose, "main.goose"),
            (Compiler::Bend, "main.bend"),
            (Compiler::Swift, "main.swift"),
            (Compiler::Java, "Main.java"),
            (Compiler::Kotlin, "Main.kt"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("project");
            fs::create_dir(&root).unwrap();
            write(root.join(filename), "source payload").unwrap();
            write(root.join("main.wit"), "contract snapshot").unwrap();
            write(root.join("answer.txt"), "*").unwrap();
            write(
                root.join("dist/previous-build.wasm"),
                "old generated output",
            )
            .unwrap();
            if matches!(compiler, Compiler::Javascript | Compiler::Typescript) {
                write(
                    root.join("node_modules/local-package/dist/index.js"),
                    "export const value = 7;",
                )
                .unwrap();
            }
            let script = temp.path().join("compile.py");
            let fixture = temp.path().join("fixture.wasm");
            write(&fixture, &component).unwrap();
            let script_text = format!(
                "import pathlib, sys\na=sys.argv[1:]\ns=pathlib.Path(a[0])\nassert s.read_text() == 'source payload'\nassert 'staged' in str(s)\nassert (s.parent / 'main.wit').read_text() == 'contract snapshot'\nassert (s.parent / 'answer.txt').read_bytes() == b'*'\nout=pathlib.Path(a[a.index('--output')+1])\nout.write_bytes(pathlib.Path({:?}).read_bytes())\n",
                fixture.to_str().unwrap()
            );
            write(&script, &script_text).unwrap();
            let toolchain = Toolchain {
                compiler,
                script: Tool {
                    path: script,
                    sha256: digest(script_text.as_bytes()),
                },
                python: Tool {
                    path: python.path.clone(),
                    sha256: python.sha256.clone(),
                },
                runtime: crate::compiler_hosts::ExecutionHost {
                    path: "/unused".into(),
                    sha256: String::new(),
                    backend: crate::compiler_hosts::Backend::WasmtimeCli,
                },
                environment: BTreeMap::new(),
            };
            let source = temp.path().join("staged");
            fs::create_dir(&source).unwrap();
            let scratch = temp.path().join("scratch");
            fs::create_dir(&scratch).unwrap();
            let cache = temp.path().join("cache");
            fs::create_dir(&cache).unwrap();
            let mut files = BTreeMap::new();
            let mut total = 0;
            let mut commands = Vec::new();
            let mut source_unit = unit(compiler, filename);
            if matches!(compiler, Compiler::Python) {
                source_unit.project = Some(".".into());
            }
            let bytes = toolchain
                .compile(
                    &root,
                    &source_unit,
                    &source,
                    &scratch,
                    &cache,
                    &resolve,
                    world,
                    matches!(compiler, Compiler::Python),
                    &mut files,
                    &mut total,
                    &mut commands,
                )
                .unwrap();
            assert_eq!(bytes, component, "{}", compiler.name());
            assert_eq!(files.get(filename), Some(&digest(b"source payload")));
            assert_eq!(files.get("main.wit"), Some(&digest(b"contract snapshot")));
            assert_eq!(files.get("answer.txt"), Some(&digest(b"*")));
            assert!(!files.contains_key("dist/previous-build.wasm"));
            if matches!(compiler, Compiler::Javascript | Compiler::Typescript) {
                assert!(files.contains_key("node_modules/local-package/dist/index.js"));
            }
            assert_eq!(commands[0]["status"], 0);
            if matches!(compiler, Compiler::Python) {
                let arguments = commands[0]["arguments"].as_array().unwrap();
                assert!(arguments.iter().any(|value| value == "--inferred"));
                let cache_index = arguments
                    .iter()
                    .position(|value| value == "--cache-dir")
                    .unwrap();
                assert_eq!(
                    Path::new(arguments[cache_index + 1].as_str().unwrap()),
                    cache.join("python")
                );
                let index = arguments
                    .iter()
                    .position(|value| value == "--package-root")
                    .unwrap();
                assert_eq!(
                    Path::new(arguments[index + 1].as_str().unwrap()),
                    source.as_path()
                );
            }
        }
    }

    #[test]
    fn python_inference_calls_the_packaged_guest_driver_and_reports_errors() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("api.py");
        write(&source, "raise RuntimeError('must not run on the host')").unwrap();
        let script = temp.path().join("infer.py");
        write(&script, "import sys\nassert sys.argv[1].endswith('api.py')\nassert sys.argv[2] == '--declarations'\nprint('ready: func() -> bool;')\n").unwrap();
        let toolchain = Toolchain {
            compiler: Compiler::Python,
            script: Tool {
                path: temp.path().join("compile.py"),
                sha256: String::new(),
            },
            python: executable(
                Path::new("/"),
                &None,
                "WISP_ADAPTER_TEST_PYTHON_HOST",
                "python3",
            )
            .unwrap(),
            runtime: crate::compiler_hosts::ExecutionHost {
                path: "/unused".into(),
                sha256: String::new(),
                backend: crate::compiler_hosts::Backend::WasmtimeCli,
            },
            environment: BTreeMap::new(),
        };
        let wit = toolchain.infer_python(&source).unwrap();
        Resolve::default()
            .push_str(
                "inferred.wit",
                &format!(
                    "package test:python; interface api {{ {wit} }} world library {{ export api; }}"
                ),
            )
            .unwrap();
        write(
            &script,
            "raise SystemExit('function bad: unsupported Python type dict[str, int]')",
        )
        .unwrap();
        let error = toolchain.infer_python(&source).unwrap_err().to_string();
        assert!(error.contains("function bad: unsupported Python type dict[str, int]"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_source_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        write(temp.path().join("main.zig"), "source").unwrap();
        std::os::unix::fs::symlink("main.zig", temp.path().join("other.zig")).unwrap();
        let error = snapshot(
            temp.path(),
            temp.path(),
            &temp.path().join("staged"),
            Compiler::Zig,
            &mut BTreeMap::new(),
            &mut 0,
        )
        .unwrap_err();
        assert!(error.to_string().contains("symlink"));
    }
}
