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
