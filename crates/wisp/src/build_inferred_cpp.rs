//! Convert ordinary C++ values to upstream-generated C++ values, never ABI slots.
use super::{Function, SourceKind, SourceType};
use anyhow::{Context, Result, bail, ensure};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use wit_parser::{InterfaceId, Resolve, WorldId, WorldItem};

fn identifier(name: &str) -> String {
    let name = name.to_snake_case();
    const KEYWORDS: &str = "alignas alignof and and_eq asm auto bitand bitor bool break case catch char char8_t char16_t char32_t class compl concept const consteval constexpr constinit const_cast continue co_await co_return co_yield decltype default delete do double dynamic_cast else enum explicit export extern false float for friend goto if inline int long mutable namespace new noexcept not not_eq nullptr operator or or_eq private protected public register reinterpret_cast requires return short signed sizeof static static_assert static_cast struct switch template this thread_local throw true try typedef typeid typename union unsigned using virtual void volatile wchar_t while xor xor_eq";
    if KEYWORDS.split_whitespace().any(|keyword| keyword == name) {
        format!("{name}_")
    } else {
        name
    }
}

fn interface_scope(resolve: &Resolve, id: InterfaceId) -> Result<String> {
    let interface = &resolve.interfaces[id];
    let name = identifier(interface.name.as_deref().context("named C++ interface")?);
    if let Some(package) = interface.package {
        let package = &resolve.packages[package].name;
        Ok(format!(
            "{}::{}::{name}",
            identifier(&package.namespace),
            identifier(&package.name)
        ))
    } else {
        Ok(name)
    }
}

fn scalar(wire: &str) -> Result<&'static str> {
    Ok(match wire {
        "bool" => "bool",
        "u8" => "uint8_t",
        "s8" => "int8_t",
        "u16" => "uint16_t",
        "s16" => "int16_t",
        "u32" => "uint32_t",
        "s32" => "int32_t",
        "u64" => "uint64_t",
        "s64" => "int64_t",
        "f32" => "float",
        "f64" => "double",
        _ => bail!("unsupported inferred C++ scalar {wire}"),
    })
}

fn result_arguments(ty: &SourceType) -> (&str, &str) {
    let arguments = ty
        .name
        .split_once('<')
        .map(|(_, value)| value.strip_suffix('>').unwrap_or(value))
        .unwrap_or("");
    let mut depth = 0;
    for (i, ch) in arguments.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => return (arguments[..i].trim(), arguments[i + 1..].trim()),
            _ => {}
        }
    }
    ("void", "void")
}

fn source_type(ty: &SourceType) -> String {
    match &ty.kind {
        SourceKind::Scalar => ty.name.clone(),
        SourceKind::String | SourceKind::Bytes | SourceKind::Record(_) | SourceKind::Enum(_) => {
            format!("::{}", ty.name.trim_start_matches("::"))
        }
        SourceKind::List(element) => format!("::std::vector<{}>", source_type(element)),
        SourceKind::Option(element) => format!("::std::optional<{}>", source_type(element)),
        SourceKind::Result { ok, err } => {
            let (a, b) = result_arguments(ty);
            let empty = |name: &str| {
                if name.trim_start_matches("::") == "wisp::unit" {
                    "::wisp::unit"
                } else {
                    "void"
                }
            };
            format!(
                "::{}<{}, {}>",
                if ty
                    .name
                    .trim_start_matches("::")
                    .starts_with("std::expected<")
                {
                    "std::expected"
                } else {
                    "wisp::result"
                },
                ok.as_deref()
                    .map(source_type)
                    .unwrap_or_else(|| empty(a).into()),
                err.as_deref()
                    .map(source_type)
                    .unwrap_or_else(|| empty(b).into())
            )
        }
    }
}

