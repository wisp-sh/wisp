//! Give the Preview1 adapter storage that the Go runtime owns and will not reuse.
//!
//! A synthetic memory.grow allocator is unsafe with Go's independent sbrk high
//! water mark. The generated Go array is static, pointer-free module data. The
//! low-level allocator appended here can run before Go's runtime initializes.
use anyhow::{Context, Result, ensure};
use wasm_encoder::{
    reencode::{self, Reencode},
    *,
};
use wasmparser::{DataKind, Operator, Parser, Payload, TypeRef};
const PAGE: u32 = 65536;
const ARENA: u32 = 3 * PAGE;
const BEGIN: &[u8] = b"WISP-REACTOR-ARENA-BEGIN-v1-MARKER!";
const END: &[u8] = b"WISP-REACTOR-ARENA-END-v1-MARKER!!!";
const CANONICAL_SIZE: u32 = 4 * 1024 * 1024;
const CANONICAL_BEGIN: &[u8; 32] = b"WISP-CANON-ARENA-BEGIN-v1-MARKER";
const CANONICAL_END: &[u8; 32] = b"WISP-CANON-ARENA-END-v1-MARKER!!";
const ALLOCATED: i32 = -1;

#[derive(Clone, Debug)]
pub enum Cleanup {
    Buffer {
        offset: u32,
    },
    Variant {
        offset: u32,
        ok: Vec<Cleanup>,
        err: Vec<Cleanup>,
    },
}
#[derive(Clone, Debug)]
pub struct PostReturn {
    pub export: String,
    pub size: u32,
    pub alignment: u32,
    pub cleanup: Vec<Cleanup>,
}

#[derive(Clone, Copy)]
struct CanonicalArena {
    control: u32,
    limit: u32,
}

pub fn values_source() -> String {
    let mut entries = CANONICAL_BEGIN
        .iter()
        .enumerate()
        .map(|(i, b)| format!("{i}:{b}"))
        .collect::<Vec<_>>();
    entries.extend(
        CANONICAL_END
            .iter()
            .enumerate()
            .map(|(i, b)| format!("{}:{b}", CANONICAL_SIZE as usize - CANONICAL_END.len() + i)),
    );
    format!(
        "{}\nvar wispCanonicalArena = [{CANONICAL_SIZE}]byte{{{}}}\nfunc wispCanonicalControl() uintptr {{ return uintptr(unsafe.Pointer(&wispCanonicalArena[32])) }}\nfunc wispCanonicalLimit() uintptr {{ return uintptr(unsafe.Pointer(&wispCanonicalArena[{}])) }}\n",
        source(),
        entries.join(","),
        CANONICAL_SIZE - 32
    )
}

fn emit(f: &mut Function, ops: impl IntoIterator<Item = Instruction<'static>>) {
    for op in ops {
        f.instruction(&op);
    }
}
fn trap_if(f: &mut Function, condition: impl IntoIterator<Item = Instruction<'static>>) {
    emit(f, condition);
    emit(
        f,
        [
            Instruction::If(BlockType::Empty),
            Instruction::Unreachable,
            Instruction::End,
        ],
    );
}
fn load(offset: u64) -> Instruction<'static> {
    Instruction::I32Load(MemArg {
        offset,
        align: 2,
        memory_index: 0,
    })
}
fn store(offset: u64) -> Instruction<'static> {
    Instruction::I32Store(MemArg {
        offset,
        align: 2,
        memory_index: 0,
    })
}

