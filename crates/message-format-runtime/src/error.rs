// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Runtime errors.

use alloc::{boxed::Box, string::String};
use core::{error::Error, fmt};

/// Errors returned while decoding or verifying catalog bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// Header magic did not match the expected value.
    BadMagic,
    /// Catalog major version is not supported by this runtime.
    UnsupportedVersion {
        /// Encountered major version.
        major: u16,
        /// Encountered minor version.
        minor: u16,
    },
    /// A chunk offset or length pointed outside the payload.
    ChunkOutOfBounds,
    /// Required chunk was missing.
    MissingChunk(&'static str),
    /// String pool data was not valid UTF-8.
    InvalidUtf8,
    /// Message entry point pointed outside the bytecode section.
    BadPc {
        /// Invalid program counter.
        pc: u32,
    },
    /// A jump target did not land on an instruction boundary.
    BadJump {
        /// Source instruction program counter.
        from_pc: u32,
        /// Target absolute program counter.
        to_pc: i64,
    },
    /// Bytecode contained an unknown opcode.
    UnknownOpcode {
        /// Program counter where unknown opcode was found.
        pc: u32,
        /// Unknown opcode value.
        opcode: u8,
    },
    /// Bytecode instruction was truncated.
    TruncatedInstruction {
        /// Program counter for truncated decode.
        pc: u32,
    },
    /// Bytecode referenced an invalid string-pool entry.
    InvalidStringRef {
        /// Program counter of the offending instruction.
        pc: u32,
        /// Referenced string id.
        id: u32,
    },
    /// Bytecode referenced an invalid literal slice.
    InvalidLiteralRef {
        /// Program counter of the offending instruction.
        pc: u32,
        /// Referenced literal offset.
        offset: u32,
        /// Referenced literal length.
        len: u32,
    },
    /// Bytecode referenced an invalid function table entry.
    InvalidFunctionRef {
        /// Program counter of the offending instruction.
        pc: u32,
        /// Referenced function id.
        fn_id: u16,
    },
    /// A message table entry referenced an invalid string id.
    InvalidMessageNameRef {
        /// Message table index.
        index: usize,
        /// Referenced string id.
        id: u32,
    },
    /// Message table entries were not strictly sorted by message id text.
    InvalidMessageOrder {
        /// Message table index where ordering first broke.
        index: usize,
    },
    /// A function table entry referenced an invalid function-name string id.
    InvalidFunctionNameRef {
        /// Function table index.
        index: usize,
        /// Referenced string id.
        id: u32,
    },
    /// A function table entry referenced an invalid static-option key string id.
    InvalidFunctionOptionKeyRef {
        /// Function table index.
        index: usize,
        /// Referenced string id.
        id: u32,
    },
    /// A function table entry referenced an invalid static-option value string id.
    InvalidFunctionOptionValueRef {
        /// Function table index.
        index: usize,
        /// Referenced string id.
        id: u32,
    },
    /// Bytecode used select opcodes outside a valid select sequence.
    InvalidSelectSequence {
        /// Program counter of the offending instruction.
        pc: u32,
        /// Offending opcode.
        opcode: u8,
    },
    /// Bytecode set an expression fallback without an immediate call to consume it.
    InvalidExprFallbackSequence {
        /// Program counter of the offending instruction.
        pc: u32,
    },
    /// A message entry did not have a reachable halt.
    UnterminatedEntry {
        /// Entrypoint lacking a reachable halt.
        entry_pc: u32,
    },
}

/// Errors returned while formatting a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageFunctionError {
    /// A function operand failed validation.
    BadOperand,
    /// A function option failed validation.
    BadOption,
    /// The function or one of its features is not supported.
    UnsupportedOperation(UnsupportedOperation),
    /// Implementation-defined function failure detail.
    Implementation(ImplementationFailure),
}

/// Unsupported message-function operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedOperation {
    /// Date formatting is unavailable for the active locale.
    DateFormattingForLocale,
    /// Time formatting is unavailable for the active locale.
    TimeFormattingForLocale,
    /// Datetime formatting is unavailable for the active locale.
    DateTimeFormattingForLocale,
}

