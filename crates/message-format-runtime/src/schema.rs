// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared executable catalog schema.
//!
//! This module owns the stable bytecode/catalog contract shared between the
//! compiler and runtime:
//!
//! - function and message table entry layouts
//! - opcode assignments
//! - instruction decoding metadata
//!
//! The compiler targets this schema when emitting catalogs. The runtime
//! verifier and VM interpret the same schema when loading and executing them.

use alloc::vec::Vec;

use crate::{catalog::read_i32, error::CatalogError};

/// Decoded message entry in the `MSGS` chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageEntry {
    /// Message name string-pool id.
    pub name_str_id: u32,
    /// Entrypoint program counter in `CODE` bytes.
    pub entry_pc: u32,
}

/// Decoded function entry in the `FUNC` chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuncEntry {
    /// Function name string-pool id (e.g. `"number"`).
    pub name_str_id: u32,
    /// Static options as (`key_str_id`, `value_str_id`) pairs.
    pub static_options: Vec<(u32, u32)>,
}

/// Stop execution.
pub const OP_HALT: u8 = 0x00;
/// Relative unconditional jump.
pub const OP_JMP: u8 = 0x01;
/// Relative jump when popped value is falsey.
pub const OP_JMP_IF_FALSE: u8 = 0x02;
/// Push constant from pool (reserved for next milestone).
pub const OP_PUSH_CONST: u8 = 0x10;
/// Load argument by string-pool id.
pub const OP_LOAD_ARG: u8 = 0x11;
/// Output pool string by id.
pub const OP_OUT_LIT: u8 = 0x20;
/// Output literal slice by offset and length.
pub const OP_OUT_SLICE: u8 = 0x21;
/// Pop and output runtime value.
pub const OP_OUT_VAL: u8 = 0x22;
/// Output literal expression slice by offset and length (sink: expression event).
pub const OP_OUT_EXPR: u8 = 0x23;
/// Output one argument directly by string-pool id.
pub const OP_OUT_ARG: u8 = 0x24;
/// Load one selector argument directly by string-pool id.
pub const OP_SELECT_ARG: u8 = 0x25;
/// Begin select dispatch.
pub const OP_SELECT_BEGIN: u8 = 0x30;
/// Case compare against string-pool id.
pub const OP_CASE_STR: u8 = 0x31;
/// Default case jump.
pub const OP_CASE_DEFAULT: u8 = 0x32;
/// End select dispatch.
pub const OP_SELECT_END: u8 = 0x33;
/// Host function call.
pub const OP_CALL_FUNC: u8 = 0x40;
/// Set expression fallback string for error recovery.
pub const OP_EXPR_FALLBACK: u8 = 0x41;
/// Host function call for selection (zero-allocation fast path).
pub const OP_CALL_SELECT: u8 = 0x42;
/// Markup open tag with name and options.
pub const OP_MARKUP_OPEN: u8 = 0x50;
/// Markup close tag with name and options.
pub const OP_MARKUP_CLOSE: u8 = 0x51;

/// Decoded instruction metadata.
#[derive(Debug, Clone, Copy)]
pub struct Decoded {
    /// Opcode byte.
    pub opcode: u8,
    /// Program counter of this instruction.
    pub pc: u32,
    /// Program counter of next linear instruction.
    pub next_pc: u32,
    rel32: Option<i32>,
}

impl Decoded {
    /// Return jump target absolute pc, if this instruction carries a relative jump operand.
    #[must_use]
    pub fn jump_target(self) -> Option<i64> {
        self.rel32
            .map(|rel| i64::from(self.next_pc) + i64::from(rel))
    }

    /// Control-flow category for this opcode.
    #[must_use]
    pub fn flow_kind(self) -> FlowKind {
        match self.opcode {
            OP_HALT => FlowKind::Stop,
            OP_JMP | OP_CASE_DEFAULT => FlowKind::JumpOnly,
            OP_JMP_IF_FALSE | OP_CASE_STR => FlowKind::Conditional,
            _ => FlowKind::Linear,
        }
    }
}

/// High-level control-flow behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowKind {
    /// Execution stops.
    Stop,
    /// Only linear fallthrough.
    Linear,
    /// Only jumps to target.
    JumpOnly,
    /// Both fallthrough and jump edge.
    Conditional,
}

