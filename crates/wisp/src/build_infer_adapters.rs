//! Contracts for adapter compilers, preserving adjacent WIT or explicit scalar signatures.
use super::*;
use regex::Regex;

fn package_item(resolve: &Resolve, package: PackageId, item: &str) -> String {
    let name = &resolve.packages[package].name;
    format!(
        "{}:{}/%{}{}",
        name.namespace,
        name.name,
        item,
        name.version
            .as_ref()
            .map(|v| format!("@{v}"))
            .unwrap_or_default()
    )
}

pub(super) fn contract(
    root: &Path,
    unit: &Unit,
    compiler: Compiler,
    resolve: &mut Resolve,
) -> Result<(String, BTreeMap<String, String>)> {
    ensure!(
        !unit.sources.is_empty(),
        "{} requires sources",
        compiler.name()
    );
    let mut hashes = BTreeMap::new();
    let mut source_text = String::new();
    let mut sidecars = BTreeSet::new();
    for path in &unit.sources {
        let absolute = input(root, path)?;
        ensure!(
            absolute.is_file(),
            "directory source inference requires build.wit and an explicit world; list individual source files to infer signatures"
        );
        let bytes = read(&absolute, MAX_SOURCE)?;
        hashes.insert(
            absolute
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/"),
            digest(&bytes),
        );
        source_text.push_str(std::str::from_utf8(&bytes)?);
        source_text.push('\n');
        let sidecar = path.with_extension("wit");
        if root.join(&sidecar).try_exists()? {
            sidecars.insert(sidecar);
        }
    }
    // Keep interface identities and imports from a language's existing contract.
    // A generated wrapper world gives the release its own package identity.
    ensure!(
        sidecars.len() <= 1,
        "multiple adjacent WIT contracts; select one explicitly with build.wit"
    );
    if let Some(path) = sidecars.into_iter().next() {
        let absolute = input(root, &path)?;
        let bytes = read(&absolute, MAX_SOURCE)?;
        hashes.insert(
            absolute
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/"),
            digest(&bytes),
        );
        let package = resolve.push_str(&path, std::str::from_utf8(&bytes)?)?;
        let world = resolve.select_world(&[package], None)
            .context("adjacent WIT must select one world; use build.wit and an explicit unit world otherwise")?;
        return Ok((
            format!(
                "include {};",
                package_item(resolve, package, &resolve.worlds[world].name)
            ),
            hashes,
        ));
    }
    if matches!(compiler, Compiler::Python) {
        let package = resolve.push_str("python-api.wit", "package wisp:python@0.1.0; interface api { validate: func() -> result<_, string>; add: func(a: s32, b: s32) -> s32; runtime-info: func() -> string; }")?;
        return Ok((
            format!("export {};", package_item(resolve, package, "api")),
            hashes,
        ));
    }
    Ok((scalar_exports(&source_text, compiler)?, hashes))
}

fn ty(name: &str, compiler: Compiler) -> Result<Option<String>> {
    let name = name.trim();
    if matches!(compiler, Compiler::Bend) {
        return match name {
            "U32" => Ok(Some("u32".into())),
            "F32" => Ok(Some("f32".into())),
            _ => bail!(
                "cannot infer public bend type {name}; use U32 or F32, or provide build.wit with explicit ABI bindings"
            ),
        };
    }
    let mapped = match name {
        "void" | "Void" | "Unit" => return Ok(None),
        "bool" | "boolean" | "Boolean" | "Bool" => "bool",
        "u8" | "uint8" | "UInt8" => "u8",
        "i8" | "int8" | "Int8" | "byte" | "Byte" => "s8",
        "u16" | "uint16" | "UInt16" => "u16",
        "i16" | "int16" | "Int16" | "short" | "Short" => "s16",
        "u32" | "uint32" | "UInt32" => "u32",
        "i32" | "int32" | "Int32" | "int" | "Int" | "WasmI32" => "s32",
        "u64" | "uint64" | "UInt64" => "u64",
        "i64" | "int64" | "Int64" | "long" | "Long" | "WasmI64" => "s64",
        "f32" | "float32" | "float" | "Float" | "WasmF32" => "f32",
        "f64" | "float64" | "double" | "Double" | "number" | "WasmF64" => "f64",
        "string" if matches!(compiler, Compiler::Typescript | Compiler::Javascript) => "string",
        "Uint8Array" if matches!(compiler, Compiler::Typescript | Compiler::Javascript) => {
            "list<u8>"
        }
        _ => bail!(
            "cannot infer public {} type {name}; provide build.wit with explicit ABI bindings",
            compiler.name()
        ),
    };
    Ok(Some(mapped.into()))
}

