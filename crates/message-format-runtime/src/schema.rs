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
use core::fmt;

use crate::{
    catalog::{read_i32, read_i32_unchecked},
    error::CatalogError,
};

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

/// Bytecode opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[repr(u8)]
pub enum Opcode {
    /// Stop execution.
    Halt = 0x00,
    /// Relative unconditional jump.
    Jmp = 0x01,
    /// Relative jump when popped value is falsey.
    JmpIfFalse = 0x02,
    /// Push constant from pool.
    PushConst = 0x10,
    /// Load argument by string-pool id.
    LoadArg = 0x11,
    /// Output pool string by id.
    OutLit = 0x20,
    /// Output literal slice by offset and length.
    OutSlice = 0x21,
    /// Pop and output runtime value.
    OutVal = 0x22,
    /// Output literal expression slice by offset and length (sink: expression event).
    OutExpr = 0x23,
    /// Output one argument directly by string-pool id.
    OutArg = 0x24,
    /// Load one selector argument directly by string-pool id.
    SelectArg = 0x25,
    /// Begin select dispatch.
    SelectBegin = 0x30,
    /// Case compare against string-pool id.
    CaseStr = 0x31,
    /// Default case jump.
    CaseDefault = 0x32,
    /// End select dispatch.
    SelectEnd = 0x33,
    /// Host function call.
    CallFunc = 0x40,
    /// Set expression fallback string for error recovery.
    ExprFallback = 0x41,
    /// Host function call for selection (zero-allocation fast path).
    CallSelect = 0x42,
    /// Markup open tag with name and options.
    MarkupOpen = 0x50,
    /// Markup close tag with name and options.
    MarkupClose = 0x51,
}

impl Opcode {
    /// Instruction width in bytes (opcode byte + operands).
    #[must_use]
    pub fn bytes(self) -> usize {
        match self {
            Self::Halt => 1,
            Self::Jmp => 5,
            Self::JmpIfFalse => 5,
            Self::PushConst => 5,
            Self::LoadArg => 5,
            Self::OutLit => 5,
            Self::OutSlice => 9,
            Self::OutVal => 1,
            Self::OutExpr => 9,
            Self::OutArg => 5,
            Self::SelectArg => 5,
            Self::SelectBegin => 1,
            Self::CaseStr => 9,
            Self::CaseDefault => 5,
            Self::SelectEnd => 1,
            Self::CallFunc => 5,
            Self::ExprFallback => 5,
            Self::CallSelect => 5,
            Self::MarkupOpen => 6,
            Self::MarkupClose => 6,
        }
    }
}

impl TryFrom<u8> for Opcode {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, u8> {
        match value {
            0x00 => Ok(Self::Halt),
            0x01 => Ok(Self::Jmp),
            0x02 => Ok(Self::JmpIfFalse),
            0x10 => Ok(Self::PushConst),
            0x11 => Ok(Self::LoadArg),
            0x20 => Ok(Self::OutLit),
            0x21 => Ok(Self::OutSlice),
            0x22 => Ok(Self::OutVal),
            0x23 => Ok(Self::OutExpr),
            0x24 => Ok(Self::OutArg),
            0x25 => Ok(Self::SelectArg),
            0x30 => Ok(Self::SelectBegin),
            0x31 => Ok(Self::CaseStr),
            0x32 => Ok(Self::CaseDefault),
            0x33 => Ok(Self::SelectEnd),
            0x40 => Ok(Self::CallFunc),
            0x41 => Ok(Self::ExprFallback),
            0x42 => Ok(Self::CallSelect),
            0x50 => Ok(Self::MarkupOpen),
            0x51 => Ok(Self::MarkupClose),
            other => Err(other),
        }
    }
}

impl fmt::Display for Opcode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?} (0x{:02x})", *self as u8)
    }
}

/// Decoded instruction metadata.
#[derive(Debug, Clone, Copy)]
pub struct Decoded {
    /// Opcode.
    pub opcode: Opcode,
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
            Opcode::Halt => FlowKind::Stop,
            Opcode::Jmp | Opcode::CaseDefault => FlowKind::JumpOnly,
            Opcode::JmpIfFalse | Opcode::CaseStr => FlowKind::Conditional,
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
        Opcode::Jmp | Opcode::JmpIfFalse | Opcode::CaseDefault => {
            Some(read_i32(code, pc_usize + 1)?)
        }
        Opcode::CaseStr => Some(read_i32(code, pc_usize + 5)?),
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
) -> Result<(usize, Opcode, u32), CatalogError> {
    let pc_usize = usize::try_from(pc).map_err(|_| CatalogError::TruncatedInstruction { pc })?;
    let raw = *code
        .get(pc_usize)
        .ok_or(CatalogError::TruncatedInstruction { pc })?;
    let opcode =
        Opcode::try_from(raw).map_err(|opcode| CatalogError::UnknownOpcode { pc, opcode })?;
    let len = opcode.bytes();
    let len_u32 = u32::try_from(len).map_err(|_| CatalogError::TruncatedInstruction { pc })?;
    let next_pc = pc
        .checked_add(len_u32)
        .ok_or(CatalogError::TruncatedInstruction { pc })?;
    if next_pc as usize > code.len() {
        return Err(CatalogError::TruncatedInstruction { pc });
    }
    Ok((pc_usize, opcode, next_pc))
}

