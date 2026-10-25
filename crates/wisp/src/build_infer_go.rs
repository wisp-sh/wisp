//! Infer ordinary Go values; upstream wit-bindgen owns their Component ABI.
use super::*;

#[derive(Clone)]
enum Declaration {
    Alias(String),
    Record(Vec<(String, String)>),
    Enum(Vec<SourceCase>),
    Unsupported(String),
}
struct Types {
    declarations: BTreeMap<String, Declaration>,
    remaining: usize,
    used_names: BTreeMap<String, String>,
}
impl Types {
    fn infer(
        &mut self,
        name: &str,
        stack: &mut BTreeSet<String>,
        depth: usize,
    ) -> Result<SourceType> {
        ensure!(
            self.remaining > 0 && stack.len() < 32 && depth < 32,
            "Go public API type expansion limit"
        );
        self.remaining -= 1;
        let name = name.trim();
        if let Some(declaration) = self.declarations.get(name).cloned() {
            ensure!(
                stack.insert(name.into()),
                "recursive Go public API type {name} is unsupported"
            );
            let result = match declaration {
                Declaration::Alias(target) => {
                    let mut ty = self.infer(&target, stack, depth + 1)?;
                    ty.name = name.into();
                    ty
                }
                Declaration::Record(fields) => {
                    let wire = wire_name(name)?;
                    if let Some(previous) = self.used_names.insert(wire.clone(), name.into()) {
                        ensure!(
                            previous == name,
                            "public Go record names collide after normalization: {previous}, {name}"
                        );
                    }
                    let fields = fields
                        .into_iter()
                        .map(|(name, ty)| {
                            ensure!(
                                name.chars().next().is_some_and(char::is_uppercase),
                                "public Go record fields must be exported: {name}"
                            );
                            Ok(SourceField {
                                wire: wire_name(&name)?,
                                name,
                                ty: self.infer(&ty, stack, depth + 1)?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    SourceType {
                        name: name.into(),
                        wire: format!("%{}", wire_name(name)?),
                        kind: SourceKind::Record(fields),
                    }
                }
                Declaration::Enum(cases) => {
                    let wire = wire_name(name)?;
                    if let Some(previous) = self.used_names.insert(wire.clone(), name.into()) {
                        ensure!(
                            previous == name,
                            "public Go enum names collide after normalization: {previous}, {name}"
                        );
                    }
                    SourceType {
                        name: name.into(),
                        wire: format!("%{wire}"),
                        kind: SourceKind::Enum(cases),
                    }
                }
                Declaration::Unsupported(reason) => {
                    bail!("unsupported public Go type {name}: {reason}")
                }
            };
            stack.remove(name);
            return Ok(result);
        }
        if name == "string" {
            return Ok(SourceType {
                name: name.into(),
                wire: "string".into(),
                kind: SourceKind::String,
            });
        }
        let compact = name.split_whitespace().collect::<String>();
        if matches!(compact.as_str(), "[]byte" | "[]uint8") {
            return Ok(SourceType {
                name: compact,
                wire: "list<u8>".into(),
                kind: SourceKind::Bytes,
            });
        }
        if let Some(element) = compact.strip_prefix("[]") {
            let element = self.infer(element, stack, depth + 1)?;
            return Ok(SourceType {
                name: compact,
                wire: format!("list<{}>", element.wire),
                kind: SourceKind::List(Box::new(element)),
            });
        }
        if let Some(element) = compact.strip_prefix('*') {
            let element = self.infer(element, stack, depth + 1)?;
            return Ok(SourceType {
                name: compact,
                wire: format!("option<{}>", element.wire),
                kind: SourceKind::Option(Box::new(element)),
            });
        }
        let wire = match name {
            "bool" => "bool",
            "uint8" | "byte" => "u8",
            "uint16" => "u16",
            "uint32" => "u32",
            "uint64" | "uint" => "u64",
            "int8" => "s8",
            "int16" => "s16",
            "int32" | "rune" => "s32",
            "int64" | "int" => "s64",
            "float32" => "f32",
            "float64" => "f64",
            _ => bail!(
                "unsupported public Go type {name}; use scalars, strings, slices, optional pointers, integer enums, or named structs with exported fields"
            ),
        };
        Ok(SourceType::scalar(name, wire))
    }
    fn ty(&mut self, name: &str) -> Result<SourceType> {
        self.infer(name, &mut BTreeSet::new(), 0)
    }
}
fn parameter_types(node: Node<'_>, source: &str) -> Result<Vec<String>> {
    let mut result = Vec::new();
    for parameter in node.named_children(&mut node.walk()) {
        if parameter.kind() == "comment" {
            continue;
        }
        ensure!(
            parameter.kind() == "parameter_declaration",
            "variadic public Go functions are unsupported"
        );
        let ty = text(child(parameter, "type")?, source);
        let count = parameter
            .children_by_field_name("name", &mut parameter.walk())
            .count()
            .max(1);
        result.extend(std::iter::repeat_n(ty.into(), count));
    }
    Ok(result)
}
fn declaration(node: Node<'_>, source: &str) -> Result<Declaration> {
    if node.child_by_field_name("type_parameters").is_some() {
        return Ok(Declaration::Unsupported(
            "generic public records are unsupported".into(),
        ));
    }
    let ty = child(node, "type")?;
    if ty.kind() != "struct_type" {
        return Ok(Declaration::Alias(text(ty, source).into()));
    }
    let mut fields = Vec::new();
    let body = ty
        .named_children(&mut ty.walk())
        .find(|n| n.kind() == "field_declaration_list")
        .context("Go struct fields")?;
    for field in body.named_children(&mut body.walk()) {
        if field.kind() == "comment" {
            continue;
        }
        let names = field
            .children_by_field_name("name", &mut field.walk())
            .collect::<Vec<_>>();
        if names.is_empty() {
            return Ok(Declaration::Unsupported(
                "embedded public record fields are unsupported".into(),
            ));
        }
        let ty = text(child(field, "type")?, source);
        for name in names {
            fields.push((text(name, source).into(), ty.into()));
        }
    }
    Ok(Declaration::Record(fields))
}

/// A deliberately bounded integer constant evaluator. Unknown expressions make
/// a typed enum unsupported instead of guessing ordinals or treating it as an
/// unrestricted scalar. Numeric values stay in Go; adapters map named cases.
fn integer_constant(
    node: Node<'_>,
    source: &str,
    iota: i128,
    constants: &BTreeMap<String, i128>,
    depth: usize,
) -> Result<i128> {
    ensure!(depth < 32, "Go enum constant expression depth limit");
    let recurse = |n| integer_constant(n, source, iota, constants, depth + 1);
    Ok(match node.kind() {
        "iota" => iota,
        "identifier" => *constants
            .get(text(node, source))
            .context("unknown Go enum constant expression")?,
        "int_literal" => {
            let digits = text(node, source).replace('_', "");
            let (radix, digits) = if digits.starts_with("0x") || digits.starts_with("0X") {
                (16, &digits[2..])
            } else if digits.starts_with("0b") || digits.starts_with("0B") {
                (2, &digits[2..])
            } else if digits.starts_with("0o") || digits.starts_with("0O") {
                (8, &digits[2..])
            } else if digits.len() > 1 && digits.starts_with('0') {
                (8, &digits[1..])
            } else {
                (10, digits.as_str())
            };
            i128::from_str_radix(digits, radix).context("Go enum integer literal limit")?
        }
        "parenthesized_expression" => recurse(node.named_child(0).context("Go enum parentheses")?)?,
        "unary_expression" => {
            let value = recurse(child(node, "operand")?)?;
            match text(child(node, "operator")?, source) {
                "+" => value,
                "-" => value.checked_neg().context("Go enum constant overflow")?,
                _ => bail!("unsupported Go enum unary expression"),
            }
        }
        "binary_expression" => {
            let left = recurse(child(node, "left")?)?;
            let right = recurse(child(node, "right")?)?;
            match text(child(node, "operator")?, source) {
                "+" => left.checked_add(right),
                "-" => left.checked_sub(right),
                "*" => left.checked_mul(right),
                "/" => left.checked_div(right),
                "%" => left.checked_rem(right),
                _ => bail!("unsupported Go enum arithmetic; use integer literals or iota"),
            }
            .context("Go enum constant overflow or division by zero")?
        }
        _ => bail!("unsupported Go enum expression; use integer literals or iota"),
    })
}

fn integer_base<'a>(
    name: &'a str,
    declarations: &'a BTreeMap<String, Declaration>,
    depth: usize,
) -> Option<&'a str> {
    if depth >= 32 {
        return None;
    }
    match name {
        "int" | "int8" | "int16" | "int32" | "int64" | "rune" | "uint" | "uint8" | "uint16"
        | "uint32" | "uint64" | "byte" => Some(name),
        _ => match declarations.get(name) {
            Some(Declaration::Alias(target)) => {
                integer_base(target.trim(), declarations, depth + 1)
            }
            _ => None,
        },
    }
}

fn enum_range(base: &str, value: i128) -> bool {
    match base {
        "int8" => i8::try_from(value).is_ok(),
        "int16" => i16::try_from(value).is_ok(),
        "int32" | "int" | "rune" => i32::try_from(value).is_ok(),
        "int64" => i64::try_from(value).is_ok(),
        "uint8" | "byte" => u8::try_from(value).is_ok(),
        "uint16" => u16::try_from(value).is_ok(),
        "uint32" | "uint" => u32::try_from(value).is_ok(),
        "uint64" => u64::try_from(value).is_ok(),
        _ => false,
    }
}

fn infer_enums(
    sources: &[(std::path::PathBuf, String)],
    types: &mut Types,
    named: &BTreeSet<String>,
) -> Result<()> {
    let mut candidates: BTreeMap<String, Vec<(String, i128)>> = BTreeMap::new();
    let mut invalid: BTreeMap<String, String> = BTreeMap::new();
    let mut constants = BTreeMap::new();
    let mut constant_types: BTreeMap<String, String> = BTreeMap::new();
    for (_, source) in sources {
        let tree = parse(source, true)?;
        for declaration in tree
            .root_node()
            .named_children(&mut tree.root_node().walk())
        {
            if declaration.kind() != "const_declaration" {
                continue;
            }
            let mut previous: Option<(Option<String>, Vec<Node<'_>>)> = None;
            for (ordinal, spec) in declaration
                .named_children(&mut declaration.walk())
                .filter(|n| n.kind() == "const_spec")
                .enumerate()
            {
                let names = spec
                    .children_by_field_name("name", &mut spec.walk())
                    .filter(|n| n.kind() == "identifier")
                    .map(|n| text(n, source).to_owned())
                    .collect::<Vec<_>>();
                let (explicit_type, values) =
                    if let Some(values) = spec.child_by_field_name("value") {
                        let ty = spec
                            .child_by_field_name("type")
                            .map(|n| text(n, source).to_owned());
                        let values = values
                            .named_children(&mut values.walk())
                            .filter(|n| n.kind() != "comment")
                            .collect::<Vec<_>>();
                        previous = Some((ty.clone(), values.clone()));
                        (ty, values)
                    } else {
                        previous
                            .clone()
                            .context("Go constant group must start with a value")?
                    };
                ensure!(
                    names.len() == values.len(),
                    "Go constant name/value count differs"
                );
                for (name, expression) in names.into_iter().zip(values) {
                    let inferred_type = if expression.kind() == "identifier" {
                        constant_types.get(text(expression, source)).cloned()
                    } else {
                        None
                    };
                    let ty = explicit_type.as_ref().or(inferred_type.as_ref());
                    let value =
                        integer_constant(expression, source, ordinal as i128, &constants, 0);
                    if name != "_" {
                        if let Ok(value) = &value {
                            constants.insert(name.clone(), *value);
                        }
                        if let Some(ty) = ty {
                            constant_types.insert(name.clone(), ty.clone());
                        }
                    }
                    let Some(ty) = ty.filter(|ty| named.contains(*ty)) else {
                        continue;
                    };
                    if name == "_" {
                        continue;
                    }
                    let Some(base) = integer_base(ty, &types.declarations, 0) else {
                        // String and bool typed constants do not turn aliases
                        // into integer enums.
                        continue;
                    };
                    match value {
                        Ok(value) if enum_range(base, value) => candidates
                            .entry(ty.clone())
                            .or_default()
                            .push((name, value)),
                        Ok(_) => {
                            invalid.insert(
                                ty.clone(),
                                "enum constant exceeds its target Go integer type".into(),
                            );
                        }
                        Err(error) => {
                            invalid.insert(ty.clone(), error.to_string());
                        }
                    }
                }
            }
        }
    }
    for (name, values) in candidates {
        let mut numbers = BTreeSet::new();
        let mut wires = BTreeSet::new();
        let mut cases = Vec::new();
        for (constant, value) in values {
            if !numbers.insert(value) {
                invalid.insert(
                    name.clone(),
                    "enum constants have duplicate integer values".into(),
                );
            }
            let suffix = constant
                .strip_prefix(&name)
                .filter(|s| !s.is_empty())
                .unwrap_or(&constant);
            let wire = wire_name(suffix).or_else(|_| wire_name(&constant))?;
            if !wires.insert(wire.clone()) {
                invalid.insert(
                    name.clone(),
                    "enum case names collide after normalization".into(),
                );
            }
            cases.push(SourceCase {
                name: constant,
                wire,
            });
        }
        ensure!(cases.len() <= 1024, "Go public enum case limit");
        types.declarations.insert(name, Declaration::Enum(cases));
    }
    for (name, reason) in invalid {
        types
            .declarations
            .insert(name, Declaration::Unsupported(reason));
    }
    Ok(())
}

pub(super) fn functions(project: &Path) -> Result<(Vec<Function>, BTreeMap<String, String>)> {
    let mut sources = Vec::new();
    for entry in fs::read_dir(project)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "go") || entry.file_name() == "wisp_bindings.go" {
            continue;
        }
        let source = String::from_utf8(read(&path, MAX_SOURCE)?)?;
        if crate::build_go::selected(&path, &source)? {
            sources.push((path, source));
        }
    }
    sources.sort_by(|a, b| a.0.cmp(&b.0));
    let mut types = Types {
        declarations: BTreeMap::new(),
        remaining: 8192,
        used_names: BTreeMap::new(),
    };
    let mut named = BTreeSet::new();
    for (_, source) in &sources {
        let tree = parse(source, true)?;
        for node in tree
            .root_node()
            .named_children(&mut tree.root_node().walk())
        {
            if node.kind() != "type_declaration" {
                continue;
            }
            for decl in node.named_children(&mut node.walk()) {
                if !matches!(decl.kind(), "type_alias" | "type_spec") {
                    continue;
                }
                let name = text(child(decl, "name")?, source).to_owned();
                if decl.kind() == "type_spec" {
                    named.insert(name.clone());
                }
                ensure!(
                    types
                        .declarations
                        .insert(name.clone(), declaration(decl, source)?)
                        .is_none(),
                    "duplicate Go type declaration {name}"
                );
            }
        }
    }
    infer_enums(&sources, &mut types, &named)?;
    let mut functions = Vec::new();
    for (path, source) in &sources {
        let tree =
            parse(source, true).with_context(|| format!("reading Go API {}", path.display()))?;
        for function in tree
            .root_node()
            .named_children(&mut tree.root_node().walk())
        {
            if function.kind() != "function_declaration" {
                continue;
            }
            let name = text(child(function, "name")?, source);
            if !name.chars().next().is_some_and(char::is_uppercase) {
                continue;
            }
            ensure!(
                function.child_by_field_name("type_parameters").is_none(),
                "generic Go function {name} cannot be exported"
            );
            ensure!(
                function.child_by_field_name("body").is_some(),
                "exported Go function {name} requires an implementation"
            );
            types.remaining = 8192;
            let go_params = parameter_types(child(function, "parameters")?, source)?;
            ensure!(
                go_params.len() <= 16,
                "public function parameter limit is 16"
            );
            let source_params = go_params
                .iter()
                .map(|ty| types.ty(ty))
                .collect::<Result<Vec<_>>>()
                .with_context(|| format!("Go export {name}"))?;
            let results = function
                .child_by_field_name("result")
                .map(|node| {
                    if node.kind() == "parameter_list" {
                        parameter_types(node, source)
                    } else {
                        Ok(vec![text(node, source).into()])
                    }
                })
                .transpose()?
                .unwrap_or_default();
            let go_error_result = results.last().is_some_and(|ty| ty == "error");
            let source_result = if go_error_result {
                ensure!(
                    results.len() <= 2,
                    "Go export {name} supports error or (value, error) results"
                );
                let ok = if results.len() == 2 {
                    Some(types.ty(&results[0])?)
                } else {
                    None
                };
                let err = types.ty("string")?;
                let name = if results.len() == 2 {
                    format!("({}, error)", results[0])
                } else {
                    "error".into()
                };
                Some(SourceType::result(name, ok, Some(err)))
            } else {
                ensure!(
                    results.len() <= 1,
                    "Go export {name} has multiple results; use one result or (value, error)"
                );
                results.first().map(|ty| types.ty(ty)).transpose()?
            };
            let declarations = type_declarations(
                source_params
                    .iter()
                    .cloned()
                    .chain(source_result.iter().cloned()),
            )?;
            functions.push(Function {
                original: name.into(),
                wire: wire_name(name)?,
                params: source_params.iter().map(|ty| ty.wire.clone()).collect(),
                result: source_result.as_ref().map(|ty| ty.wire.clone()),
                source_params,
                source_result,
                go_error_result,
                types: declarations,
            });
        }
    }
    let hashes = sources
        .iter()
        .map(|(path, source)| {
            Ok((
                path.strip_prefix(project)?
                    .to_str()
                    .context("Go source path")?
                    .replace('\\', "/"),
                digest(source.as_bytes()),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok((functions, hashes))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn api(source: &str) -> Result<Vec<Function>> {
        let project = tempfile::tempdir()?;
        fs::write(
            project.path().join("main.go"),
            format!("package main\n{source}\nfunc main() {{}}"),
        )?;
        Ok(functions(project.path())?.0)
    }
    #[test]
    fn strings_bytes_records_and_errors_keep_ordinary_go_types() {
        let functions = api("type Text string\ntype Packet struct { Name Text; Data []byte; Count uint32 }\nfunc Echo(value Packet) (Packet,error) { return value,nil }\nfunc Check(value string) error { return nil }\nfunc Buffer(value []uint8) []byte { return value }").unwrap();
        let echo = functions.iter().find(|f| f.original == "Echo").unwrap();
        assert_eq!(echo.params, ["%packet"]);
        assert_eq!(echo.result.as_deref(), Some("result<%packet, string>"));
        assert!(echo.go_error_result);
        let SourceKind::Record(fields) = &echo.source_params[0].kind else {
            panic!()
        };
        assert_eq!(fields[0].name, "Name");
        assert_eq!(fields[0].ty.name, "Text");
        assert!(matches!(fields[1].ty.kind, SourceKind::Bytes));
        assert!(echo.types["%packet"].contains("%count: u32"));
        assert_eq!(
            functions
                .iter()
                .find(|f| f.original == "Check")
                .unwrap()
                .result
                .as_deref(),
            Some("result<_, string>")
        );
        let wit = declarations(&functions).unwrap();
        Resolve::default()
            .push_str(
                "api.wit",
                &format!("package test:go; interface api {{ {wit} }} world app {{ export api; }}"),
            )
            .unwrap();
    }
    #[test]
    fn rejects_unsupported_go_graphs_and_ambiguous_fields() {
        for source in [
            "type A B; type B A; func Echo(value A) A { return value }",
            "type FooBar struct { X uint32 }; type Foo_Bar struct { X uint32 }; func First(value FooBar) FooBar{return value}; func Second(value Foo_Bar) Foo_Bar{return value}",
            "type Node struct { Next *Node }; func Echo(value Node) Node { return value }",
            "type Data struct { value string }; func Echo(value Data) Data { return value }",
            "type Data struct { X string; X_ string }; func Echo(value Data) Data { return value }",
            "type Data struct { []byte }; func Echo(value Data) Data { return value }",
            "func Echo(value map[string]string) map[string]string { return value }",
            "func Pair() (uint32,uint32) { return 1,2 }",
        ] {
            assert!(api(source).is_err(), "{source}");
        }
    }

    #[test]
    fn slices_optional_pointers_and_iota_enums_are_value_types() {
        let functions = api("type Mode uint8\nconst (\nModeFast Mode = iota\nModeCareful\n)\ntype Batch struct { Labels []string; Mode *Mode; Choices []*Mode }\nfunc Echo(value Batch) Batch {return value}\nfunc Optional(value **Mode) **Mode {return value}").unwrap();
        let echo = functions.iter().find(|f| f.original == "Echo").unwrap();
        let SourceKind::Record(fields) = &echo.source_params[0].kind else {
            panic!()
        };
        assert_eq!(fields[0].ty.wire, "list<string>");
        assert!(matches!(fields[0].ty.kind, SourceKind::List(_)));
        assert_eq!(fields[1].ty.wire, "option<%mode>");
        assert_eq!(fields[2].ty.wire, "list<option<%mode>>");
        let SourceKind::Option(mode) = &fields[1].ty.kind else {
            panic!()
        };
        let SourceKind::Enum(cases) = &mode.kind else {
            panic!()
        };
        assert_eq!(
            cases
                .iter()
                .map(|c| (c.name.as_str(), c.wire.as_str()))
                .collect::<Vec<_>>(),
            [("ModeFast", "fast"), ("ModeCareful", "careful")]
        );
        assert_eq!(
            functions
                .iter()
                .find(|f| f.original == "Optional")
                .unwrap()
                .params,
            ["option<option<%mode>>"]
        );
        let wit = declarations(&functions).unwrap();
        Resolve::default()
            .push_str(
                "api.wit",
                &format!("package test:go; interface api {{ {wit} }} world app {{ export api; }}"),
            )
            .unwrap();
    }

    #[test]
    fn enums_map_source_constants_and_scalar_aliases_stay_scalars() {
        let functions = api("type Mode int16\nconst (\nModeFast Mode = -4\nModeCareful Mode = 19\n)\ntype Count uint32\nfunc Enum(value Mode) Mode{return value}\nfunc Alias(value Count) Count{return value}").unwrap();
        let f = functions.iter().find(|f| f.original == "Enum").unwrap();
        assert!(matches!(f.source_params[0].kind, SourceKind::Enum(_)));
        let f = functions.iter().find(|f| f.original == "Alias").unwrap();
        assert!(matches!(f.source_params[0].kind, SourceKind::Scalar));
        assert_eq!(f.params, ["u32"]);
    }

    #[test]
    fn ambiguous_enums_and_recursive_optional_records_are_rejected() {
        for (source, expected) in [
            (
                "type Mode uint8\nconst (\nModeA Mode = 0\nModeB Mode = 0\n)\nfunc Echo(value Mode) Mode{return value}",
                "duplicate integer values",
            ),
            (
                "type Mode uint8\nconst ModeA Mode = 256\nfunc Echo(value Mode) Mode{return value}",
                "exceeds its target Go integer type",
            ),
            (
                "type Mode int\nconst ModeA Mode = unknown\nfunc Echo(value Mode) Mode{return value}",
                "unknown Go enum constant expression",
            ),
            (
                "type Node struct { Children []*Node }\nfunc Echo(value Node) Node{return value}",
                "recursive Go public API type",
            ),
        ] {
            let error = api(source).unwrap_err();
            assert!(format!("{error:#}").contains(expected), "{error:#}");
        }
    }

    #[test]
    fn upstream_export_adapters_keep_ordinary_go_functions() {
        let project = tempfile::tempdir().unwrap();
        let source = "package main\ntype Mode uint8\nconst (\nModeFast Mode=iota\nModeCareful\n)\ntype Batch struct {Labels []string; Selected *Mode; Modes []Mode}\nfunc Echo(value Batch) (Batch,error){return value,nil}\nfunc Choices(value []*Mode) []*Mode{return value}\nfunc main(){}\n";
        fs::write(project.path().join("main.go"), source).unwrap();
        fs::write(
            project.path().join("go.mod"),
            "module example.org/inferred\ngo 1.27.0\n",
        )
        .unwrap();
        let (mut functions, _) = functions(project.path()).unwrap();
        normalize_record_names(&mut functions).unwrap();
        let body = declarations(&functions).unwrap();
        let mut resolve = Resolve::default();
        let package = resolve
            .push_str(
                "generated.wit",
                &format!(
                    "package test:go-inferred; interface api {{ {body} }} world app {{export api;}}"
                ),
            )
            .unwrap();
        let world = resolve.select_world(&[package], Some("app")).unwrap();
        let functions = functions.into_iter().map(|f| (f.wire.clone(), f)).collect();
        crate::build::inferred::go::generate(&resolve, world, &functions, project.path()).unwrap();
        let glue = fs::read_to_string(project.path().join("wisp_bindings.go")).unwrap();
        assert!(glue.contains("package main"));
        assert!(glue.contains("//go:wasmexport test:go-inferred/api#echo"));
        assert!(glue.contains("witTypes.Option[test_go_inferred_api.Mode]"));
        assert!(glue.contains("value, err := Echo("));
        assert!(glue.contains("return witTypes.Err["));
        assert!(glue.contains("value == nil"));
        assert!(glue.contains("value.IsNone()"));
        assert!(!glue.contains("\"example.org/inferred/export_test_go_inferred_api\""));
        assert!(!glue.contains("export_test_go_inferred_api."));
        assert!(!glue.contains("func main("));
        assert_eq!(
            fs::read_to_string(project.path().join("main.go")).unwrap(),
            source
        );
    }

    #[test]
    fn ordinary_examples_infer_without_authored_wit() {
        for source in [
            include_str!("../../../examples/build-go/src/main.go"),
            include_str!("../../../examples/build-values/go/main.go"),
        ] {
            let project = tempfile::tempdir().unwrap();
            fs::write(project.path().join("main.go"), source).unwrap();
            let (functions, _) = functions(project.path()).unwrap();
            let wit = declarations(&functions).unwrap();
            Resolve::default().push_str("api.wit", &format!("package test:example; interface api {{ {wit} }} world app {{ export api; }}")).unwrap();
            if source.contains("type Mode") {
                let choices = functions.iter().find(|f| f.original == "Choices").unwrap();
                assert_eq!(choices.params, ["list<option<%mode>>"]);
                assert_eq!(choices.result.as_deref(), Some("list<option<%mode>>"));
            }
        }
    }
}