// Ignore declarations in comments and quoted strings. Preserve string byte
// positions for export attributes, whose names are read from the original text.
fn mask(source: &str, hash_comments: bool) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut at = 0;
    while at < bytes.len() {
        let line = bytes[at..].starts_with(b"//") || hash_comments && bytes[at] == b'#';
        let block = bytes[at..].starts_with(b"/*");
        let quote = matches!(bytes[at], b'\'' | b'"' | b'`');
        if !(line || block || quote) {
            at += 1;
            continue;
        }
        let start = at;
        if line {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
        } else if block {
            at += 2;
            let mut depth = 1;
            while at < bytes.len() && depth > 0 {
                if bytes[at..].starts_with(b"/*") {
                    depth += 1;
                    at += 2;
                } else if bytes[at..].starts_with(b"*/") {
                    depth -= 1;
                    at += 2;
                } else {
                    at += 1;
                }
            }
        } else {
            let delimiter = bytes[at];
            at += 1;
            while at < bytes.len() {
                if bytes[at] == b'\\' {
                    at = (at + 2).min(bytes.len());
                } else if bytes[at] == delimiter {
                    at += 1;
                    break;
                } else {
                    at += 1;
                }
            }
        }
        for i in start..at {
            if bytes[i] != b'\n' && !(quote && (i == start || i + 1 == at)) {
                out[i] = b' ';
            }
        }
    }
    String::from_utf8(out).expect("mask preserves valid UTF8")
}