/// Decode one instruction according to the shared executable schema.
///
/// # Safety
/// Assumptions shared with [`decode_opcode_and_next_pc`]:
/// - `usize::try_from(pc).is_ok()`
/// - `pc < code.len()`
/// - valid opcode ([`Opcode::try_from`] returns `Ok`)
/// - `pc + opcode.bytes()` does not overflow `u32`
///
/// Also assumes:
/// - `pc + opcode.len() < code.len()`
pub(crate) unsafe fn decode_unchecked(code: &[u8], pc: u32) -> Decoded {
    let (pc_usize, opcode, next_pc) = unsafe { decode_opcode_and_next_pc_unchecked(code, pc) };

    let rel32 = match opcode {
        Opcode::Jmp | Opcode::JmpIfFalse | Opcode::CaseDefault => {
            Some(unsafe { read_i32_unchecked(code, pc_usize + 1) })
        }
        Opcode::CaseStr => Some(unsafe { read_i32_unchecked(code, pc_usize + 5) }),
        _ => None,
    };

    Decoded {
        opcode,
        pc,
        next_pc,
        rel32,
    }
}

/// Unchecked version of [`decode_opcode_and_next_pc`].
///
/// # Safety
/// Assumes:
/// - `usize::try_from(pc).is_ok()`
/// - `pc < code.len()`
/// - valid opcode ([`Opcode::try_from`] returns `Ok`)
/// - `pc + opcode.bytes()` does not overflow `u32`
pub(crate) unsafe fn decode_opcode_and_next_pc_unchecked(
    code: &[u8],
    pc: u32,
) -> (usize, Opcode, u32) {
    let pc_usize = pc as usize;
    let raw = *unsafe { code.get_unchecked(pc_usize) };
    let opcode = unsafe { Opcode::try_from(raw).unwrap_unchecked() };
    let len = opcode.bytes();
    #[expect(clippy::cast_possible_truncation, reason = "unchecked")]
    let len_u32 = len as u32;
    let next_pc = pc + len_u32;
    (pc_usize, opcode, next_pc)
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
        self.code.push(Opcode::Halt as u8);
        self
    }

    pub fn select_begin(mut self) -> Self {
        self.code.push(Opcode::SelectBegin as u8);
        self
    }

    pub fn select_end(mut self) -> Self {
        self.code.push(Opcode::SelectEnd as u8);
        self
    }

    pub fn out_val(mut self) -> Self {
        self.code.push(Opcode::OutVal as u8);
        self
    }

    // -- 5-byte: opcode + u32 -------------------------------------------------

    pub fn push_const(mut self, str_id: u32) -> Self {
        self.code.push(Opcode::PushConst as u8);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self
    }

    pub fn load_arg(mut self, str_id: u32) -> Self {
        self.code.push(Opcode::LoadArg as u8);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self
    }

    pub fn out_lit(mut self, str_id: u32) -> Self {
        self.code.push(Opcode::OutLit as u8);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self
    }

    pub fn out_arg(mut self, str_id: u32) -> Self {
        self.code.push(Opcode::OutArg as u8);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self
    }

    pub fn select_arg(mut self, str_id: u32) -> Self {
        self.code.push(Opcode::SelectArg as u8);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self
    }

    pub fn expr_fallback(mut self, str_id: u32) -> Self {
        self.code.push(Opcode::ExprFallback as u8);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self
    }

    // -- 9-byte: opcode + u32 + u32 -------------------------------------------

    pub fn out_slice(mut self, offset: u32, len: u32) -> Self {
        self.code.push(Opcode::OutSlice as u8);
        self.code.extend_from_slice(&offset.to_le_bytes());
        self.code.extend_from_slice(&len.to_le_bytes());
        self
    }

    pub fn out_expr(mut self, offset: u32, len: u32) -> Self {
        self.code.push(Opcode::OutExpr as u8);
        self.code.extend_from_slice(&offset.to_le_bytes());
        self.code.extend_from_slice(&len.to_le_bytes());
        self
    }

    // -- label-based jumps (rel32 resolved in build) --------------------------

    pub fn jmp(mut self, label: &'static str) -> Self {
        self.code.push(Opcode::Jmp as u8);
        let patch = self.code.len();
        self.code.extend_from_slice(&0_i32.to_le_bytes());
        let next_pc = self.code.len();
        self.fixups.push((label, patch, next_pc));
        self
    }

    pub fn jmp_if_false(mut self, label: &'static str) -> Self {
        self.code.push(Opcode::JmpIfFalse as u8);
        let patch = self.code.len();
        self.code.extend_from_slice(&0_i32.to_le_bytes());
        let next_pc = self.code.len();
        self.fixups.push((label, patch, next_pc));
        self
    }

    pub fn case_str(mut self, str_id: u32, label: &'static str) -> Self {
        self.code.push(Opcode::CaseStr as u8);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        let patch = self.code.len();
        self.code.extend_from_slice(&0_i32.to_le_bytes());
        let next_pc = self.code.len();
        self.fixups.push((label, patch, next_pc));
        self
    }

    pub fn case_default(mut self, label: &'static str) -> Self {
        self.code.push(Opcode::CaseDefault as u8);
        let patch = self.code.len();
        self.code.extend_from_slice(&0_i32.to_le_bytes());
        let next_pc = self.code.len();
        self.fixups.push((label, patch, next_pc));
        self
    }

    // -- raw-offset jumps (no label resolution) -------------------------------

    pub fn jmp_rel(mut self, rel: i32) -> Self {
        self.code.push(Opcode::Jmp as u8);
        self.code.extend_from_slice(&rel.to_le_bytes());
        self
    }

    pub fn jmp_if_false_rel(mut self, rel: i32) -> Self {
        self.code.push(Opcode::JmpIfFalse as u8);
        self.code.extend_from_slice(&rel.to_le_bytes());
        self
    }

    pub fn case_str_rel(mut self, str_id: u32, rel: i32) -> Self {
        self.code.push(Opcode::CaseStr as u8);
        self.code.extend_from_slice(&str_id.to_le_bytes());
        self.code.extend_from_slice(&rel.to_le_bytes());
        self
    }

    pub fn case_default_rel(mut self, rel: i32) -> Self {
        self.code.push(Opcode::CaseDefault as u8);
        self.code.extend_from_slice(&rel.to_le_bytes());
        self
    }

    // -- function calls: opcode + u16(fn_id) + u8(arg_count) + u8(optc) ------

    pub fn call_func(mut self, fn_id: u16, arg_count: u8, optc: u8) -> Self {
        self.code.push(Opcode::CallFunc as u8);
        self.code.extend_from_slice(&fn_id.to_le_bytes());
        self.code.push(arg_count);
        self.code.push(optc);
        self
    }

    pub fn call_select(mut self, fn_id: u16, arg_count: u8, optc: u8) -> Self {
        self.code.push(Opcode::CallSelect as u8);
        self.code.extend_from_slice(&fn_id.to_le_bytes());
        self.code.push(arg_count);
        self.code.push(optc);
        self
    }

    // -- markup: opcode + u32(name_id) + u8(optc) ----------------------------

    pub fn markup_open(mut self, name_id: u32, optc: u8) -> Self {
        self.code.push(Opcode::MarkupOpen as u8);
        self.code.extend_from_slice(&name_id.to_le_bytes());
        self.code.push(optc);
        self
    }

    pub fn markup_close(mut self, name_id: u32, optc: u8) -> Self {
        self.code.push(Opcode::MarkupClose as u8);
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
        assert_eq!(
            code,
            vec![Opcode::OutArg as u8, 1, 0, 0, 0, Opcode::Halt as u8]
        );
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
            vec![
                Opcode::Jmp as u8,
                5,
                0,
                0,
                0,
                Opcode::OutArg as u8,
                1,
                0,
                0,
                0,
                Opcode::Halt as u8,
            ]
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
                Opcode::LoadArg as u8,
                1,
                0,
                0,
                0,
                Opcode::JmpIfFalse as u8,
                rel[0],
                rel[1],
                rel[2],
                rel[3],
                Opcode::Halt as u8,
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
            Opcode::SelectArg as u8,
            1,
            0,
            0,
            0,
            Opcode::CaseStr as u8,
            2,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            Opcode::CaseDefault as u8,
            14,
            0,
            0,
            0,
            Opcode::OutSlice as u8,
            0,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            Opcode::Jmp as u8,
            9,
            0,
            0,
            0,
            Opcode::OutSlice as u8,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            Opcode::SelectEnd as u8,
            Opcode::Halt as u8,
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
