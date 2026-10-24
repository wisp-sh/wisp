use crate::runtime::Session;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use wasmtime::component::{Type, Val};

/// Reject transfers whose ownership lifecycle is not implemented by the IPC API.
/// Walk types, not values: an absent option or empty list must not hide an
/// unsupported transfer in a function's signature.
pub fn ensure_transfer_supported(ty: &Type) -> Result<()> {
    match ty {
        Type::Future(_) => bail!("future transfer is not supported by the worker protocol"),
        Type::Stream(_) => bail!("stream transfer is not supported by the worker protocol"),
        Type::List(list) => ensure_transfer_supported(&list.ty())?,
        Type::FixedLengthList(list) => ensure_transfer_supported(&list.ty())?,
        Type::Map(map) => {
            ensure_transfer_supported(&map.key())?;
            ensure_transfer_supported(&map.value())?;
        }
        Type::Tuple(tuple) => {
            for ty in tuple.types() {
                ensure_transfer_supported(&ty)?;
            }
        }
        Type::Record(record) => {
            for field in record.fields() {
                ensure_transfer_supported(&field.ty)?;
            }
        }
        Type::Option(option) => ensure_transfer_supported(&option.ty())?,
        Type::Result(result) => {
            for ty in result.ok().into_iter().chain(result.err()) {
                ensure_transfer_supported(&ty)?;
            }
        }
        Type::Variant(variant) => {
            for ty in variant.cases().filter_map(|case| case.ty) {
                ensure_transfer_supported(&ty)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn core_params(types: Vec<wasmtime::ValType>, values: &[Value]) -> Result<Vec<wasmtime::Val>> {
    ensure!(
        types.len() == values.len(),
        "incorrect core parameter count"
    );
    types
        .into_iter()
        .zip(values)
        .map(|(ty, value)| {
            Ok(match ty {
                wasmtime::ValType::I32 => {
                    wasmtime::Val::I32(value.as_i64().context("expected i32")?.try_into()?)
                }
                wasmtime::ValType::I64 => {
                    wasmtime::Val::I64(value.as_i64().context("expected i64")?)
                }
                wasmtime::ValType::F32 => {
                    wasmtime::Val::F32((value.as_f64().context("expected f32")? as f32).to_bits())
                }
                wasmtime::ValType::F64 => {
                    wasmtime::Val::F64(value.as_f64().context("expected f64")?.to_bits())
                }
                _ => bail!("unsupported core parameter type"),
            })
        })
        .collect()
}
pub fn core_json(value: &wasmtime::Val) -> Result<Value> {
    Ok(match value {
        wasmtime::Val::I32(v) => json!(v),
        wasmtime::Val::I64(v) => json!(v),
        wasmtime::Val::F32(v) => json!(f32::from_bits(*v)),
        wasmtime::Val::F64(v) => json!(f64::from_bits(*v)),
        _ => bail!("unsupported core result type"),
    })
}
pub fn decode(session: &mut Session, ty: &Type, value: &Value) -> Result<Val> {
    macro_rules! signed {
        ($variant:ident, $target:ty) => {
            Val::$variant(<$target>::try_from(
                value.as_i64().context("expected signed integer")?,
            )?)
        };
    }
    macro_rules! unsigned {
        ($variant:ident, $target:ty) => {
            Val::$variant(<$target>::try_from(
                value.as_u64().context("expected unsigned integer")?,
            )?)
        };
    }
    Ok(match ty {
        Type::Bool => Val::Bool(value.as_bool().context("expected bool")?),
        Type::S8 => signed!(S8, i8),
        Type::S16 => signed!(S16, i16),
        Type::S32 => signed!(S32, i32),
        Type::S64 => signed!(S64, i64),
        Type::U8 => unsigned!(U8, u8),
        Type::U16 => unsigned!(U16, u16),
        Type::U32 => unsigned!(U32, u32),
        Type::U64 => unsigned!(U64, u64),
        Type::Float32 => Val::Float32(if let Some(bits) = value.get("f32") {
            f32::from_bits(bits.as_u64().context("expected f32 bits")?.try_into()?)
        } else {
            value.as_f64().context("expected float")? as f32
        }),
        Type::Float64 => Val::Float64(if let Some(bits) = value.get("f64") {
            f64::from_bits(bits.as_u64().context("expected f64 bits")?)
        } else {
            value.as_f64().context("expected float")?
        }),
        Type::Char => {
            let s = value.as_str().context("expected character")?;
            let mut chars = s.chars();
            let c = chars.next().context("empty character")?;
            ensure!(chars.next().is_none(), "expected one character");
            Val::Char(c)
        }
        Type::String => Val::String(value.as_str().context("expected string")?.into()),
        Type::List(list) => Val::List(
            value
                .as_array()
                .context("expected list")?
                .iter()
                .map(|v| decode(session, &list.ty(), v))
                .collect::<Result<_>>()?,
        ),
        Type::Tuple(tuple) => {
            let values = value.as_array().context("expected tuple")?;
            ensure!(values.len() == tuple.types().len(), "tuple length mismatch");
            Val::Tuple(
                tuple
                    .types()
                    .zip(values)
                    .map(|(t, v)| decode(session, &t, v))
                    .collect::<Result<_>>()?,
            )
        }
        Type::Record(record) => {
            let object = value.as_object().context("expected record")?;
            ensure!(
                object.len() == record.fields().len(),
                "record field count mismatch"
            );
            Val::Record(
                record
                    .fields()
                    .map(|field| {
                        Ok((
                            field.name.to_string(),
                            decode(
                                session,
                                &field.ty,
                                object.get(field.name).context("missing record field")?,
                            )?,
                        ))
                    })
                    .collect::<Result<_>>()?,
            )
        }
        Type::Enum(_) => Val::Enum(value.as_str().context("expected enum name")?.into()),
        Type::Flags(_) => Val::Flags(
            value
                .as_array()
                .context("expected flags")?
                .iter()
                .map(|v| Ok(v.as_str().context("expected flag name")?.into()))
                .collect::<Result<_>>()?,
        ),
        Type::Option(option) => {
            let object = value.as_object().context("expected tagged option")?;
            ensure!(
                object.len() == 1,
                "option must contain exactly one of none or some"
            );
            Val::Option(if let Some(value) = object.get("some") {
                Some(Box::new(decode(session, &option.ty(), value)?))
            } else {
                ensure!(
                    object.get("none") == Some(&Value::Null),
                    "expected option some or none with null payload"
                );
                None
            })
        }
        Type::Result(result) => {
            let object = value.as_object().context("expected result")?;
            ensure!(object.len() == 1, "result must contain ok or err");
            if let Some(v) = object.get("ok") {
                Val::Result(Ok(result
                    .ok()
                    .map(|t| decode(session, &t, v).map(Box::new))
                    .transpose()?))
            } else {
                let v = object.get("err").context("expected result ok or err")?;
                Val::Result(Err(result
                    .err()
                    .map(|t| decode(session, &t, v).map(Box::new))
                    .transpose()?))
            }
        }
        Type::Variant(variant) => {
            let tag = value
                .get("tag")
                .and_then(Value::as_str)
                .context("expected variant tag")?;
            let case = variant
                .cases()
                .find(|c| c.name == tag)
                .context("unknown variant case")?;
            Val::Variant(
                tag.into(),
                case.ty
                    .map(|t| {
                        decode(session, &t, value.get("value").unwrap_or(&Value::Null))
                            .map(Box::new)
                    })
                    .transpose()?,
            )
        }
        Type::Future(_) => bail!("future transfer is not supported by the worker protocol"),
        Type::Stream(_) => bail!("stream transfer is not supported by the worker protocol"),
        Type::Own(_) | Type::Borrow(_) | Type::ErrorContext => {
            let id = value
                .get("handle")
                .and_then(Value::as_u64)
                .context("expected handle")?;
            ensure!(
                value.get("instance").and_then(Value::as_u64) == Some(session.owner),
                "foreign handle"
            );
            let handle = session
                .handles
                .get(&id)
                .context("closed or foreign handle")?
                .clone();
            match (ty, &handle) {
                (Type::Own(expected) | Type::Borrow(expected), Val::Resource(resource)) => {
                    ensure!(resource.ty() == *expected, "resource handle type mismatch");
                    ensure!(
                        !matches!(ty, Type::Own(_)) || resource.owned(),
                        "cannot transfer ownership of a borrowed resource"
                    );
                }
                (Type::ErrorContext, Val::ErrorContext(_)) => {}
                _ => bail!("handle type mismatch"),
            }
            if matches!(ty, Type::Own(_)) {
                session.handles.remove(&id);
            }
            handle
        }
        _ => bail!("unsupported Component value type"),
    })
}
pub fn encode(session: &mut Session, value: Val) -> Result<Value> {
    Ok(match value {
        Val::Bool(v) => json!(v),
        Val::S8(v) => json!(v),
        Val::S16(v) => json!(v),
        Val::S32(v) => json!(v),
        Val::S64(v) => json!(v),
        Val::U8(v) => json!(v),
        Val::U16(v) => json!(v),
        Val::U32(v) => json!(v),
        Val::U64(v) => json!(v),
        Val::Float32(v) => json!({"f32":v.to_bits()}),
        Val::Float64(v) => json!({"f64":v.to_bits()}),
        Val::Char(v) => json!(v.to_string()),
        Val::String(v) | Val::Enum(v) => json!(v),
        Val::Flags(v) => json!(v),
        Val::List(v) | Val::Tuple(v) | Val::FixedLengthList(v) => Value::Array(
            v.into_iter()
                .map(|v| encode(session, v))
                .collect::<Result<_>>()?,
        ),
        Val::Record(v) => Value::Object(
            v.into_iter()
                .map(|(k, v)| Ok((k, encode(session, v)?)))
                .collect::<Result<_>>()?,
        ),
        Val::Option(None) => json!({"none": null}),
        Val::Option(Some(v)) => json!({"some": encode(session, *v)?}),
        Val::Result(Ok(v)) => json!({"ok":v.map(|v|encode(session,*v)).transpose()?}),
        Val::Result(Err(v)) => json!({"err":v.map(|v|encode(session,*v)).transpose()?}),
        Val::Variant(tag, v) => {
            json!({"tag":tag,"value":v.map(|v|encode(session,*v)).transpose()?})
        }
        Val::Future(_) => bail!("future transfer is not supported by the worker protocol"),
        Val::Stream(_) => bail!("stream transfer is not supported by the worker protocol"),
        handle @ (Val::Resource(_) | Val::ErrorContext(_)) => {
            let id = session.next_handle;
            session.next_handle += 1;
            session.handles.insert(id, handle);
            json!({"instance":session.owner,"handle":id})
        }
        _ => bail!("unsupported Component result"),
    })
}