/// Implementation-defined message-function failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplementationFailure {
    /// A host-provided function failed for implementation-defined reasons.
    Host,
    /// The builtin `test:select` function was instructed to fail.
    TestSelect,
    /// The builtin `test:format` function was instructed to fail.
    TestFormat,
}

/// Errors returned by [`Host`](crate::Host) callback implementations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCallError {
    /// A host function id is unknown.
    UnknownFunction {
        /// Function id that was not provided by host.
        fn_id: u16,
    },
    /// A message function failed.
    Function(MessageFunctionError),
}

/// Errors returned while formatting a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// Message id was not found in the catalog.
    UnknownMessageId(String),
    /// A requested argument was not provided.
    MissingArg(String),
    /// Selector resolution failed.
    ///
    /// The selector itself still resolves to the catch-all `*` arm, but this
    /// preserves why the selector could not be used when a primary cause is known.
    BadSelector {
        /// Underlying resolution or selector-call failure.
        source: Option<Box<Self>>,
    },
    /// VM tried to pop more values than available.
    StackUnderflow,
    /// A host function id is unknown.
    UnknownFunction {
        /// Function id that was not provided by host.
        fn_id: u16,
    },
    /// A message function failed.
    Function(MessageFunctionError),
    /// Runtime trap condition.
    Trap(Trap),
    /// Program counter was invalid during execution.
    BadPc {
        /// Invalid program counter.
        pc: u32,
    },
    /// Decoding the next instruction failed.
    Decode(CatalogError),
}

/// Enumerates runtime trap conditions produced by the VM and builtin host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trap {
    /// Builtin locale data could not be loaded.
    UnsupportedLocale,
    /// A requested localized catalog was not present.
    MissingLocaleCatalog,
    /// String pool size exceeded the runtime id range.
    StringIdOverflow,
    /// A string pool lookup referenced an invalid id.
    InvalidStringId,
    /// Function table size exceeded the runtime id range.
    FunctionIdOverflow,
    /// A function table lookup referenced an invalid index.
    InvalidFunctionIndex,
    /// A function name string id was invalid.
    InvalidFunctionNameStringId,
    /// An option key string id was invalid.
    InvalidOptionKeyStringId,
    /// An option value string id was invalid.
    InvalidOptionValueStringId,
    /// A selector case referenced an invalid string id.
    InvalidCaseStringId,
    /// Execution exhausted the configured fuel budget.
    FuelExhausted,
    /// A constant string id in bytecode was invalid.
    InvalidConstStringId,
    /// Bytecode dispatched an invalid output opcode.
    InvalidOutputOpcode,
    /// A string case opcode executed without an active selector.
    CaseStringWithoutSelector,
    /// Bytecode dispatched an invalid selector opcode.
    InvalidSelectorOpcode,
    /// A call option key integer could not be converted to a valid string id.
    CallOptionKeyOutOfRange,
    /// A call option key string was not interned in the catalog.
    CallOptionKeyUnknown,
    /// A call option key had an unsupported runtime type.
    CallOptionKeyWrongType,
    /// A markup option key integer could not be converted to a valid string id.
    MarkupOptionKeyOutOfRange,
    /// A markup option key had an unsupported runtime type.
    MarkupOptionKeyWrongType,
    /// An expression fallback referenced an invalid string id.
    InvalidFallbackStringId,
    /// A program counter computation overflowed the runtime range.
    ProgramCounterOverflow,
}

impl From<CatalogError> for FormatError {
    fn from(value: CatalogError) -> Self {
        Self::Decode(value)
    }
}

impl From<MessageFunctionError> for FormatError {
    fn from(value: MessageFunctionError) -> Self {
        Self::Function(value)
    }
}