/// Decode one instruction according to the shared executable schema.
pub fn decode(code: &[u8], pc: u32) -> Result<Decoded, CatalogError> {
    let (pc_usize, opcode, next_pc) = decode_opcode_and_next_pc(code, pc)?;

    let rel32 = match opcode {
        OP_JMP | OP_JMP_IF_FALSE | OP_CASE_DEFAULT => Some(read_i32(code, pc_usize + 1)?),
        OP_CASE_STR => Some(read_i32(code, pc_usize + 5)?),
        _ => None,
    };

    Ok(Decoded {
        opcode,
        pc,
        next_pc,
        rel32,
    })
}

pub(crate) fn decode_opcode_and_next_pc(
    code: &[u8],
    pc: u32,
) -> Result<(usize, u8, u32), CatalogError> {
    let pc_usize = usize::try_from(pc).map_err(|_| CatalogError::TruncatedInstruction { pc })?;
    let opcode = *code
        .get(pc_usize)
        .ok_or(CatalogError::TruncatedInstruction { pc })?;
    let len = opcode_len(opcode).ok_or(CatalogError::UnknownOpcode { pc, opcode })?;
    let len_u32 = u32::try_from(len).map_err(|_| CatalogError::TruncatedInstruction { pc })?;
    let next_pc = pc
        .checked_add(len_u32)
        .ok_or(CatalogError::TruncatedInstruction { pc })?;
    if next_pc as usize > code.len() {
        return Err(CatalogError::TruncatedInstruction { pc });
    }
    Ok((pc_usize, opcode, next_pc))
}

fn opcode_len(opcode: u8) -> Option<usize> {
    Some(match opcode {
        OP_HALT => 1,
        OP_JMP => 5,
        OP_JMP_IF_FALSE => 5,
        OP_PUSH_CONST => 5,
        OP_LOAD_ARG => 5,
        OP_OUT_LIT => 5,
        OP_OUT_SLICE => 9,
        OP_OUT_VAL => 1,
        OP_OUT_ARG => 5,
        OP_SELECT_BEGIN => 1,
        OP_CASE_STR => 9,
        OP_CASE_DEFAULT => 5,
        OP_SELECT_END => 1,
        OP_CALL_FUNC => 5,
        OP_EXPR_FALLBACK => 5,
        OP_CALL_SELECT => 5,
        OP_OUT_EXPR => 9,
        OP_SELECT_ARG => 5,
        OP_MARKUP_OPEN => 6,
        OP_MARKUP_CLOSE => 6,
        _ => return None,
    })
}