// Each block is an aligned data pointer with {capacity,next} at p-8. All
// operations stay in static Go-owned memory. In particular, canonical realloc
// cannot call Go: a GC/WASI call here would violate Component reentrancy rules.
fn check_pointer(f: &mut Function, p: u32, arena: CanonicalArena, allocated: bool) {
    use Instruction::*;
    trap_if(
        f,
        [
            LocalGet(p),
            I32Const((arena.control + 24) as i32),
            I32LtU,
            LocalGet(p),
            I32Const(arena.limit as i32),
            I32GtU,
            I32Or,
            LocalGet(p),
            I32Const(7),
            I32And,
            I32Or,
        ],
    );
    emit(f, [LocalGet(p), I32Const(8), I32Sub, load(0), LocalSet(4)]);
    trap_if(
        f,
        [
            LocalGet(4),
            I32Const(8),
            I32LtU,
            LocalGet(4),
            I32Const(arena.limit as i32),
            LocalGet(p),
            I32Sub,
            I32GtU,
            I32Or,
        ],
    );
    trap_if(
        f,
        [
            LocalGet(p),
            I32Const(8),
            I32Sub,
            load(4),
            I32Const(ALLOCATED),
            if allocated { I32Ne } else { I32Eq },
        ],
    );
}
fn free_block(f: &mut Function, p: u32, arena: CanonicalArena) {
    use Instruction::*;
    emit(
        f,
        [
            LocalGet(p),
            I32Const(8),
            I32Sub,
            I32Const(arena.control as i32),
            load(4),
            store(4),
            I32Const(arena.control as i32),
            LocalGet(p),
            store(4),
        ],
    );
}
fn canonical_allocator(arena: CanonicalArena) -> Function {
    use Instruction::*;
    // 0 old pointer, 1 old size, 2 alignment, 3 new size; 4 capacity,
    // 5 free block, 6 previous free block, 7 next, 8 cursor, 9 result,
    // 10 end, 11 copy size, 12 free-list traversal budget.
    let mut f = Function::new([(9, ValType::I32)]);
    trap_if(
        &mut f,
        [
            LocalGet(2),
            I32Eqz,
            LocalGet(2),
            I32Const(PAGE as i32),
            I32GtU,
            I32Or,
            LocalGet(2),
            LocalGet(2),
            I32Const(1),
            I32Sub,
            I32And,
            I32Or,
        ],
    );
    emit(&mut f, [LocalGet(1), If(BlockType::Empty)]);
    check_pointer(&mut f, 0, arena, true);
    trap_if(&mut f, [LocalGet(1), LocalGet(4), I32GtU]);
    emit(
        &mut f,
        [
            End,
            LocalGet(3),
            I32Eqz,
            If(BlockType::Empty),
            LocalGet(1),
            If(BlockType::Empty),
        ],
    );
    free_block(&mut f, 0, arena);
    emit(&mut f, [End, LocalGet(2), Return, End]);
    // A smaller value can retain its block, provided the new alignment fits.
    emit(
        &mut f,
        [
            LocalGet(1),
            If(BlockType::Empty),
            LocalGet(3),
            LocalGet(4),
            I32LeU,
            LocalGet(0),
            LocalGet(2),
            I32Const(1),
            I32Sub,
            I32And,
            I32Eqz,
            I32And,
            If(BlockType::Empty),
            LocalGet(0),
            Return,
            End,
            End,
        ],
    );
    emit(
        &mut f,
        [
            I32Const(arena.control as i32),
            load(4),
            LocalSet(5),
            I32Const(0),
            LocalSet(6),
            I32Const((CANONICAL_SIZE / 16) as i32),
            LocalSet(12),
            Block(BlockType::Empty),
            Loop(BlockType::Empty),
            LocalGet(5),
            I32Eqz,
            BrIf(1),
        ],
    );
    trap_if(&mut f, [LocalGet(12), I32Eqz]);
    emit(&mut f, [LocalGet(12), I32Const(1), I32Sub, LocalSet(12)]);
    check_pointer(&mut f, 5, arena, false);
    emit(
        &mut f,
        [
            LocalGet(5),
            I32Const(8),
            I32Sub,
            load(4),
            LocalSet(7),
            LocalGet(4),
            LocalGet(3),
            I32GeU,
            LocalGet(5),
            LocalGet(2),
            I32Const(1),
            I32Sub,
            I32And,
            I32Eqz,
            I32And,
            If(BlockType::Empty),
            LocalGet(6),
            If(BlockType::Empty),
            LocalGet(6),
            I32Const(8),
            I32Sub,
            LocalGet(7),
            store(4),
            Else,
            I32Const(arena.control as i32),
            LocalGet(7),
            store(4),
            End,
            LocalGet(5),
            LocalSet(9),
            Br(2),
            End,
            LocalGet(5),
            LocalSet(6),
            LocalGet(7),
            LocalSet(5),
            Br(0),
            End,
            End,
        ],
    );
    // Fresh blocks follow the cursor. Capabilities require alignment <=64KiB;
    // the arena's validated addresses stay below i32::MAX, so alignment adds
    // cannot wrap. Check every size before adding it to an address.
    emit(
        &mut f,
        [
            LocalGet(9),
            I32Eqz,
            If(BlockType::Empty),
            I32Const(arena.control as i32),
            load(0),
            LocalSet(8),
            LocalGet(8),
            I32Eqz,
            If(BlockType::Empty),
            I32Const((arena.control + 16) as i32),
            LocalSet(8),
            End,
        ],
    );
    trap_if(
        &mut f,
        [
            LocalGet(8),
            I32Const((arena.control + 16) as i32),
            I32LtU,
            LocalGet(8),
            I32Const(arena.limit as i32),
            I32GtU,
            I32Or,
        ],
    );
    emit(
        &mut f,
        [
            LocalGet(2),
            I32Const(8),
            I32LtU,
            If(BlockType::Empty),
            I32Const(8),
            LocalSet(2),
            End,
            LocalGet(8),
            I32Const(8),
            I32Add,
            LocalGet(2),
            I32Const(1),
            I32Sub,
            I32Add,
            I32Const(0),
            LocalGet(2),
            I32Sub,
            I32And,
            LocalSet(9),
        ],
    );
    trap_if(
        &mut f,
        [
            LocalGet(9),
            I32Const(arena.limit as i32),
            I32GtU,
            LocalGet(3),
            I32Const(arena.limit as i32),
            LocalGet(9),
            I32Sub,
            I32GtU,
            I32Or,
        ],
    );
    emit(
        &mut f,
        [
            LocalGet(9),
            LocalGet(3),
            I32Add,
            I32Const(7),
            I32Add,
            I32Const(-8),
            I32And,
            LocalSet(10),
        ],
    );
    trap_if(&mut f, [LocalGet(10), I32Const(arena.limit as i32), I32GtU]);
    emit(
        &mut f,
        [
            I32Const(arena.control as i32),
            LocalGet(10),
            store(0),
            LocalGet(9),
            I32Const(8),
            I32Sub,
            LocalGet(10),
            LocalGet(9),
            I32Sub,
            store(0),
            End,
            LocalGet(9),
            I32Const(8),
            I32Sub,
            I32Const(ALLOCATED),
            store(4),
            LocalGet(1),
            If(BlockType::Empty),
            LocalGet(1),
            LocalGet(3),
            LocalGet(1),
            LocalGet(3),
            I32LtU,
            Select,
            LocalSet(11),
            LocalGet(9),
            LocalGet(0),
            LocalGet(11),
            MemoryCopy {
                src_mem: 0,
                dst_mem: 0,
            },
        ],
    );
    free_block(&mut f, 0, arena);
    emit(&mut f, [End, LocalGet(9), End]);
    f
}