fn wire_type(ty: &SourceType, scope: &str) -> Result<String> {
    Ok(match &ty.kind {
        SourceKind::Scalar => scalar(&ty.wire)?.into(),
        SourceKind::String => "::wit::string".into(),
        SourceKind::Bytes => "::wit::vector<uint8_t>".into(),
        SourceKind::List(element) => format!("::wit::vector<{}>", wire_type(element, scope)?),
        SourceKind::Option(element) => format!("::std::optional<{}>", wire_type(element, scope)?),
        SourceKind::Record(_) | SourceKind::Enum(_) => format!(
            "::{scope}::{}",
            ty.wire.trim_start_matches('%').to_upper_camel_case()
        ),
        SourceKind::Result { ok, err } => format!(
            "::std::expected<{}, {}>",
            ok.as_deref()
                .map(|t| wire_type(t, scope))
                .transpose()?
                .unwrap_or_else(|| "void".into()),
            err.as_deref()
                .map(|t| wire_type(t, scope))
                .transpose()?
                .unwrap_or_else(|| "::wit::Void".into())
        ),
    })
}

/// Every branch operates on C++ values owned by their respective runtimes.
/// In particular, no generated canonical allocation or post-return logic is replaced.
fn convert(ty: &SourceType, expression: &str, scope: &str, to_wire: bool) -> Result<String> {
    let source = source_type(ty);
    let wire = wire_type(ty, scope)?;
    let target = if to_wire { &wire } else { &source };
    let value = "__wisp_value";
    let body = match &ty.kind {
        SourceKind::Scalar => return Ok(format!("static_cast<{target}>({expression})")),
        SourceKind::String => {
            if to_wire {
                let view = if ty.name.trim_start_matches("::") == "std::string" {
                    format!("std::string_view({value}.data(), {value}.size())")
                } else {
                    format!("std::string_view({value}.data ? {value}.data : \"\", {value}.size)")
                };
                format!("return ::wit::string::from_view({view});")
            } else if ty.name.trim_start_matches("::") == "std::string" {
                format!("return {source}({value}.get_view());")
            } else {
                format!("return {source}::copy({value}.data(), {value}.size());")
            }
        }
        SourceKind::Bytes => {
            if to_wire {
                format!(
                    "auto result = {wire}::allocate({value}.size); for (size_t i=0; i<{value}.size; ++i) result.initialize(i, uint8_t({value}.data[i])); return result;"
                )
            } else {
                format!("return {source}::copy({value}.data(), {value}.size());")
            }
        }
        SourceKind::List(element) => {
            let entry = convert(element, &format!("{value}[i]"), scope, to_wire)?;
            if to_wire {
                format!(
                    "auto result = {wire}::allocate({value}.size()); for (size_t i=0; i<{value}.size(); ++i) result.initialize(i, {entry}); return result;"
                )
            } else {
                format!(
                    "{source} result; result.reserve({value}.size()); for (size_t i=0; i<{value}.size(); ++i) result.push_back({entry}); return result;"
                )
            }
        }
        SourceKind::Option(element) => {
            let entry = convert(element, &format!("*{value}"), scope, to_wire)?;
            format!(
                "if ({value}.has_value()) return {target}(std::in_place, {entry}); return std::nullopt;"
            )
        }
        SourceKind::Enum(cases) => {
            let mut body = format!("switch ({value}) {{");
            for case in cases {
                let native = format!("::{}", case.name.trim_start_matches("::"));
                let generated = format!("{wire}::k{}", case.wire.to_upper_camel_case());
                let (from, to) = if to_wire {
                    (native, generated)
                } else {
                    (generated, native)
                };
                body.push_str(&format!("case {from}: return {to};"));
            }
            body.push_str("} std::abort();");
            body
        }
        SourceKind::Record(fields) => {
            let values = fields
                .iter()
                .map(|field| {
                    let member = if to_wire {
                        field.name.clone()
                    } else {
                        identifier(&field.wire)
                    };
                    convert(&field.ty, &format!("{value}.{member}"), scope, to_wire)
                })
                .collect::<Result<Vec<_>>>()?;
            format!("return {target}{{ {} }};", values.join(", "))
        }
        SourceKind::Result { ok, err } => {
            let native_expected = ty
                .name
                .trim_start_matches("::")
                .starts_with("std::expected<");
            let input_expected = !to_wire || native_expected;
            let condition = if input_expected {
                format!("{value}.has_value()")
            } else {
                format!("!{value}.is_err")
            };
            let ok_value = if input_expected {
                format!("{value}.value()")
            } else {
                format!("{value}.ok")
            };
            let err_value = if input_expected {
                format!("{value}.error()")
            } else {
                format!("{value}.err")
            };
            let output_expected = to_wire || native_expected;
            let ok_expr = ok
                .as_deref()
                .map(|t| convert(t, &ok_value, scope, to_wire))
                .transpose()?;
            let err_expr = err
                .as_deref()
                .map(|t| convert(t, &err_value, scope, to_wire))
                .transpose()?;
            let success = if output_expected {
                ok_expr
                    .map(|value| format!("{target}(std::in_place, {value})"))
                    .unwrap_or_else(|| format!("{target}()"))
            } else {
                format!("{source}::success({})", ok_expr.unwrap_or_default())
            };
            let failure = if output_expected {
                let empty = if to_wire {
                    "::wit::Void{}"
                } else {
                    "::wisp::unit{}"
                };
                format!(
                    "{target}(std::unexpected({}))",
                    err_expr.unwrap_or_else(|| empty.into())
                )
            } else {
                format!("{source}::failure({})", err_expr.unwrap_or_default())
            };
            format!("if ({condition}) return {success}; return {failure};")
        }
    };
    Ok(format!(
        "([&](auto&& {value}) -> {target} {{ {body} }}(({expression})))"
    ))
}