impl From<MessageFunctionError> for HostCallError {
    fn from(value: MessageFunctionError) -> Self {
        Self::Function(value)
    }
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => f.write_str("catalog magic did not match expected value"),
            Self::UnsupportedVersion { major, minor } => {
                write!(f, "unsupported catalog version {major}.{minor}")
            }
            Self::ChunkOutOfBounds => f.write_str("catalog chunk pointed outside payload"),
            Self::MissingChunk(tag) => write!(f, "required catalog chunk {tag} was missing"),
            Self::InvalidUtf8 => f.write_str("catalog string data was not valid UTF-8"),
            Self::BadPc { pc } => write!(f, "catalog message entry pointed at invalid pc {pc}"),
            Self::BadJump { from_pc, to_pc } => {
                write!(
                    f,
                    "invalid jump target {to_pc} from instruction at pc {from_pc}"
                )
            }
            Self::UnknownOpcode { pc, opcode } => {
                write!(f, "unknown opcode 0x{opcode:02x} at pc {pc}")
            }
            Self::TruncatedInstruction { pc } => {
                write!(f, "truncated instruction while decoding at pc {pc}")
            }
            Self::InvalidStringRef { pc, id } => {
                write!(f, "invalid string id {id} referenced at pc {pc}")
            }
            Self::InvalidLiteralRef { pc, offset, len } => {
                write!(
                    f,
                    "invalid literal slice {offset}:{len} referenced at pc {pc}"
                )
            }
            Self::InvalidFunctionRef { pc, fn_id } => {
                write!(f, "invalid function id {fn_id} referenced at pc {pc}")
            }
            Self::InvalidMessageNameRef { index, id } => {
                write!(
                    f,
                    "invalid message name string id {id} at message index {index}"
                )
            }
            Self::InvalidMessageOrder { index } => {
                write!(
                    f,
                    "message table is not strictly sorted at message index {index}"
                )
            }
            Self::InvalidFunctionNameRef { index, id } => {
                write!(
                    f,
                    "invalid function name string id {id} at function index {index}"
                )
            }
            Self::InvalidFunctionOptionKeyRef { index, id } => {
                write!(
                    f,
                    "invalid function option key string id {id} at function index {index}"
                )
            }
            Self::InvalidFunctionOptionValueRef { index, id } => {
                write!(
                    f,
                    "invalid function option value string id {id} at function index {index}"
                )
            }
            Self::InvalidSelectSequence { pc, opcode } => {
                write!(
                    f,
                    "invalid select opcode 0x{opcode:02x} sequencing at pc {pc}"
                )
            }
            Self::InvalidExprFallbackSequence { pc } => {
                write!(
                    f,
                    "expression fallback at pc {pc} was not followed by a call"
                )
            }
            Self::UnterminatedEntry { entry_pc } => {
                write!(f, "message entry at pc {entry_pc} has no reachable halt")
            }
        }
    }
}

impl Error for CatalogError {}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownMessageId(message_id) => {
                write!(f, "unknown message id {message_id}")
            }
            Self::MissingArg(name) => write!(f, "missing argument {name}"),
            Self::BadSelector { source } => {
                if let Some(source) = source {
                    write!(f, "bad selector: {source}")
                } else {
                    f.write_str("bad selector")
                }
            }
            Self::StackUnderflow => f.write_str("stack underflow during message formatting"),
            Self::UnknownFunction { fn_id } => write!(f, "unknown function id {fn_id}"),
            Self::Function(error) => write!(f, "{error}"),
            Self::Trap(trap) => write!(f, "runtime trap: {trap}"),
            Self::BadPc { pc } => write!(f, "invalid execution pc {pc}"),
            Self::Decode(err) => write!(f, "catalog decode error: {err}"),
        }
    }
}

impl fmt::Display for MessageFunctionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadOperand => f.write_str("bad-operand"),
            Self::BadOption => f.write_str("bad-option"),
            Self::UnsupportedOperation(operation) => write!(f, "{operation}"),
            Self::Implementation(failure) => write!(f, "{failure}"),
        }
    }
}

impl fmt::Display for UnsupportedOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DateFormattingForLocale => f.write_str(
                "unsupported operation: date formatting is not supported for this locale",
            ),
            Self::TimeFormattingForLocale => f.write_str(
                "unsupported operation: time formatting is not supported for this locale",
            ),
            Self::DateTimeFormattingForLocale => f.write_str(
                "unsupported operation: datetime formatting is not supported for this locale",
            ),
        }
    }
}

