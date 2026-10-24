//! Public Go APIs use native strings, byte slices, structs, and error returns.
use super::*;

#[derive(Clone)]
enum Declaration {
    Alias(String),
    Record(Vec<(String, String)>),
    Unsupported(String),
}
struct Types {
    declarations: BTreeMap<String, Declaration>,
    remaining: usize,
    used_names: BTreeMap<String, String>,
}
impl Types {
    fn infer(&mut self, name: &str, stack: &mut BTreeSet<String>) -> Result<SourceType> {
        ensure!(
            self.remaining > 0 && stack.len() < 32,
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
                    let mut ty = self.infer(&target, stack)?;
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
                                ty: self.infer(&ty, stack)?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    SourceType {
                        name: name.into(),
                        wire: format!("%{}", wire_name(name)?),
                        kind: SourceKind::Record(fields),
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
                "unsupported public Go type {name}; use scalars, string, []byte, or named structs with exported fields"
            ),
        };
        Ok(SourceType::scalar(name, wire))
    }
    fn ty(&mut self, name: &str) -> Result<SourceType> {
        self.infer(name, &mut BTreeSet::new())
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
                go_params,
                go_result: source_result.as_ref().map(|ty| ty.name.clone()),
                rust_params: Vec::new(),
                rust_result: None,
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
            "func Echo(value []string) []string { return value }",
            "func Pair() (uint32,uint32) { return 1,2 }",
        ] {
            assert!(api(source).is_err(), "{source}");
        }
    }
}
