//! Scalar canonical ABI bindings for the Wasm-hosted compiler entry points.
use anyhow::{Result, bail, ensure};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::collections::BTreeSet;
use wit_parser::{Function, FunctionKind, Resolve, Type, TypeDefKind, WorldId, WorldItem};

#[derive(Clone, Debug)]
pub struct Scalar {
    pub c: &'static str,
    pub cpp: String,
    pub go: String,
    pub rust: String,
    pub abi_rust: &'static str,
    pub abi_go: &'static str,
    pub kind: ValueKind,
    pub generated: bool,
}
#[derive(Clone, Debug)]
pub enum ValueKind {
    Scalar,
    String,
    Bytes,
    Record {
        name: String,
        fields: Vec<ValueField>,
    },
    Result {
        ok: Option<Box<Scalar>>,
        err: Option<Box<Scalar>>,
    },
}
#[derive(Clone, Debug)]
pub struct ValueField {
    pub name: String,
    pub go_name: String,
    pub rust_name: String,
    pub ty: Scalar,
}
fn scalar(resolve: &Resolve, ty: Type, depth: usize) -> Result<Scalar> {
    ensure!(depth < 32, "build binding type depth limit");
    let (c, go, abi_go) = match ty {
        Type::Bool => ("_Bool", "bool", "int32"),
        Type::U8 => ("__UINT8_TYPE__", "uint8", "uint32"),
        Type::S8 => ("__INT8_TYPE__", "int8", "int32"),
        Type::U16 => ("__UINT16_TYPE__", "uint16", "uint32"),
        Type::S16 => ("__INT16_TYPE__", "int16", "int32"),
        Type::U32 => ("__UINT32_TYPE__", "uint32", "uint32"),
        Type::S32 => ("__INT32_TYPE__", "int32", "int32"),
        Type::U64 => ("__UINT64_TYPE__", "uint64", "uint64"),
        Type::S64 => ("__INT64_TYPE__", "int64", "int64"),
        Type::F32 => ("float", "float32", "float32"),
        Type::F64 => ("double", "float64", "float64"),
        Type::String => return Ok(crate::build_values::string()),
        Type::Id(id) => match &resolve.types[id].kind {
            TypeDefKind::Type(ty) => return scalar(resolve, *ty, depth + 1),
            TypeDefKind::List(Type::U8) => return Ok(crate::build_values::bytes()),
            TypeDefKind::Record(record) => {
                let name = resolve.types[id]
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("record-{}", id.index()));
                return Ok(crate::build_values::record(
                    name,
                    record
                        .fields
                        .iter()
                        .map(|field| {
                            Ok(ValueField {
                                name: field.name.to_snake_case(),
                                go_name: field.name.to_upper_camel_case(),
                                rust_name: field.name.to_snake_case(),
                                ty: scalar(resolve, field.ty, depth + 1)?,
                            })
                        })
                        .collect::<Result<_>>()?,
                ));
            }
            TypeDefKind::Result(result) => {
                return Ok(crate::build_values::result(
                    result
                        .ok
                        .map(|ty| scalar(resolve, ty, depth + 1))
                        .transpose()?,
                    result
                        .err
                        .map(|ty| scalar(resolve, ty, depth + 1))
                        .transpose()?,
                ));
            }
            _ => bail!(
                "unsupported-build-binding-type: build currently supports scalar types and aliases; records, strings, lists, resources and async values require compiler bindings that are not implemented"
            ),
        },
        _ => bail!("unsupported-build-binding-type: {ty:?}"),
    };
    Ok(Scalar {
        c,
        cpp: c.into(),
        rust: match go {
            "bool" => "bool",
            "uint8" => "u8",
            "uint16" => "u16",
            "uint32" => "u32",
            "uint64" => "u64",
            "int8" => "i8",
            "int16" => "i16",
            "int32" => "i32",
            "int64" => "i64",
            "float32" => "f32",
            "float64" => "f64",
            _ => unreachable!(),
        }
        .into(),
        abi_rust: match abi_go {
            "int32" => "i32",
            "uint32" => "u32",
            "int64" => "i64",
            "uint64" => "u64",
            "float32" => "f32",
            "float64" => "f64",
            _ => unreachable!(),
        },
        go: go.into(),
        abi_go,
        kind: ValueKind::Scalar,
        generated: false,
    })
}
#[derive(Clone, Debug)]
pub struct Binding {
    pub import: bool,
    pub module: Option<String>,
    pub name: String,
    pub c_name: String,
    pub go_name: String,
    pub rust_name: String,
    pub params: Vec<Scalar>,
    pub result: Option<Scalar>,
    pub go_error_result: bool,
}
pub fn bindings(resolve: &Resolve, world: WorldId) -> Result<Vec<Binding>> {
    let world = &resolve.worlds[world];
    ensure!(world.exports.len() <= 64, "build export count limit");
    let mut result = Vec::new();
    let mut names = BTreeSet::new();
    for (import, items) in [(true, &world.imports), (false, &world.exports)] {
        for (key, item) in items {
            let module = resolve.name_world_key(key);
            let (interface_name, functions): (Option<&str>, Vec<&Function>) = match item {
                WorldItem::Function(function) => (None, vec![function]),
                WorldItem::Interface { id, .. } => {
                    let interface = &resolve.interfaces[*id];
                    (
                        Some(interface.name.as_deref().unwrap_or(&module)),
                        interface.functions.values().collect(),
                    )
                }
                WorldItem::Type { .. } => continue,
            };
            ensure!(functions.len() <= 256, "build function count limit");
            for function in functions {
                ensure!(
                    matches!(function.kind, FunctionKind::Freestanding),
                    "unsupported-build-function: only synchronous freestanding functions are supported"
                );
                ensure!(
                    function.params.len() <= 16,
                    "build scalar parameter limit is 16"
                );
                let base = format!(
                    "{}{}",
                    interface_name.map(|n| format!("{n}_")).unwrap_or_default(),
                    function.name
                );
                let c_name = format!(
                    "{}_{}",
                    if import { "imports" } else { "exports" },
                    base.to_snake_case()
                );
                let go_name = format!(
                    "{}{}",
                    if import { "Import" } else { "Export" },
                    base.to_upper_camel_case()
                );
                ensure!(
                    names.insert(c_name.clone()),
                    "build binding name collision: {c_name}"
                );
                result.push(Binding {
                    import,
                    module: interface_name.map(|_| module.clone()),
                    name: function.name.clone(),
                    rust_name: c_name.clone(),
                    c_name,
                    go_name,
                    params: function
                        .params
                        .iter()
                        .map(|p| scalar(resolve, p.ty, 0))
                        .collect::<Result<_>>()?,
                    result: function
                        .result
                        .map(|ty| scalar(resolve, ty, 0))
                        .transpose()?,
                    go_error_result: false,
                });
            }
        }
    }
    ensure!(
        result.iter().any(|b| !b.import),
        "build unit must export a function"
    );
    fn check_symbols(
        ty: &Scalar,
        names: &mut std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        match &ty.kind {
            ValueKind::Record { name, fields } => {
                let symbol = format!("Wisp{}", name.to_upper_camel_case());
                let shape = format!("{fields:?}");
                if let Some(previous) = names.insert(symbol.clone(), shape.clone()) {
                    ensure!(
                        previous == shape,
                        "unsupported-build-binding-type: different records normalize to {symbol}; use distinct type names"
                    );
                }
                for field in fields {
                    check_symbols(&field.ty, names)?;
                }
            }
            ValueKind::Result { ok, err } => {
                for ty in ok.iter().chain(err.iter()) {
                    check_symbols(ty, names)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut symbols = std::collections::BTreeMap::new();
    for ty in result
        .iter()
        .flat_map(|binding| binding.params.iter().chain(binding.result.iter()))
    {
        check_symbols(ty, &mut symbols)?;
    }
    Ok(result)
}
pub fn c(bindings: &[Binding], cpp: bool, sources: &[String]) -> (String, String) {
    if crate::build_values::needed(bindings) {
        return crate::build_values::c(bindings, cpp, sources);
    }
    let mut header = String::from(
        "#pragma once\n#ifdef __cplusplus\nextern \"C\" {\n#define WISP_BOOL bool\n#else\n#define WISP_BOOL _Bool\n#endif\n",
    );
    let mut glue = String::from("#include \"wisp.h\"\n");
    for source in sources {
        glue.push_str(&format!("#include {source:?}\n"));
    }
    for (i, binding) in bindings.iter().enumerate() {
        let ty = |scalar: &Scalar| {
            if scalar.go == "bool" {
                "WISP_BOOL"
            } else {
                scalar.c
            }
        };
        let result = binding.result.as_ref().map(ty).unwrap_or("void");
        let params = binding
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| format!("{} a{i}", ty(p)))
            .collect::<Vec<_>>()
            .join(", ");
        let params = if params.is_empty() {
            "void".into()
        } else {
            params
        };
        let args = (0..binding.params.len())
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        if binding.import {
            header.push_str(&format!(
                "__attribute__((import_module({:?}), import_name({:?}))) {result} {}({params});\n",
                binding.module.as_deref().unwrap_or("$root"),
                binding.name,
                binding.c_name
            ));
        } else {
            header.push_str(&format!("{result} {}({params});\n", binding.c_name));
            let export = binding
                .module
                .as_ref()
                .map(|m| format!("{m}#{}", binding.name))
                .unwrap_or(binding.name.clone());
            glue.push_str(&format!("{}__attribute__((export_name({export:?}))) {result} __wisp_export_{i}({params}) {{ {}{}({args}); }}\n", if cpp { "extern \"C\" " } else { "" }, if binding.result.is_some() { "return " } else { "" }, binding.c_name));
        }
    }
    header.push_str("#ifdef __cplusplus\n}\n#endif\n");
    (header, glue)
}
pub(super) fn cpp_identifier(name: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "alignas",
        "alignof",
        "and",
        "and_eq",
        "asm",
        "auto",
        "bitand",
        "bitor",
        "bool",
        "break",
        "case",
        "catch",
        "char",
        "char8_t",
        "char16_t",
        "char32_t",
        "class",
        "compl",
        "concept",
        "const",
        "consteval",
        "constexpr",
        "constinit",
        "const_cast",
        "continue",
        "co_await",
        "co_return",
        "co_yield",
        "decltype",
        "default",
        "delete",
        "do",
        "double",
        "dynamic_cast",
        "else",
        "enum",
        "explicit",
        "export",
        "extern",
        "false",
        "float",
        "for",
        "friend",
        "goto",
        "if",
        "inline",
        "int",
        "long",
        "mutable",
        "namespace",
        "new",
        "noexcept",
        "not",
        "not_eq",
        "nullptr",
        "operator",
        "or",
        "or_eq",
        "private",
        "protected",
        "public",
        "register",
        "reinterpret_cast",
        "requires",
        "return",
        "short",
        "signed",
        "sizeof",
        "static",
        "static_assert",
        "static_cast",
        "struct",
        "switch",
        "template",
        "this",
        "thread_local",
        "throw",
        "true",
        "try",
        "typedef",
        "typeid",
        "typename",
        "union",
        "unsigned",
        "using",
        "virtual",
        "void",
        "volatile",
        "wchar_t",
        "while",
        "xor",
        "xor_eq",
    ];
    if KEYWORDS.contains(&name) || name == "detail" {
        format!("{name}_")
    } else {
        name.to_owned()
    }
}
pub(super) fn cpp_namespace(name: &str) -> Result<()> {
    ensure!(
        name.len() <= 64
            && name.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            && cpp_identifier(name) == name
            && !matches!(name, "std" | "wisp"),
        "C++ dependency alias must be a non-keyword lowercase identifier: {name}"
    );
    Ok(())
}

/// A normal C++ include for an installed Component, with private canonical imports.
pub fn cpp_header(resolve: &Resolve, world: WorldId, namespace: &str) -> Result<String> {
    cpp_namespace(namespace)?;
    let mut exports = resolve.clone();
    exports.worlds[world].imports.clear();
    let bindings = bindings(&exports, world)?;
    if crate::build_values::needed(&bindings) {
        return crate::build_values::cpp_header(&bindings, namespace);
    }
    let flat = resolve.worlds[world]
        .exports
        .iter()
        .all(|(_, item)| match item {
            WorldItem::Interface { id, .. } => {
                resolve.interfaces[*id].name.as_deref() == Some("api")
            }
            _ => true,
        });
    let mut header = format!(
        "// Code generated by wisp build. DO NOT EDIT.\n#pragma once\nnamespace {namespace} {{\n"
    );
    let mut names = BTreeSet::new();
    for (index, binding) in bindings.iter().enumerate() {
        let interface = binding
            .module
            .as_ref()
            .and_then(|module| module.split('/').nth(1))
            .map(|name| name.split('@').next().unwrap().to_snake_case());
        let scope = if flat { None } else { interface };
        let name = binding.name.to_snake_case();
        let name = cpp_identifier(&name);
        let scope = scope.map(|name| cpp_identifier(&name));
        ensure!(
            names.insert((scope.clone(), name.clone())),
            "C++ dependency binding name collision"
        );
        if let Some(scope) = &scope {
            header.push_str(&format!("namespace {scope} {{\n"));
        }
        let ty = |p: &Scalar| if p.c == "_Bool" { "bool" } else { p.c };
        let result = binding.result.as_ref().map(ty).unwrap_or("void");
        let params = binding
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| format!("{} a{i}", ty(p)))
            .collect::<Vec<_>>()
            .join(", ");
        let args = (0..binding.params.len())
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let symbol = format!("wisp_import_{namespace}_{index}");
        header.push_str(&format!("namespace detail {{\nextern \"C\" __attribute__((import_module({:?}), import_name({:?}))) {result} {symbol}({params});\n}}\ninline {result} {name}({params}) {{ {}detail::{symbol}({args}); }}\n", binding.module.as_deref().unwrap_or("$root"), binding.name, if binding.result.is_some() { "return " } else { "" }));
        if scope.is_some() {
            header.push_str("}\n");
        }
    }
    header.push_str("}\n");
    Ok(header)
}

fn go_convert(scalar: &Scalar, expression: &str, lower: bool) -> String {
    if scalar.c == "_Bool" {
        if lower {
            if scalar.go == "bool" {
                format!("wispBool({expression})")
            } else {
                format!("wispBool(bool({expression}))")
            }
        } else {
            if scalar.go == "bool" {
                format!("({expression} != 0)")
            } else {
                format!("{}({expression} != 0)", scalar.go)
            }
        }
    } else {
        format!(
            "{}({expression})",
            if lower {
                scalar.abi_go
            } else {
                scalar.go.as_str()
            }
        )
    }
}
pub fn go(bindings: &[Binding]) -> String {
    if crate::build_values::needed(bindings) {
        return crate::build_values::go(bindings);
    }
    let mut source = String::from(
        "// Code generated by wisp build. DO NOT EDIT.\npackage main\nimport \"unsafe\"\nfunc wispBool(value bool) int32 { if value { return 1 }; return 0 }\n",
    );
    source.push_str(&crate::build_go_memory::source());
    for (i, binding) in bindings.iter().enumerate() {
        let signature = |abi: bool| {
            binding
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| format!("a{i} {}", if abi { p.abi_go } else { p.go.as_str() }))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let result = |abi: bool| {
            binding
                .result
                .as_ref()
                .map(|p| format!(" {}", if abi { p.abi_go } else { p.go.as_str() }))
                .unwrap_or_default()
        };
        let args = |lower: bool| {
            binding
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| go_convert(p, &format!("a{i}"), lower))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if binding.import {
            source.push_str(&format!(
                "//go:wasmimport {} {}\nfunc wispImport{i}({}){}\n",
                binding.module.as_deref().unwrap_or("$root"),
                binding.name,
                signature(true),
                result(true)
            ));
            let call = format!("wispImport{i}({})", args(true));
            let body = binding
                .result
                .as_ref()
                .map(|p| format!("return {}", go_convert(p, &call, false)))
                .unwrap_or(call);
            source.push_str(&format!(
                "func {}({}){} {{ {body} }}\n",
                binding.go_name,
                signature(false),
                result(false)
            ));
        } else {
            let export = binding
                .module
                .as_ref()
                .map(|m| format!("{m}#{}", binding.name))
                .unwrap_or(binding.name.clone());
            let call = format!("{}({})", binding.go_name, args(false));
            let body = binding
                .result
                .as_ref()
                .map(|p| format!("return {}", go_convert(p, &call, true)))
                .unwrap_or(call);
            source.push_str(&format!(
                "//go:wasmexport {export}\nfunc wispExport{i}({}){} {{ {body} }}\n",
                signature(true),
                result(true)
            ));
        }
    }
    source
}

#[cfg(test)]
mod cpp_tests {
    use super::*;
    #[test]
    fn dependency_headers_skip_runtime_imports_and_preserve_canonical_names() {
        let mut resolve = Resolve::default();
        let package = resolve.push_str("test.wit", "package test:dependency@1.2.3; interface runtime { log: func(value: string); } interface api { add: func(a:u32,b:u32)->u32; negate:func(value:bool)->bool; widen:func(value:u64)->u64; %switch:func()->u32; } world library { import runtime; export api; }").unwrap();
        let world = resolve.select_world(&[package], Some("library")).unwrap();
        let header = cpp_header(&resolve, world, "arithmetic").unwrap();
        assert!(header.contains("namespace arithmetic"));
        assert!(!header.contains("namespace api"));
        assert!(!header.contains("runtime"));
        assert!(header.contains("import_module(\"test:dependency/api@1.2.3\")"));
        assert!(header.contains("inline bool negate(bool a0)"));
        assert!(header.contains("inline __UINT64_TYPE__ widen(__UINT64_TYPE__ a0)"));
        assert!(header.contains("inline __UINT32_TYPE__ switch_()"));
        let another = cpp_header(&resolve, world, "other").unwrap();
        assert!(another.contains("wisp_import_other_0"));
        assert!(!another.contains("wisp_import_arithmetic_0"));
        assert!(cpp_header(&resolve, world, "namespace").is_err());
        assert!(cpp_header(&resolve, world, "a/b").is_err());
        assert!(cpp_header(&resolve, world, "wisp").is_err());
    }
}

/// Private Rust crate wrapper. The application keeps its ordinary library API.
pub fn rust(bindings: &[Binding], library: &str) -> Result<String> {
    if crate::build_values::needed(bindings) {
        return crate::build_values::rust(bindings, library);
    }
    ensure!(
        !bindings.iter().any(|b| b.import),
        "Rust authored import bindings require installed guest crates"
    );
    let mut source = format!(
        "// Code generated by wisp build. DO NOT EDIT.\nextern crate {library} as __wisp_application;\n"
    );
    source.push_str("#[export_name = \"cabi_realloc\"]\npub unsafe extern \"C\" fn __wisp_realloc(old: *mut u8, old_size: usize, align: usize, new_size: usize) -> *mut u8 {\n    if new_size == 0 { if old_size != 0 { std::alloc::dealloc(old, std::alloc::Layout::from_size_align_unchecked(old_size, align)); } return align as *mut u8; }\n    let layout = std::alloc::Layout::from_size_align_unchecked(old_size.max(1), align);\n    let result = if old_size == 0 { std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(new_size, align)) } else { std::alloc::realloc(old, layout, new_size) };\n    if result.is_null() { std::alloc::handle_alloc_error(std::alloc::Layout::from_size_align_unchecked(new_size, align)); }\n    result\n}\n");
    for (index, binding) in bindings.iter().enumerate() {
        let export = binding
            .module
            .as_ref()
            .map(|m| format!("{m}#{}", binding.name))
            .unwrap_or_else(|| binding.name.clone());
        let parameters = binding
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| format!("a{i}: {}", p.abi_rust))
            .collect::<Vec<_>>()
            .join(", ");
        let arguments = binding
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if p.rust == "bool" {
                    format!("a{i} != 0")
                } else {
                    format!("a{i} as {}", p.rust)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let call = format!("__wisp_application::{}({arguments})", binding.rust_name);
        let (result, body) = if let Some(result) = &binding.result {
            (
                format!(" -> {}", result.abi_rust),
                format!("({call}) as {}", result.abi_rust),
            )
        } else {
            (String::new(), format!("{call};"))
        };
        source.push_str(&format!("#[export_name = {export:?}]\npub extern \"C\" fn __wisp_export_{index}({parameters}){result} {{ {body} }}\n"));
    }
    Ok(source)
}

#[cfg(test)]
mod rust_tests {
    use super::*;
    #[test]
    fn rust_exports_use_scalar_canonical_abi_and_original_functions() {
        let mut resolve = Resolve::default();
        let package = resolve.push_str("test.wit","package test:rust; interface api { negate: func(value:bool)->bool; widen: func(value:u64)->u64; narrow: func(value:u8)->s8; } world library { export api; }").unwrap();
        let world = resolve.select_world(&[package], Some("library")).unwrap();
        let mut bindings = bindings(&resolve, world).unwrap();
        for binding in &mut bindings {
            binding.rust_name = format!("math::{}", binding.name);
        }
        let source = rust(&bindings, "application").unwrap();
        assert!(source.contains("extern crate application as __wisp_application"));
        assert!(source.contains("__wisp_application::math::negate(a0 != 0)"));
        assert!(source.contains("a0: u64"));
        assert!(source.contains("__wisp_application::math::narrow(a0 as u8)"));
        assert!(source.contains("as i32"));
        assert!(source.contains("test:rust/api#negate"));
        assert!(source.contains("cabi_realloc"));
    }
}