pub fn source() -> String {
    let mut entries = BEGIN
        .iter()
        .enumerate()
        .map(|(i, b)| format!("{i}:{b}"))
        .collect::<Vec<_>>();
    entries.extend(
        END.iter()
            .enumerate()
            .map(|(i, b)| format!("{}:{b}", ARENA as usize - END.len() + i)),
    );
    format!(
        "var wispAdapterArena = [{ARENA}]byte{{{}}}\n//go:wasmexport wisp-adapter-arena\nfunc wispAdapterArenaAddress() uint32 {{ return uint32(uintptr(unsafe.Pointer(&wispAdapterArena[0]))) }}\n",
        entries.join(",")
    )
}
fn emit_cleanup(f: &mut Function, cleanup: &[Cleanup], raw: u32) {
    use Instruction::*;
    for item in cleanup {
        match item {
            Cleanup::Buffer { offset } => emit(
                f,
                [
                    LocalGet(0),
                    load(*offset as u64),
                    LocalGet(0),
                    load((*offset + 4) as u64),
                    I32Const(1),
                    I32Const(0),
                    Call(raw),
                    Drop,
                ],
            ),
            Cleanup::Variant { offset, ok, err } => {
                let tag = I32Load8U(MemArg {
                    offset: *offset as u64,
                    align: 0,
                    memory_index: 0,
                });
                emit(f, [LocalGet(0), tag.clone(), I32Eqz, If(BlockType::Empty)]);
                emit_cleanup(f, ok, raw);
                emit(f, [Else]);
                trap_if(f, [LocalGet(0), tag, I32Const(1), I32Ne]);
                emit_cleanup(f, err, raw);
                emit(f, [End]);
            }
        }
    }
}
fn post_return(plan: &PostReturn, raw: u32) -> Function {
    use Instruction::*;
    let mut f = Function::new([]);
    emit_cleanup(&mut f, &plan.cleanup, raw);
    emit(
        &mut f,
        [
            LocalGet(0),
            I32Const(plan.size as i32),
            I32Const(plan.alignment as i32),
            I32Const(0),
            Call(raw),
            Drop,
            End,
        ],
    );
    f
}
fn validate_cleanup(cleanup: &[Cleanup], size: u32, depth: usize, count: &mut usize) -> Result<()> {
    ensure!(depth <= 32, "Go post-return cleanup nesting limit");
    for item in cleanup {
        *count += 1;
        ensure!(*count <= 4096, "Go post-return cleanup size limit");
        match item {
            Cleanup::Buffer { offset } => ensure!(
                offset.checked_add(8).is_some_and(|end| end <= size),
                "Go post-return buffer outside result"
            ),
            Cleanup::Variant { offset, ok, err } => {
                ensure!(*offset < size, "Go post-return variant outside result");
                validate_cleanup(ok, size, depth + 1, count)?;
                validate_cleanup(err, size, depth + 1, count)?;
            }
        }
    }
    Ok(())
}

