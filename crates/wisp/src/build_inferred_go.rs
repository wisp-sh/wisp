//! Adapt ordinary Go values to upstream generated types. wit-bindgen owns the ABI.
use super::super::infer::{Function, SourceKind, SourceType};
use anyhow::{Context, Result, bail, ensure};
use heck::ToUpperCamelCase;
use regex::Regex;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use wit_parser::{Resolve, WorldId, WorldItem};

struct Conversions<'a> {
    module: &'a str,
    package: &'a str,
    imports: BTreeSet<(String, String)>,
    helpers: BTreeMap<String, String>,
    code: String,
}

impl Conversions<'_> {
    fn wire_type(&mut self, ty: &SourceType) -> Result<String> {
        Ok(match &ty.kind {
            SourceKind::Scalar => match ty.wire.as_str() {
                "bool" => "bool",
                "u8" => "uint8",
                "u16" => "uint16",
                "u32" => "uint32",
                "u64" => "uint64",
                "s8" => "int8",
                "s16" => "int16",
                "s32" => "int32",
                "s64" => "int64",
                "f32" => "float32",
                "f64" => "float64",
                _ => bail!("unsupported inferred Go scalar {}", ty.wire),
            }
            .into(),
            SourceKind::String => "string".into(),
            SourceKind::Bytes => "[]uint8".into(),
            SourceKind::List(element) => format!("[]{}", self.wire_type(element)?),
            SourceKind::Option(element) => {
                self.runtime_types();
                format!("witTypes.Option[{}]", self.wire_type(element)?)
            }
            SourceKind::Record(_) | SourceKind::Enum(_) => {
                self.imports.insert((
                    self.package.into(),
                    format!("{}/{}", self.module, self.package),
                ));
                format!(
                    "{}.{}",
                    self.package,
                    ty.wire.trim_start_matches('%').to_upper_camel_case()
                )
            }
            SourceKind::Result { ok, err } => {
                self.runtime_types();
                let ok = ok
                    .as_ref()
                    .map(|ty| self.wire_type(ty))
                    .transpose()?
                    .unwrap_or_else(|| "witTypes.Unit".into());
                let err = err
                    .as_ref()
                    .map(|ty| self.wire_type(ty))
                    .transpose()?
                    .unwrap_or_else(|| "witTypes.Unit".into());
                format!("witTypes.Result[{ok}, {err}]")
            }
        })
    }

    fn runtime_types(&mut self) {
        self.imports.insert((
            "witTypes".into(),
            "go.bytecodealliance.org/pkg/wit/types".into(),
        ));
    }

    fn value(&mut self, ty: &SourceType, value: &str, to_native: bool) -> Result<String> {
        let wire_type = self.wire_type(ty)?;
        let target = if to_native { &ty.name } else { &wire_type };
        if matches!(
            ty.kind,
            SourceKind::Scalar | SourceKind::String | SourceKind::Bytes
        ) {
            return Ok(format!("({target})({value})"));
        }
        ensure!(
            !matches!(ty.kind, SourceKind::Result { .. }),
            "Go error results must be adapted at the function boundary"
        );
        let key = format!("{}|{}|{}|{to_native}", self.package, ty.name, ty.wire);
        if let Some(helper) = self.helpers.get(&key) {
            return Ok(format!("{helper}({value})"));
        }
        let helper = format!("__wisp_value_{}", self.helpers.len());
        self.helpers.insert(key, helper.clone());
        let (input, output) = if to_native {
            (wire_type.as_str(), ty.name.as_str())
        } else {
            (ty.name.as_str(), wire_type.as_str())
        };
        let body = match &ty.kind {
            SourceKind::Record(fields) => {
                let mut fields_code = String::new();
                for field in fields {
                    let generated = field.wire.to_upper_camel_case();
                    let (source, destination) = if to_native {
                        (&generated, &field.name)
                    } else {
                        (&field.name, &generated)
                    };
                    let value = self.value(&field.ty, &format!("value.{source}"), to_native)?;
                    fields_code.push_str(&format!("{destination}: {value},\n"));
                }
                format!("return {output}{{\n{fields_code}}}\n")
            }
            SourceKind::List(element) => {
                let converted = self.value(element, "element", to_native)?;
                format!(
                    "result := make({output}, len(value))\nfor index, element := range value {{ result[index] = {converted} }}\nreturn result\n"
                )
            }
            SourceKind::Option(element) => {
                if to_native {
                    let converted = self.value(element, "value.Some()", true)?;
                    format!(
                        "if value.IsNone() {{ return nil }}\nresult := {converted}\nreturn ({output})(&result)\n"
                    )
                } else {
                    let element_type = self.wire_type(element)?;
                    let converted = self.value(element, "*value", false)?;
                    format!(
                        "if value == nil {{ return witTypes.None[{element_type}]() }}\nreturn witTypes.Some[{element_type}]({converted})\n"
                    )
                }
            }
            SourceKind::Enum(cases) => {
                let name = ty.wire.trim_start_matches('%').to_upper_camel_case();
                let mut code = String::from("switch value {\n");
                for case in cases {
                    let upstream =
                        format!("{}.{name}{}", self.package, case.wire.to_upper_camel_case());
                    let native = format!("({})({})", ty.name, case.name);
                    let (from, to) = if to_native {
                        (upstream, native)
                    } else {
                        (native, upstream)
                    };
                    code.push_str(&format!("case {from}: return {to}\n"));
                }
                code.push_str("default: panic(\"invalid inferred Go enum value\")\n}\n");
                code
            }
            _ => unreachable!(),
        };
        self.code.push_str(&format!(
            "func {helper}(value {input}) {output} {{\n{body}}}\n"
        ));
        Ok(format!("{helper}({value})"))
    }

    fn adapter(&mut self, function: &Function, name: &str) -> Result<String> {
        let mut params = Vec::new();
        let mut args = Vec::new();
        for (index, ty) in function.source_params.iter().enumerate() {
            params.push(format!("a{index} {}", self.wire_type(ty)?));
            args.push(self.value(ty, &format!("a{index}"), true)?);
        }
        let call = format!("{}({})", function.original, args.join(", "));
        let Some(result) = &function.source_result else {
            return Ok(format!("func {name}({}) {{ {call} }}\n", params.join(", ")));
        };
        let result_type = self.wire_type(result)?;
        let body = if function.go_error_result {
            let SourceKind::Result { ok, err } = &result.kind else {
                bail!("inferred Go error result shape")
            };
            ensure!(
                err.as_ref()
                    .is_some_and(|ty| matches!(ty.kind, SourceKind::String)),
                "Go error must map to string"
            );
            let ok_type = ok
                .as_ref()
                .map(|ty| self.wire_type(ty))
                .transpose()?
                .unwrap_or_else(|| "witTypes.Unit".into());
            let (binding, value) = if let Some(ty) = ok {
                ("value, err", self.value(ty, "value", false)?)
            } else {
                ("err", "witTypes.Unit{}".into())
            };
            format!(
                "{binding} := {call}\nif err != nil {{ return witTypes.Err[{ok_type}, string](err.Error()) }}\nreturn witTypes.Ok[{ok_type}, string]({value})\n"
            )
        } else {
            format!("return {}\n", self.value(result, &call, false)?)
        };
        Ok(format!(
            "func {name}({}) {result_type} {{\n{body}}}\n",
            params.join(", ")
        ))
    }
}