fn scalar_exports(source: &str, compiler: Compiler) -> Result<String> {
    let masked = mask(
        source,
        matches!(compiler, Compiler::Nim | Compiler::Python | Compiler::Bend),
    );
    // These patterns only accept explicitly typed, exported functions. The
    // compiler remains responsible for full source syntax and semantic checks.
    let pattern = match compiler {
        Compiler::Zig => {
            r"(?m)^\s*(?:pub\s+)?export\s+fn\s+(?P<name>\w+)\s*\((?P<params>[^()]*)\)\s*(?P<result>[\w]+)\s*\{"
        }
        Compiler::Goose => {
            r"(?m)^\s*export\s+fn\s+(?P<name>\w+)\s*\((?P<params>[^()]*)\)\s*(?:->\s*(?P<result>\w+)\s*)?\{"
        }
        Compiler::Assemblyscript | Compiler::Typescript => {
            r"(?m)^\s*export\s+function\s+(?P<name>\w+)\s*\((?P<params>[^()]*)\)\s*:\s*(?P<result>[\w]+)\s*\{"
        }
        Compiler::Bend => {
            r"(?m)^def\s+(?P<name>\w+)\s*\((?P<params>[^()]*)\)\s*->\s*(?P<result>\w+)\s*:"
        }
        Compiler::Nim => {
            r"(?m)^\s*proc\s+(?P<name>\w+)\*?\s*\((?P<params>[^()]*)\)\s*:\s*(?P<result>\w+)\s*\{\.[^\n]*\bexportc\b[^\n]*\.\}\s*="
        }
        Compiler::Swift => {
            r#"(?m)@_expose\(\s*wasm\s*,\s*"(?P<export>[^"]+)"\s*\)\s*(?:@_cdecl\([^\n]*\)\s*)?(?:public\s+)?func\s+(?P<name>\w+)\s*\((?P<params>[^()]*)\)\s*(?:->\s*(?P<result>\w+)\s*)?\{"#
        }
        Compiler::Java => {
            r#"(?m)@(?:org\.teavm\.interop\.)?Export\(\s*name\s*=\s*"(?P<export>[^"]+)"\s*\)\s*public\s+static\s+(?P<result>\w+)\s+(?P<name>\w+)\s*\((?P<params>[^()]*)\)\s*\{"#
        }
        Compiler::Kotlin => {
            r#"(?m)@(?:org\.teavm\.interop\.)?Export\(\s*name\s*=\s*"(?P<export>[^"]+)"\s*\)\s*(?:public\s+)?fun\s+(?P<name>\w+)\s*\((?P<params>[^()]*)\)\s*(?::\s*(?P<result>\w+)\s*)?[={]"#
        }
        _ => bail!(
            "{} APIs require an adjacent .wit contract or build.wit; public return types cannot be inferred safely",
            compiler.name()
        ),
    };
    let regex = Regex::new(pattern)?;
    let mut exports = BTreeMap::new();
    for captures in regex.captures_iter(&masked) {
        let name = captures
            .name("export")
            .map(|m| source[m.range()].trim())
            .unwrap_or(&captures["name"]);
        let wire = wire_name(name)?;
        ensure!(
            wire == name,
            "export {name} needs an explicit WIT contract because its ABI name is not kebab-case"
        );
        let mut params = Vec::new();
        let mut pending = 0;
        for part in captures["params"]
            .split(',')
            .filter(|s| !s.trim().is_empty())
        {
            if matches!(compiler, Compiler::Bend) {
                ensure!(
                    !part.trim_start().starts_with(['-', '~']),
                    "Bend erased and template parameters require an explicit WIT contract"
                );
            }
            let type_name = if matches!(compiler, Compiler::Java) {
                part.split_whitespace()
                    .next()
                    .context("Java parameter type")?
            } else if let Some((_, ty)) = part.rsplit_once(':') {
                ty.trim()
            } else if matches!(compiler, Compiler::Nim) {
                pending += 1;
                continue;
            } else {
                bail!("export {name} requires explicit parameter types or build.wit");
            };
            let ty = ty(type_name, compiler)?.context("void is not a parameter type")?;
            for _ in 0..=pending {
                params.push(ty.clone());
            }
            pending = 0;
        }
        ensure!(
            pending == 0 && params.len() <= 16,
            "export {name} has invalid parameters or exceeds the parameter limit"
        );
        let result = match captures.name("result") {
            Some(result) => ty(result.as_str(), compiler)?,
            None if matches!(compiler, Compiler::Swift | Compiler::Goose) => None,
            None => bail!("export {name} requires an explicit return type or build.wit"),
        };
        let parameters = params
            .iter()
            .enumerate()
            .map(|(i, ty)| format!("a{i}: {ty}"))
            .collect::<Vec<_>>()
            .join(", ");
        let result = result.map(|ty| format!(" -> {ty}")).unwrap_or_default();
        ensure!(
            exports
                .insert(
                    wire.clone(),
                    format!("export %{wire}: func({parameters}){result};")
                )
                .is_none(),
            "duplicate public export {wire}"
        );
    }
    ensure!(
        !exports.is_empty() && exports.len() <= 64,
        "cannot infer {} exports; use explicit typed exports, an adjacent .wit contract, or build.wit",
        compiler.name()
    );
    let markers = match compiler {
        Compiler::Zig => r"\bexport\s+fn\b",
        Compiler::Goose => r#"\bexport\s+(?:"[^"\n]*"\s+)?fn\b"#,
        Compiler::Assemblyscript | Compiler::Typescript => r"\bexport\s+(?:async\s+)?function\b",
        Compiler::Nim => r"\bexportc\b",
        Compiler::Bend => r"(?m)^(?:@unsafe\s+)?(?:def|law)\s+",
        Compiler::Swift => r"@_expose\s*\(",
        Compiler::Java | Compiler::Kotlin => r"@(?:org\.teavm\.interop\.)?Export\s*\(",
        _ => unreachable!("only typed adapter exports reach this check"),
    };
    ensure!(
        Regex::new(markers)?.find_iter(&masked).count() == exports.len(),
        "some public {} exports have signatures that cannot be inferred; provide an adjacent .wit contract or build.wit",
        compiler.name()
    );
    Ok(exports.into_values().collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_adapter_signatures_generate_valid_contracts() {
        let cases = [
            (
                Compiler::Zig,
                "export fn add(a: i32, b: i32) i32 { return a + b; }",
            ),
            (
                Compiler::Goose,
                "export fn add(a: i32, b: i32) -> i32 { a + b }",
            ),
            (
                Compiler::Assemblyscript,
                "export function add(a: i32, b: i32): i32 { return a+b; }",
            ),
            (
                Compiler::Typescript,
                "export function add(a: number, b: number): number { return a+b; }",
            ),
            (
                Compiler::Bend,
                "import Base\ndef add(a: U32, b: U32) -> U32:\n  (a + b : U32)",
            ),
            (
                Compiler::Nim,
                "proc add*(a, b: int32): int32 {.exportc, cdecl.} = a+b",
            ),
            (
                Compiler::Swift,
                "@_expose(wasm, \"add\")\n@_cdecl(\"add\")\npublic func add(_ a: Int32, _ b: Int32) -> Int32 { a+b }",
            ),
            (
                Compiler::Java,
                "@Export(name = \"add\")\npublic static int add(int a, int b) { return a+b; }",
            ),
            (
                Compiler::Kotlin,
                "@Export(name = \"add\")\nfun add(a: Int, b: Int): Int = a+b",
            ),
        ];
        for (compiler, source) in cases {
            let body = scalar_exports(source, compiler).unwrap();
            let mut resolve = Resolve::default();
            let p = resolve
                .push_str(
                    "test.wit",
                    &format!("package test:inferred; world application {{ {body} }}"),
                )
                .unwrap();
            let w = resolve.select_world(&[p], None).unwrap();
            assert_eq!(resolve.worlds[w].exports.len(), 1);
            assert!(body.contains("a0:") && body.contains("a1:"));
        }
    }
    #[test]
    fn ignores_comments_and_strings_and_rejects_ambiguous_types() {
        let source = "/* export fn fake() i32 {} */\nconst note = `\nexport fn fake2() i32 {}\n`;\nexport fn actual() i32 { return 42; }";
        let body = scalar_exports(source, Compiler::Zig).unwrap();
        assert!(body.contains("actual"));
        assert!(!body.contains("fake"));
        assert!(
            scalar_exports("export fn pointer(a: *u8) i32 { return 0; }", Compiler::Zig).is_err()
        );
        assert!(
            scalar_exports(
                "export function add(a, b) { return a+b; }",
                Compiler::Typescript
            )
            .is_err()
        );
    }

    #[test]
    fn goose_infers_scalar_and_void_exports_and_rejects_partial_apis() {
        let body = scalar_exports(
            "export fn flip(x: bool) -> bool { !x }\nexport fn reset() {}",
            Compiler::Goose,
        )
        .unwrap();
        assert!(body.contains("flip: func(a0: bool) -> bool"));
        assert!(body.contains("reset: func()"));
        for source in [
            "export fn text() -> const u8[:] { \"hello\" }",
            "export fn add(a: i32, b: i32) -> i32 { a+b }\nexport fn pair() -> Pair { Pair {} }",
            "export \"sum\" fn renamed(a: i32) -> i32 { a }",
        ] {
            assert!(scalar_exports(source, Compiler::Goose).is_err(), "{source}");
        }
    }
    #[test]
    fn adjacent_contract_preserves_imports_and_interface_identity() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("main.ts"),
            "export function add(a,b) {return a+b}",
        )
        .unwrap();
        fs::write(root.path().join("main.wit"), "package demo:original@1.2.0; interface math { add: func(a:f64,b:f64)->f64; } world library { import math; export answer: func()->f64; }").unwrap();
        let unit: Unit = toml::from_str("compiler='typescript'\nsources=['./main.ts']").unwrap();
        let mut resolve = Resolve::default();
        let (body, hashes) =
            contract(root.path(), &unit, Compiler::Typescript, &mut resolve).unwrap();
        let p = resolve
            .push_str(
                "generated.wit",
                &format!("package test:wrapper@0.1.0; world application {{ {body} }}"),
            )
            .unwrap();
        let w = resolve.select_world(&[p], None).unwrap();
        assert_eq!(resolve.worlds[w].imports.len(), 1);
        assert_eq!(resolve.worlds[w].exports.len(), 1);
        assert!(hashes.contains_key("main.wit"));
        assert!(hashes.contains_key("main.ts"));
        assert!(!hashes.contains_key("./main.ts"));
    }

    #[test]
    fn rejects_partial_api_inference() {
        let source =
            "export fn answer() i32 { return 42; }\nexport fn pointer() *u8 { return undefined; }";
        let error = scalar_exports(source, Compiler::Zig).unwrap_err();
        assert!(error.to_string().contains("some public zig exports"));
    }

    #[test]
    fn bend_infers_numeric_types_and_requires_contracts_for_other_apis() {
        let source =
            "# def fake() -> Nat: 0n\ndef multiply(+a: F32, b: F32) -> F32:\n  (a * b : F32)";
        let body = scalar_exports(source, Compiler::Bend).unwrap();
        assert_eq!(body, "export %multiply: func(a0: f32, a1: f32) -> f32;");
        for source in [
            "def answer() -> Nat:\n  42n",
            "def erased(-x: U32) -> U32:\n  42",
            "def value() -> U32:\n  42\nlaw proof:\n  {0 == 0 : U32}",
            "def value() -> U32:\n  42\ndef helper(a: List<U32>) -> U32:\n  0",
        ] {
            assert!(scalar_exports(source, Compiler::Bend).is_err(), "{source}");
        }
    }
}