impl fmt::Display for ImplementationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host => f.write_str("implementation-defined host function failure"),
            Self::TestSelect => f.write_str("test:select failed"),
            Self::TestFormat => f.write_str("test:format failed"),
        }
    }
}

impl fmt::Display for HostCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFunction { fn_id } => write!(f, "unknown function id {fn_id}"),
            Self::Function(error) => write!(f, "{error}"),
        }
    }
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedLocale => "unsupported locale",
            Self::MissingLocaleCatalog => "missing locale catalog",
            Self::StringIdOverflow => "string id overflow",
            Self::InvalidStringId => "invalid string id",
            Self::FunctionIdOverflow => "function id overflow",
            Self::InvalidFunctionIndex => "invalid function index",
            Self::InvalidFunctionNameStringId => "invalid function name string id",
            Self::InvalidOptionKeyStringId => "invalid option key string id",
            Self::InvalidOptionValueStringId => "invalid option value string id",
            Self::InvalidCaseStringId => "invalid case str id",
            Self::FuelExhausted => "fuel exhausted",
            Self::InvalidConstStringId => "invalid const str id",
            Self::InvalidOutputOpcode => "invalid output opcode",
            Self::CaseStringWithoutSelector => "CASE_STR without selector",
            Self::InvalidSelectorOpcode => "invalid selector opcode",
            Self::CallOptionKeyOutOfRange => "CALL_FUNC option key out of range",
            Self::CallOptionKeyUnknown => "CALL_FUNC option key unknown",
            Self::CallOptionKeyWrongType => "CALL_FUNC option key must be int/strref/str",
            Self::MarkupOptionKeyOutOfRange => "MARKUP option key out of range",
            Self::MarkupOptionKeyWrongType => "MARKUP option key must be int/strref",
            Self::InvalidFallbackStringId => "invalid fallback str id",
            Self::ProgramCounterOverflow => "program counter overflow",
        })
    }
}

impl Error for UnsupportedOperation {}

impl Error for ImplementationFailure {}

impl Error for MessageFunctionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnsupportedOperation(operation) => Some(operation),
            Self::Implementation(failure) => Some(failure),
            Self::BadOperand | Self::BadOption => None,
        }
    }
}

impl Error for HostCallError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Function(error) => Some(error),
            Self::UnknownFunction { .. } => None,
        }
    }
}

impl Error for FormatError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::BadSelector { source } => source
                .as_deref()
                .map(|source| source as &(dyn Error + 'static)),
            Self::Function(error) => Some(error),
            Self::Decode(err) => Some(err),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn catalog_error_implements_display() {
        let err = CatalogError::UnsupportedVersion { major: 2, minor: 1 };
        assert_eq!(err.to_string(), "unsupported catalog version 2.1");
    }

    #[test]
    fn format_error_reports_decode_source() {
        let err = FormatError::Decode(CatalogError::MissingChunk("CODE"));
        assert_eq!(
            err.to_string(),
            "catalog decode error: required catalog chunk CODE was missing"
        );
        let source = err.source().expect("decode source");
        assert_eq!(
            source.to_string(),
            "required catalog chunk CODE was missing"
        );
    }

    #[test]
    fn bad_selector_error_preserves_first_cause_as_source() {
        let err = FormatError::BadSelector {
            source: Some(Box::new(FormatError::MissingArg("count".to_string()))),
        };
        assert_eq!(err.to_string(), "bad selector: missing argument count");
        let source = err.source().expect("bad-selector source");
        assert_eq!(source.to_string(), "missing argument count");
    }

    #[test]
    fn trap_display_stays_human_readable() {
        assert_eq!(MessageFunctionError::BadOption.to_string(), "bad-option");
        assert_eq!(
            MessageFunctionError::UnsupportedOperation(
                UnsupportedOperation::DateTimeFormattingForLocale
            )
            .to_string(),
            "unsupported operation: datetime formatting is not supported for this locale"
        );
        assert_eq!(
            MessageFunctionError::Implementation(ImplementationFailure::TestSelect).to_string(),
            "test:select failed"
        );
        assert_eq!(
            FormatError::Trap(Trap::ProgramCounterOverflow).to_string(),
            "runtime trap: program counter overflow"
        );
    }
}