/// Fluent bytecode builder for tests.
///
/// Eliminates manual byte arithmetic — especially for jump offsets — by
/// supporting symbolic labels that are resolved in `build()`.
#[cfg(test)]
pub(crate) struct TestOps {
    code: Vec<u8>,
    labels: alloc::collections::BTreeMap<&'static str, usize>,
    /// (label name, patch offset of rel32, `next_pc` after the instruction)
    fixups: Vec<(&'static str, usize, usize)>,
}

#[cfg(test)]
#[allow(
    unreachable_pub,
    dead_code,
    reason = "pub methods are the builder API surface used in tests"
)]
impl TestOps {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            labels: alloc::collections::BTreeMap::new(),
            fixups: Vec::new(),
        }
    }

    // -- 1-byte instructions --------------------------------------------------

    pub fn halt(mut self) -> Self {
        self.code.push(OP_HALT);
        self
    }

    pub fn select_begin(mut self) -> Self {
        self.code.push(OP_SELECT_BEGIN);
        self
    }

    pub fn select_end(mut self) -> Self {
        self.code.push(OP_SELECT_END);
        self
    }

    pub fn out_val(mut self) -> Self {
        self.code.push(OP_OUT_VAL);
        self
    }

    // -- 5-byte: opcode + u32 -------------------------------------------------

    pub fn push_const(mut self, str_id: u32) -> Self {
        self.code.push(OP_PUSH_CONST);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self
    }

    pub fn load_arg(mut self, str_id: u32) -> Self {
        self.code.push(OP_LOAD_ARG);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self
    }

    pub fn out_lit(mut self, str_id: u32) -> Self {
        self.code.push(OP_OUT_LIT);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self
    }

    pub fn out_arg(mut self, str_id: u32) -> Self {
        self.code.push(OP_OUT_ARG);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self
    }

    pub fn select_arg(mut self, str_id: u32) -> Self {
        self.code.push(OP_SELECT_ARG);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self
    }

    pub fn expr_fallback(mut self, str_id: u32) -> Self {
        self.code.push(OP_EXPR_FALLBACK);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self
    }

    // -- 9-byte: opcode + u32 + u32 -------------------------------------------

    pub fn out_slice(mut self, offset: u32, len: u32) -> Self {
        self.code.push(OP_OUT_SLICE);
        self.code.extend_from_slice(&offset.to_le_bytes());
        self.code.extend_from_slice(&len.to_le_bytes());
        self
    }

    pub fn out_expr(mut self, offset: u32, len: u32) -> Self {
        self.code.push(OP_OUT_EXPR);
        self.code.extend_from_slice(&offset.to_le_bytes());
        self.code.extend_from_slice(&len.to_le_bytes());
        self
    }

    // -- label-based jumps (rel32 resolved in build) --------------------------

    pub fn jmp(mut self, label: &'static str) -> Self {
        self.code.push(OP_JMP);
        let patch = self.code.len();
        self.code.extend_from_slice(&0_i32.to_le_bytes());
        let next_pc = self.code.len();
        self.fixups.push((label, patch, next_pc));
        self
    }

    pub fn jmp_if_false(mut self, label: &'static str) -> Self {
        self.code.push(OP_JMP_IF_FALSE);
        let patch = self.code.len();
        self.code.extend_from_slice(&0_i32.to_le_bytes());
        let next_pc = self.code.len();
        self.fixups.push((label, patch, next_pc));
        self
    }

    pub fn case_str(mut self, str_id: u32, label: &'static str) -> Self {
        self.code.push(OP_CASE_STR);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        let patch = self.code.len();
        self.code.extend_from_slice(&0_i32.to_le_bytes());
        let next_pc = self.code.len();
        self.fixups.push((label, patch, next_pc));
        self
    }

    pub fn case_default(mut self, label: &'static str) -> Self {
        self.code.push(OP_CASE_DEFAULT);
        let patch = self.code.len();
        self.code.extend_from_slice(&0_i32.to_le_bytes());
        let next_pc = self.code.len();
        self.fixups.push((label, patch, next_pc));
        self
    }

    // -- raw-offset jumps (no label resolution) -------------------------------

    pub fn jmp_rel(mut self, rel: i32) -> Self {
        self.code.push(OP_JMP);
        self.code.extend_from_slice(&rel.to_le_bytes());
        self
    }

    pub fn jmp_if_false_rel(mut self, rel: i32) -> Self {
        self.code.push(OP_JMP_IF_FALSE);
        self.code.extend_from_slice(&rel.to_le_bytes());
        self
    }

    pub fn case_str_rel(mut self, str_id: u32, rel: i32) -> Self {
        self.code.push(OP_CASE_STR);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self.code.extend_from_slice(&rel.to_le_bytes());
        self
    }

    pub fn case_default_rel(mut self, rel: i32) -> Self {
        self.code.push(OP_CASE_DEFAULT);
        self.code.extend_from_slice(&rel.to_le_bytes());
        self
    }

    // -- function calls: opcode + u16(fn_id) + u8(arg_count) + u8(optc) ------

    pub fn call_func(mut self, fn_id: u16, arg_count: u8, optc: u8) -> Self {
        self.code.push(OP_CALL_FUNC);
        self.code.extend_from_slice(&fn_id.to_le_bytes());
        self.code.push(arg_count);
        self.code.push(optc);
        self
    }

    pub fn call_select(mut self, fn_id: u16, arg_count: u8, optc: u8) -> Self {
        self.code.push(OP_CALL_SELECT);
        self.code.extend_from_slice(&fn_id.to_le_bytes());
        self.code.push(arg_count);
        self.code.push(optc);
        self
    }

    // -- markup: opcode + u32(name_id) + u8(optc) ----------------------------

    pub fn markup_open(mut self, name_id: u32, optc: u8) -> Self {
        self.code.push(OP_MARKUP_OPEN);
        self.code.extend_from_slice(&name_id.to_le_bytes());
        self.code.push(optc);
        self
    }

    pub fn markup_close(mut self, name_id: u32, optc: u8) -> Self {
        self.code.push(OP_MARKUP_CLOSE);
        self.code.extend_from_slice(&name_id.to_le_bytes());
        self.code.push(optc);
        self
    }

    // -- pseudo-instructions --------------------------------------------------

    /// Zero-width label marker at the current byte offset.
    pub fn label(mut self, name: &'static str) -> Self {
        let offset = self.code.len();
        if self.labels.insert(name, offset).is_some() {
            panic!("duplicate label: {name}");
        }
        self
    }

    /// Append raw bytes (escape hatch for unusual encodings).
    pub fn raw(mut self, bytes: &[u8]) -> Self {
        self.code.extend_from_slice(bytes);
        self
    }

    /// Consume the builder and return the bytecode, resolving all label fixups.
    ///
    /// # Panics
    ///
    /// Panics if any label referenced by a jump was never defined.
    pub fn build(mut self) -> Vec<u8> {
        for (label, patch, next_pc) in &self.fixups {
            let target = self
                .labels
                .get(label)
                .unwrap_or_else(|| panic!("unresolved label: {label}"));
            let target_i = isize::try_from(*target).expect("target overflows isize");
            let next_i = isize::try_from(*next_pc).expect("next_pc overflows isize");
            let rel = i32::try_from(target_i - next_i).expect("jump offset overflows i32");
            self.code[*patch..*patch + 4].copy_from_slice(&rel.to_le_bytes());
        }
        self.code
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn simple_bytecode_matches_expected_bytes() {
        let code = TestOps::new().out_arg(1).halt().build();
        assert_eq!(code, vec![OP_OUT_ARG, 1, 0, 0, 0, OP_HALT]);
    }

    #[test]
    fn forward_label_resolution() {
        let code = TestOps::new()
            .jmp("end")
            .out_arg(1)
            .label("end")
            .halt()
            .build();
        // JMP next_pc=5, target=10 → rel=5
        assert_eq!(
            code,
            vec![OP_JMP, 5, 0, 0, 0, OP_OUT_ARG, 1, 0, 0, 0, OP_HALT]
        );
    }

    #[test]
    fn backward_label_resolution() {
        let code = TestOps::new()
            .label("top")
            .load_arg(1)
            .jmp_if_false("top")
            .halt()
            .build();
        // JMP_IF_FALSE next_pc=10, target=0 → rel=-10
        let rel = (-10_i32).to_le_bytes();
        assert_eq!(
            code,
            vec![
                OP_LOAD_ARG,
                1,
                0,
                0,
                0,
                OP_JMP_IF_FALSE,
                rel[0],
                rel[1],
                rel[2],
                rel[3],
                OP_HALT,
            ]
        );
    }

    #[test]
    #[should_panic(expected = "duplicate label")]
    fn duplicate_label_panics() {
        TestOps::new().label("x").label("x").build();
    }

    #[test]
    #[should_panic(expected = "unresolved label")]
    fn unresolved_label_panics() {
        TestOps::new().jmp("missing").build();
    }

    #[test]
    fn select_dispatch_matches_handwritten_bytes() {
        // Reproduce the canonical select pattern from vm.rs tests.
        let handwritten: Vec<u8> = vec![
            OP_SELECT_ARG,
            1,
            0,
            0,
            0,
            OP_CASE_STR,
            2,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            OP_CASE_DEFAULT,
            14,
            0,
            0,
            0,
            OP_OUT_SLICE,
            0,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            OP_JMP,
            9,
            0,
            0,
            0,
            OP_OUT_SLICE,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            OP_SELECT_END,
            OP_HALT,
        ];
        let built = TestOps::new()
            .select_arg(1)
            .case_str(2, "hit")
            .case_default("default")
            .label("hit")
            .out_slice(0, 1)
            .jmp("end")
            .label("default")
            .out_slice(1, 1)
            .label("end")
            .select_end()
            .halt()
            .build();
        assert_eq!(built, handwritten);
    }
}
