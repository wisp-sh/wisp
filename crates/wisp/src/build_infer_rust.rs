//! Rust API inference. Function bodies and dependencies remain ordinary Rust.
use super::*;
use syn::{Item, ReturnType, Type as RustType, Visibility};

fn unconditional(attributes: &[syn::Attribute], name: &str) -> Result<()> {
    ensure!(
        !attributes
            .iter()
            .any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr")),
        "conditional public Rust API {name} is unsupported; expose an unconditional function"
    );
    Ok(())
}
#[derive(Clone)]
enum Declaration {
    Alias(syn::ItemType, String),
    Record(syn::ItemStruct, String),
    Import(String, String),
}
struct Api {
    declarations: BTreeMap<String, Declaration>,
    functions: Vec<(syn::ItemFn, String)>,
    hashes: BTreeMap<String, String>,
    remaining: usize,
    depth: usize,
    used_names: BTreeMap<String, String>,
}
impl Api {
    fn items(
        &mut self,
        project: &Path,
        directory: &Path,
        prefix: &str,
        items: &[Item],
    ) -> Result<()> {
        ensure!(prefix.len() < 512, "Rust source module depth/count limit");
        for item in items {
            match item {
                Item::Type(alias) => self.insert(
                    format!("{prefix}{}", alias.ident),
                    Declaration::Alias(alias.clone(), prefix.into()),
                )?,
                Item::Struct(record) => self.insert(
                    format!("{prefix}{}", record.ident),
                    Declaration::Record(record.clone(), prefix.into()),
                )?,
                Item::Use(item) if matches!(item.vis, Visibility::Public(_)) => {
                    bail!("public Rust re-exports are unsupported; expose a wrapper function")
                }
                Item::Use(item) => self.imports(prefix, "", &item.tree)?,
                Item::Fn(function) if matches!(function.vis, Visibility::Public(_)) => {
                    unconditional(&function.attrs, &function.sig.ident.to_string())?;
                    self.functions.push((function.clone(), prefix.into()));
                }
                Item::Mod(module) if matches!(module.vis, Visibility::Public(_)) => {
                    let name = module.ident.to_string();
                    unconditional(&module.attrs, &name)?;
                    ensure!(
                        !module.attrs.iter().any(|a| a.path().is_ident("path")),
                        "public Rust modules with #[path] are unsupported"
                    );
                    let prefix = format!("{prefix}{name}::");
                    if let Some((_, items)) = &module.content {
                        self.items(project, &directory.join(&name), &prefix, items)?;
                    } else {
                        let direct = directory.join(format!("{name}.rs"));
                        let nested = directory.join(&name).join("mod.rs");
                        ensure!(
                            direct.is_file() != nested.is_file(),
                            "public Rust module {name} requires exactly one of {} or {}",
                            direct.display(),
                            nested.display()
                        );
                        let file = if direct.is_file() { direct } else { nested };
                        self.file(project, &file, &directory.join(&name), &prefix)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn insert(&mut self, key: String, declaration: Declaration) -> Result<()> {
        ensure!(
            self.declarations.insert(key.clone(), declaration).is_none(),
            "duplicate Rust public API type {key}"
        );
        Ok(())
    }
    fn imports(&mut self, scope: &str, prefix: &str, tree: &syn::UseTree) -> Result<()> {
        match tree {
            syn::UseTree::Path(path) => {
                self.imports(scope, &format!("{prefix}{}::", path.ident), &path.tree)?
            }
            syn::UseTree::Name(name) => {
                let target = format!("{prefix}{}", name.ident);
                self.insert(
                    format!("{scope}{}", name.ident),
                    Declaration::Import(target, scope.into()),
                )?;
            }
            syn::UseTree::Rename(rename) if rename.rename == "_" => {}
            syn::UseTree::Rename(rename) => {
                self.insert(
                    format!("{scope}{}", rename.rename),
                    Declaration::Import(format!("{prefix}{}", rename.ident), scope.into()),
                )?;
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    self.imports(scope, prefix, item)?;
                }
            }
            syn::UseTree::Glob(_) => {}
        }
        Ok(())
    }
    fn file(&mut self, project: &Path, file: &Path, directory: &Path, prefix: &str) -> Result<()> {
        ensure!(
            self.hashes.len() < 256 && prefix.len() < 512,
            "Rust source module depth/count limit"
        );
        let absolute = file.canonicalize()?;
        ensure!(
            absolute.starts_with(project),
            "Rust source module escapes project root"
        );
        let bytes = read(file, MAX_SOURCE)?;
        let tree = syn::parse_file(std::str::from_utf8(&bytes)?)
            .with_context(|| format!("reading Rust API {}", file.display()))?;
        unconditional(&tree.attrs, "crate")?;
        let name = absolute
            .strip_prefix(project)?
            .to_str()
            .context("UTF8 Rust source path")?
            .replace('\\', "/");
        ensure!(
            self.hashes.insert(name, digest(&bytes)).is_none(),
            "Rust source module cycle or duplicate path"
        );
        self.items(project, directory, prefix, &tree.items)
    }
    fn key(scope: &str, path: &syn::Path) -> Result<String> {
        let mut segments = path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>();
        let mut prefix = scope.to_owned();
        if path.leading_colon.is_some() || segments.first().is_some_and(|s| s == "crate") {
            prefix.clear();
            if segments.first().is_some_and(|s| s == "crate") {
                segments.remove(0);
            }
        } else {
            while segments
                .first()
                .is_some_and(|s| matches!(s.as_str(), "self" | "super"))
            {
                if segments.remove(0) == "super" {
                    let parent = prefix
                        .trim_end_matches("::")
                        .rsplit_once("::")
                        .map(|(p, _)| format!("{p}::"))
                        .unwrap_or_default();
                    ensure!(
                        !prefix.is_empty(),
                        "Rust public API type path escapes crate root"
                    );
                    prefix = parent;
                }
            }
        }
        Ok(format!("{prefix}{}", segments.join("::")))
    }
    fn infer(
        &mut self,
        ty: &RustType,
        scope: &str,
        stack: &mut BTreeSet<String>,
    ) -> Result<SourceType> {
        ensure!(self.depth < 32, "Rust public API type depth limit");
        self.depth += 1;
        let result = self.infer_inner(ty, scope, stack);
        self.depth -= 1;
        result
    }
    fn infer_inner(
        &mut self,
        ty: &RustType,
        scope: &str,
        stack: &mut BTreeSet<String>,
    ) -> Result<SourceType> {
        ensure!(
            self.remaining > 0 && stack.len() < 32,
            "Rust public API type expansion limit"
        );
        self.remaining -= 1;
        if let RustType::Paren(paren) = ty {
            return self.infer(&paren.elem, scope, stack);
        }
        if let RustType::Group(group) = ty {
            return self.infer(&group.elem, scope, stack);
        }
        let RustType::Path(path) = ty else {
            bail!(
                "unsupported public Rust type; use scalars, String, Vec<u8>, public records, or Result<T,E>"
            )
        };
        ensure!(
            path.qself.is_none(),
            "qualified associated public Rust types are unsupported"
        );
        let key = Self::key(scope, &path.path)?;
        let last = path.path.segments.last().context("Rust type name")?;
        if let Some(declaration) = self.declarations.get(&key).cloned() {
            ensure!(
                stack.insert(key.clone()),
                "recursive Rust public API type {key} is unsupported"
            );
            let result = match declaration {
                Declaration::Alias(alias, scope) => {
                    unconditional(&alias.attrs, &alias.ident.to_string())?;
                    ensure!(
                        alias.generics.params.is_empty() && last.arguments.is_empty(),
                        "generic Rust public API aliases are unsupported"
                    );
                    self.infer(&alias.ty, &scope, stack)?
                }
                Declaration::Import(target, scope) => {
                    let mut imported = syn::parse_str::<syn::Path>(&target)?;
                    imported
                        .segments
                        .last_mut()
                        .context("Rust imported type")?
                        .arguments = last.arguments.clone();
                    self.infer(
                        &RustType::Path(syn::TypePath {
                            qself: None,
                            path: imported,
                        }),
                        &scope,
                        stack,
                    )?
                }
                Declaration::Record(record, scope) => {
                    let wire = wire_name(&key.replace("::", "-").replace("r#", ""))?;
                    if let Some(previous) = self.used_names.insert(wire, key.clone()) {
                        ensure!(
                            previous == key,
                            "public Rust record names collide after normalization: {previous}, {key}"
                        );
                    }
                    unconditional(&record.attrs, &record.ident.to_string())?;
                    ensure!(
                        matches!(record.vis, Visibility::Public(_)),
                        "public Rust API record {key} must be public"
                    );
                    ensure!(
                        record.generics.params.is_empty() && last.arguments.is_empty(),
                        "generic Rust public API records are unsupported"
                    );
                    let syn::Fields::Named(fields) = &record.fields else {
                        bail!("public Rust records require named fields: {key}")
                    };
                    let fields = fields
                        .named
                        .iter()
                        .map(|field| {
                            let name = field.ident.as_ref().context("Rust field name")?.to_string();
                            unconditional(&field.attrs, &name)?;
                            ensure!(
                                matches!(field.vis, Visibility::Public(_)),
                                "public Rust record fields must be public: {key}::{name}"
                            );
                            Ok(SourceField {
                                wire: wire_name(name.trim_start_matches("r#"))?,
                                name,
                                ty: self.infer(&field.ty, &scope, stack)?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    SourceType {
                        name: format!("__wisp_application::{key}"),
                        wire: format!("%{}", wire_name(&key.replace("::", "-").replace("r#", ""))?),
                        kind: SourceKind::Record(fields),
                    }
                }
            };
            stack.remove(&key);
            return Ok(result);
        }
        let parts = path
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>();
        let name = last.ident.to_string();
        let generic = match &last.arguments {
            syn::PathArguments::None => Vec::new(),
            syn::PathArguments::AngleBracketed(arguments) => arguments
                .args
                .iter()
                .map(|arg| match arg {
                    syn::GenericArgument::Type(ty) => Ok(ty.clone()),
                    _ => bail!("public Rust API generic arguments must be types"),
                })
                .collect::<Result<Vec<_>>>()?,
            _ => bail!("unsupported public Rust type arguments"),
        };
        ensure!(
            path.path
                .segments
                .iter()
                .take(parts.len() - 1)
                .all(|s| s.arguments.is_empty()),
            "unsupported public Rust type arguments"
        );
        let qualified = parts.join("::");
        if matches!(
            qualified.as_str(),
            "String" | "std::string::String" | "alloc::string::String"
        ) {
            ensure!(
                generic.is_empty(),
                "String takes no public API type arguments"
            );
            return Ok(SourceType {
                name: "std::string::String".into(),
                wire: "string".into(),
                kind: SourceKind::String,
            });
        }
        if matches!(
            qualified.as_str(),
            "Vec" | "std::vec::Vec" | "alloc::vec::Vec"
        ) {
            ensure!(
                generic.len() == 1,
                "public Rust Vec requires one element type"
            );
            let element = self.infer(&generic[0], scope, stack)?;
            ensure!(
                element.wire == "u8",
                "public Rust lists currently support Vec<u8>"
            );
            return Ok(SourceType {
                name: "std::vec::Vec<u8>".into(),
                wire: "list<u8>".into(),
                kind: SourceKind::Bytes,
            });
        }
        if matches!(
            qualified.as_str(),
            "Result" | "std::result::Result" | "core::result::Result"
        ) {
            ensure!(
                generic.len() == 2,
                "public Rust Result requires success and error types"
            );
            let mut payload = |ty: &RustType| -> Result<Option<SourceType>> {
                if matches!(ty, RustType::Tuple(tuple) if tuple.elems.is_empty()) {
                    Ok(None)
                } else {
                    Ok(Some(self.infer(ty, scope, stack)?))
                }
            };
            let ok = payload(&generic[0])?;
            let err = payload(&generic[1])?;
            return Ok(SourceType::result(
                format!(
                    "std::result::Result<{}, {}>",
                    ok.as_ref().map(|t| t.name.as_str()).unwrap_or("()"),
                    err.as_ref().map(|t| t.name.as_str()).unwrap_or("()")
                ),
                ok,
                err,
            ));
        }
        ensure!(
            generic.is_empty(),
            "unsupported generic public Rust type {qualified}"
        );
        ensure!(
            parts.len() == 1
                || (parts.len() == 3
                    && matches!(parts[0].as_str(), "core" | "std")
                    && parts[1] == "primitive"),
            "unsupported qualified public Rust type {qualified}"
        );
        let wire = match name.as_str() {
            "bool" => "bool",
            "u8" => "u8",
            "u16" => "u16",
            "u32" | "usize" => "u32",
            "u64" => "u64",
            "i8" => "s8",
            "i16" => "s16",
            "i32" | "isize" => "s32",
            "i64" => "s64",
            "f32" => "f32",
            "f64" => "f64",
            _ => bail!(
                "unsupported public Rust type {name}; use scalars, String, Vec<u8>, public records, or Result<T,E>"
            ),
        };
        Ok(SourceType::scalar(name, wire))
    }
    fn function(&mut self, function: &syn::ItemFn, scope: &str) -> Result<Function> {
        let name = function.sig.ident.to_string();
        ensure!(
            function.sig.asyncness.is_none()
                && function.sig.unsafety.is_none()
                && function.sig.variadic.is_none()
                && function.sig.generics.params.is_empty()
                && function.sig.generics.where_clause.is_none(),
            "Rust export {name} must be a synchronous safe function without generics"
        );
        self.remaining = 8192;
        let source_params = function
            .sig
            .inputs
            .iter()
            .map(|argument| {
                let syn::FnArg::Typed(argument) = argument else {
                    bail!("Rust methods are unsupported")
                };
                self.infer(&argument.ty, scope, &mut BTreeSet::new())
            })
            .collect::<Result<Vec<_>>>()
            .with_context(|| format!("Rust export {name}"))?;
        ensure!(
            source_params.len() <= 16,
            "public function parameter limit is 16"
        );
        let source_result = match &function.sig.output {
            ReturnType::Default => None,
            ReturnType::Type(_, ty) if matches!(ty.as_ref(), RustType::Tuple(tuple) if tuple.elems.is_empty()) => {
                None
            }
            ReturnType::Type(_, ty) => Some(
                self.infer(ty, scope, &mut BTreeSet::new())
                    .with_context(|| format!("Rust export {name}"))?,
            ),
        };
        let original = format!("{scope}{name}");
        let types = type_declarations(
            source_params
                .iter()
                .cloned()
                .chain(source_result.iter().cloned()),
        )?;
        Ok(Function {
            wire: wire_name(&original.replace("::", "-").replace("r#", ""))?,
            original,
            params: source_params.iter().map(|ty| ty.wire.clone()).collect(),
            result: source_result.as_ref().map(|ty| ty.wire.clone()),
            rust_params: source_params.iter().map(|ty| ty.name.clone()).collect(),
            rust_result: source_result.as_ref().map(|ty| ty.name.clone()),
            go_params: Vec::new(),
            go_result: None,
            source_params,
            source_result,
            go_error_result: false,
            types,
        })
    }
}
pub(super) fn functions(project: &Path) -> Result<(Vec<Function>, BTreeMap<String, String>)> {
    let manifest = crate::build_rust::project(project)?;
    let mut api = Api {
        declarations: BTreeMap::new(),
        functions: Vec::new(),
        remaining: 8192,
        depth: 0,
        used_names: BTreeMap::new(),
        hashes: BTreeMap::from([(
            "Cargo.toml".into(),
            digest(&read(&project.join("Cargo.toml"), MAX_SOURCE)?),
        )]),
    };
    api.file(
        project,
        &manifest.source,
        manifest.source.parent().context("Rust source parent")?,
        "",
    )?;
    let functions = api
        .functions
        .clone()
        .iter()
        .map(|(function, scope)| api.function(function, scope))
        .collect::<Result<Vec<_>>>()?;
    Ok((functions, api.hashes))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project(source: &str) -> tempfile::TempDir {
        let project = tempfile::tempdir().unwrap();
        fs::write(
            project.path().join("Cargo.toml"),
            "[package]\nname='ordinary-rust'\nversion='0.1.0'\n[lib]\npath='lib.rs'\n",
        )
        .unwrap();
        fs::write(project.path().join("lib.rs"), source).unwrap();
        project
    }
    #[test]
    fn infers_ordinary_rust_functions_and_public_modules() {
        let project = project(
            "type Word = u32; pub fn add(a: Word, b: u32) -> Word { vec![a,b].iter().sum() } fn helper(text: &str) -> String { text.into() } pub mod math; pub(crate) fn hidden(value: String) -> String { value }",
        );
        fs::write(project.path().join("math.rs"), "pub fn flip(value: bool) -> bool { !value }\npub fn width(value: usize) -> usize { value + 1 }\n").unwrap();
        let (api, hashes) = functions(project.path()).unwrap();
        assert_eq!(api.len(), 3);
        assert_eq!(api[0].wire, "add");
        assert_eq!(api[0].params, ["u32", "u32"]);
        assert_eq!(api[1].original, "math::flip");
        assert_eq!(api[2].params, ["u32"]);
        assert_eq!(api[2].rust_params, ["usize"]);
        assert!(hashes.contains_key("math.rs"));
    }
    #[test]
    fn infers_strings_buffers_records_typed_results_and_unit_payloads() {
        let project = project(
            r#"
pub struct Packet { pub name: String, pub data: Vec<u8>, pub r#type: u32 }
pub struct Fault { pub message: std::string::String, pub code: i32 }
type Reply = Result<Packet, Fault>;
pub fn echo(value: Packet) -> Reply { Ok(value) }
pub fn check(value: String) -> Result<(), String> { Ok(()) }
pub fn done() -> Result<(), ()> { Ok(()) }
pub mod nested { use crate::Packet; pub fn echo(value: Packet) -> super::Packet { value } }
"#,
        );
        let (api, _) = functions(project.path()).unwrap();
        let echo = &api[0];
        assert_eq!(echo.params, ["%packet"]);
        assert_eq!(echo.rust_params, ["__wisp_application::Packet"]);
        assert_eq!(echo.result.as_deref(), Some("result<%packet, %fault>"));
        assert!(echo.types["%packet"].contains("%type: u32"));
        assert_eq!(api[1].result.as_deref(), Some("result<_, string>"));
        assert_eq!(api[2].result.as_deref(), Some("result"));
        assert_eq!(api[3].params, ["%packet"]);
        let wit = declarations(&api).unwrap();
        Resolve::default()
            .push_str(
                "api.wit",
                &format!(
                    "package test:rust; interface api {{ {wit} }} world app {{ export api; }}"
                ),
            )
            .unwrap();
    }
    #[test]
    fn record_names_avoid_function_names_without_changing_fields() {
        let project = project(
            "pub struct Packet { pub packet: String } pub fn packet(value: Packet)->Packet{value}",
        );
        let (mut api, _) = functions(project.path()).unwrap();
        normalize_record_names(&mut api).unwrap();
        assert_eq!(api[0].wire, "packet");
        assert_eq!(api[0].params, ["%packet-record"]);
        let SourceKind::Record(fields) = &api[0].source_params[0].kind else {
            panic!()
        };
        assert_eq!(fields[0].wire, "packet");
        let wit = declarations(&api).unwrap();
        Resolve::default()
            .push_str(
                "api.wit",
                &format!(
                    "package test:collision; interface api {{ {wit} }} world app {{ export api; }}"
                ),
            )
            .unwrap();
    }
    #[test]
    fn rejects_excessively_nested_result_types() {
        let mut ty = "u32".to_owned();
        for _ in 0..40 {
            ty = format!("Result<{ty},u32>");
        }
        assert!(
            functions(project(&format!("pub fn deep()->{ty}{{unimplemented!()}}")).path()).is_err()
        );
    }
    #[test]
    fn qualified_records_are_distinct_and_private_aliases_resolve() {
        let project = project(
            "pub mod a { pub struct Value { pub x: u32 } type Alias = Value; pub fn echo(x: Alias)->Alias{x} } pub mod b { pub struct Value { pub y: String } pub fn echo(x: Value)->Value{x} }",
        );
        let (api, _) = functions(project.path()).unwrap();
        assert_eq!(api[0].params, ["%a-value"]);
        assert_eq!(api[1].params, ["%b-value"]);
        assert_eq!(api[0].source_params[0].name, "__wisp_application::a::Value");
    }
    #[test]
    fn rejects_unsupported_recursive_private_or_conditional_rust_apis() {
        for source in [
            "pub fn bad(value: &str) -> u32 { 1 }",
            "pub async fn bad() -> u32 { 1 }",
            "pub fn bad<T>(value: T) -> u32 { 1 }",
            "#[cfg(target_os=\"linux\")] pub fn bad() -> u32 { 1 }",
            "pub use dependency::function;",
            "pub struct Private { value: String } pub fn bad(value: Private)->Private{value}",
            "struct Private { pub value: String } pub fn bad(value: Private)->Private{value}",
            "pub struct Node { pub child: Box<Node> } pub fn bad(value: Node)->Node{value}",
            "type A = B; type B = A; pub fn bad(value: A)->A{value}",
            "pub struct FooBar { pub x: u32 } pub struct Foo_Bar { pub x: u32 } pub fn first(value: FooBar)->FooBar{value} pub fn second(value: Foo_Bar)->Foo_Bar{value}",
            "pub struct Pair(pub u32, pub u32); pub fn bad(value: Pair)->Pair{value}",
            "pub fn bad(value: Vec<String>)->Vec<String>{value}",
            "pub struct Collision { pub one_word: u32, pub oneWord: u32 } pub fn bad(value: Collision)->Collision{value}",
        ] {
            assert!(functions(project(source).path()).is_err(), "{source}");
        }
    }
}
