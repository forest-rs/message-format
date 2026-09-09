// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{collections::BTreeMap, format, string::String, vec, vec::Vec};
use core::hash::BuildHasher;

use hashbrown::{DefaultHashBuilder, HashMap, hash_map::RawEntryMut};

use crate::runtime::schema;

use crate::compiler::semantic::{
    CallExpr, FunctionOption, FunctionOptionValue, FunctionSpec, Operand, Part, SelectExpr,
    SelectorExpr,
};

use super::interning::{FunctionCatalogKey, function_catalog_key};
use super::{
    CompileError, LiteralDeduplication, LiteralStats, escape_fallback_literal,
    function_dynamic_options,
};

pub(super) struct LiteralPool {
    mode: LiteralDeduplication,
    bytes: String,
    offsets: HashMap<LiteralSpan, ()>,
    hash_builder: DefaultHashBuilder,
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
            hash_builder: DefaultHashBuilder::default(),
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

        let hash = hash_str(&self.hash_builder, value);
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
                    hash_str(
                        &self.hash_builder,
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

fn hash_str(builder: &DefaultHashBuilder, value: &str) -> u64 {
    builder.hash_one(value)
}

pub(super) fn lower_parts(
    parts: &[Part],
    string_map: &BTreeMap<String, u32>,
    func_map: &BTreeMap<FunctionCatalogKey, u16>,
    literals: &mut LiteralPool,
    code: &mut Vec<u8>,
) -> Result<(), CompileError> {
    let mut state = LoweringState::default();
    lower_parts_inner(parts, string_map, func_map, literals, code, &mut state).map(|_| ())
}

struct DefaultContinuation<'a> {
    parts: &'a [Part],
    depth: usize,
    jump_sites: Vec<usize>,
}

#[derive(Default)]
struct LoweringState<'a> {
    defaults: Vec<DefaultContinuation<'a>>,
    depth: usize,
}

fn lower_parts_inner<'a>(
    parts: &'a [Part],
    string_map: &BTreeMap<String, u32>,
    func_map: &BTreeMap<FunctionCatalogKey, u16>,
    literals: &mut LiteralPool,
    code: &mut Vec<u8>,
    state: &mut LoweringState<'a>,
) -> Result<bool, CompileError> {
    if let Some((default_index, continuation)) = state
        .defaults
        .iter()
        .enumerate()
        .rev()
        .find(|(_, continuation)| continuation.parts == parts)
    {
        let unwind = state
            .depth
            .checked_sub(continuation.depth)
            .ok_or(CompileError::internal(
                "default continuation outside selector scope",
            ))?;
        for _ in 0..unwind {
            code.push(schema::Opcode::SelectEnd as u8);
        }
        code.push(schema::Opcode::Jmp as u8);
        let rel_pos = code.len();
        code.extend_from_slice(&0_i32.to_le_bytes());
        state.defaults[default_index].jump_sites.push(rel_pos);
        return Ok(true);
    }
    for part in parts {
        match part {
            Part::CheckSelector(slot) => {
                code.push(schema::Opcode::CheckSelector as u8);
                code.extend_from_slice(&slot.to_le_bytes());
            }
            Part::Text(value) => {
                let (off, len) = literals.intern(value)?;
                code.push(schema::Opcode::OutSlice as u8);
                code.extend_from_slice(&off.to_le_bytes());
                code.extend_from_slice(&len.to_le_bytes());
            }
            Part::Literal(value) => {
                let (off, len) = literals.intern(value)?;
                code.push(schema::Opcode::OutExpr as u8);
                code.extend_from_slice(&off.to_le_bytes());
                code.extend_from_slice(&len.to_le_bytes());
            }
            Part::Var(name) => {
                let str_id = *string_map
                    .get(name)
                    .ok_or(CompileError::internal("missing interned variable"))?;
                code.push(schema::Opcode::OutArg as u8);
                code.extend_from_slice(&str_id.to_le_bytes());
            }
            Part::Local(slot) => {
                code.push(schema::Opcode::LoadLocal as u8);
                code.extend_from_slice(&slot.to_le_bytes());
                code.push(schema::Opcode::OutVal as u8);
            }
            Part::Call(call) => {
                emit_call(call, string_map, func_map, code)?;
                code.push(schema::Opcode::OutVal as u8);
            }
            Part::MarkupOpen { name, options } => {
                emit_markup_options(options, string_map, code)?;
                let name_str_id = *string_map
                    .get(name)
                    .ok_or(CompileError::internal("missing interned markup name"))?;
                code.push(schema::Opcode::MarkupOpen as u8);
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
                code.push(schema::Opcode::MarkupClose as u8);
                code.extend_from_slice(&name_str_id.to_le_bytes());
                code.push(
                    u8::try_from(options.len())
                        .map_err(|_| CompileError::size_overflow("option count"))?,
                );
            }
            Part::Select(select) => {
                lower_select(select, string_map, func_map, literals, code, state)?;
            }
            Part::Bind {
                slot,
                fallback,
                value,
            } => {
                emit_value_part(value, string_map, func_map, literals, code)?;
                let fallback_id = *string_map
                    .get(fallback)
                    .ok_or(CompileError::internal("missing declaration fallback"))?;
                code.push(schema::Opcode::ExprFallback as u8);
                code.extend_from_slice(&fallback_id.to_le_bytes());
                code.push(schema::Opcode::StoreLocal as u8);
                code.extend_from_slice(&slot.to_le_bytes());
            }
        }
    }

    Ok(false)
}

