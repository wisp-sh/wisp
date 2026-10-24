//! Preserve scalar stack-return values across Wago's ARM64 pinned-global writes.
use anyhow::{Context, Result};
use std::collections::VecDeque;
use wasm_encoder::{CodeSection, Function, Instruction, RawSection, ValType};
use wasmparser::{CompositeInnerType, FunctionBody, Operator, Parser, Payload};

fn saved_result(bytes: &[u8], body: &FunctionBody<'_>, params: u32) -> Result<Option<Function>> {
    let mut reader = body.get_operators_reader()?;
    let start = reader.original_position();
    let mut tail = VecDeque::with_capacity(9);
    while !reader.eof() {
        let offset = reader.original_position();
        let op = reader.read()?;
        if tail.len() == 9 {
            tail.pop_front();
        }
        tail.push_back((op, offset));
    }
    let tail = tail.into_iter().collect::<Vec<_>>();
    let [
        (
            Operator::GlobalGet {
                global_index: address,
            },
            _,
        ),
        (load, _),
        (
            Operator::GlobalGet {
                global_index: before,
            },
            after_load,
        ),
        (Operator::I32Const { .. }, _),
        (Operator::I32Add, _),
        (
            Operator::GlobalSet {
                global_index: after,
            },
            _,
        ),
        (Operator::Return, before_return),
        (Operator::Unreachable, _),
        (Operator::End, _),
    ] = tail.as_slice()
    else {
        return Ok(None);
    };
    if address != before || before != after {
        return Ok(None);
    }
    let ty = match load {
        Operator::I32Load { .. } => ValType::I32,
        Operator::I64Load { .. } => ValType::I64,
        Operator::F32Load { .. } => ValType::F32,
        Operator::F64Load { .. } => ValType::F64,
        _ => return Ok(None),
    };
    let mut locals = Vec::new();
    let mut result = params;
    for local in body.get_locals_reader()? {
        let (count, ty) = local?;
        result = result
            .checked_add(count)
            .context("Wago local count overflow")?;
        locals.push((count, ValType::try_from(ty)?));
    }
    locals.push((1, ty));
    let mut function = Function::new(locals);
    // Wago's ARM64 deferred memory reference borrows the pinned SP register.
    // Save the load before global.set changes that register, then return the
    // saved value. The original Component remains the guest/identity artifact.
    function.raw(bytes[start..*after_load].iter().copied());
    function.instruction(&Instruction::LocalSet(result));
    function.raw(bytes[*after_load..*before_return].iter().copied());
    function.instruction(&Instruction::LocalGet(result));
    function.raw(bytes[*before_return..body.range().end].iter().copied());
    Ok(Some(function))
}

fn core(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut params = Vec::new();
    let mut functions = Vec::new();
    let mut code = CodeSection::new();
    let mut sections = Vec::new();
    let mut index = 0;
    let mut changed = false;
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload?;
        if let Some(section) = payload.as_section() {
            sections.push(section);
        }
        match payload {
            Payload::TypeSection(section) => {
                for group in section {
                    for ty in group?.into_types() {
                        params.push(match ty.composite_type.inner {
                            CompositeInnerType::Func(ty) => Some(ty.params().len() as u32),
                            _ => None,
                        });
                    }
                }
            }
            Payload::FunctionSection(section) => {
                functions = section
                    .into_iter()
                    .collect::<std::result::Result<Vec<_>, _>>()?;
            }
            Payload::CodeSectionEntry(body) => {
                let count = functions
                    .get(index)
                    .and_then(|ty| params.get(*ty as usize))
                    .copied()
                    .flatten()
                    .context("Wago function type")?;
                if let Some(function) = saved_result(bytes, &body, count)? {
                    code.function(&function);
                    changed = true;
                } else {
                    code.raw(&bytes[body.range()]);
                }
                index += 1;
            }
            _ => {}
        }
    }
    if !changed {
        return Ok(bytes.to_vec());
    }
    let mut module = wasm_encoder::Module::new();
    for (id, range) in sections {
        if id == 10 {
            module.section(&code);
        } else {
            module.section(&RawSection {
                id,
                data: &bytes[range],
            });
        }
    }
    Ok(module.finish())
}

fn rewrite_component(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut output = wasm_encoder::Component::new();
    let mut depth = 0;
    let mut changed = false;
    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload?;
        match payload {
            Payload::ModuleSection {
                unchecked_range, ..
            } => {
                if depth == 0 {
                    let original = &bytes[unchecked_range];
                    let adapted = core(original)?;
                    changed |= adapted != original;
                    output.section(&RawSection {
                        id: 1,
                        data: &adapted,
                    });
                }
                depth += 1;
            }
            Payload::ComponentSection {
                unchecked_range, ..
            } => {
                if depth == 0 {
                    let original = &bytes[unchecked_range];
                    let adapted = rewrite_component(original)?;
                    changed |= adapted != original;
                    output.section(&RawSection {
                        id: 4,
                        data: &adapted,
                    });
                }
                depth += 1;
            }
            Payload::End(_) if depth > 0 => depth -= 1,
            other if depth == 0 => {
                if let Some((id, range)) = other.as_section() {
                    output.section(&RawSection {
                        id,
                        data: &bytes[range],
                    });
                }
            }
            _ => {}
        }
    }
    if !changed {
        return Ok(bytes.to_vec());
    }
    Ok(output.finish())
}

pub(super) fn component(bytes: &[u8]) -> Result<Vec<u8>> {
    let bytes = rewrite_component(bytes)?;
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)?;
    Ok(bytes)
}