fn wrapper(function: &Function, scope: &str, name: &str) -> Result<String> {
    let parameters = function
        .source_params
        .iter()
        .enumerate()
        .map(|(i, ty)| Ok(format!("{} a{i}", wire_type(ty, scope)?)))
        .collect::<Result<Vec<_>>>()?
        .join(", ");
    let result = function
        .source_result
        .as_ref()
        .map(|ty| wire_type(ty, scope))
        .transpose()?
        .unwrap_or_else(|| "void".into());
    let mut output = format!(
        "namespace {scope} {{\n{result} {}({parameters}) {{\n",
        name.to_upper_camel_case()
    );
    let mut arguments = Vec::new();
    for (i, ty) in function.source_params.iter().enumerate() {
        output.push_str(&format!(
            "auto source{i} = {};\n",
            convert(ty, &format!("a{i}"), scope, false)?
        ));
        arguments.push(format!("std::move(source{i})"));
    }
    let call = format!(
        "::{}({})",
        function.original.trim_start_matches("::"),
        arguments.join(", ")
    );
    if let Some(ty) = &function.source_result {
        output.push_str(&format!(
            "auto result = {call};\nreturn {};\n",
            convert(ty, "result", scope, true)?
        ));
    } else {
        output.push_str(&format!("{call};\n"));
    }
    output.push_str("}\n}\n");
    Ok(output)
}

pub(in crate::build) fn clang(
    resolve: &Resolve,
    world: WorldId,
    functions: &BTreeMap<String, Function>,
    sources: &[String],
    directory: &Path,
) -> Result<(String, Vec<String>)> {
    super::super::write(directory.join("wisp_types.hpp"), TYPES_HEADER)?;
    let (unit, objects) = super::super::upstream::clang(resolve, world, true, sources, directory)?;
    let mut output = format!(
        "#include <string>\n#include <vector>\n#include <optional>\n#include <expected>\n#include <cstdlib>\n#include \"wisp_types.hpp\"\n#define WISP_CPP_ROOT_IMPORTS ::{}\n{unit}",
        identifier(&resolve.worlds[world].name)
    );
    let mut seen = BTreeSet::new();
    for (_, item) in &resolve.worlds[world].exports {
        match item {
            WorldItem::Interface { id, .. } => {
                let scope = format!("exports::{}", interface_scope(resolve, *id)?);
                for (name, _) in &resolve.interfaces[*id].functions {
                    let function = functions
                        .get(name)
                        .with_context(|| format!("missing inferred C++ function {name}"))?;
                    ensure!(
                        seen.insert(name.clone()),
                        "duplicate inferred C++ export {name}"
                    );
                    output.push_str(&wrapper(function, &scope, name)?);
                }
            }
            WorldItem::Function(function) => {
                let source = functions
                    .get(&function.name)
                    .context("missing inferred root C++ function")?;
                seen.insert(function.name.clone());
                output.push_str(&wrapper(
                    source,
                    &format!("exports::{}", identifier(&resolve.worlds[world].name)),
                    &function.name,
                )?);
            }
            _ => {}
        }
    }
    ensure!(
        seen.len() == functions.len(),
        "inferred C++ functions differ from generated world"
    );
    Ok((output, objects))
}