fn emit_value_part(
    part: &Part,
    string_map: &BTreeMap<String, u32>,
    func_map: &BTreeMap<FunctionCatalogKey, u16>,
    literals: &mut LiteralPool,
    code: &mut Vec<u8>,
) -> Result<(), CompileError> {
    match part {
        Part::Literal(value) => {
            let value_id = *string_map.get(value).ok_or(CompileError::internal(
                "missing interned declaration literal",
            ))?;
            code.push(schema::Opcode::PushConst as u8);
            code.extend_from_slice(&value_id.to_le_bytes());
        }
        Part::Var(name) => {
            let value_id = *string_map.get(name).ok_or(CompileError::internal(
                "missing interned declaration variable",
            ))?;
            code.push(schema::Opcode::LoadArg as u8);
            code.extend_from_slice(&value_id.to_le_bytes());
        }
        Part::Local(slot) => {
            code.push(schema::Opcode::LoadLocal as u8);
            code.extend_from_slice(&slot.to_le_bytes());
        }
        Part::Call(call) => emit_call(call, string_map, func_map, code)?,
        Part::Bind { .. }
        | Part::CheckSelector(_)
        | Part::Text(_)
        | Part::Select(_)
        | Part::MarkupOpen { .. }
        | Part::MarkupClose { .. } => {
            return Err(CompileError::internal("non-scalar declaration expression"));
        }
    }
    let _ = literals;
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
        code.push(schema::Opcode::PushConst as u8);
        code.extend_from_slice(&key_str_id.to_le_bytes());
        match &option.value {
            FunctionOptionValue::Literal(value) => {
                let value_str_id = *string_map
                    .get(value)
                    .ok_or(CompileError::internal("missing interned string"))?;
                code.push(schema::Opcode::PushConst as u8);
                code.extend_from_slice(&value_str_id.to_le_bytes());
            }
            FunctionOptionValue::Var(var) => {
                let var_str_id = *string_map
                    .get(var)
                    .ok_or(CompileError::internal("missing interned variable"))?;
                code.push(schema::Opcode::LoadArg as u8);
                code.extend_from_slice(&var_str_id.to_le_bytes());
            }
            FunctionOptionValue::ResolvedVar { value, .. } => {
                let value_str_id = *string_map
                    .get(value)
                    .ok_or(CompileError::internal("missing interned string"))?;
                code.push(schema::Opcode::PushConst as u8);
                code.extend_from_slice(&value_str_id.to_le_bytes());
            }
            FunctionOptionValue::LocalVar { slot, .. } => {
                code.push(schema::Opcode::LoadLocal as u8);
                code.extend_from_slice(&slot.to_le_bytes());
            }
        }
    }
    Ok(())
}

