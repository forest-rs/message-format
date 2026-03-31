// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(
    deprecated,
    reason = "this module intentionally re-exports deprecated aliases"
)]

//! Legacy alias for the compiler semantic representation.
//!
//! Prefer [`crate::semantic`] in new code.

#[deprecated(note = "use message_format_compiler::semantic instead")]
pub use crate::semantic::{
    CallExpr, FunctionOption, FunctionOptionValue, FunctionSpec, Message, Operand, Part, SelectArm,
    SelectExpr, SelectorExpr, SourceId, SourceInfo, SourceKind, SourceSpan,
};
