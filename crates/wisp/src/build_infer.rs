//! Infer private Component contracts from ordinary source APIs and installed modules.
use super::*;
use heck::ToKebabCase;
use tree_sitter::{Node, Parser};
use wit_parser::PackageId;
#[path = "build_infer_adapters.rs"]
mod adapters;
#[path = "build_infer_cpp.rs"]
mod cpp;
#[path = "build_infer_go.rs"]
mod go;
#[path = "build_infer_rust.rs"]
mod rust;
use cpp::functions as cpp_functions;
use go::functions as go_functions;

#[derive(Debug, Clone)]
pub(super) struct SourceType {
    pub name: String,
    pub wire: String,
    pub kind: SourceKind,
}
#[derive(Debug, Clone)]
pub(super) enum SourceKind {
    Scalar,
    String,
    Bytes,
    List(Box<SourceType>),
    Option(Box<SourceType>),
    Enum(Vec<SourceCase>),
    Record(Vec<SourceField>),
    Result {
        ok: Option<Box<SourceType>>,
        err: Option<Box<SourceType>>,
    },
}
#[derive(Debug, Clone)]
pub(super) struct SourceCase {
    /// Qualified native expression identifying the case, independent of its discriminant.
    pub name: String,
    /// Normalized WIT case identifier without the `%` escape.
    pub wire: String,
}
#[derive(Debug, Clone)]
pub(super) struct SourceField {
    pub name: String,
    pub wire: String,
    pub ty: SourceType,
}
impl SourceType {
    fn scalar(name: impl Into<String>, wire: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            wire: wire.into(),
            kind: SourceKind::Scalar,
        }
    }
    fn result(name: String, ok: Option<SourceType>, err: Option<SourceType>) -> Self {
        let wire = match (&ok, &err) {
            (Some(ok), Some(err)) => format!("result<{}, {}>", ok.wire, err.wire),
            (Some(ok), None) => format!("result<{}>", ok.wire),
            (None, Some(err)) => format!("result<_, {}>", err.wire),
            (None, None) => "result".into(),
        };
        Self {
            name,
            wire,
            kind: SourceKind::Result {
                ok: ok.map(Box::new),
                err: err.map(Box::new),
            },
        }
    }
}
fn type_declarations(types: impl Iterator<Item = SourceType>) -> Result<BTreeMap<String, String>> {
    fn visit(
        ty: &SourceType,
        declarations: &mut BTreeMap<String, String>,
        count: &mut usize,
    ) -> Result<()> {
        *count += 1;
        ensure!(*count <= 4096, "public API type expansion limit");
        match &ty.kind {
            SourceKind::List(element) | SourceKind::Option(element) => {
                visit(element, declarations, count)?;
            }
            SourceKind::Enum(cases) => {
                ensure!(
                    !cases.is_empty() && cases.len() <= 256,
                    "public enums require 1 to 256 cases"
                );
                let mut names = BTreeSet::new();
                let mut declaration = format!("  enum {} {{\n", ty.wire);
                for case in cases {
                    ensure!(
                        names.insert(&case.wire),
                        "enum case names collide after normalization: {}",
                        case.wire
                    );
                    declaration.push_str(&format!("    %{},\n", case.wire));
                }
                declaration.push_str("  }\n");
                if let Some(previous) = declarations.insert(ty.wire.clone(), declaration.clone()) {
                    ensure!(
                        previous == declaration,
                        "public type names collide after normalization: {}",
                        ty.wire
                    );
                }
            }
            SourceKind::Record(fields) => {
                let mut names = BTreeSet::new();
                ensure!(
                    !fields.is_empty() && fields.len() <= 64,
                    "public records require 1 to 64 fields"
                );
                let mut declaration = format!("  record {} {{\n", ty.wire);
                for field in fields {
                    ensure!(
                        names.insert(&field.wire),
                        "record field names collide after normalization: {}",
                        field.wire
                    );
                    visit(&field.ty, declarations, count)?;
                    declaration.push_str(&format!("    %{}: {},\n", field.wire, field.ty.wire));
                }
                declaration.push_str("  }\n");
                if let Some(previous) = declarations.insert(ty.wire.clone(), declaration.clone()) {
                    ensure!(
                        previous == declaration,
                        "public record names collide after normalization: {}",
                        ty.wire
                    );
                }
            }
            SourceKind::Result { ok, err } => {
                for ty in [ok, err].into_iter().flatten() {
                    visit(ty, declarations, count)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut declarations = BTreeMap::new();
    let mut count = 0;
    for ty in types {
        visit(&ty, &mut declarations, &mut count)?;
    }
    Ok(declarations)
}
#[derive(Debug, Clone)]
pub(super) struct Function {
    pub original: String,
    pub source_params: Vec<SourceType>,
    pub source_result: Option<SourceType>,
    pub go_error_result: bool,
    types: BTreeMap<String, String>,
    wire: String,
    params: Vec<String>,
    result: Option<String>,
}
pub(super) struct Contracts {
    pub package: PackageId,
    pub worlds: BTreeMap<String, WorldId>,
    pub functions: BTreeMap<String, BTreeMap<String, Function>>,
    pub sources: BTreeMap<String, BTreeMap<String, String>>,
    pub complete_sources: BTreeSet<String>,
    pub cpp_headers: BTreeMap<String, BTreeMap<String, String>>,
    pub notices: BTreeMap<String, String>,
}
fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
fn child<'a>(node: Node<'a>, name: &str) -> Result<Node<'a>> {
    node.child_by_field_name(name)
        .with_context(|| format!("missing {name} in source declaration"))
}
fn wire_name(name: &str) -> Result<String> {
    let name = name.to_kebab_case();
    ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && name.as_bytes()[0].is_ascii_lowercase(),
        "public API names must use ASCII letters: {name}"
    );
    Ok(name)
}
fn parse(source: &str, go: bool) -> Result<tree_sitter::Tree> {
    let mut parser = Parser::new();
    let language = if go {
        tree_sitter_go::LANGUAGE.into()
    } else {
        tree_sitter_cpp::LANGUAGE.into()
    };
    parser.set_language(&language)?;
    let tree = parser
        .parse(source, None)
        .context("source parser returned no syntax tree")?;
    ensure!(
        !tree.root_node().has_error(),
        "cannot infer public API from invalid source syntax"
    );
    Ok(tree)
}
fn normalize_record_names(functions: &mut [Function]) -> Result<()> {
    let type_names = functions
        .iter()
        .flat_map(|f| f.types.keys().cloned())
        .collect::<BTreeSet<_>>();
    let function_names = functions
        .iter()
        .map(|f| format!("%{}", f.wire))
        .collect::<BTreeSet<_>>();
    let mut occupied = type_names
        .union(&function_names)
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut renamed = BTreeMap::new();
    for name in type_names.intersection(&function_names) {
        let kind = if functions.iter().any(|function| {
            function
                .types
                .get(name)
                .is_some_and(|declaration| declaration.trim_start().starts_with("enum "))
        }) {
            "enum"
        } else {
            "record"
        };
        let mut replacement = format!("{name}-{kind}");
        let mut suffix = 2;
        while occupied.contains(&replacement) {
            replacement = format!("{name}-{kind}{suffix}");
            suffix += 1;
        }
        occupied.insert(replacement.clone());
        renamed.insert(name.clone(), replacement);
    }
    fn rename(ty: &mut SourceType, renamed: &BTreeMap<String, String>) {
        match &mut ty.kind {
            SourceKind::List(element) => {
                rename(element, renamed);
                ty.wire = format!("list<{}>", element.wire);
            }
            SourceKind::Option(element) => {
                rename(element, renamed);
                ty.wire = format!("option<{}>", element.wire);
            }
            SourceKind::Enum(_) => {
                if let Some(name) = renamed.get(&ty.wire) {
                    ty.wire = name.clone();
                }
            }
            SourceKind::Record(fields) => {
                if let Some(name) = renamed.get(&ty.wire) {
                    ty.wire = name.clone();
                }
                for field in fields {
                    rename(&mut field.ty, renamed);
                }
            }
            SourceKind::Result { ok, err } => {
                for ty in ok.iter_mut().chain(err.iter_mut()) {
                    rename(ty, renamed);
                }
                ty.wire = match (ok, err) {
                    (Some(ok), Some(err)) => format!("result<{}, {}>", ok.wire, err.wire),
                    (Some(ok), None) => format!("result<{}>", ok.wire),
                    (None, Some(err)) => format!("result<_, {}>", err.wire),
                    (None, None) => "result".into(),
                };
            }
            _ => {}
        }
    }
    for function in functions {
        for ty in function
            .source_params
            .iter_mut()
            .chain(function.source_result.iter_mut())
        {
            rename(ty, &renamed);
        }
        function.params = function
            .source_params
            .iter()
            .map(|t| t.wire.clone())
            .collect();
        function.result = function.source_result.as_ref().map(|t| t.wire.clone());
        function.types = type_declarations(
            function
                .source_params
                .iter()
                .cloned()
                .chain(function.source_result.iter().cloned()),
        )?;
    }
    Ok(())
}
fn declarations(functions: &[Function]) -> Result<String> {
    let mut functions = functions.to_vec();
    normalize_record_names(&mut functions)?;
    let mut declarations = BTreeMap::new();
    let mut types = BTreeMap::new();
    for function in functions {
        for (name, declaration) in &function.types {
            if let Some(previous) = types.insert(name.clone(), declaration.clone()) {
                ensure!(
                    previous == *declaration,
                    "public record names collide after normalization: {name}"
                );
            }
        }
        let params = function
            .params
            .iter()
            .enumerate()
            .map(|(i, t)| format!("a{i}: {t}"))
            .collect::<Vec<_>>()
            .join(", ");
        let result = function
            .result
            .as_ref()
            .map(|ty| format!(" -> {ty}"))
            .unwrap_or_default();
        ensure!(
            declarations
                .insert(
                    function.wire.clone(),
                    format!("  %{}: func({params}){result};\n", function.wire)
                )
                .is_none(),
            "public function names collide after normalization: {}; overloads require distinct public names",
            function.wire
        );
    }
    ensure!(
        !declarations.is_empty() && declarations.len() <= 64,
        "a source package must expose 1 to 64 public functions"
    );
    Ok(types
        .into_values()
        .chain(declarations.into_values())
        .collect())
}

pub(super) fn generate(
    config: &mut Config,
    root: &Path,
    resolve: &mut Resolve,
) -> Result<Contracts> {
    ensure!(
        config.dependencies.len() <= 16,
        "installed package dependency limit is 16"
    );
    let source_units = config.build.units.keys().cloned().collect::<Vec<_>>();
    let mut functions = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut complete_sources = BTreeSet::new();
    let mut module_imports = Vec::new();
    let mut rust_imports = Vec::new();
    let mut wit = format!(
        "package {}@{};\n",
        config.package.name, config.package.version
    );
    for name in &source_units {
        let unit = &mut config.build.units.get_mut(name).context("source unit")?;
        ensure!(
            unit.world.is_empty() && unit.dependencies.is_empty() && unit.component.is_none(),
            "inferred builds derive worlds and dependencies automatically; remove world/dependencies or supply build.wit for an authored contract"
        );
        let compiler = unit
            .compiler
            .context("inferred source unit requires compiler")?;
        if matches!(compiler, Compiler::Csharp)
            && !unit
                .sources
                .iter()
                .any(|path| root.join(path.with_extension("wit")).is_file())
        {
            let toolchain = super::adapters::Toolchain::load(root, &config.build.tools, compiler)?;
            let (api, hashes) = toolchain.discover_csharp(root, unit)?;
            complete_sources.insert(name.clone());
            let interface = if source_units.len() == 1 {
                "api".into()
            } else {
                format!("{}-api", wire_name(name)?)
            };
            unit.world = wire_name(name)?;
            wit.push_str(&format!(
                "interface %{interface} {{\n{api}}}\nworld %{} {{ export %{interface}; }}\n",
                unit.world
            ));
            sources.insert(name.clone(), hashes);
            functions.insert(name.clone(), BTreeMap::new());
            continue;
        }
        if compiler.is_adapter() {
            let (body, hashes) = adapters::contract(root, unit, compiler, resolve)?;
            unit.world = wire_name(name)?;
            wit.push_str(&format!("world %{} {{\n{body}\n}}\n", unit.world));
            sources.insert(name.clone(), hashes);
            functions.insert(name.clone(), BTreeMap::new());
            continue;
        }
        let (mut api, hashes) = match unit
            .compiler
            .context("inferred source unit requires compiler")?
        {
            Compiler::Go => {
                let project = input(root, unit.project.as_ref().context("Go requires project")?)?;
                for module in crate::build_go::component_modules(&project)? {
                    module_imports.push((name.clone(), module, project.clone()));
                }
                go_functions(&project)?
            }
            Compiler::Clang => cpp_functions(root, &unit.sources)?,
            Compiler::Rust => {
                let project = input(
                    root,
                    unit.project.as_ref().context("rust requires project")?,
                )?;
                for dependency in crate::build_rust::guest_dependencies(&project)? {
                    rust_imports.push((name.clone(), dependency));
                }
                rust::functions(&project)?
            }
            _ => unreachable!("adapter compilers were handled above"),
        };
        normalize_record_names(&mut api)?;
        sources.insert(name.clone(), hashes);
        let interface = if source_units.len() == 1 {
            "api".into()
        } else {
            format!("{}-api", wire_name(name)?)
        };
        unit.world = wire_name(name)?;
        wit.push_str(&format!(
            "interface %{interface} {{\n{}}}\nworld %{} {{ export %{interface}; }}\n",
            declarations(&api)?,
            unit.world
        ));
        functions.insert(
            name.clone(),
            api.into_iter().map(|f| (f.wire.clone(), f)).collect(),
        );
    }
    let package = resolve.push_str("generated-application.wit", &wit)?;
    let mut worlds = BTreeMap::new();
    for name in &source_units {
        worlds.insert(
            name.clone(),
            resolve.select_world(&[package], Some(&config.build.units[name].world))?,
        );
    }
    let mut providers: BTreeMap<String, (String, String)> = BTreeMap::new();
    for (consumer, module, project) in module_imports {
        let (path, sha256) = crate::build_go::published_component(&project, &module)?
            .with_context(|| format!("module {module} must be installed with go mod vendor"))?;
        let dep_name = if let Some((name, hash)) = providers.get(&module) {
            ensure!(
                hash == &sha256,
                "conflicting Component versions of module {module}"
            );
            name.clone()
        } else {
            let name = format!("dependency-p{}", &digest(module.as_bytes())[..12]);
            ensure!(
                !config.build.units.contains_key(&name),
                "generated dependency unit name collision"
            );
            let bytes = read(&path, MAX_COMPONENT)?;
            let world = merge_provider(resolve, &bytes, &name)?;
            worlds.insert(name.clone(), world);
            config.build.units.insert(
                name.clone(),
                Unit {
                    compiler: None,
                    component: Some(ComponentInput::Module(ComponentModule {
                        module: module.clone(),
                    })),
                    world: resolve.worlds[world].name.clone(),
                    sources: Vec::new(),
                    includes: Vec::new(),
                    project: None,
                    optimization: None,
                    javascript_engine: None,
                    typecheck: false,
                    entry: None,
                    dependencies: BTreeMap::new(),
                },
            );
            providers.insert(module.clone(), (name.clone(), sha256));
            name
        };
        connect(
            config,
            resolve,
            worlds[&consumer],
            worlds[&dep_name],
            &consumer,
            &dep_name,
        )?;
    }
    let mut cpp_headers = BTreeMap::new();
    let mut notices = BTreeMap::new();
    let mut rust_providers: BTreeMap<String, String> = BTreeMap::new();
    for (consumer, dependency) in rust_imports {
        let path = dependency.directory.join("component.wasm");
        let bytes = read(&path, MAX_COMPONENT)?;
        let sha256 = digest(&bytes);
        ensure!(
            dependency.descriptor["component"]["sha256"] == sha256,
            "Rust Component dependency changed during inference"
        );
        for (notice, _) in dependency.descriptor["notices"]
            .as_object()
            .context("Rust guest notices")?
        {
            let name = Path::new(notice)
                .file_name()
                .context("Rust guest notice filename")?
                .to_str()
                .context("UTF8 Rust guest notice filename")?
                .to_owned();
            let text = String::from_utf8(read(&dependency.directory.join(notice), 256 * 1024)?)?;
            if let Some(existing) = notices.insert(name.clone(), text.clone()) {
                ensure!(existing == text, "conflicting dependency notice: {name}");
            }
        }
        let name = if let Some(name) = rust_providers.get(&sha256) {
            name.clone()
        } else {
            let name = format!("dependency-p{}", &sha256[..12]);
            ensure!(
                !config.build.units.contains_key(&name),
                "generated Rust dependency unit name collision"
            );
            let world = merge_provider(resolve, &bytes, &name)?;
            worlds.insert(name.clone(), world);
            config.build.units.insert(
                name.clone(),
                Unit {
                    compiler: None,
                    component: Some(ComponentInput::File(ComponentFile {
                        path: path.strip_prefix(root)?.to_owned(),
                        sha256: sha256.clone(),
                    })),
                    world: resolve.worlds[world].name.clone(),
                    sources: Vec::new(),
                    includes: Vec::new(),
                    project: None,
                    optimization: None,
                    javascript_engine: None,
                    typecheck: false,
                    entry: None,
                    dependencies: BTreeMap::new(),
                },
            );
            rust_providers.insert(sha256.clone(), name.clone());
            name
        };
        connect(
            config,
            resolve,
            worlds[&consumer],
            worlds[&name],
            &consumer,
            &name,
        )?;
    }
    for (alias, installed) in &config.dependencies.clone() {
        super::inferred::cpp::cpp_namespace(alias)?;
        let directory = input(root, &installed.path)?;
        let admitted = crate::library::validate(&directory)
            .with_context(|| format!("installed C++ package {alias}"))?;
        ensure!(
            admitted["identity"] == installed.package
                && admitted["version"] == installed.version.to_string(),
            "installed package {alias} identity/version differs from dependency requirement"
        );
        let bytes = read(&directory.join("component.wasm"), MAX_COMPONENT)?;
        ensure!(
            admitted["componentSha256"] == digest(&bytes),
            "installed package {alias} changed during admission"
        );
        let release = crate::release::strict_json(&read(
            &directory.join("library-release.json"),
            1024 * 1024,
        )?)?;
        let metadata: crate::library_metadata::Metadata =
            serde_json::from_value(release["metadata"].clone())?;
        metadata.validate()?;
        for (name, text) in metadata.notices {
            if let Some(existing) = notices.insert(name.clone(), text.clone()) {
                ensure!(existing == text, "conflicting dependency notice: {name}");
            }
        }
        let name = format!("dependency-p{}", &digest(alias.as_bytes())[..12]);
        ensure!(
            !config.build.units.contains_key(&name),
            "generated dependency unit name collision"
        );
        let world = merge_provider(resolve, &bytes, &name)?;
        let header = super::inferred::cpp::cpp_header(resolve, world, alias)?;
        let consumers = source_units
            .iter()
            .filter(|name| matches!(config.build.units[*name].compiler, Some(Compiler::Clang)))
            .cloned()
            .collect::<Vec<_>>();
        ensure!(
            !consumers.is_empty(),
            "installed package dependencies require a C++ source unit"
        );
        for consumer in consumers {
            cpp_headers
                .entry(consumer.clone())
                .or_insert_with(BTreeMap::new)
                .insert(format!("{alias}.hpp"), header.clone());
            connect(config, resolve, worlds[&consumer], world, &consumer, &name)?;
        }
        worlds.insert(name.clone(), world);
        config.build.units.insert(
            name,
            Unit {
                compiler: None,
                component: Some(ComponentInput::File(ComponentFile {
                    path: directory
                        .join("component.wasm")
                        .strip_prefix(root)?
                        .to_owned(),
                    sha256: digest(&bytes),
                })),
                world: resolve.worlds[world].name.clone(),
                sources: Vec::new(),
                includes: Vec::new(),
                project: None,
                optimization: None,
                javascript_engine: None,
                typecheck: false,
                entry: None,
                dependencies: BTreeMap::new(),
            },
        );
    }
    Ok(Contracts {
        package,
        worlds,
        functions,
        sources,
        complete_sources,
        cpp_headers,
        notices,
    })
}

fn merge_provider(resolve: &mut Resolve, bytes: &[u8], name: &str) -> Result<WorldId> {
    let wit_parser::decoding::DecodedWasm::Component(mut dependency, world) =
        wit_parser::decoding::decode(bytes)?
    else {
        bail!("expected dependency Component")
    };
    crate::library::validate_type_expansion(&dependency, world)?;
    let owner = dependency.worlds[world]
        .package
        .context("dependency world package")?;
    let old = dependency.packages[owner].name.clone();
    let mut renamed = old.clone();
    renamed.namespace = "wisp-generated".into();
    renamed.name = name.into();
    dependency.package_names.swap_remove(&old);
    dependency.package_names.insert(renamed.clone(), owner);
    dependency.packages[owner].name = renamed;
    let remap = resolve.merge(dependency)?;
    remap.worlds[world.index()].context("mapped dependency world")
}
fn connect(
    config: &mut Config,
    resolve: &mut Resolve,
    consumer_world: WorldId,
    provider: WorldId,
    consumer: &str,
    dependency: &str,
) -> Result<()> {
    let exports = resolve.worlds[provider].exports.clone();
    for (key, item) in exports {
        let import = resolve.name_world_key(&key);
        ensure!(
            !resolve.worlds[consumer_world].imports.contains_key(&key),
            "ambiguous imported API {import} from dependency {dependency}"
        );
        if !matches!(item, WorldItem::Type { .. }) {
            config
                .build
                .units
                .get_mut(consumer)
                .unwrap()
                .dependencies
                .insert(import, dependency.into());
        }
        resolve.worlds[consumer_world].imports.insert(key, item);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn go_api_uses_syntax_build_selection_and_scalar_aliases() {
        let project = tempfile::tempdir().unwrap();
        fs::write(
            project.path().join("main.go"),
            r#"package main
// func Fake(a string) string { return a }
var note = "func Fake(a string) string {}"
type Word = uint32
func Sum(a, b Word) (result Word) { return a+b }
func helper(value *int) {}
func main() {}
"#,
        )
        .unwrap();
        fs::write(
            project.path().join("other_windows.go"),
            "package main\nfunc Wrong(value string) string { return value }\n",
        )
        .unwrap();
        let (api, _) = go_functions(project.path()).unwrap();
        assert_eq!(api.len(), 1);
        assert_eq!(api[0].original, "Sum");
        assert_eq!(api[0].params, ["u32", "u32"]);
        assert_eq!(api[0].result.as_deref(), Some("u32"));
        let declarations = declarations(&api).unwrap();
        let mut resolve = Resolve::default();
        resolve.push_str("generated.wit",&format!("package test:inferred; interface api {{ {declarations} }} world application {{ export api; }}")).unwrap();
    }
    #[test]
    fn go_api_preserves_source_types_for_generated_casts() {
        let project = tempfile::tempdir().unwrap();
        fs::write(project.path().join("main.go"), "package main\ntype Counter uint32\ntype Flag bool\nfunc Total(a Counter,b int) int { return int(a)+b }\nfunc Flip(value Flag) Flag { return !value }\nfunc main() {}\n").unwrap();
        let (api, _) = go_functions(project.path()).unwrap();
        let total = api.iter().find(|f| f.original == "Total").unwrap();
        assert_eq!(total.params, ["u32", "s64"]);
        assert_eq!(
            total
                .source_params
                .iter()
                .map(|ty| ty.name.as_str())
                .collect::<Vec<_>>(),
            ["Counter", "int"]
        );
        assert_eq!(
            total.source_result.as_ref().map(|ty| ty.name.as_str()),
            Some("int")
        );
        let flip = api.iter().find(|f| f.original == "Flip").unwrap();
        assert_eq!(flip.params, ["bool"]);
        assert_eq!(
            flip.source_result.as_ref().map(|ty| ty.name.as_str()),
            Some("Flag")
        );
    }
    #[test]
    fn cpp_api_keeps_namespace_calls_and_hides_internal_helpers() {
        let source = "static unsigned hidden(unsigned x) { return x; } namespace { unsigned local(unsigned x) { return x; } } namespace math { unsigned Add(unsigned a,unsigned b) { return a+b; } }";
        let tree = parse(source, false).unwrap();
        let mut functions = Vec::new();
        cpp::walk_test(tree.root_node(), source, &mut functions).unwrap();
        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0].original, "math::Add");
        assert_eq!(functions[0].wire, "math-add");
        assert_eq!(functions[0].params, ["u32", "u32"]);
    }
    #[test]
    fn inferred_apis_reject_unsupported_types_and_ambiguous_overloads() {
        let project = tempfile::tempdir().unwrap();
        fs::write(
            project.path().join("main.go"),
            "package main\nfunc Bad(value map[string]string) string { return value[\"key\"] }\nfunc main() {}\n",
        )
        .unwrap();
        assert!(
            format!("{:#}", go_functions(project.path()).unwrap_err())
                .contains("unsupported public Go type map[string]string")
        );
        let source = "unsigned *bad(unsigned *p) { return p; }";
        let tree = parse(source, false).unwrap();
        assert!(cpp::walk_test(tree.root_node(), source, &mut Vec::new()).is_err());
        let source = "unsigned add(unsigned a,unsigned b) { return a+b; } double add(double a,double b) { return a+b; }";
        let tree = parse(source, false).unwrap();
        let mut functions = Vec::new();
        cpp::walk_test(tree.root_node(), source, &mut functions).unwrap();
        assert!(
            declarations(&functions)
                .unwrap_err()
                .to_string()
                .contains("overloads")
        );
    }
}
