//! Owned synchronous values for source compilers. Layout follows the canonical wasm32 ABI.
use crate::build_bindings::{Binding, Scalar, ValueField, ValueKind};
use anyhow::{Result, ensure};
use heck::{ToSnakeCase, ToUpperCamelCase};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
#[path = "build_values_c.rs"]
mod cpp;
pub use cpp::{c, cpp_header, types_header};

fn value(kind: ValueKind, go: String, rust: String, cpp: String) -> Scalar {
    Scalar {
        c: "void",
        cpp,
        go,
        rust,
        abi_rust: "u32",
        abi_go: "uint32",
        kind,
        generated: true,
    }
}
pub fn string() -> Scalar {
    value(
        ValueKind::String,
        "string".into(),
        "String".into(),
        "wisp::string".into(),
    )
}
pub fn bytes() -> Scalar {
    value(
        ValueKind::Bytes,
        "[]byte".into(),
        "Vec<u8>".into(),
        "wisp::bytes".into(),
    )
}
pub fn record(name: String, fields: Vec<ValueField>) -> Scalar {
    let symbol = format!("Wisp{}", name.to_upper_camel_case());
    value(
        ValueKind::Record { name, fields },
        symbol.clone(),
        symbol.clone(),
        symbol,
    )
}
pub fn result(ok: Option<Scalar>, err: Option<Scalar>) -> Scalar {
    let rust = format!(
        "Result<{}, {}>",
        ok.as_ref().map(|t| t.rust.as_str()).unwrap_or("()"),
        err.as_ref().map(|t| t.rust.as_str()).unwrap_or("()")
    );
    let cpp = format!(
        "wisp::result<{}, {}>",
        ok.as_ref().map(|t| t.cpp.as_str()).unwrap_or("wisp::unit"),
        err.as_ref().map(|t| t.cpp.as_str()).unwrap_or("wisp::unit")
    );
    value(
        ValueKind::Result {
            ok: ok.map(Box::new),
            err: err.map(Box::new),
        },
        "".into(),
        rust,
        cpp,
    )
}
pub fn needed(bindings: &[Binding]) -> bool {
    bindings.iter().any(|b| {
        b.params
            .iter()
            .chain(b.result.iter())
            .any(|t| !matches!(t.kind, ValueKind::Scalar))
    })
}
pub fn go_post_returns(bindings: &[Binding]) -> Vec<crate::build_go_memory::PostReturn> {
    use crate::build_go_memory::{Cleanup, PostReturn};
    fn cleanup(ty: &Scalar, offset: usize) -> Vec<Cleanup> {
        match &ty.kind {
            ValueKind::String | ValueKind::Bytes => vec![Cleanup::Buffer {
                offset: offset as u32,
            }],
            ValueKind::Record { fields, .. } => fields
                .iter()
                .zip(record_layout(fields.iter().map(|field| &field.ty)).1)
                .flat_map(|(field, field_offset)| cleanup(&field.ty, offset + field_offset))
                .collect(),
            ValueKind::Result { ok, err } => {
                let payload = offset + align(1, layout(ty).1);
                vec![Cleanup::Variant {
                    offset: offset as u32,
                    ok: ok
                        .as_ref()
                        .map(|ty| cleanup(ty, payload))
                        .unwrap_or_default(),
                    err: err
                        .as_ref()
                        .map(|ty| cleanup(ty, payload))
                        .unwrap_or_default(),
                }]
            }
            ValueKind::Scalar => Vec::new(),
        }
    }
    bindings
        .iter()
        .filter(|binding| !binding.import)
        .filter_map(|binding| {
            let ty = binding.result.as_ref()?;
            if flat(ty).len() <= 1 {
                return None;
            }
            let (size, alignment) = layout(ty);
            Some(PostReturn {
                export: export_name(binding),
                size: size as u32,
                alignment: alignment as u32,
                cleanup: cleanup(ty, 0),
            })
        })
        .collect()
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Abi {
    I32,
    I64,
    F32,
    F64,
}
impl Abi {
    fn rust(self) -> &'static str {
        match self {
            Self::I32 => "u32",
            Self::I64 => "u64",
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }
    fn go(self) -> &'static str {
        match self {
            Self::I32 => "uint32",
            Self::I64 => "uint64",
            Self::F32 => "float32",
            Self::F64 => "float64",
        }
    }
    fn c(self) -> &'static str {
        match self {
            Self::I32 => "__UINT32_TYPE__",
            Self::I64 => "__UINT64_TYPE__",
            Self::F32 => "float",
            Self::F64 => "double",
        }
    }
}
fn primitive(t: &Scalar) -> &'static str {
    match t.c {
        "_Bool" | "__UINT8_TYPE__" => "u8",
        "__INT8_TYPE__" => "i8",
        "__UINT16_TYPE__" => "u16",
        "__INT16_TYPE__" => "i16",
        "__UINT32_TYPE__" => "u32",
        "__INT32_TYPE__" => "i32",
        "__UINT64_TYPE__" => "u64",
        "__INT64_TYPE__" => "i64",
        "float" => "f32",
        "double" => "f64",
        _ => unreachable!("non scalar primitive"),
    }
}
fn align(n: usize, a: usize) -> usize {
    (n + a - 1) & !(a - 1)
}
fn layout(t: &Scalar) -> (usize, usize) {
    match &t.kind {
        ValueKind::Scalar => {
            let n = match primitive(t) {
                "u8" | "i8" => 1,
                "u16" | "i16" => 2,
                "u32" | "i32" | "f32" => 4,
                _ => 8,
            };
            (n, n)
        }
        ValueKind::String | ValueKind::Bytes => (8, 4),
        ValueKind::Record { fields, .. } => record_layout(fields.iter().map(|f| &f.ty)).0,
        ValueKind::Result { ok, err } => {
            let types = ok
                .iter()
                .chain(err.iter())
                .map(|v| layout(v))
                .collect::<Vec<_>>();
            let a = types.iter().map(|t| t.1).max().unwrap_or(1);
            let n = types.iter().map(|t| t.0).max().unwrap_or(0);
            (align(align(1, a) + n, a), a)
        }
    }
}
fn record_layout<'a>(types: impl Iterator<Item = &'a Scalar>) -> ((usize, usize), Vec<usize>) {
    let (mut size, mut alignment) = (0, 1);
    let mut offsets = Vec::new();
    for ty in types {
        let (n, a) = layout(ty);
        alignment = alignment.max(a);
        size = align(size, a);
        offsets.push(size);
        size += n;
    }
    ((align(size, alignment), alignment), offsets)
}
fn flat(t: &Scalar) -> Vec<Abi> {
    match &t.kind {
        ValueKind::Scalar => vec![match primitive(t) {
            "i64" | "u64" => Abi::I64,
            "f32" => Abi::F32,
            "f64" => Abi::F64,
            _ => Abi::I32,
        }],
        ValueKind::String | ValueKind::Bytes => vec![Abi::I32; 2],
        ValueKind::Record { fields, .. } => fields.iter().flat_map(|f| flat(&f.ty)).collect(),
        ValueKind::Result { ok, err } => {
            let a = ok.as_ref().map(|t| flat(t)).unwrap_or_default();
            let b = err.as_ref().map(|t| flat(t)).unwrap_or_default();
            let mut result = vec![Abi::I32];
            for i in 0..a.len().max(b.len()) {
                result.push(match (a.get(i), b.get(i)) {
                    (Some(a), Some(b)) if a == b => *a,
                    (Some(Abi::I32), Some(Abi::F32)) | (Some(Abi::F32), Some(Abi::I32)) => Abi::I32,
                    (Some(a), None) | (None, Some(a)) => *a,
                    _ => Abi::I64,
                });
            }
            result
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Lang {
    Rust,
    Go,
    C,
}
struct Gen {
    lang: Lang,
    next: usize,
    code: String,
}
impl Gen {
    fn new(lang: Lang) -> Self {
        Self {
            lang,
            next: 0,
            code: String::new(),
        }
    }
    fn line(&mut self, s: impl AsRef<str>) {
        self.code.push_str(s.as_ref());
        self.code.push('\n');
    }
    fn condition(&mut self, expression: &str) {
        self.line(if self.lang == Lang::C {
            format!("if ({expression}) {{")
        } else {
            format!("if {expression} {{")
        });
    }
    fn local(&mut self, expression: &str) -> String {
        let name = format!("v{}", self.next);
        self.next += 1;
        match self.lang {
            Lang::Rust => self.line(format!("let {name} = {expression};")),
            Lang::Go => self.line(format!("{name} := {expression}")),
            Lang::C => self.line(format!("auto {name} = {expression};")),
        };
        name
    }
    fn cast(&self, expr: &str, ty: &str) -> String {
        match self.lang {
            Lang::Rust => format!("({expr}) as {ty}"),
            Lang::Go => format!("{ty}({expr})"),
            Lang::C => format!("({ty})({expr})"),
        }
    }
    fn abi_name(&self, a: Abi) -> &'static str {
        match self.lang {
            Lang::Rust => a.rust(),
            Lang::Go => a.go(),
            Lang::C => a.c(),
        }
    }
    fn raw_type(&self, ty: &str) -> &'static str {
        match (self.lang, ty) {
            (Lang::Rust, "u8") => "u8",
            (Lang::Rust, "i8") => "i8",
            (Lang::Rust, "u16") => "u16",
            (Lang::Rust, "i16") => "i16",
            (Lang::Rust, "u32") => "u32",
            (Lang::Rust, "i32") => "i32",
            (Lang::Rust, "u64") => "u64",
            (Lang::Rust, "i64") => "i64",
            (Lang::Rust, "f32") => "f32",
            (Lang::Rust, "f64") => "f64",
            (Lang::Go, "u8") => "uint8",
            (Lang::Go, "i8") => "int8",
            (Lang::Go, "u16") => "uint16",
            (Lang::Go, "i16") => "int16",
            (Lang::Go, "u32") => "uint32",
            (Lang::Go, "i32") => "int32",
            (Lang::Go, "u64") => "uint64",
            (Lang::Go, "i64") => "int64",
            (Lang::Go, "f32") => "float32",
            (Lang::Go, "f64") => "float64",
            (Lang::C, "u8") => "__UINT8_TYPE__",
            (Lang::C, "i8") => "__INT8_TYPE__",
            (Lang::C, "u16") => "__UINT16_TYPE__",
            (Lang::C, "i16") => "__INT16_TYPE__",
            (Lang::C, "u32") => "__UINT32_TYPE__",
            (Lang::C, "i32") => "__INT32_TYPE__",
            (Lang::C, "u64") => "__UINT64_TYPE__",
            (Lang::C, "i64") => "__INT64_TYPE__",
            (Lang::C, "f32") => "float",
            (Lang::C, "f64") => "double",
            _ => unreachable!(),
        }
    }
    fn load(&self, p: &str, ty: &str) -> String {
        let name = self.raw_type(ty);
        match self.lang {
            Lang::Rust => format!("__wisp_read::<{name}>({p})"),
            Lang::Go => format!("*(*{name})(unsafe.Pointer(uintptr({p})))"),
            Lang::C => format!("*({name}*)(__UINTPTR_TYPE__)({p})"),
        }
    }
    fn store(&mut self, p: &str, ty: &str, expr: &str) {
        let name = self.raw_type(ty);
        let expr = self.cast(expr, name);
        self.line(match self.lang {
            Lang::Rust => format!("__wisp_write::<{name}>({p}, {expr});"),
            Lang::Go => format!("*(*{name})(unsafe.Pointer(uintptr({p}))) = {expr}"),
            Lang::C => format!("*({name}*)(__UINTPTR_TYPE__)({p}) = {expr};"),
        });
    }
    fn alloc(&mut self, n: usize, a: usize) -> String {
        self.local(&format!(
            "{}({n}, {a})",
            match self.lang {
                Lang::Rust => "__wisp_alloc",
                Lang::Go => "wispAlloc",
                Lang::C => "wisp_alloc",
            }
        ))
    }
    fn free(&mut self, p: &str, n: usize, a: usize) {
        self.line(format!(
            "{}({p}, {n}, {a});",
            match self.lang {
                Lang::Rust => "__wisp_free",
                Lang::Go => "wispFree",
                Lang::C => "wisp_free",
            }
        ));
    }
    fn convert_flat(&self, e: &str, from: Abi, to: Abi) -> String {
        if from == to {
            return e.to_owned();
        }
        let bits = match (from, self.lang) {
            (Abi::F32, Lang::Rust) | (Abi::F64, Lang::Rust) => format!("({e}).to_bits()"),
            (Abi::F32, Lang::Go) => format!("math.Float32bits({e})"),
            (Abi::F64, Lang::Go) => format!("math.Float64bits({e})"),
            (Abi::F32, Lang::C) => format!("wisp_f32_bits({e})"),
            (Abi::F64, Lang::C) => format!("wisp_f64_bits({e})"),
            _ => e.to_owned(),
        };
        match (to, self.lang) {
            (Abi::F32, Lang::Rust) => format!("f32::from_bits(({bits}) as u32)"),
            (Abi::F64, Lang::Rust) => format!("f64::from_bits(({bits}) as u64)"),
            (Abi::F32, Lang::Go) => format!("math.Float32frombits(uint32({bits}))"),
            (Abi::F64, Lang::Go) => format!("math.Float64frombits(uint64({bits}))"),
            (Abi::F32, Lang::C) => format!("wisp_bits_f32((__UINT32_TYPE__)({bits}))"),
            (Abi::F64, Lang::C) => format!("wisp_bits_f64((__UINT64_TYPE__)({bits}))"),
            _ => self.cast(&bits, self.abi_name(to)),
        }
    }
    fn flat_store(&mut self, t: &Scalar, p: &str, args: &[String], abi: &[Abi]) {
        match &t.kind {
            ValueKind::Scalar => self.store(
                p,
                primitive(t),
                &self.convert_flat(&args[0], abi[0], flat(t)[0]),
            ),
            ValueKind::String | ValueKind::Bytes => {
                self.store(p, "u32", &self.convert_flat(&args[0], abi[0], Abi::I32));
                self.store(
                    &format!("({p})+4"),
                    "u32",
                    &self.convert_flat(&args[1], abi[1], Abi::I32),
                );
            }
            ValueKind::Record { fields, .. } => {
                let offsets = record_layout(fields.iter().map(|f| &f.ty)).1;
                let mut at = 0;
                for (f, o) in fields.iter().zip(offsets) {
                    let n = flat(&f.ty).len();
                    self.flat_store(
                        &f.ty,
                        &format!("({p})+{o}"),
                        &args[at..at + n],
                        &abi[at..at + n],
                    );
                    at += n;
                }
            }
            ValueKind::Result { ok, err } => {
                self.store(p, "u8", &args[0]);
                for (index, ty) in [ok, err].iter().enumerate() {
                    if let Some(ty) = ty {
                        self.condition(&format!("{} == {index}", args[0]));
                        let n = flat(ty).len();
                        let a = layout(t).1;
                        self.flat_store(
                            ty,
                            &format!("({p})+{}", align(1, a)),
                            &args[1..1 + n],
                            &abi[1..1 + n],
                        );
                        self.line("}");
                    }
                }
            }
        }
    }
    fn lift(&mut self, t: &Scalar, p: &str) -> String {
        match &t.kind {
            ValueKind::Scalar => {
                let raw = self.load(p, primitive(t));
                if t.c == "_Bool" {
                    let raw = format!("({raw} != 0)");
                    match self.lang {
                        Lang::Rust => raw,
                        Lang::Go => self.cast(&raw, &t.go),
                        Lang::C => raw,
                    }
                } else {
                    self.cast(
                        &raw,
                        match self.lang {
                            Lang::Rust => &t.rust,
                            Lang::Go => &t.go,
                            Lang::C => &t.cpp,
                        },
                    )
                }
            }
            ValueKind::String | ValueKind::Bytes => {
                let ptr = self.load(p, "u32");
                let len = self.load(&format!("({p})+4"), "u32");
                let string = matches!(t.kind, ValueKind::String);
                match self.lang {
                    Lang::Rust => format!(
                        "{}({ptr}, {len})",
                        if string {
                            "__wisp_string"
                        } else {
                            "__wisp_bytes"
                        }
                    ),
                    Lang::Go => {
                        let bytes = format!("wispBytes({ptr}, {len})");
                        let expr = if string {
                            format!("wispString({ptr}, {len})")
                        } else {
                            bytes
                        };
                        self.cast(&expr, &t.go)
                    }
                    Lang::C => format!(
                        "{}{{({}*)(__UINTPTR_TYPE__)({ptr}), {len}}}",
                        t.cpp,
                        if string {
                            "const char"
                        } else {
                            "const unsigned char"
                        }
                    ),
                }
            }
            ValueKind::Record { fields, .. } => {
                let offsets = record_layout(fields.iter().map(|f| &f.ty)).1;
                let values = fields
                    .iter()
                    .zip(offsets)
                    .map(|(f, o)| {
                        let v = self.lift(&f.ty, &format!("({p})+{o}"));
                        match self.lang {
                            Lang::Rust => format!("{}: {v}", rust_identifier(&f.rust_name)),
                            Lang::Go => format!("{}: {v}", f.go_name),
                            Lang::C => v,
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{}{{{values}}}",
                    match self.lang {
                        Lang::Rust => &t.rust,
                        Lang::Go => &t.go,
                        Lang::C => &t.cpp,
                    }
                )
            }
            ValueKind::Result { ok, err } => {
                let tag = self.load(p, "u8");
                let payload = format!("({p})+{}", align(1, layout(t).1));
                let a = ok
                    .as_ref()
                    .map(|t| self.lift(t, &payload))
                    .unwrap_or_else(|| match self.lang {
                        Lang::Rust => "()".into(),
                        Lang::Go => "struct{}{}".into(),
                        Lang::C => "wisp::unit{}".into(),
                    });
                let b = err
                    .as_ref()
                    .map(|t| self.lift(t, &payload))
                    .unwrap_or_else(|| match self.lang {
                        Lang::Rust => "()".into(),
                        Lang::Go => "struct{}{}".into(),
                        Lang::C => "wisp::unit{}".into(),
                    });
                match self.lang {
                    Lang::Rust => format!(
                        "match {tag} {{ 0 => Ok({a}), 1 => Err({b}), _ => panic!(\"invalid canonical result tag\") }}"
                    ),
                    Lang::Go => {
                        let name = format!("v{}", self.next);
                        self.next += 1;
                        self.line(format!("var {name} {}", t.go));
                        self.line(format!("switch {tag} {{ case 0: {name}.Ok = {a}; case 1: {name}.IsErr = true; {name}.Err = {b}; default: panic(\"invalid canonical result tag\") }}"));
                        name
                    }
                    Lang::C => format!(
                        "({tag} == 0 ? {}::success({a}) : {}::failure({b}))",
                        t.cpp, t.cpp
                    ),
                }
            }
        }
    }
    fn lower(&mut self, t: &Scalar, p: &str, e: &str) {
        match &t.kind {
            ValueKind::Scalar => {
                let e = if t.c == "_Bool" && self.lang == Lang::Go {
                    format!("wispBool(bool({e}))")
                } else {
                    e.to_owned()
                };
                self.store(p, primitive(t), &e);
            }
            ValueKind::String | ValueKind::Bytes => {
                let string = matches!(t.kind, ValueKind::String);
                let pair = match self.lang {
                    Lang::Rust => self.local(&format!(
                        "__wisp_copy(({}).as_ref())",
                        if string {
                            format!("({e}).as_bytes()")
                        } else {
                            format!("{e}")
                        }
                    )),
                    Lang::Go => self.local(&format!(
                        "wispCopy({})",
                        if string {
                            format!("wispStringBytes(string({e}))")
                        } else {
                            format!("[]byte({e})")
                        }
                    )),
                    Lang::C => self.local(&format!("wisp_copy(({e}).data, ({e}).size)")),
                };
                let ptr = match self.lang {
                    Lang::Rust => format!("{pair}.0"),
                    Lang::Go => format!("{pair}.ptr"),
                    Lang::C => format!("{pair}.ptr"),
                };
                let len = match self.lang {
                    Lang::Rust => format!("{pair}.1"),
                    Lang::Go => format!("{pair}.len"),
                    Lang::C => format!("{pair}.len"),
                };
                self.store(p, "u32", &ptr);
                self.store(&format!("({p})+4"), "u32", &len);
            }
            ValueKind::Record { fields, .. } => {
                let offsets = record_layout(fields.iter().map(|f| &f.ty)).1;
                for (f, o) in fields.iter().zip(offsets) {
                    let name = match self.lang {
                        Lang::Rust => rust_identifier(&f.rust_name),
                        Lang::Go => f.go_name.clone(),
                        Lang::C => f.rust_name.clone(),
                    };
                    self.lower(&f.ty, &format!("({p})+{o}"), &format!("({e}).{name}"));
                }
            }
            ValueKind::Result { ok, err } => {
                let payload = format!("({p})+{}", align(1, layout(t).1));
                match self.lang {
                    Lang::Rust => {
                        self.line(format!("match &({e}) {{ Ok(value) => {{"));
                        self.store(p, "u8", "0");
                        if let Some(ty) = ok {
                            self.lower(ty, &payload, "(*value)");
                        }
                        self.line("}, Err(value) => {");
                        self.store(p, "u8", "1");
                        if let Some(ty) = err {
                            self.lower(ty, &payload, "(*value)");
                        }
                        self.line("} }");
                    }
                    Lang::Go | Lang::C => {
                        let (tag, a, b) = if self.lang == Lang::Go {
                            ("IsErr", "Ok", "Err")
                        } else {
                            ("is_err", "ok", "err")
                        };
                        self.condition(&format!("({e}).{tag}"));
                        self.store(p, "u8", "1");
                        if let Some(ty) = err {
                            self.lower(ty, &payload, &format!("({e}).{b}"));
                        }
                        self.line("} else {");
                        self.store(p, "u8", "0");
                        if let Some(ty) = ok {
                            self.lower(ty, &payload, &format!("({e}).{a}"));
                        }
                        self.line("}");
                    }
                }
            }
        }
    }
    fn cleanup(&mut self, t: &Scalar, p: &str) {
        match &t.kind {
            ValueKind::String | ValueKind::Bytes => {
                let ptr = self.load(p, "u32");
                let len = self.load(&format!("({p})+4"), "u32");
                self.line(format!(
                    "{}({ptr}, {len}, 1);",
                    match self.lang {
                        Lang::Rust => "__wisp_free",
                        Lang::Go => "wispFree",
                        Lang::C => "wisp_free",
                    }
                ));
            }
            ValueKind::Record { fields, .. } => {
                for (f, o) in fields
                    .iter()
                    .zip(record_layout(fields.iter().map(|f| &f.ty)).1)
                {
                    self.cleanup(&f.ty, &format!("({p})+{o}"));
                }
            }
            ValueKind::Result { ok, err } => {
                let p2 = format!("({p})+{}", align(1, layout(t).1));
                for (i, ty) in [ok, err].iter().enumerate() {
                    if let Some(ty) = ty {
                        self.condition(&format!("{} == {i}", self.load(p, "u8")));
                        self.cleanup(ty, &p2);
                        self.line("}");
                    }
                }
            }
            _ => (),
        }
    }
    fn flatten_memory(&mut self, t: &Scalar, p: &str) -> Vec<String> {
        match &t.kind {
            ValueKind::Scalar => {
                vec![self.cast(&self.load(p, primitive(t)), self.abi_name(flat(t)[0]))]
            }
            ValueKind::String | ValueKind::Bytes => {
                vec![self.load(p, "u32"), self.load(&format!("({p})+4"), "u32")]
            }
            ValueKind::Record { fields, .. } => fields
                .iter()
                .zip(record_layout(fields.iter().map(|f| &f.ty)).1)
                .flat_map(|(f, o)| self.flatten_memory(&f.ty, &format!("({p})+{o}")))
                .collect(),
            ValueKind::Result { ok, err } => {
                let abi = flat(t);
                let names = abi
                    .iter()
                    .map(|a| {
                        let name = format!("v{}", self.next);
                        self.next += 1;
                        self.line(match self.lang {
                            Lang::Rust => {
                                format!("let mut {name}: {} = 0 as {};", a.rust(), a.rust())
                            }
                            Lang::Go => format!("var {name} {}", a.go()),
                            Lang::C => format!("{} {name} = 0;", a.c()),
                        });
                        name
                    })
                    .collect::<Vec<_>>();
                self.line(format!(
                    "{} = {};",
                    names[0],
                    self.cast(&self.load(p, "u8"), self.abi_name(Abi::I32))
                ));
                for (index, ty) in [ok, err].iter().enumerate() {
                    if let Some(ty) = ty {
                        self.condition(&format!("{} == {index}", names[0]));
                        let src = flat(ty);
                        let values =
                            self.flatten_memory(ty, &format!("({p})+{}", align(1, layout(t).1)));
                        for (i, e) in values.iter().enumerate() {
                            self.line(format!(
                                "{} = {};",
                                names[i + 1],
                                self.convert_flat(e, src[i], abi[i + 1])
                            ));
                        }
                        self.line("}");
                    }
                }
                names
            }
        }
    }
}
fn export_name(binding: &Binding) -> String {
    binding
        .module
        .as_ref()
        .map(|m| format!("{m}#{}", binding.name))
        .unwrap_or_else(|| binding.name.clone())
}
fn rust_identifier(name: &str) -> String {
    if name.starts_with("r#") {
        return name.into();
    }
    if matches!(name, "self" | "Self" | "super" | "crate") {
        return format!("{name}_");
    }
    format!("r#{name}")
}
fn gather(t: &Scalar, map: &mut BTreeMap<String, Scalar>) {
    match &t.kind {
        ValueKind::Record { fields, .. } => {
            for f in fields {
                gather(&f.ty, map);
            }
            map.insert(t.go.clone(), t.clone());
        }
        ValueKind::Result { ok, err } => {
            for t in ok.iter().chain(err.iter()) {
                gather(t, map);
            }
        }
        _ => (),
    }
}

pub fn rust(bindings: &[Binding], library: &str) -> Result<String> {
    ensure!(
        !bindings.iter().any(|b| b.import),
        "Rust authored import bindings require installed guest crates"
    );
    let mut output = format!(
        "// Generated by Wisp.\nextern crate {library} as __wisp_application;\n{RUST_RUNTIME}\n"
    );
    let mut definitions = BTreeMap::new();
    for t in bindings
        .iter()
        .flat_map(|b| b.params.iter().chain(b.result.iter()))
    {
        gather(t, &mut definitions);
    }
    for t in definitions.values() {
        if let ValueKind::Record { fields, .. } = &t.kind {
            if t.generated {
                output.push_str(&format!(
                    "pub struct {} {{ {} }}\n",
                    t.rust,
                    fields
                        .iter()
                        .map(|f| format!("pub {}: {},", rust_identifier(&f.rust_name), f.ty.rust))
                        .collect::<String>()
                ));
            }
        }
    }
    for (index, b) in bindings.iter().enumerate() {
        let abi = b.params.iter().flat_map(flat).collect::<Vec<_>>();
        let indirect = abi.len() > 16;
        let result_abi = b.result.as_ref().map(flat).unwrap_or_default();
        let retptr = result_abi.len() > 1;
        let parameters = if indirect {
            "args: u32".into()
        } else {
            abi.iter()
                .enumerate()
                .map(|(i, a)| format!("a{i}: {}", a.rust()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let ret = if retptr {
            " -> u32".into()
        } else {
            result_abi
                .first()
                .map(|a| format!(" -> {}", a.rust()))
                .unwrap_or_default()
        };
        let mut g = Gen::new(Lang::Rust);
        let ((n, a), offsets) = record_layout(b.params.iter());
        let p = if indirect {
            "args".into()
        } else {
            g.line(format!(
                "let mut __args = [0u64; {}];",
                n.div_ceil(8).max(1)
            ));
            g.local("__args.as_mut_ptr() as u32")
        };
        if !indirect {
            let mut at = 0;
            for (t, o) in b.params.iter().zip(&offsets) {
                let count = flat(t).len();
                let args = (at..at + count)
                    .map(|i| format!("a{i}"))
                    .collect::<Vec<_>>();
                g.flat_store(t, &format!("{p}+{o}"), &args, &abi[at..at + count]);
                at += count;
            }
        }
        let arguments = b
            .params
            .iter()
            .zip(&offsets)
            .map(|(t, o)| {
                let value = g.lift(t, &format!("{p}+{o}"));
                g.local(&value)
            })
            .collect::<Vec<_>>();
        for (t, o) in b.params.iter().zip(&offsets) {
            g.cleanup(t, &format!("{p}+{o}"));
        }
        if indirect {
            g.free(&p, n, a);
        }
        let call = format!(
            "__wisp_application::{}({})",
            b.rust_name,
            arguments.join(", ")
        );
        if let Some(t) = &b.result {
            let value = g.local(&call);
            let (n, a) = layout(t);
            let ptr = g.alloc(n, a);
            g.lower(t, &ptr, &value);
            if retptr {
                g.line(ptr);
            } else {
                let v = g.flatten_memory(t, &ptr).into_iter().next();
                if let Some(v) = v {
                    let v = g.local(&v);
                    g.free(&ptr, n, a);
                    g.line(v);
                } else {
                    g.free(&ptr, n, a);
                }
            }
        } else {
            g.line(format!("{call};"));
        }
        output.push_str(&format!("#[export_name = {:?}]\npub unsafe extern \"C\" fn __wisp_export_{index}({parameters}){ret} {{\n{}\n}}\n",export_name(b),g.code));
        if retptr {
            let t = b.result.as_ref().unwrap();
            let (n, a) = layout(t);
            let mut post = Gen::new(Lang::Rust);
            post.cleanup(t, "p");
            post.free("p", n, a);
            output.push_str(&format!("#[export_name = {:?}]\npub unsafe extern \"C\" fn __wisp_post_{index}(p:u32) {{ {} }}\n",format!("cabi_post_{}",export_name(b)),post.code));
        }
    }
    Ok(output)
}
const RUST_RUNTIME: &str = r#"
unsafe fn __wisp_read<T:Copy>(p:u32)->T { (p as *const T).read_unaligned() }
unsafe fn __wisp_write<T>(p:u32,v:T) { (p as *mut T).write_unaligned(v); }
unsafe fn __wisp_alloc(n:u32,a:u32)->u32 { if n==0{return a} let layout=std::alloc::Layout::from_size_align(n as usize,a as usize).unwrap(); let p=std::alloc::alloc_zeroed(layout); if p.is_null(){std::alloc::handle_alloc_error(layout)} p as u32 }
unsafe fn __wisp_free(p:u32,n:u32,a:u32) { if n!=0 {std::alloc::dealloc(p as *mut u8,std::alloc::Layout::from_size_align(n as usize,a as usize).unwrap());} }
#[export_name="cabi_realloc"]
pub unsafe extern "C" fn __wisp_realloc(p:u32,old:u32,a:u32,n:u32)->u32 { if n==0{__wisp_free(p,old,a);return a} let q=__wisp_alloc(n,a); if old!=0{std::ptr::copy_nonoverlapping(p as *const u8,q as *mut u8,old.min(n) as usize);__wisp_free(p,old,a);}q }
unsafe fn __wisp_bytes(p:u32,n:u32)->Vec<u8> { if n==0{Vec::new()}else{std::slice::from_raw_parts(p as *const u8,n as usize).to_vec()} }
unsafe fn __wisp_string(p:u32,n:u32)->String { String::from_utf8(__wisp_bytes(p,n)).expect("invalid canonical UTF-8") }
unsafe fn __wisp_copy(bytes:&[u8])->(u32,u32) {let p=__wisp_alloc(bytes.len().try_into().expect("canonical length overflow"),1);if !bytes.is_empty(){std::ptr::copy_nonoverlapping(bytes.as_ptr(),p as *mut u8,bytes.len());}(p,bytes.len() as u32)}
"#;

fn go_types(t: &mut Scalar, definitions: &mut BTreeMap<String, Scalar>) {
    match &mut t.kind {
        ValueKind::Record { fields, .. } => {
            for f in fields {
                go_types(&mut f.ty, definitions);
            }
            if t.generated {
                definitions.insert(t.go.clone(), t.clone());
            }
        }
        ValueKind::Result { ok, err } => {
            for v in ok.iter_mut().chain(err.iter_mut()) {
                go_types(v, definitions);
            }
            let key = format!("{:?}", t.kind);
            let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
            let name = format!("WispResult{}", &hash[..12]);
            t.go = name.clone();
            definitions.insert(name, t.clone());
        }
        _ => (),
    }
}
pub fn go(bindings: &[Binding]) -> String {
    let mut bindings = bindings.to_vec();
    let mut definitions = BTreeMap::new();
    for b in &mut bindings {
        for t in b.params.iter_mut().chain(b.result.iter_mut()) {
            go_types(t, &mut definitions);
        }
    }
    let mut output = format!(
        "// Generated by Wisp.\npackage main\nimport (\"unsafe\"; \"math\"; \"unicode/utf8\"; \"errors\")\n{GO_RUNTIME}\n{}\n",
        crate::build_go_memory::values_source()
    );
    for (name, t) in &definitions {
        match &t.kind {
            ValueKind::Record { fields, .. } => output.push_str(&format!(
                "type {name} struct {{ {} }}\n",
                fields
                    .iter()
                    .map(|f| format!("{} {};", f.go_name, f.ty.go))
                    .collect::<String>()
            )),
            ValueKind::Result { ok, err } => output.push_str(&format!(
                "type {name} struct {{ IsErr bool; Ok {}; Err {} }}\n",
                ok.as_ref().map(|t| t.go.as_str()).unwrap_or("struct{}"),
                err.as_ref().map(|t| t.go.as_str()).unwrap_or("struct{}")
            )),
            _ => (),
        }
    }
    for (index, b) in bindings.iter().enumerate() {
        let abi = b.params.iter().flat_map(flat).collect::<Vec<_>>();
        let indirect = abi.len() > 16;
        let result_abi = b.result.as_ref().map(flat).unwrap_or_default();
        let retptr = result_abi.len() > 1;
        let mut parameters = if indirect {
            vec!["args uint32".into()]
        } else {
            abi.iter()
                .enumerate()
                .map(|(i, a)| format!("a{i} {}", a.go()))
                .collect::<Vec<_>>()
        };
        if b.import && retptr {
            parameters.push("ret uint32".into());
        }
        let ret = if retptr {
            if b.import {
                String::new()
            } else {
                " uint32".into()
            }
        } else {
            result_abi
                .first()
                .map(|a| format!(" {}", a.go()))
                .unwrap_or_default()
        };
        if b.import {
            output.push_str(&format!(
                "//go:wasmimport {} {}\nfunc wispImport{index}({}){ret}\n",
                b.module.as_deref().unwrap_or("$root"),
                b.name,
                parameters.join(", ")
            ));
            let mut g = Gen::new(Lang::Go);
            let ((n, a), offsets) = record_layout(b.params.iter());
            let p = g.alloc(n, a);
            for (i, (t, o)) in b.params.iter().zip(&offsets).enumerate() {
                g.lower(t, &format!("{p}+{o}"), &format!("a{i}"));
            }
            let mut args = if indirect {
                vec![p.clone()]
            } else {
                b.params
                    .iter()
                    .zip(&offsets)
                    .flat_map(|(t, o)| g.flatten_memory(t, &format!("{p}+{o}")))
                    .collect()
            };
            let result_ptr = if retptr {
                let (n, a) = layout(b.result.as_ref().unwrap());
                let p = g.alloc(n, a);
                args.push(p.clone());
                Some(p)
            } else {
                None
            };
            let call = format!("wispImport{index}({})", args.join(", "));
            let direct = if b.result.is_some() && !retptr && !result_abi.is_empty() {
                Some(g.local(&call))
            } else {
                g.line(call);
                None
            };
            for (t, o) in b.params.iter().zip(&offsets) {
                g.cleanup(t, &format!("{p}+{o}"));
            }
            g.free(&p, n, a);
            if let Some(t) = &b.result {
                let (n, a) = layout(t);
                let p = if let Some(p) = result_ptr {
                    p
                } else {
                    let p = g.alloc(n, a);
                    g.flat_store(t, &p, &direct.into_iter().collect::<Vec<_>>(), &result_abi);
                    p
                };
                let e = g.lift(t, &p);
                let e = g.local(&e);
                g.cleanup(t, &p);
                g.free(&p, n, a);
                g.line(format!("return {e}"));
            }
            let params = b
                .params
                .iter()
                .enumerate()
                .map(|(i, t)| format!("a{i} {}", t.go))
                .collect::<Vec<_>>()
                .join(", ");
            let ret = b
                .result
                .as_ref()
                .map(|t| format!(" {}", t.go))
                .unwrap_or_default();
            output.push_str(&format!(
                "func {}({params}){ret} {{\n{}\n}}\n",
                b.go_name, g.code
            ));
        } else {
            let mut g = Gen::new(Lang::Go);
            let ((n, a), offsets) = record_layout(b.params.iter());
            let p = if indirect {
                "args".into()
            } else {
                g.alloc(n, a)
            };
            if !indirect {
                let mut at = 0;
                for (t, o) in b.params.iter().zip(&offsets) {
                    let count = flat(t).len();
                    let args = (at..at + count)
                        .map(|i| format!("a{i}"))
                        .collect::<Vec<_>>();
                    g.flat_store(t, &format!("{p}+{o}"), &args, &abi[at..at + count]);
                    at += count;
                }
            }
            let args = b
                .params
                .iter()
                .zip(&offsets)
                .map(|(t, o)| {
                    let e = g.lift(t, &format!("{p}+{o}"));
                    g.local(&e)
                })
                .collect::<Vec<_>>();
            for (t, o) in b.params.iter().zip(&offsets) {
                g.cleanup(t, &format!("{p}+{o}"));
            }
            g.free(&p, n, a);
            let call = format!("{}({})", b.go_name, args.join(", "));
            if let Some(t) = &b.result {
                let e = if b.go_error_result {
                    let ValueKind::Result { ok, .. } = &t.kind else {
                        unreachable!()
                    };
                    if ok.is_some() {
                        g.line(format!("__ok, __err := {call}"));
                    } else {
                        g.line(format!("__err := {call}"));
                    }
                    g.line(format!("var __result {}", t.go));
                    g.line("if __err != nil { __result.IsErr = true; __result.Err = __err.Error() } else {");
                    if ok.is_some() {
                        g.line("__result.Ok = __ok");
                    }
                    g.line("}");
                    "__result".into()
                } else {
                    g.local(&call)
                };
                let (n, a) = layout(t);
                let p = g.alloc(n, a);
                g.lower(t, &p, &e);
                if retptr {
                    g.line(format!("return {p}"));
                } else {
                    let v = g.flatten_memory(t, &p).into_iter().next();
                    if let Some(v) = v {
                        let v = g.local(&v);
                        g.free(&p, n, a);
                        g.line(format!("return {v}"));
                    } else {
                        g.free(&p, n, a);
                    }
                }
            } else {
                g.line(call);
            }
            output.push_str(&format!(
                "//go:wasmexport {}\nfunc wispExport{index}({}){ret} {{\n{}\n}}\n",
                export_name(b),
                parameters.join(", "),
                g.code
            ));
            if retptr {
                let t = b.result.as_ref().unwrap();
                let (n, a) = layout(t);
                let mut post = Gen::new(Lang::Go);
                post.cleanup(t, "p");
                post.free("p", n, a);
                output.push_str(&format!(
                    "//go:wasmexport cabi_post_{}\nfunc wispPost{index}(p uint32) {{ {} }}\n",
                    export_name(b),
                    post.code
                ));
            }
        }
    }
    output
}
const GO_RUNTIME: &str = r#"
var _ = math.Float32bits
var _ = errors.New
func wispStringBytes(value string) []byte { if !utf8.ValidString(value) { panic("invalid canonical UTF-8") }; return []byte(value) }
func wispString(p,n uint32) string { value:=string(wispBytes(p,n)); if !utf8.ValidString(value) { panic("invalid canonical UTF-8") }; return value }
func wispBool(value bool) uint32 { if value { return 1 }; return 0 }
func wispWord(p uint32) *uint32 { return (*uint32)(unsafe.Pointer(uintptr(p))) }
func wispAlloc(n, alignment uint32) uint32 {
    if alignment == 0 || alignment > 65536 || alignment & (alignment-1) != 0 || n > 0x7fffffff-alignment {panic("canonical allocation overflow")}
    if n == 0 {return alignment}
    if alignment < 8 {alignment = 8}
    control, limit := uint32(wispCanonicalControl()), uint32(wispCanonicalLimit())
    head := wispWord(control+4)
    previous, p := uint32(0), *head
    for p != 0 {
        if p < control+24 || p >= limit {panic("invalid canonical free list")}
        capacity, next := *wispWord(p-8), *wispWord(p-4)
        if capacity >= n && p & (alignment-1) == 0 {
            if previous == 0 {*head = next} else {*wispWord(previous-4) = next}
            *wispWord(p-4) = 0xffffffff
            return p
        }
        previous, p = p, next
    }
    cursor := *wispWord(control)
    if cursor == 0 {cursor = control+16}
    p = (cursor+8+alignment-1) & ^(alignment-1)
    end := uint64(p)+uint64(n)+7
    end &= ^uint64(7)
    if end > uint64(limit) {panic("canonical value arena exhausted (4 MiB)")}
    *wispWord(p-8) = uint32(end)-p
    *wispWord(p-4) = 0xffffffff
    *wispWord(control) = uint32(end)
    return p
}
func wispFree(p, n, alignment uint32) {
    if n == 0 {return}
    control, limit := uint32(wispCanonicalControl()), uint32(wispCanonicalLimit())
    if p < control+24 || p >= limit {panic("unknown canonical allocation")}
    capacity := *wispWord(p-8)
    if capacity < n || uint64(p)+uint64(capacity) > uint64(limit) || *wispWord(p-4) != 0xffffffff {panic("invalid canonical allocation")}
    *wispWord(p-4) = *wispWord(control+4)
    *wispWord(control+4) = p
}
//go:wasmexport wisp-cabi-realloc
func wispRealloc(p, old, alignment, n uint32) uint32 {panic("canonical allocator requires the pure Wasm shim")}
func wispBytes(p,n uint32) []byte {if n==0{return []byte{}};return append([]byte(nil),unsafe.Slice((*byte)(unsafe.Pointer(uintptr(p))),int(n))...)}
type wispPair struct {ptr uint32;len uint32}
func wispCopy(bytes []byte) wispPair {p:=wispAlloc(uint32(len(bytes)),1);if len(bytes)!=0{copy(unsafe.Slice((*byte)(unsafe.Pointer(uintptr(p))),len(bytes)),bytes)};return wispPair{p,uint32(len(bytes))}}
"#;

/// Dependency-free Rust imports use the same layouts and ownership rules as source exports.
pub fn rust_guest(bindings: &[Binding]) -> Result<String> {
    let mut output = String::from("// Generated by Wisp.\n#![allow(unused_parens, dead_code)]\n");
    output.push_str(&RUST_RUNTIME.replace("#[export_name=\"cabi_realloc\"]", ""));
    let mut definitions = BTreeMap::new();
    for t in bindings
        .iter()
        .flat_map(|b| b.params.iter().chain(b.result.iter()))
    {
        gather(t, &mut definitions);
    }
    for t in definitions.values() {
        if let ValueKind::Record { fields, .. } = &t.kind {
            output.push_str(&format!(
                "#[derive(Clone, Debug)]\npub struct {} {{ {} }}\n",
                t.rust,
                fields
                    .iter()
                    .map(|f| format!("pub {}: {},", rust_identifier(&f.rust_name), f.ty.rust))
                    .collect::<String>()
            ));
        }
    }
    let flat_api = bindings.iter().all(|b| {
        b.module.as_ref().is_none_or(|m| {
            m.split('/')
                .nth(1)
                .is_some_and(|n| n.split('@').next() == Some("api"))
        })
    });
    let mut names = std::collections::BTreeSet::new();
    for (index, b) in bindings.iter().enumerate() {
        let scope = if flat_api {
            None
        } else {
            b.module
                .as_ref()
                .and_then(|m| m.split('/').nth(1))
                .map(|m| m.split('@').next().unwrap().to_snake_case())
        };
        let name = b.name.to_snake_case();
        let name = if matches!(name.as_str(), "self" | "super" | "crate") {
            format!("{name}_")
        } else {
            format!("r#{name}")
        };
        ensure!(
            names.insert((scope.clone(), name.clone())),
            "Rust guest binding name collision"
        );
        if let Some(scope) = &scope {
            output.push_str(&format!(
                "pub mod {} {{ use super::*;\n",
                rust_identifier(scope)
            ));
        }
        let abi = b.params.iter().flat_map(flat).collect::<Vec<_>>();
        let indirect = abi.len() > 16;
        let result_abi = b.result.as_ref().map(flat).unwrap_or_default();
        let retptr = result_abi.len() > 1;
        let mut params = if indirect {
            vec!["args:u32".into()]
        } else {
            abi.iter()
                .enumerate()
                .map(|(i, a)| format!("a{i}:{}", a.rust()))
                .collect::<Vec<_>>()
        };
        if retptr {
            params.push("ret:u32".into());
        }
        let ret = if retptr {
            String::new()
        } else {
            result_abi
                .first()
                .map(|a| format!(" -> {}", a.rust()))
                .unwrap_or_default()
        };
        output.push_str(&format!("#[cfg(target_arch=\"wasm32\")]\n#[link(wasm_import_module={:?})]\nextern \"C\" {{ #[link_name={:?}] fn __wisp_import_{index}({}){ret}; }}\n",b.module.as_deref().unwrap_or("$root"),b.name,params.join(",")));
        let mut g = Gen::new(Lang::Rust);
        let ((n, a), offsets) = record_layout(b.params.iter());
        let p = g.alloc(n, a);
        for (i, (t, o)) in b.params.iter().zip(&offsets).enumerate() {
            g.lower(t, &format!("{p}+{o}"), &format!("a{i}"));
        }
        let mut args = if indirect {
            vec![p.clone()]
        } else {
            b.params
                .iter()
                .zip(&offsets)
                .flat_map(|(t, o)| g.flatten_memory(t, &format!("{p}+{o}")))
                .collect()
        };
        let result_ptr = if retptr {
            let (n, a) = layout(b.result.as_ref().unwrap());
            let p = g.alloc(n, a);
            args.push(p.clone());
            Some(p)
        } else {
            None
        };
        let call = format!("__wisp_import_{index}({})", args.join(","));
        let direct = if b.result.is_some() && !retptr && !result_abi.is_empty() {
            Some(g.local(&call))
        } else {
            g.line(format!("{call};"));
            None
        };
        for (t, o) in b.params.iter().zip(&offsets) {
            g.cleanup(t, &format!("{p}+{o}"));
        }
        g.free(&p, n, a);
        if let Some(t) = &b.result {
            let (n, a) = layout(t);
            let p = if let Some(p) = result_ptr {
                p
            } else {
                let p = g.alloc(n, a);
                g.flat_store(t, &p, &direct.into_iter().collect::<Vec<_>>(), &result_abi);
                p
            };
            let e = g.lift(t, &p);
            let e = g.local(&e);
            g.cleanup(t, &p);
            g.free(&p, n, a);
            g.line(e);
        }
        let params = b
            .params
            .iter()
            .enumerate()
            .map(|(i, t)| format!("a{i}:{}", t.rust))
            .collect::<Vec<_>>()
            .join(",");
        let ret = b
            .result
            .as_ref()
            .map(|t| format!(" -> {}", t.rust))
            .unwrap_or_default();
        output.push_str(&format!("pub fn {name}({params}){ret} {{\n#[cfg(not(target_arch=\"wasm32\"))] {{ panic!(\"Wisp guest dependencies require wasm32-wasip1\") }}\n#[cfg(target_arch=\"wasm32\")] unsafe {{ {} }}\n}}\n",g.code));
        if scope.is_some() {
            output.push_str("}\n");
        }
    }
    Ok(output)
}

#[cfg(test)]
#[path = "build_values_tests.rs"]
mod tests;
