// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
};

use hashbrown::{HashMap, hash_map::RawEntryMut};
use message_format_runtime::schema;

use crate::semantic::{
    CallExpr, FunctionOption, FunctionOptionValue, FunctionSpec, Operand, Part, SelectExpr,
    SelectorExpr,
};

use super::interning::{FunctionCatalogKey, function_catalog_key};
use super::{
    CompileError, LiteralDeduplication, LiteralStats, escape_fallback_literal,
    function_dynamic_options,
};

#[derive(Debug)]
pub(super) struct LiteralPool {
    mode: LiteralDeduplication,
    bytes: String,
    offsets: HashMap<LiteralSpan, ()>,
    stats: LiteralStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct LiteralSpan {
    off: u32,
    len: u32,
}

impl LiteralPool {
    pub(super) fn new(mode: LiteralDeduplication) -> Self {
        Self {
            mode,
            bytes: String::new(),
            offsets: HashMap::new(),
            stats: LiteralStats {
                deduplication: mode,
                ..LiteralStats::default()
            },
        }
    }

    pub(super) fn intern(&mut self, value: &str) -> Result<(u32, u32), CompileError> {
        let len =
            u32::try_from(value.len()).map_err(|_| CompileError::size_overflow("literal data"))?;
        self.stats.literal_slices += 1;
        self.stats.input_literal_bytes += value.len();

        if value.is_empty() {
            return Ok((0, len));
        }

        if self.mode == LiteralDeduplication::Disabled {
            return self.append(value, len);
        }

        let hash = literal_hash(value);
        match self
            .offsets
            .raw_entry_mut()
            .from_hash(hash, |span| span_matches(self.bytes.as_str(), *span, value))
        {
            RawEntryMut::Occupied(entry) => {
                self.stats.duplicate_literals += 1;
                self.stats.duplicate_literal_bytes += value.len();
                if self.mode == LiteralDeduplication::Enabled {
                    self.stats.saved_literal_bytes += value.len();
                    return Ok((entry.key().off, len));
                }
            }
            RawEntryMut::Vacant(entry) => {
                let (offset, len) = append_literal(&mut self.bytes, &mut self.stats, value, len)?;
                entry.insert_with_hasher(hash, LiteralSpan { off: offset, len }, (), |span| {
                    literal_hash(
                        span_text(self.bytes.as_str(), *span)
                            .expect("literal span must reference appended bytes"),
                    )
                });
                self.stats.unique_literals += 1;
                self.stats.unique_literal_bytes += value.len();
                return Ok((offset, len));
            }
        }

        self.append(value, len)
    }

    fn append(&mut self, value: &str, len: u32) -> Result<(u32, u32), CompileError> {
        append_literal(&mut self.bytes, &mut self.stats, value, len)
    }