/// Rewrite only upstream's source call sites. Raw pointer signatures, canonical
/// layout, allocation, pinning, and post-return functions remain upstream code.
pub fn generate(
    resolve: &Resolve,
    world: WorldId,
    functions: &BTreeMap<String, Function>,
    source: &Path,
) -> Result<()> {
    let module = crate::build_go::module_prefix(source)?;
    let mut files = crate::library_go::guest::upstream::generate(
        &super::super::world_text(resolve, world)?,
        &resolve.worlds[world].name,
        &module,
    )?;
    let bytes = files
        .remove("wit_exports/wit_exports.go")
        .context("upstream Go export glue")?;
    let mut glue = String::from_utf8(bytes)?.replacen("package wit_exports", "package main", 1);
    let calls = Regex::new(r"\b(export_[A-Za-z_0-9]+)\.([A-Za-z_][A-Za-z_0-9]*)\(")?;
    let mut imports = BTreeSet::new();
    let mut helpers = String::new();
    let mut adapters = String::new();
    let mut converted = BTreeSet::new();
    for (key, item) in &resolve.worlds[world].exports {
        let exports = match item {
            WorldItem::Interface { id, .. } => resolve.interfaces[*id]
                .functions
                .values()
                .collect::<Vec<_>>(),
            WorldItem::Function(f) => vec![f],
            WorldItem::Type { .. } => continue,
        };
        for f in exports {
            let function = functions
                .get(&f.name)
                .with_context(|| format!("ordinary Go export {} is missing", f.name))?;
            ensure!(
                function.source_params.len() == f.params.len(),
                "inferred Go parameter count differs"
            );
            let export = if matches!(item, WorldItem::Function(_)) {
                f.name.clone()
            } else {
                format!("{}#{}", resolve.name_world_key(key), f.name)
            };
            let marker = format!("//go:wasmexport {export}\n");
            let start = glue
                .find(&marker)
                .with_context(|| format!("missing upstream Go export {export}"))?;
            let body = &glue[start + marker.len()..];
            let end = body.find("//go:wasmexport ").unwrap_or(body.len());
            let call = calls
                .captures_iter(&body[..end])
                .find(|c| c[2] == f.name.to_upper_camel_case())
                .context("upstream Go implementation call")?;
            let export_package = call[1].to_owned();
            let type_package = export_package
                .strip_prefix("export_")
                .context("upstream Go export package prefix")?;
            let mut conversions = Conversions {
                module: &module,
                package: type_package,
                imports: BTreeSet::new(),
                helpers: BTreeMap::new(),
                code: String::new(),
            };
            // Helper names must be unique across source functions as well as
            // nested types. The generated source never calls a host compiler.
            let adapter = format!("__wisp_source_adapter_{}", converted.len());
            let code = conversions.adapter(function, &adapter)?;
            let prefix = format!("__wisp_fn{}_value_", converted.len());
            helpers.push_str(&conversions.code.replace("__wisp_value_", &prefix));
            adapters.push_str(&code.replace("__wisp_value_", &prefix));
            imports.extend(conversions.imports);
            glue = glue.replace(
                &format!("{export_package}.{}(", f.name.to_upper_camel_case()),
                &format!("{adapter}("),
            );
            converted.insert(f.name.clone());
        }
    }
    ensure!(
        converted.len() == functions.len(),
        "inferred Go functions differ from selected exports"
    );
    glue = glue
        .lines()
        .filter(|line| !line.contains(&format!("\"{module}/export_")))
        .collect::<Vec<_>>()
        .join("\n");
    for (alias, path) in imports {
        if !glue.contains(&format!("\"{path}\"")) {
            glue = glue.replacen("import (", &format!("import (\n{alias} \"{path}\""), 1);
        }
    }
    for name in ["staticPinner", "exportReturnArea", "syncExportPinner"] {
        glue = Regex::new(&format!(r"\b{name}\b"))?
            .replace_all(&glue, format!("__wisp_upstream_{name}"))
            .into_owned();
    }
    glue.push_str(&format!("\n{helpers}\n{adapters}"));
    super::super::write(source.join("wisp_bindings.go"), glue)?;
    for (name, bytes) in files {
        ensure!(
            !name.starts_with("export_"),
            "unexpected upstream Go implementation stubs"
        );
        super::super::write(source.join(name), bytes)?;
    }
    // An existing main remains application source. Library-style source files
    // may omit it, in which case supply only the required empty Go entry point.
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_go::LANGUAGE.into())?;
    let mut has_main = false;
    for entry in fs::read_dir(source)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "go")
            && path.file_name().is_some_and(|n| n != "wisp_bindings.go")
        {
            let text = fs::read_to_string(&path)?;
            if !crate::build_go::selected(&path, &text)? {
                continue;
            }
            let tree = parser
                .parse(&text, None)
                .context("parse Go main declaration")?;
            has_main |= tree
                .root_node()
                .named_children(&mut tree.root_node().walk())
                .any(|node| {
                    node.kind() == "function_declaration"
                        && node
                            .child_by_field_name("name")
                            .is_some_and(|name| &text[name.byte_range()] == "main")
                });
        }
    }
    if !has_main {
        super::super::write(
            source.join("wisp_main.go"),
            "package main\nfunc main() {}\n",
        )?;
    }
    Ok(())
}