struct Allocator<'a> {
    ty: u32,
    function: u32,
    global: u32,
    start: u32,
    end: u32,
    canonical: Option<CanonicalArena>,
    posts: &'a [PostReturn],
}
impl Reencode for Allocator<'_> {
    type Error = std::convert::Infallible;
    fn parse_type_section(
        &mut self,
        types: &mut TypeSection,
        section: wasmparser::TypeSectionReader<'_>,
    ) -> std::result::Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_type_section(self, types, section)?;
        types.ty().function([ValType::I32; 4], [ValType::I32]);
        if !self.posts.is_empty() {
            types.ty().function([ValType::I32], []);
        }
        Ok(())
    }
    fn parse_function_section(
        &mut self,
        functions: &mut FunctionSection,
        section: wasmparser::FunctionSectionReader<'_>,
    ) -> std::result::Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_function_section(self, functions, section)?;
        functions.function(self.ty);
        if self.canonical.is_some() {
            functions.function(self.ty);
        }
        for _ in self.posts {
            functions.function(self.ty + 1);
        }
        Ok(())
    }
    fn parse_global_section(
        &mut self,
        globals: &mut GlobalSection,
        section: wasmparser::GlobalSectionReader<'_>,
    ) -> std::result::Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_global_section(self, globals, section)?;
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(self.start as i32),
        );
        Ok(())
    }
    fn parse_export_section(
        &mut self,
        exports: &mut ExportSection,
        section: wasmparser::ExportSectionReader<'_>,
    ) -> std::result::Result<(), reencode::Error<Self::Error>> {
        for export in section {
            let export = export?;
            if let Some(index) = self
                .posts
                .iter()
                .position(|plan| export.name == format!("cabi_post_{}", plan.export))
            {
                exports.export(
                    export.name,
                    ExportKind::Func,
                    self.function + 2 + index as u32,
                );
            } else {
                exports.export(
                    export.name,
                    self.export_kind(export.kind)?,
                    self.external_index(export.kind, export.index)?,
                );
            }
        }
        exports.export(
            "cabi_realloc",
            ExportKind::Func,
            self.function + u32::from(self.canonical.is_some()),
        );
        Ok(())
    }
    fn parse_code_section(
        &mut self,
        code: &mut CodeSection,
        section: wasmparser::CodeSectionReader<'_>,
    ) -> std::result::Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_code_section(self, code, section)?;
        use Instruction::*;
        if let Some(arena) = self.canonical {
            code.function(&canonical_allocator(arena));
        }
        let mut f = Function::new([(1, ValType::I32)]);
        if self.canonical.is_some() {
            // Adapter storage is available before Go initializes. Later requests
            // use the pure core allocator, which never enters Go or WASI.
            for op in [
                GlobalGet(self.global),
                I32Const(self.end as i32),
                I32GeU,
                If(BlockType::Empty),
                LocalGet(0),
                LocalGet(1),
                LocalGet(2),
                LocalGet(3),
                Call(self.function),
                Return,
                End,
            ] {
                f.instruction(&op);
            }
        }
        // The pinned adapter only allocates whole pages, once for its stack and
        // once for State. Reject reallocations and any request outside this ABI.
        for op in [
            LocalGet(0),
            LocalGet(1),
            I32Or,
            LocalGet(3),
            I32Const(PAGE as i32),
            I32Ne,
            I32Or,
            LocalGet(2),
            I32Const(PAGE as i32),
            I32GtU,
            I32Or,
            LocalGet(2),
            I32Eqz,
            I32Or,
            If(BlockType::Empty),
            Unreachable,
            End,
            GlobalGet(self.global),
            LocalTee(4),
            I32Const(PAGE as i32),
            I32Add,
            I32Const(self.end as i32),
            I32GtU,
            If(BlockType::Empty),
            Unreachable,
            End,
            LocalGet(4),
            I32Const(PAGE as i32),
            I32Add,
            GlobalSet(self.global),
            LocalGet(4),
            End,
        ] {
            f.instruction(&op);
        }
        code.function(&f);
        for plan in self.posts {
            code.function(&post_return(plan, self.function));
        }
        Ok(())
    }
}
pub fn reserve(bytes: &[u8]) -> Result<Vec<u8>> {
    reserve_values(bytes, &[])
}
pub fn reserve_values(bytes: &[u8], posts: &[PostReturn]) -> Result<Vec<u8>> {
    ensure!(posts.len() <= 64, "Go post-return function limit");
    let mut planned = std::collections::BTreeSet::new();
    for plan in posts {
        ensure!(
            planned.insert(format!("cabi_post_{}", plan.export)),
            "duplicate Go post-return plan"
        );
        ensure!(
            plan.size > 0 && plan.alignment.is_power_of_two() && plan.alignment <= PAGE,
            "invalid Go post-return root layout"
        );
        validate_cleanup(&plan.cleanup, plan.size, 0, &mut 0)?;
    }
    let mut existing_posts = std::collections::BTreeSet::new();
    let (mut types, mut functions, mut globals, mut memory_pages) = (0, 0, 0, 0);
    let (mut begin, mut end) = (Vec::new(), Vec::new());
    let (mut canonical_begin, mut canonical_end) = (Vec::new(), Vec::new());
    let mut dynamic = None;
    for payload in Parser::new(0).parse_all(bytes) {
        match payload? {
            Payload::TypeSection(reader) => {
                for ty in reader.into_iter_err_on_gc_types() {
                    ty?;
                    types += 1;
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    match import?.ty {
                        TypeRef::Func(_) => functions += 1,
                        TypeRef::Global(_) => globals += 1,
                        TypeRef::Memory(_) => anyhow::bail!("Go reactor cannot import its memory"),
                        _ => (),
                    }
                }
            }
            Payload::FunctionSection(reader) => functions += reader.count(),
            Payload::GlobalSection(reader) => globals += reader.count(),
            Payload::MemorySection(reader) => {
                ensure!(reader.count() == 1, "Go reactor requires one memory");
                memory_pages = reader.into_iter().next().context("Go memory")??.initial;
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export?;
                    ensure!(
                        export.name != "cabi_realloc",
                        "reserved Go export cabi_realloc"
                    );
                    if export.name.starts_with("cabi_post_") {
                        ensure!(
                            export.kind == wasmparser::ExternalKind::Func,
                            "invalid Go post-return export"
                        );
                        existing_posts.insert(export.name.to_owned());
                    }
                    if export.name == "wisp-cabi-realloc" {
                        ensure!(
                            export.kind == wasmparser::ExternalKind::Func
                                && dynamic.replace(export.index).is_none(),
                            "invalid Go canonical allocator export"
                        );
                    }
                }
            }
            Payload::DataSection(reader) => {
                for data in reader {
                    let data = data?;
                    let DataKind::Active {
                        memory_index: 0,
                        offset_expr,
                    } = data.kind
                    else {
                        anyhow::bail!("unsupported Go data segment")
                    };
                    let mut ops = offset_expr.get_operators_reader();
                    let Operator::I32Const { value } = ops.read()? else {
                        anyhow::bail!("unsupported Go data address")
                    };
                    ensure!(
                        matches!(ops.read()?, Operator::End),
                        "unsupported Go data address"
                    );
                    let base = value as u32;
                    for (offset, window) in data.data.windows(BEGIN.len()).enumerate() {
                        if window == BEGIN {
                            begin.push(
                                base.checked_add(offset as u32)
                                    .context("arena address overflow")?,
                            );
                        }
                    }
                    for (marker, matches) in [
                        (CANONICAL_BEGIN.as_slice(), &mut canonical_begin),
                        (CANONICAL_END.as_slice(), &mut canonical_end),
                    ] {
                        for (offset, window) in data.data.windows(marker.len()).enumerate() {
                            if window == marker {
                                matches.push(
                                    base.checked_add(offset as u32)
                                        .context("canonical arena address overflow")?,
                                );
                            }
                        }
                    }
                    for (offset, window) in data.data.windows(END.len()).enumerate() {
                        if window == END {
                            end.push(
                                base.checked_add(offset as u32)
                                    .context("arena address overflow")?,
                            );
                        }
                    }
                }
            }
            _ => (),
        }
    }
    ensure!(
        begin.len() == 1 && end.len() == 1,
        "cannot locate unique reserved Go adapter arena"
    );
    let base = begin[0];
    let limit = base.checked_add(ARENA).context("Go arena overflow")?;
    ensure!(
        end[0].checked_add(END.len() as u32) == Some(limit)
            && u64::from(limit) <= memory_pages * PAGE as u64,
        "invalid Go adapter arena bounds"
    );
    let start = base
        .checked_add(PAGE - 1)
        .context("Go arena alignment overflow")?
        & !(PAGE - 1);
    ensure!(
        start.checked_add(2 * PAGE).is_some_and(|v| v <= limit)
            && limit < i32::MAX as u32
            && types > 0
            && functions > 0
            && globals > 0,
        "Go adapter arena must contain two aligned pages"
    );
    ensure!(
        existing_posts == planned,
        "Go owned-value post-return plans differ from exports"
    );
    ensure!(
        posts.is_empty() || dynamic.is_some(),
        "Go post-return cleanup requires the canonical arena"
    );
    let canonical = if dynamic.is_some() {
        ensure!(
            canonical_begin.len() == 1 && canonical_end.len() == 1,
            "cannot locate unique Go canonical arena"
        );
        let control = canonical_begin[0]
            .checked_add(32)
            .context("canonical control overflow")?;
        let limit = canonical_begin[0]
            .checked_add(CANONICAL_SIZE - 32)
            .context("canonical arena overflow")?;
        ensure!(
            canonical_end[0] == limit
                && u64::from(limit) + 32 <= memory_pages * PAGE as u64
                && limit < (i32::MAX as u32) - PAGE
                && (limit + 32 <= base || canonical_begin[0] >= base + ARENA),
            "invalid or overlapping Go canonical arena bounds"
        );
        Some(CanonicalArena { control, limit })
    } else {
        ensure!(
            canonical_begin.is_empty() && canonical_end.is_empty(),
            "Go canonical arena requires its marker export"
        );
        None
    };
    let mut module = Module::new();
    Allocator {
        ty: types,
        function: functions,
        global: globals,
        start,
        end: start + 2 * PAGE,
        canonical,
        posts,
    }
    .parse_core_module(&mut module, Parser::new(0), bytes)?;
    let result = module.finish();
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn module(corrupt: bool) -> Vec<u8> {
        let mut m = Module::new();
        let mut types = TypeSection::new();
        types.ty().function([], []);
        m.section(&types);
        let mut funcs = FunctionSection::new();
        funcs.function(0);
        m.section(&funcs);
        let mut memory = MemorySection::new();
        memory.memory(MemoryType {
            minimum: 5,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        m.section(&memory);
        let mut globals = GlobalSection::new();
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        m.section(&globals);
        let mut exports = ExportSection::new();
        exports.export("probe", ExportKind::Func, 0);
        m.section(&exports);
        let mut code = CodeSection::new();
        let mut f = Function::new([]);
        f.instruction(&Instruction::End);
        code.function(&f);
        m.section(&code);
        let mut data = DataSection::new();
        data.active(0, &ConstExpr::i32_const(4096), BEGIN.iter().copied());
        data.active(
            0,
            &ConstExpr::i32_const(
                4096 + ARENA as i32 - END.len() as i32 + if corrupt { 1 } else { 0 },
            ),
            END.iter().copied(),
        );
        m.section(&data);
        m.finish()
    }
    #[test]
    fn reserves_static_adapter_storage_and_preserves_valid_wasm() {
        let result = reserve(&module(false)).unwrap();
        wasmparser::Validator::new().validate_all(&result).unwrap();
        assert!(Parser::new(0).parse_all(&result).any(|p|matches!(p,Ok(Payload::ExportSection(reader)) if reader.clone().into_iter().any(|e|e.unwrap().name=="cabi_realloc"))));
    }
    #[test]
    fn rejects_missing_or_corrupted_arena_boundaries() {
        assert!(
            reserve(&module(true))
                .unwrap_err()
                .to_string()
                .contains("arena bounds")
        );
        assert!(
            reserve(&Module::new().finish())
                .unwrap_err()
                .to_string()
                .contains("unique reserved")
        );
    }
    fn values_module() -> Vec<u8> {
        let mut m = Module::new();
        let mut types = TypeSection::new();
        types.ty().function([], []);
        types.ty().function([ValType::I32; 4], [ValType::I32]);
        types.ty().function([ValType::I32], []);
        m.section(&types);
        let mut funcs = FunctionSection::new();
        funcs.function(0);
        funcs.function(1);
        funcs.function(2);
        m.section(&funcs);
        let mut memory = MemorySection::new();
        memory.memory(MemoryType {
            minimum: 80,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        m.section(&memory);
        let mut globals = GlobalSection::new();
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        m.section(&globals);
        let mut exports = ExportSection::new();
        exports.export("_initialize", ExportKind::Func, 0);
        exports.export("wisp-cabi-realloc", ExportKind::Func, 1);
        exports.export("cabi_post_api#echo", ExportKind::Func, 2);
        exports.export("memory", ExportKind::Memory, 0);
        m.section(&exports);
        let mut code = CodeSection::new();
        let mut empty = Function::new([]);
        empty.instruction(&Instruction::End);
        code.function(&empty);
        for _ in 0..2 {
            let mut trap = Function::new([]);
            trap.instruction(&Instruction::Unreachable);
            trap.instruction(&Instruction::End);
            code.function(&trap);
        }
        m.section(&code);
        let mut data = DataSection::new();
        for (address, marker) in [
            (4096, BEGIN),
            (4096 + ARENA - END.len() as u32, END),
            (4 * PAGE, CANONICAL_BEGIN.as_slice()),
            (4 * PAGE + CANONICAL_SIZE - 32, CANONICAL_END.as_slice()),
        ] {
            data.active(
                0,
                &ConstExpr::i32_const(address as i32),
                marker.iter().copied(),
            );
        }
        m.section(&data);
        m.finish()
    }
    fn post_plan() -> PostReturn {
        PostReturn {
            export: "api#echo".into(),
            size: 8,
            alignment: 4,
            cleanup: vec![Cleanup::Buffer { offset: 0 }],
        }
    }
    #[test]
    fn canonical_alloc_and_post_return_cannot_call_go_or_wasi() {
        let output = reserve_values(&values_module(), &[post_plan()]).unwrap();
        wasmparser::Validator::new().validate_all(&output).unwrap();
        let mut index = 0;
        let mut exports = std::collections::BTreeMap::new();
        for payload in Parser::new(0).parse_all(&output) {
            match payload.unwrap() {
                Payload::ExportSection(reader) => {
                    for export in reader {
                        let export = export.unwrap();
                        exports.insert(export.name.to_owned(), export.index);
                    }
                }
                Payload::CodeSectionEntry(body) => {
                    if index >= 3 {
                        for op in body.get_operators_reader().unwrap() {
                            match op.unwrap() {
                                Operator::Call { function_index } => assert!(
                                    index > 3 && function_index == 3,
                                    "injected allocator/cleanup calls outside pure Wasm allocator"
                                ),
                                Operator::CallIndirect { .. }
                                | Operator::CallRef { .. }
                                | Operator::ReturnCall { .. }
                                | Operator::ReturnCallIndirect { .. }
                                | Operator::ReturnCallRef { .. }
                                | Operator::MemoryGrow { .. } => panic!(
                                    "canonical hook can leave static memory or invoke unknown code"
                                ),
                                _ => (),
                            }
                        }
                    }
                    index += 1;
                }
                _ => (),
            }
        }
        assert_eq!(exports["cabi_realloc"], 4);
        assert_eq!(exports["cabi_post_api#echo"], 5);
        assert_eq!(index, 6);
    }
    #[test]
    fn owned_values_require_complete_safe_post_return_plans() {
        assert!(
            reserve(&values_module())
                .unwrap_err()
                .to_string()
                .contains("post-return plans differ")
        );
        let mut invalid = post_plan();
        invalid.cleanup = vec![Cleanup::Buffer { offset: 4 }];
        assert!(
            reserve_values(&values_module(), &[invalid])
                .unwrap_err()
                .to_string()
                .contains("buffer outside result")
        );
        let plan = post_plan();
        assert!(
            reserve_values(&values_module(), &[plan.clone(), plan])
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );
    }
    #[test]
    fn scalar_sources_keep_their_original_static_memory_footprint() {
        assert!(!source().contains("wispCanonicalArena"));
        assert!(values_source().contains("wispCanonicalArena = [4194304]byte"));
        assert!(values_source().contains("&wispCanonicalArena[32]"));
    }
}