pub(in crate::build) fn cpp_namespace(name: &str) -> Result<()> {
    ensure!(
        name.len() <= 64
            && name.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            && identifier(name) == name
            && !matches!(name, "std" | "wisp" | "exports"),
        "C++ dependency alias must be a non-keyword lowercase identifier: {name}"
    );
    Ok(())
}

/// The main upstream world generates the dependency's actual imports exactly
/// once. This header only supplies a convenient installed-package namespace.
pub(in crate::build) fn cpp_header(
    resolve: &Resolve,
    world: WorldId,
    namespace: &str,
) -> Result<String> {
    cpp_namespace(namespace)?;
    let flat = resolve.worlds[world]
        .exports
        .values()
        .all(|item| match item {
            WorldItem::Interface { id, .. } => {
                resolve.interfaces[*id].name.as_deref() == Some("api")
            }
            _ => true,
        });
    let mut output = format!(
        "// Wisp facade for upstream-generated C++ imports.\n#pragma once\n#include <utility>\nnamespace {namespace} {{\n"
    );
    let mut names = BTreeSet::new();
    for (_, item) in &resolve.worlds[world].exports {
        match item {
            WorldItem::Interface { id, .. } => {
                let interface = &resolve.interfaces[*id];
                let scope = format!("::{}", interface_scope(resolve, *id)?);
                let local_scope = if flat {
                    None
                } else {
                    Some(identifier(
                        interface
                            .name
                            .as_deref()
                            .context("dependency interface name")?,
                    ))
                };
                if let Some(local) = &local_scope {
                    output.push_str(&format!("namespace {local} {{\n"));
                }
                for (name, _) in &interface.types {
                    let name = name.to_upper_camel_case();
                    ensure!(
                        names.insert((local_scope.clone(), name.clone())),
                        "C++ dependency type name collision"
                    );
                    output.push_str(&format!("using {name} = {scope}::{name};\n"));
                }
                for (name, _) in &interface.functions {
                    let local = identifier(name);
                    ensure!(
                        names.insert((local_scope.clone(), local.clone())),
                        "C++ dependency function name collision"
                    );
                    output.push_str(&facade_function(&local, &scope, name));
                }
                if local_scope.is_some() {
                    output.push_str("}\n");
                }
            }
            WorldItem::Function(function) => output.push_str(&facade_function(
                &identifier(&function.name),
                "WISP_CPP_ROOT_IMPORTS",
                &function.name,
            )),
            _ => {}
        }
    }
    output.push_str("}\n");
    Ok(output)
}

fn facade_function(local: &str, scope: &str, wire: &str) -> String {
    format!(
        "template<class... Args> inline decltype(auto) {local}(Args&&... args) {{ return {scope}::{}(std::forward<Args>(args)...); }}\n",
        wire.to_upper_camel_case()
    )
}

// Compatibility source values only. Their storage uses the C++ standard
// library, with no canonical allocator, flat-memory layout, or import/export ABI.
const TYPES_HEADER: &str = r#"// Generated source-level Wisp value helpers.
#pragma once
#include <cstdint>
#include <memory>
#include <type_traits>
#include <utility>
#include <vector>
namespace wisp {
using size_type = std::size_t;
struct unit {};
template<class T> struct buffer {
  const T* data = nullptr;
  size_type size = 0;
  std::shared_ptr<std::vector<T>> _owner;
  buffer() = default;
  buffer(const T* p, size_type n) : data(p), size(n) {}
  template<std::size_t N> buffer(const T (&p)[N]) : data(p), size(N - 1) {}
  static buffer copy(const T* p, size_type n) {
    buffer value;
    value._owner = std::make_shared<std::vector<T>>();
    if (n) value._owner->assign(p, p + n);
    value.data = value._owner->data(); value.size = n;
    return value;
  }
};
using string = buffer<char>;
using bytes = buffer<unsigned char>;
template<class Ok, class Err> struct result {
  using ok_type = std::conditional_t<std::is_void_v<Ok>, unit, Ok>;
  using err_type = std::conditional_t<std::is_void_v<Err>, unit, Err>;
  bool is_err;
  ok_type ok;
  err_type err;
  static result success(ok_type value = {}) { return {false, std::move(value), {}}; }
  static result failure(err_type value = {}) { return {true, {}, std::move(value)}; }
};
}
"#;