    pub(super) fn into_parts(self) -> (String, LiteralStats) {
        (self.bytes, self.stats)
    }
}

fn span_matches(bytes: &str, span: LiteralSpan, value: &str) -> bool {
    span_text(bytes, span) == Some(value)
}

fn span_text(bytes: &str, span: LiteralSpan) -> Option<&str> {
    let start = span.off as usize;
    let len = span.len as usize;
    let end = start.checked_add(len)?;
    bytes.get(start..end)
}

fn append_literal(
    bytes: &mut String,
    stats: &mut LiteralStats,
    value: &str,
    len: u32,
) -> Result<(u32, u32), CompileError> {
    let offset =
        u32::try_from(bytes.len()).map_err(|_| CompileError::size_overflow("literal data"))?;
    bytes.push_str(value);
    stats.emitted_literal_bytes += value.len();
    Ok((offset, len))
}

fn literal_hash(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Compute the fallback string for a call part.
fn compute_fallback(part: &Part) -> String {
    match part {
        Part::Call(CallExpr {
            operand,
            func,
            fallback,
        }) => fallback
            .clone()
            .unwrap_or_else(|| render_operand_fallback(operand, func)),
        _ => String::new(),
    }
}

/// Emit an `OP_EXPR_FALLBACK` instruction before a call in the output path.
fn emit_expr_fallback(
    part: &Part,
    string_map: &BTreeMap<String, u32>,
    code: &mut Vec<u8>,
) -> Result<(), CompileError> {
    let fb = compute_fallback(part);
    let fb_str_id = *string_map
        .get(&fb)
        .ok_or(CompileError::internal("missing interned string"))?;
    code.push(schema::OP_EXPR_FALLBACK);
    code.extend_from_slice(&fb_str_id.to_le_bytes());
    Ok(())
}

pub(super) fn lower_parts(
    parts: &[Part],
    string_map: &BTreeMap<String, u32>,
    func_map: &BTreeMap<FunctionCatalogKey, u16>,
    literals: &mut LiteralPool,
    code: &mut Vec<u8>,
) -> Result<(), CompileError> {
    for part in parts {
        match part {
            Part::Text(value) => {
                let (off, len) = literals.intern(value)?;
                code.push(schema::OP_OUT_SLICE);
                code.extend_from_slice(&off.to_le_bytes());
                code.extend_from_slice(&len.to_le_bytes());
            }
            Part::Literal(value) => {
                let (off, len) = literals.intern(value)?;
                code.push(schema::OP_OUT_EXPR);
                code.extend_from_slice(&off.to_le_bytes());
                code.extend_from_slice(&len.to_le_bytes());
            }
            Part::Var(name) => {
                let str_id = *string_map
                    .get(name)
                    .ok_or(CompileError::internal("missing interned variable"))?;
                code.push(schema::OP_OUT_ARG);
                code.extend_from_slice(&str_id.to_le_bytes());
            }
            Part::Call(CallExpr { operand, func, .. }) => {
                let func_key = function_catalog_key(func);
                let fn_id = *func_map
                    .get(&func_key)
                    .ok_or(CompileError::internal("missing function entry"))?;
                let dynamic_options = function_dynamic_options(func);

                emit_operand(operand, string_map, code)?;
                for (key, value) in dynamic_options.iter().copied() {
                    let key_str_id = *string_map
                        .get(key)
                        .ok_or(CompileError::internal("missing interned string"))?;
                    code.push(schema::OP_PUSH_CONST);
                    code.extend_from_slice(&key_str_id.to_le_bytes());
                    let var_str_id = *string_map
                        .get(value)
                        .ok_or(CompileError::internal("missing interned variable"))?;
                    code.push(schema::OP_LOAD_ARG);
                    code.extend_from_slice(&var_str_id.to_le_bytes());
                }
                emit_expr_fallback(part, string_map, code)?;
                code.push(schema::OP_CALL_FUNC);
                code.extend_from_slice(&fn_id.to_le_bytes());
                code.push(1);
                code.push(
                    u8::try_from(dynamic_options.len())
                        .map_err(|_| CompileError::size_overflow("option count"))?,
                );
                code.push(schema::OP_OUT_VAL);
            }
            Part::MarkupOpen { name, options } => {
                emit_markup_options(options, string_map, code)?;
                let name_str_id = *string_map
                    .get(name)
                    .ok_or(CompileError::internal("missing interned markup name"))?;
                code.push(schema::OP_MARKUP_OPEN);
                code.extend_from_slice(&name_str_id.to_le_bytes());
                code.push(
                    u8::try_from(options.len())
                        .map_err(|_| CompileError::size_overflow("option count"))?,
                );
            }
            Part::MarkupClose { name, options } => {
                emit_markup_options(options, string_map, code)?;
                let name_str_id = *string_map
                    .get(name)
                    .ok_or(CompileError::internal("missing interned markup name"))?;
                code.push(schema::OP_MARKUP_CLOSE);
                code.extend_from_slice(&name_str_id.to_le_bytes());
                code.push(
                    u8::try_from(options.len())
                        .map_err(|_| CompileError::size_overflow("option count"))?,
                );
            }
            Part::Select(select) => {
                lower_select(select, string_map, func_map, literals, code)?;
            }
        }
    }

    Ok(())
}

fn emit_markup_options(
    options: &[FunctionOption],
    string_map: &BTreeMap<String, u32>,
    code: &mut Vec<u8>,
) -> Result<(), CompileError> {
    for option in options {
        let key_str_id = *string_map
            .get(&option.key)
            .ok_or(CompileError::internal("missing interned string"))?;
        code.push(schema::OP_PUSH_CONST);
        code.extend_from_slice(&key_str_id.to_le_bytes());
        match &option.value {
            FunctionOptionValue::Literal(value) => {
                let value_str_id = *string_map
                    .get(value)
                    .ok_or(CompileError::internal("missing interned string"))?;
                code.push(schema::OP_PUSH_CONST);
                code.extend_from_slice(&value_str_id.to_le_bytes());
            }
            FunctionOptionValue::Var(var) => {
                let var_str_id = *string_map
                    .get(var)
                    .ok_or(CompileError::internal("missing interned variable"))?;
                code.push(schema::OP_LOAD_ARG);
                code.extend_from_slice(&var_str_id.to_le_bytes());
            }
        }
    }
    Ok(())
}

fn lower_select(
    select: &SelectExpr,
    string_map: &BTreeMap<String, u32>,
    func_map: &BTreeMap<FunctionCatalogKey, u16>,
    literals: &mut LiteralPool,
    code: &mut Vec<u8>,
) -> Result<(), CompileError> {
    emit_selector_start(&select.selector, string_map, func_map, code)?;

    let mut dispatch_patches = Vec::new();
    for (arm_idx, arm) in select.arms.iter().enumerate() {
        let key_str_id = *string_map
            .get(&arm.key)
            .ok_or(CompileError::internal("missing interned string"))?;
        code.push(schema::OP_CASE_STR);
        code.extend_from_slice(&key_str_id.to_le_bytes());
        let rel_pos = code.len();
        code.extend_from_slice(&0_i32.to_le_bytes());
        dispatch_patches.push((rel_pos, arm_idx));
    }

    code.push(schema::OP_CASE_DEFAULT);
    let default_rel_pos = code.len();
    code.extend_from_slice(&0_i32.to_le_bytes());

    let mut arm_starts = vec![0_u32; select.arms.len()];
    let mut end_jump_patch_positions = Vec::new();

    for (arm_idx, arm) in select.arms.iter().enumerate() {
        arm_starts[arm_idx] = u32::try_from(code.len())
            .map_err(|_| CompileError::size_overflow("bytecode program counter"))?;
        lower_parts(&arm.parts, string_map, func_map, literals, code)?;
        code.push(schema::OP_JMP);
        let rel_pos = code.len();
        code.extend_from_slice(&0_i32.to_le_bytes());
        end_jump_patch_positions.push(rel_pos);
    }

    let default_start = u32::try_from(code.len())
        .map_err(|_| CompileError::size_overflow("bytecode program counter"))?;
    lower_parts(&select.default, string_map, func_map, literals, code)?;
    let end_pc = u32::try_from(code.len())
        .map_err(|_| CompileError::size_overflow("bytecode program counter"))?;
    code.push(schema::OP_SELECT_END);

    for (rel_pos, arm_idx) in dispatch_patches {
        patch_rel32(code, rel_pos, arm_starts[arm_idx])?;
    }
    patch_rel32(code, default_rel_pos, default_start)?;
    for rel_pos in end_jump_patch_positions {
        patch_rel32(code, rel_pos, end_pc)?;
    }

    Ok(())
}

fn emit_selector_start(
    selector: &SelectorExpr,
    string_map: &BTreeMap<String, u32>,
    func_map: &BTreeMap<FunctionCatalogKey, u16>,
    code: &mut Vec<u8>,
) -> Result<(), CompileError> {
    match selector {
        SelectorExpr::Var(name) => {
            let str_id = *string_map
                .get(name)
                .ok_or(CompileError::internal("missing interned variable"))?;
            code.push(schema::OP_SELECT_ARG);
            code.extend_from_slice(&str_id.to_le_bytes());
            Ok(())
        }
        SelectorExpr::Call {
            operand: Operand::Var(name),
            func,
        } if func.name == "string" && func.options.is_empty() => {
            let str_id = *string_map
                .get(name)
                .ok_or(CompileError::internal("missing interned variable"))?;
            code.push(schema::OP_SELECT_ARG);
            code.extend_from_slice(&str_id.to_le_bytes());
            Ok(())
        }
        _ => {
            lower_selector(selector, string_map, func_map, code)?;
            code.push(schema::OP_SELECT_BEGIN);
            Ok(())
        }
    }
}

fn lower_selector(
    selector: &SelectorExpr,
    string_map: &BTreeMap<String, u32>,
    func_map: &BTreeMap<FunctionCatalogKey, u16>,
    code: &mut Vec<u8>,
) -> Result<(), CompileError> {
    match selector {
        SelectorExpr::Var(name) => {
            emit_operand(&Operand::Var(name.clone()), string_map, code)?;
            Ok(())
        }
        SelectorExpr::Call { operand, func } => {
            if func.name == "string" && func.options.is_empty() {
                emit_operand(operand, string_map, code)?;
                return Ok(());
            }
            let func_key = function_catalog_key(func);
            let fn_id = *func_map
                .get(&func_key)
                .ok_or(CompileError::internal("missing function entry"))?;
            let dynamic_options = function_dynamic_options(func);
            emit_operand(operand, string_map, code)?;
            for (key, value) in dynamic_options.iter().copied() {
                let key_str_id = *string_map
                    .get(key)
                    .ok_or(CompileError::internal("missing interned string"))?;
                code.push(schema::OP_PUSH_CONST);
                code.extend_from_slice(&key_str_id.to_le_bytes());
                let value_var_str_id = *string_map
                    .get(value)
                    .ok_or(CompileError::internal("missing interned variable"))?;
                code.push(schema::OP_LOAD_ARG);
                code.extend_from_slice(&value_var_str_id.to_le_bytes());
            }
            // No OP_EXPR_FALLBACK for selectors — errors abort.
            code.push(schema::OP_CALL_SELECT);
            code.extend_from_slice(&fn_id.to_le_bytes());
            code.push(1);
            code.push(
                u8::try_from(dynamic_options.len())
                    .map_err(|_| CompileError::size_overflow("option count"))?,
            );
            Ok(())
        }
        SelectorExpr::Literal(value) => {
            code.push(schema::OP_PUSH_CONST);
            let value_str_id = *string_map
                .get(value)
                .ok_or(CompileError::internal("missing interned string"))?;
            code.extend_from_slice(&value_str_id.to_le_bytes());
            Ok(())
        }
    }
}

fn emit_operand(
    operand: &Operand,
    string_map: &BTreeMap<String, u32>,
    code: &mut Vec<u8>,
) -> Result<(), CompileError> {
    match operand {
        Operand::Var(var) => {
            let var_str_id = *string_map
                .get(var)
                .ok_or(CompileError::internal("missing interned variable"))?;
            code.push(schema::OP_LOAD_ARG);
            code.extend_from_slice(&var_str_id.to_le_bytes());
        }
        Operand::Literal { value, .. } => {
            let value_str_id = *string_map
                .get(value)
                .ok_or(CompileError::internal("missing interned string"))?;
            code.push(schema::OP_PUSH_CONST);
            code.extend_from_slice(&value_str_id.to_le_bytes());
        }
    }
    Ok(())
}

fn render_operand_fallback(operand: &Operand, func: &FunctionSpec) -> String {
    match operand {
        Operand::Var(var) => format!("{{${var}}}"),
        Operand::Literal { value, .. } if value.is_empty() => format!("{{:{}}}", func.name),
        Operand::Literal { value, .. } => format!("{{|{}|}}", escape_fallback_literal(value)),
    }
}

fn patch_rel32(code: &mut [u8], rel_pos: usize, target_pc: u32) -> Result<(), CompileError> {
    let after = rel_pos + 4;
    let target = i64::from(target_pc);
    let after_i64 =
        i64::try_from(after).map_err(|_| CompileError::size_overflow("jump patch offset"))?;
    let rel = target - after_i64;
    if rel < i64::from(i32::MIN) || rel > i64::from(i32::MAX) {
        return Err(CompileError::size_overflow("jump offset"));
    }
    let rel_i32 = i32::try_from(rel).map_err(|_| CompileError::size_overflow("jump offset"))?;
    code[rel_pos..rel_pos + 4].copy_from_slice(&rel_i32.to_le_bytes());
    Ok(())
}