fn lower_select<'a>(
    select: &'a SelectExpr,
    string_map: &BTreeMap<String, u32>,
    func_map: &BTreeMap<FunctionCatalogKey, u16>,
    literals: &mut LiteralPool,
    code: &mut Vec<u8>,
    state: &mut LoweringState<'a>,
) -> Result<(), CompileError> {
    emit_selector_start(&select.selector, string_map, func_map, code)?;

    let mut dispatch_patches = Vec::new();
    for (arm_idx, arm) in select.arms.iter().enumerate() {
        let key_str_id = *string_map
            .get(&arm.key)
            .ok_or(CompileError::internal("missing interned string"))?;
        code.push(schema::Opcode::CaseStr as u8);
        code.extend_from_slice(&key_str_id.to_le_bytes());
        let rel_pos = code.len();
        code.extend_from_slice(&0_i32.to_le_bytes());
        dispatch_patches.push((rel_pos, arm_idx));
    }

    code.push(schema::Opcode::CaseDefault as u8);
    let default_rel_pos = code.len();
    code.extend_from_slice(&0_i32.to_le_bytes());

    state.defaults.push(DefaultContinuation {
        parts: &select.default,
        depth: state.depth + 1,
        jump_sites: Vec::new(),
    });
    state.depth += 1;
    let mut arm_starts = vec![0_u32; select.arms.len()];
    let mut end_jump_patch_positions = Vec::new();

    for (arm_idx, arm) in select.arms.iter().enumerate() {
        arm_starts[arm_idx] = u32::try_from(code.len())
            .map_err(|_| CompileError::size_overflow("bytecode program counter"))?;
        let forwarded = lower_parts_inner(&arm.parts, string_map, func_map, literals, code, state)?;
        if !forwarded {
            code.push(schema::Opcode::Jmp as u8);
            let rel_pos = code.len();
            code.extend_from_slice(&0_i32.to_le_bytes());
            end_jump_patch_positions.push(rel_pos);
        }
    }

    let default_start = u32::try_from(code.len())
        .map_err(|_| CompileError::size_overflow("bytecode program counter"))?;
    let current_default = state
        .defaults
        .pop()
        .expect("select continuation is registered before its arms");
    for rel_pos in current_default.jump_sites {
        patch_rel32(code, rel_pos, default_start)?;
    }
    lower_parts_inner(&select.default, string_map, func_map, literals, code, state)?;
    let end_pc = u32::try_from(code.len())
        .map_err(|_| CompileError::size_overflow("bytecode program counter"))?;
    code.push(schema::Opcode::SelectEnd as u8);
    state.depth -= 1;

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
        SelectorExpr::Local { slot, .. } => {
            code.push(schema::Opcode::LoadLocal as u8);
            code.extend_from_slice(&slot.to_le_bytes());
            code.push(schema::Opcode::SelectBegin as u8);
            Ok(())
        }
        SelectorExpr::CheckedLocal { slot, .. } => {
            code.push(schema::Opcode::SelectLocal as u8);
            code.extend_from_slice(&slot.to_le_bytes());
            Ok(())
        }
        SelectorExpr::Var(name) => {
            let str_id = *string_map
                .get(name)
                .ok_or(CompileError::internal("missing interned variable"))?;
            code.push(schema::Opcode::SelectArg as u8);
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
            code.push(schema::Opcode::SelectArg as u8);
            code.extend_from_slice(&str_id.to_le_bytes());
            Ok(())
        }
        _ => {
            lower_selector(selector, string_map, func_map, code)?;
            code.push(schema::Opcode::SelectBegin as u8);
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
        SelectorExpr::Local { slot, .. } => {
            code.push(schema::Opcode::LoadLocal as u8);
            code.extend_from_slice(&slot.to_le_bytes());
            Ok(())
        }
        SelectorExpr::CheckedLocal { .. } => Err(CompileError::internal(
            "checked local selector requires selector-start lowering",
        )),
        SelectorExpr::Var(name) => {
            emit_operand(&Operand::Var(name.clone()), string_map, func_map, code)?;
            Ok(())
        }
        SelectorExpr::Call { operand, func } => {
            if func.name == "string" && func.options.is_empty() {
                emit_operand(operand, string_map, func_map, code)?;
                return Ok(());
            }
            let func_key = function_catalog_key(func);
            let fn_id = *func_map
                .get(&func_key)
                .ok_or(CompileError::internal("missing function entry"))?;
            let dynamic_options = function_dynamic_options(func);
            emit_operand(operand, string_map, func_map, code)?;
            for (key, value, local, resolved) in dynamic_options.iter().copied() {
                let key_str_id = *string_map
                    .get(key)
                    .ok_or(CompileError::internal("missing interned string"))?;
                code.push(schema::Opcode::PushConst as u8);
                code.extend_from_slice(&key_str_id.to_le_bytes());
                if let Some(slot) = local {
                    code.push(schema::Opcode::LoadLocal as u8);
                    code.extend_from_slice(&slot.to_le_bytes());
                } else {
                    let value_str_id = *string_map
                        .get(resolved.unwrap_or(value))
                        .ok_or(CompileError::internal("missing interned variable"))?;
                    if resolved.is_some() {
                        code.push(schema::Opcode::PushConst as u8);
                        code.extend_from_slice(&value_str_id.to_le_bytes());
                    } else {
                        code.push(schema::Opcode::LoadOptionArg as u8);
                        code.extend_from_slice(&value_str_id.to_le_bytes());
                        let fallback = format!("{{${value}}}");
                        let fallback_id = *string_map
                            .get(&fallback)
                            .ok_or(CompileError::internal("missing interned option fallback"))?;
                        code.extend_from_slice(&fallback_id.to_le_bytes());
                    }
                }
            }
            // No ExprFallback for selectors — errors abort.
            code.push(schema::Opcode::CallSelect as u8);
            code.extend_from_slice(&fn_id.to_le_bytes());
            code.push(1);
            code.push(
                u8::try_from(dynamic_options.len())
                    .map_err(|_| CompileError::size_overflow("option count"))?,
            );
            Ok(())
        }
        SelectorExpr::Literal(value) => {
            code.push(schema::Opcode::PushConst as u8);
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
    func_map: &BTreeMap<FunctionCatalogKey, u16>,
    code: &mut Vec<u8>,
) -> Result<(), CompileError> {
    match operand {
        Operand::Var(var) => {
            let var_str_id = *string_map
                .get(var)
                .ok_or(CompileError::internal("missing interned variable"))?;
            code.push(schema::Opcode::LoadArg as u8);
            code.extend_from_slice(&var_str_id.to_le_bytes());
        }
        Operand::Local(slot) => {
            code.push(schema::Opcode::LoadLocal as u8);
            code.extend_from_slice(&slot.to_le_bytes());
        }
        Operand::Literal { value, .. } => {
            let value_str_id = *string_map
                .get(value)
                .ok_or(CompileError::internal("missing interned string"))?;
            code.push(schema::Opcode::PushConst as u8);
            code.extend_from_slice(&value_str_id.to_le_bytes());
        }
        Operand::Call(call) => emit_call(call, string_map, func_map, code)?,
    }
    Ok(())
}

fn emit_call(
    call: &CallExpr,
    string_map: &BTreeMap<String, u32>,
    func_map: &BTreeMap<FunctionCatalogKey, u16>,
    code: &mut Vec<u8>,
) -> Result<(), CompileError> {
    let func_key = function_catalog_key(&call.func);
    let fn_id = *func_map
        .get(&func_key)
        .ok_or(CompileError::internal("missing function entry"))?;
    let dynamic_options = function_dynamic_options(&call.func);
    emit_operand(&call.operand, string_map, func_map, code)?;
    for (key, value, local, resolved) in dynamic_options.iter().copied() {
        let key_str_id = *string_map
            .get(key)
            .ok_or(CompileError::internal("missing interned string"))?;
        code.push(schema::Opcode::PushConst as u8);
        code.extend_from_slice(&key_str_id.to_le_bytes());
        if let Some(slot) = local {
            code.push(schema::Opcode::LoadLocal as u8);
            code.extend_from_slice(&slot.to_le_bytes());
        } else {
            let value_str_id = *string_map
                .get(resolved.unwrap_or(value))
                .ok_or(CompileError::internal("missing interned variable"))?;
            if resolved.is_some() {
                code.push(schema::Opcode::PushConst as u8);
                code.extend_from_slice(&value_str_id.to_le_bytes());
            } else {
                code.push(schema::Opcode::LoadOptionArg as u8);
                code.extend_from_slice(&value_str_id.to_le_bytes());
                let fallback = format!("{{${value}}}");
                let fallback_id = *string_map
                    .get(&fallback)
                    .ok_or(CompileError::internal("missing interned option fallback"))?;
                code.extend_from_slice(&fallback_id.to_le_bytes());
            }
        }
    }
    // Fallback is needed for the outer expression; nested calls still leave
    // their resolved value on the stack and use the same VM fallback path.
    let fb = call
        .fallback
        .clone()
        .unwrap_or_else(|| render_operand_fallback(&call.operand, &call.func));
    let fb_str_id = *string_map
        .get(&fb)
        .ok_or(CompileError::internal("missing interned string"))?;
    code.push(schema::Opcode::ExprFallback as u8);
    code.extend_from_slice(&fb_str_id.to_le_bytes());
    code.push(schema::Opcode::CallFunc as u8);
    code.extend_from_slice(&fn_id.to_le_bytes());
    code.push(1);
    code.push(
        u8::try_from(dynamic_options.len())
            .map_err(|_| CompileError::size_overflow("option count"))?,
    );
    Ok(())
}

fn render_operand_fallback(operand: &Operand, func: &FunctionSpec) -> String {
    match operand {
        Operand::Var(var) => format!("{{${var}}}"),
        Operand::Local(slot) => format!("{{<local:{slot}>}}"),
        Operand::Literal { value, .. } if value.is_empty() => format!("{{:{}}}", func.name),
        Operand::Literal { value, .. } => format!("{{|{}|}}", escape_fallback_literal(value)),
        Operand::Call(call) => call
            .fallback
            .clone()
            .unwrap_or_else(|| render_operand_fallback(&call.operand, &call.func)),
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

#[cfg(all(test, feature = "icu4x"))]
mod tests {
    use super::*;
    use crate::compiler::compile_str;
    use crate::runtime::{BuiltinHost, Catalog, Formatter, Value};

    fn opcode_count(catalog: &Catalog) -> usize {
        let mut pc = 0;
        let mut count = 0;
        while pc < catalog.code().len() {
            pc += schema::Opcode::try_from(catalog.code()[pc])
                .expect("valid opcode")
                .bytes();
            count += 1;
        }
        count
    }

    fn numeric_match(exacts: usize, keywords: usize) -> Catalog {
        let mut source = String::from(".input {$a :number}\n.input {$b :number}\n.match $a $b\n");
        for n in 0..exacts {
            source.push_str(&format!("{n} {n} {{{{exact{n}}}}}\n"));
        }
        for key in ["one", "two", "few"].into_iter().take(keywords) {
            source.push_str(&format!("{key} {key} {{{{{key}}}}}\n"));
        }
        source.push_str("* * {{fallback}}");
        let bytes = compile_str(&source).expect("compiled");
        Catalog::from_bytes(&bytes).expect("shared defaults verify")
    }

    fn format_numbers(catalog: &Catalog, args: &[(&str, i64)]) -> String {
        let locale = "ru".parse().expect("locale");
        let host = BuiltinHost::new(&locale).expect("host");
        let mut formatter = Formatter::new(catalog, host).expect("formatter");
        let handle = formatter.resolve("main").expect("handle");
        let args = args
            .iter()
            .map(|(name, value)| {
                (
                    catalog.string_id(name).expect("argument id"),
                    Value::Int(*value),
                )
            })
            .collect::<Vec<_>>();
        let mut output = String::new();
        let mut errors = Vec::new();
        formatter
            .format_to(handle, &args, &mut output, Some(&mut errors))
            .expect("formatted");
        assert!(errors.is_empty(), "{errors:?}");
        output
    }

    #[test]
    fn shared_defaults_keep_two_selector_bytecode_linear() {
        for keywords in [1, 3] {
            let counts = [3, 6, 12].map(|exacts| opcode_count(&numeric_match(exacts, keywords)));
            // Previously 134/246 instructions for just three exact keys.
            assert!(counts[0] <= 100, "{keywords} keyword arms: {counts:?}");
            assert!(counts[1] - counts[0] <= 36, "{counts:?}");
            assert!(counts[2] - counts[1] <= 72, "{counts:?}");
            let catalog = numeric_match(3, keywords);
            assert_eq!(format_numbers(&catalog, &[("a", 1), ("b", 1)]), "exact1");
            // The second selector rejects the exact branch; retry the first
            // selector's plural category instead of going straight to '*'.
            assert_eq!(format_numbers(&catalog, &[("a", 1), ("b", 21)]), "one");
            assert_eq!(format_numbers(&catalog, &[("a", 1), ("b", 2)]), "fallback");
            if keywords == 3 {
                assert_eq!(format_numbers(&catalog, &[("a", 2), ("b", 22)]), "few");
            }
        }
    }

    #[test]
    fn shared_defaults_unwind_three_selector_levels() {
        let source = ".input {$a :number}\n.input {$b :number}\n.input {$c :number}\n.match $a $b $c\n1 1 0 {{exact}}\none one one {{category}}\n* * * {{fallback}}";
        let bytes = compile_str(source).expect("compiled");
        let catalog = Catalog::from_bytes(&bytes).expect("shared defaults verify");
        assert_eq!(
            format_numbers(&catalog, &[("a", 1), ("b", 1), ("c", 0)]),
            "exact"
        );
        assert_eq!(
            format_numbers(&catalog, &[("a", 1), ("b", 21), ("c", 31)]),
            "category"
        );
        assert_eq!(
            format_numbers(&catalog, &[("a", 1), ("b", 21), ("c", 2)]),
            "fallback"
        );
    }
}
