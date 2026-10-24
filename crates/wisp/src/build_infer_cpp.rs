//! Freestanding C++ APIs use aggregates and Wisp's string, byte, and result views.
use super::*;

#[derive(Clone)]
enum Declaration {
    Record(Vec<(String, String)>, String),
    Alias(String, String),
    Unsupported(String),
}
struct Types {
    declarations: BTreeMap<String, Declaration>,
    remaining: usize,
    used_names: BTreeMap<String, String>,
}
fn scalar(name: &str) -> Option<&'static str> {
    Some(match name {
        "bool" | "_Bool" => "bool",
        "uint8_t" | "std::uint8_t" | "unsigned char" => "u8",
        "int8_t" | "std::int8_t" | "signed char" | "char" => "s8",
        "uint16_t" | "std::uint16_t" | "unsigned short" | "unsigned short int" => "u16",
        "int16_t" | "std::int16_t" | "short" | "short int" | "signed short"
        | "signed short int" => "s16",
        "uint32_t" | "std::uint32_t" | "unsigned" | "unsigned int" | "unsigned long"
        | "unsigned long int" => "u32",
        "int32_t" | "std::int32_t" | "int" | "signed" | "signed int" | "long" | "long int"
        | "signed long" | "signed long int" => "s32",
        "uint64_t" | "std::uint64_t" | "unsigned long long" | "unsigned long long int" => "u64",
        "int64_t"
        | "std::int64_t"
        | "long long"
        | "long long int"
        | "signed long long"
        | "signed long long int" => "s64",
        "float" => "f32",
        "double" => "f64",
        _ => return None,
    })
}
fn arguments(name: &str) -> Result<Option<(&str, Vec<&str>)>> {
    let Some(start) = name.find('<') else {
        return Ok(None);
    };
    ensure!(
        name.ends_with('>'),
        "invalid public C++ template type {name}"
    );
    let arguments = &name[start + 1..name.len() - 1];
    let mut depth = 0usize;
    let mut begin = 0;
    let mut result = Vec::new();
    for (i, byte) in arguments.bytes().enumerate() {
        match byte {
            b'<' => depth += 1,
            b'>' => {
                ensure!(depth > 0, "invalid public C++ template type {name}");
                depth -= 1;
            }
            b',' if depth == 0 => {
                result.push(arguments[begin..i].trim());
                begin = i + 1;
            }
            _ => {}
        }
    }
    ensure!(depth == 0, "invalid public C++ template type {name}");
    result.push(arguments[begin..].trim());
    Ok(Some((name[..start].trim(), result)))
}
impl Types {
    fn lookup(&self, name: &str, prefix: &str) -> Option<String> {
        if name.starts_with("::") {
            return self
                .declarations
                .contains_key(name.trim_start_matches("::"))
                .then(|| name.trim_start_matches("::").into());
        }
        let mut scope = prefix.to_owned();
        loop {
            let key = format!("{scope}{name}");
            if self.declarations.contains_key(&key) {
                return Some(key);
            }
            if scope.is_empty() {
                return None;
            }
            scope = scope
                .trim_end_matches("::")
                .rsplit_once("::")
                .map(|(p, _)| format!("{p}::"))
                .unwrap_or_default();
        }
    }
    fn infer(
        &mut self,
        name: &str,
        prefix: &str,
        stack: &mut BTreeSet<String>,
        depth: usize,
    ) -> Result<SourceType> {
        ensure!(
            self.remaining > 0 && depth < 32,
            "C++ public API type expansion limit"
        );
        self.remaining -= 1;
        let name = name.trim().strip_prefix("struct ").unwrap_or(name.trim());
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if let Some(key) = self.lookup(&name, prefix) {
            ensure!(
                stack.insert(key.clone()),
                "recursive C++ public API type {key} is unsupported"
            );
            let result = match self.declarations[&key].clone() {
                Declaration::Alias(ty, prefix) => self.infer(&ty, &prefix, stack, depth + 1)?,
                Declaration::Record(fields, prefix) => {
                    let wire = wire_name(&key.replace("::", "-"))?;
                    if let Some(previous) = self.used_names.insert(wire, key.clone()) {
                        ensure!(
                            previous == key,
                            "public C++ record names collide after normalization: {previous}, {key}"
                        );
                    }
                    let fields = fields
                        .into_iter()
                        .map(|(name, ty)| {
                            Ok(SourceField {
                                wire: wire_name(&name)?,
                                name,
                                ty: self.infer(&ty, &prefix, stack, depth + 1)?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    SourceType {
                        name: key.clone(),
                        wire: format!("%{}", wire_name(&key.replace("::", "-"))?),
                        kind: SourceKind::Record(fields),
                    }
                }
                Declaration::Unsupported(reason) => {
                    bail!("unsupported public C++ record {key}: {reason}")
                }
            };
            stack.remove(&key);
            return Ok(result);
        }
        if matches!(name.as_str(), "wisp::string" | "::wisp::string") {
            return Ok(SourceType {
                name: "wisp::string".into(),
                wire: "string".into(),
                kind: SourceKind::String,
            });
        }
        if matches!(name.as_str(), "wisp::bytes" | "::wisp::bytes") {
            return Ok(SourceType {
                name: "wisp::bytes".into(),
                wire: "list<u8>".into(),
                kind: SourceKind::Bytes,
            });
        }
        if let Some((base, args)) = arguments(&name)? {
            ensure!(
                matches!(base, "wisp::result" | "::wisp::result"),
                "unsupported public C++ template type {name}; use wisp::result<T,E>"
            );
            ensure!(
                args.len() == 2,
                "wisp::result requires success and error types"
            );
            let mut payload = |name: &str| -> Result<Option<SourceType>> {
                if matches!(name, "void" | "wisp::unit" | "::wisp::unit") {
                    Ok(None)
                } else {
                    Ok(Some(self.infer(name, prefix, stack, depth + 1)?))
                }
            };
            let ok = payload(args[0])?;
            let err = payload(args[1])?;
            return Ok(SourceType::result(
                format!(
                    "wisp::result<{}, {}>",
                    ok.as_ref().map(|t| t.name.as_str()).unwrap_or(args[0]),
                    err.as_ref().map(|t| t.name.as_str()).unwrap_or(args[1])
                ),
                ok,
                err,
            ));
        }
        let wire = scalar(&name).with_context(|| format!("unsupported public C/C++ type {name}; use scalars, plain structs, wisp::string, wisp::bytes, or wisp::result<T,E>"))?;
        Ok(SourceType::scalar(name, wire))
    }
    fn ty(&mut self, name: &str, prefix: &str) -> Result<SourceType> {
        self.infer(name, prefix, &mut BTreeSet::new(), 0)
    }
}
fn record(node: Node<'_>, source: &str, prefix: &str) -> Result<Declaration> {
    if node
        .named_children(&mut node.walk())
        .any(|c| matches!(c.kind(), "base_class_clause" | "virtual_specifier"))
    {
        return Ok(Declaration::Unsupported(
            "inheritance and virtual records are unsupported".into(),
        ));
    }
    let mut fields = Vec::new();
    for field in child(node, "body")?.named_children(&mut node.walk()) {
        if field.kind() == "comment" {
            continue;
        }
        if field.kind() == "access_specifier" && text(field, source) == "public" {
            continue;
        }
        if field.kind() != "field_declaration" {
            return Ok(Declaration::Unsupported(
                "records require plain public fields and no methods".into(),
            ));
        }
        if field
            .named_children(&mut field.walk())
            .any(|c| matches!(c.kind(), "bitfield_clause" | "storage_class_specifier"))
        {
            return Ok(Declaration::Unsupported(
                "record bitfields and static fields are unsupported".into(),
            ));
        }
        let ty = text(child(field, "type")?, source).to_owned();
        let names = field
            .children_by_field_name("declarator", &mut field.walk())
            .collect::<Vec<_>>();
        if names.is_empty() || names.iter().any(|n| n.kind() != "field_identifier") {
            return Ok(Declaration::Unsupported(
                "record pointers, references, arrays, and methods are unsupported".into(),
            ));
        }
        for name in names {
            fields.push((text(name, source).into(), ty.clone()));
        }
    }
    Ok(Declaration::Record(fields, prefix.into()))
}
fn collect(node: Node<'_>, source: &str, prefix: &str, types: &mut Types) -> Result<()> {
    for item in node.named_children(&mut node.walk()) {
        match item.kind() {
            "namespace_definition" => {
                let Some(name) = item.child_by_field_name("name") else {
                    continue;
                };
                collect(
                    child(item, "body")?,
                    source,
                    &format!("{prefix}{}::", text(name, source)),
                    types,
                )?;
            }
            "linkage_specification" | "declaration_list" => collect(item, source, prefix, types)?,
            "struct_specifier" if item.child_by_field_name("body").is_some() => {
                let name = text(child(item, "name")?, source);
                let key = format!("{prefix}{name}");
                // Identical definitions may occur in separate translation units. Their
                // ordinary compiler still enforces C++'s definition rules.
                types
                    .declarations
                    .insert(key, record(item, source, prefix)?);
            }
            "alias_declaration" => {
                let name = text(child(item, "name")?, source);
                let ty = text(child(item, "type")?, source);
                types.declarations.insert(
                    format!("{prefix}{name}"),
                    Declaration::Alias(ty.into(), prefix.into()),
                );
            }
            _ => {}
        }
    }
    Ok(())
}
fn walk(
    node: Node<'_>,
    source: &str,
    prefix: &str,
    types: &mut Types,
    result: &mut Vec<Function>,
) -> Result<()> {
    for item in node.named_children(&mut node.walk()) {
        match item.kind() {
            "namespace_definition" => {
                let Some(name) = item.child_by_field_name("name") else {
                    continue;
                };
                walk(
                    child(item, "body")?,
                    source,
                    &format!("{prefix}{}::", text(name, source)),
                    types,
                    result,
                )?;
            }
            "linkage_specification" | "declaration_list" => {
                walk(item, source, prefix, types, result)?
            }
            "template_declaration" => {
                bail!("public C++ templates need a non-template wrapper function")
            }
            "preproc_if" | "preproc_ifdef" => bail!(
                "conditional C/C++ public declarations are unsupported; keep exported definitions unconditional"
            ),
            "function_definition" => {
                if item
                    .named_children(&mut item.walk())
                    .any(|c| c.kind() == "storage_class_specifier" && text(c, source) == "static")
                {
                    continue;
                }
                let declaration = child(item, "declarator")?;
                ensure!(
                    declaration.kind() == "function_declarator",
                    "public C/C++ pointer/reference returns are unsupported"
                );
                let name_node = child(declaration, "declarator")?;
                ensure!(
                    name_node.kind() == "identifier",
                    "unsupported public C++ function declarator"
                );
                let name = text(name_node, source);
                if name == "main" {
                    continue;
                }
                types.remaining = 8192;
                let mut source_params = Vec::new();
                for parameter in
                    child(declaration, "parameters")?.named_children(&mut declaration.walk())
                {
                    if parameter.kind() == "comment" {
                        continue;
                    }
                    ensure!(
                        parameter.kind() == "parameter_declaration",
                        "variadic/default public C++ parameters are unsupported"
                    );
                    ensure!(
                        parameter
                            .child_by_field_name("declarator")
                            .is_none_or(|d| d.kind() == "identifier"),
                        "public C/C++ pointer/reference parameters are unsupported"
                    );
                    let ty = text(child(parameter, "type")?, source);
                    if ty != "void" {
                        source_params.push(
                            types
                                .ty(ty, prefix)
                                .with_context(|| format!("C++ export {prefix}{name}"))?,
                        );
                    }
                }
                ensure!(
                    source_params.len() <= 16,
                    "public function parameter limit is 16"
                );
                let result_type = text(child(item, "type")?, source);
                let source_result = if result_type == "void" {
                    None
                } else {
                    Some(
                        types
                            .ty(result_type, prefix)
                            .with_context(|| format!("C++ export {prefix}{name}"))?,
                    )
                };
                let original = format!("{prefix}{name}");
                let declarations = type_declarations(
                    source_params
                        .iter()
                        .cloned()
                        .chain(source_result.iter().cloned()),
                )?;
                result.push(Function {
                    wire: wire_name(&original.replace("::", "-"))?,
                    original,
                    params: source_params.iter().map(|ty| ty.wire.clone()).collect(),
                    result: source_result.as_ref().map(|ty| ty.wire.clone()),
                    rust_params: Vec::new(),
                    rust_result: None,
                    go_params: Vec::new(),
                    go_result: None,
                    source_params,
                    source_result,
                    go_error_result: false,
                    types: declarations,
                });
            }
            _ => {}
        }
    }
    Ok(())
}
pub(super) fn functions(
    root: &Path,
    paths: &[PathBuf],
) -> Result<(Vec<Function>, BTreeMap<String, String>)> {
    let mut sources = Vec::new();
    let mut hashes = BTreeMap::new();
    for path in paths {
        let absolute = input(root, path)?;
        let source = String::from_utf8(read(&absolute, MAX_SOURCE)?)?;
        hashes.insert(
            absolute
                .strip_prefix(root)?
                .to_str()
                .context("C++ source path")?
                .replace('\\', "/"),
            digest(source.as_bytes()),
        );
        let tree =
            parse(&source, false).with_context(|| format!("reading C++ API {}", path.display()))?;
        sources.push((source, tree));
    }
    let mut types = Types {
        declarations: BTreeMap::new(),
        remaining: 8192,
        used_names: BTreeMap::new(),
    };
    for (source, tree) in &sources {
        collect(tree.root_node(), source, "", &mut types)?;
    }
    let mut result = Vec::new();
    for (source, tree) in &sources {
        walk(tree.root_node(), source, "", &mut types, &mut result)?;
    }
    Ok((result, hashes))
}
#[cfg(test)]
pub(super) fn walk_test(node: Node<'_>, source: &str, result: &mut Vec<Function>) -> Result<()> {
    let mut types = Types {
        declarations: BTreeMap::new(),
        remaining: 8192,
        used_names: BTreeMap::new(),
    };
    collect(node, source, "", &mut types)?;
    walk(node, source, "", &mut types, result)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn api(source: &str) -> Result<Vec<Function>> {
        let tree = parse(source, false)?;
        let mut result = Vec::new();
        walk_test(tree.root_node(), source, &mut result)?;
        Ok(result)
    }
    #[test]
    fn plain_aggregates_views_and_results_preserve_source_names() {
        let functions = api("#include \"wisp_types.hpp\"\nnamespace app { struct Packet { wisp::string label; wisp::bytes data; unsigned count; }; Packet Echo(Packet value) { return value; } wisp::result<Packet, wisp::string> Parse(wisp::bytes data) { return {}; } } ").unwrap();
        assert_eq!(functions[0].params, ["%app-packet"]);
        assert_eq!(functions[0].source_params[0].name, "app::Packet");
        assert_eq!(
            functions[1].result.as_deref(),
            Some("result<%app-packet, string>")
        );
        assert!(functions[0].types["%app-packet"].contains("%data: list<u8>"));
        let SourceKind::Record(fields) = &functions[0].source_params[0].kind else {
            panic!()
        };
        assert_eq!(fields[0].name, "label");
        let wit = declarations(&functions).unwrap();
        Resolve::default()
            .push_str(
                "api.wit",
                &format!("package test:cpp; interface api {{ {wit} }} world app {{ export api; }}"),
            )
            .unwrap();
    }
    #[test]
    fn rejects_nonplain_or_recursive_cpp_types() {
        for source in [
            "struct Data { unsigned *data; }; Data Echo(Data value) { return value; }",
            "struct Data { private: unsigned data; }; Data Echo(Data value) { return value; }",
            "struct Data { unsigned Get() { return 1; } }; Data Echo(Data value) { return value; }",
            "struct Data { unsigned data[2]; }; Data Echo(Data value) { return value; }",
            "struct Data { unsigned data: 2; }; Data Echo(Data value) { return value; }",
            "struct Data { unsigned one_word; unsigned oneWord; }; Data Echo(Data value) { return value; }",
            "using A = B; using B = A; A Echo(A value) { return value; }",
            "struct FooBar { unsigned x; }; struct Foo_Bar { unsigned x; }; FooBar First(FooBar value){return value;} Foo_Bar Second(Foo_Bar value){return value;}",
            "std::string Echo(std::string value) { return value; }",
        ] {
            assert!(api(source).is_err(), "{source}");
        }
    }
}
