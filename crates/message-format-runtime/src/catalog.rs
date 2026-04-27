// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Catalog model and decoding APIs.

use alloc::{vec, vec::Vec};
use core::{cmp::Ordering, ops::Range, str};

pub use crate::schema::{FuncEntry, MessageEntry};
use crate::{error::CatalogError, schema as vm, value::StrId};

const MAGIC: [u8; 8] = *b"MFCAT\0\0\x01";
const VERSION_MAJOR: u16 = 1;
const VERSION_MINOR: u16 = 0;
const HEADER_LEN: usize = 24;
const CHUNK_ENTRY_LEN: usize = 16;

const TAG_STRS: [u8; 4] = *b"STRS";
const TAG_LITS: [u8; 4] = *b"LITS";
const TAG_MSGS: [u8; 4] = *b"MSGS";
const TAG_CODE: [u8; 4] = *b"CODE";
const TAG_FUNC: [u8; 4] = *b"FUNC";

#[derive(Debug, Clone)]
struct ChunkRef {
    tag: [u8; 4],
    range: Range<usize>,
}

type StringsIndex = Vec<(u32, u32)>;

/// Runtime representation of a loaded and verified catalog.
#[derive(Debug, Clone)]
pub struct Catalog {
    bytes: Vec<u8>,
    strings: Vec<(u32, u32)>,
    strings_bytes: Range<usize>,
    lits_bytes: Option<Range<usize>>,
    messages: Vec<MessageEntry>,
    funcs: Vec<FuncEntry>,
    code_bytes: Range<usize>,
    instruction_pcs: Vec<u32>,
}

impl Catalog {
    /// Decode and verify a catalog payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CatalogError> {
        if bytes.len() < HEADER_LEN {
            return Err(CatalogError::ChunkOutOfBounds);
        }
        if bytes[0..8] != MAGIC {
            return Err(CatalogError::BadMagic);
        }

        let version_major = read_u16(bytes, 8)?;
        let version_minor = read_u16(bytes, 10)?;
        if version_major != VERSION_MAJOR {
            return Err(CatalogError::UnsupportedVersion {
                major: version_major,
                minor: version_minor,
            });
        }
        let _ = VERSION_MINOR;

        let chunk_count = read_u32(bytes, 16)? as usize;
        let chunk_table_offset = read_u32(bytes, 20)? as usize;
        let chunk_table_len = chunk_count
            .checked_mul(CHUNK_ENTRY_LEN)
            .ok_or(CatalogError::ChunkOutOfBounds)?;
        let chunk_table_end = chunk_table_offset
            .checked_add(chunk_table_len)
            .ok_or(CatalogError::ChunkOutOfBounds)?;
        if chunk_table_end > bytes.len() {
            return Err(CatalogError::ChunkOutOfBounds);
        }

        let mut chunks = Vec::with_capacity(chunk_count);
        let mut seen_strs = 0_u8;
        let mut seen_msgs = 0_u8;
        let mut seen_code = 0_u8;
        let mut seen_func = 0_u8;
        let mut table_pos = chunk_table_offset;
        for _ in 0..chunk_count {
            let mut tag = [0_u8; 4];
            tag.copy_from_slice(
                bytes
                    .get(table_pos..table_pos + 4)
                    .ok_or(CatalogError::ChunkOutOfBounds)?,
            );
            let offset = read_u32(bytes, table_pos + 4)? as usize;
            let length = read_u32(bytes, table_pos + 8)? as usize;
            let end = offset
                .checked_add(length)
                .ok_or(CatalogError::ChunkOutOfBounds)?;
            if end > bytes.len() {
                return Err(CatalogError::ChunkOutOfBounds);
            }
            if offset < chunk_table_end {
                return Err(CatalogError::ChunkOutOfBounds);
            }
            if tag == TAG_STRS {
                seen_strs = seen_strs.saturating_add(1);
            } else if tag == TAG_MSGS {
                seen_msgs = seen_msgs.saturating_add(1);
            } else if tag == TAG_CODE {
                seen_code = seen_code.saturating_add(1);
            } else if tag == TAG_FUNC {
                seen_func = seen_func.saturating_add(1);
            }
            chunks.push(ChunkRef {
                tag,
                range: offset..end,
            });
            table_pos += CHUNK_ENTRY_LEN;
        }
        if seen_strs > 1 || seen_msgs > 1 || seen_code > 1 || seen_func > 1 {
            return Err(CatalogError::ChunkOutOfBounds);
        }
        if has_overlapping_chunks(&chunks) {
            return Err(CatalogError::ChunkOutOfBounds);
        }

        let strs_range = find_chunk(&chunks, TAG_STRS).ok_or(CatalogError::MissingChunk("STRS"))?;
        let msgs_range = find_chunk(&chunks, TAG_MSGS).ok_or(CatalogError::MissingChunk("MSGS"))?;
        let code_range = find_chunk(&chunks, TAG_CODE).ok_or(CatalogError::MissingChunk("CODE"))?;
        let lits_range = find_chunk(&chunks, TAG_LITS);
        let func_range = find_chunk(&chunks, TAG_FUNC);

        let (strings, strings_bytes_rel) = decode_strings(bytes, &strs_range)?;
        let strings_bytes = (strs_range.start + strings_bytes_rel.start)
            ..(strs_range.start + strings_bytes_rel.end);

        let messages = decode_messages(bytes, &msgs_range)?;
        let funcs = if let Some(ref range) = func_range {
            decode_funcs(bytes, range)?
        } else {
            Vec::new()
        };
        let code_body_rel = decode_code_range(bytes, &code_range)?;
        let code_bytes =
            (code_range.start + code_body_rel.start)..(code_range.start + code_body_rel.end);

        for message in &messages {
            if message.entry_pc as usize >= code_bytes.len() {
                return Err(CatalogError::BadPc {
                    pc: message.entry_pc,
                });
            }
        }

        validate_message_table(bytes, &strings, &strings_bytes, &messages)?;
        validate_func_table(&strings, &funcs)?;

        let code = &bytes[code_bytes.clone()];
        let literal_len = lits_range
            .as_ref()
            .map_or(0, |range| range.end.saturating_sub(range.start));
        let instruction_pcs =
            verify_code(code, &messages, strings.len(), literal_len, funcs.len())?;

        let bytes = bytes.to_vec();
        let catalog = Self {
            bytes,
            strings,
            strings_bytes,
            lits_bytes: lits_range,
            messages,
            funcs,
            code_bytes,
            instruction_pcs,
        };
        catalog.verify_strings_utf8()?;
        Ok(catalog)
    }

    /// Get an entrypoint pc by message id.
    #[must_use]
    pub fn message_pc(&self, message_id: &str) -> Option<u32> {
        self.messages
            .binary_search_by(|entry| self.string(entry.name_str_id).unwrap_or("").cmp(message_id))
            .ok()
            .map(|idx| self.messages[idx].entry_pc)
    }

    /// Resolve a string-pool id by string contents.
    ///
    /// This is intended for setup-time resolution of ids that callers can cache
    /// across repeated formatting calls.
    #[must_use]
    pub fn string_id(&self, value: &str) -> Option<StrId> {
        self.strings.iter().enumerate().find_map(|(idx, _)| {
            let id = u32::try_from(idx).ok()?;
            (self.string(id).ok()? == value).then_some(id)
        })
    }

    /// Return all message entries.
    #[must_use]
    pub fn messages(&self) -> &[MessageEntry] {
        &self.messages
    }

    /// Number of entries in the string pool.
    #[must_use]
    pub fn string_count(&self) -> usize {
        self.strings.len()
    }

    /// Number of entries in the function table.
    #[must_use]
    pub fn func_count(&self) -> usize {
        self.funcs.len()
    }

    /// Get a function table entry by index.
    #[must_use]
    pub fn func(&self, fn_id: u16) -> Option<&FuncEntry> {
        self.funcs.get(fn_id as usize)
    }

    /// Get a string pool entry by id.
    pub fn string(&self, id: u32) -> Result<&str, CatalogError> {
        let slice = self.string_slice(id)?;
        // SAFETY: `Catalog::from_bytes` calls `verify_strings_utf8` before
        // constructing `Catalog`, so every indexed string slice is valid UTF-8.
        Ok(unsafe { str::from_utf8_unchecked(slice) })
    }

    /// Resolve a literal slice from the `LITS` chunk.
    pub fn literal(&self, off: u32, len: u32) -> Result<&str, CatalogError> {
        let slice = self.literal_slice(off, len)?;
        // SAFETY: `Catalog::from_bytes` calls `verify_strings_utf8` before
        // constructing `Catalog`, so the LITS byte range is valid UTF-8.
        Ok(unsafe { str::from_utf8_unchecked(slice) })
    }

    /// Return code bytes.
    #[must_use]
    pub fn code(&self) -> &[u8] {
        &self.bytes[self.code_bytes.clone()]
    }

    /// True if a program counter starts at an instruction boundary.
    #[must_use]
    pub fn is_instruction_boundary(&self, pc: u32) -> bool {
        self.instruction_pcs.binary_search(&pc).is_ok()
    }

    fn verify_strings_utf8(&self) -> Result<(), CatalogError> {
        for (index, _) in self.strings.iter().enumerate() {
            let id = u32::try_from(index).map_err(|_| CatalogError::ChunkOutOfBounds)?;
            let slice = self.string_slice(id)?;
            str::from_utf8(slice).map_err(|_| CatalogError::InvalidUtf8)?;
        }
        if let Some(lits) = &self.lits_bytes {
            str::from_utf8(&self.bytes[lits.clone()]).map_err(|_| CatalogError::InvalidUtf8)?;
        }
        Ok(())
    }

    pub(crate) fn pool_string_opt(&self, id: u32) -> Option<&str> {
        self.string(id).ok()
    }

    pub(crate) fn pool_string_len_opt(&self, id: u32) -> Option<usize> {
        self.strings.get(id as usize).map(|(_, len)| *len as usize)
    }

    pub(crate) fn literal_opt(&self, off: u32, len: u32) -> Option<&str> {
        self.literal(off, len).ok()
    }
}

impl Catalog {
    fn string_slice_from_parts<'a>(
        bytes: &'a [u8],
        strings: &[(u32, u32)],
        strings_bytes: &Range<usize>,
        id: u32,
    ) -> Result<&'a [u8], CatalogError> {
        let (off, len) = *strings
            .get(id as usize)
            .ok_or(CatalogError::ChunkOutOfBounds)?;
        let start = strings_bytes
            .start
            .checked_add(off as usize)
            .ok_or(CatalogError::ChunkOutOfBounds)?;
        let end = start
            .checked_add(len as usize)
            .ok_or(CatalogError::ChunkOutOfBounds)?;
        bytes.get(start..end).ok_or(CatalogError::ChunkOutOfBounds)
    }

    fn string_slice(&self, id: u32) -> Result<&[u8], CatalogError> {
        Self::string_slice_from_parts(&self.bytes, &self.strings, &self.strings_bytes, id)
    }

    fn literal_slice(&self, off: u32, len: u32) -> Result<&[u8], CatalogError> {
        let lits = self
            .lits_bytes
            .as_ref()
            .ok_or(CatalogError::MissingChunk("LITS"))?;
        let start = lits
            .start
            .checked_add(off as usize)
            .ok_or(CatalogError::ChunkOutOfBounds)?;
        let end = start
            .checked_add(len as usize)
            .ok_or(CatalogError::ChunkOutOfBounds)?;
        self.bytes
            .get(start..end)
            .ok_or(CatalogError::ChunkOutOfBounds)
    }
}

fn find_chunk(chunks: &[ChunkRef], tag: [u8; 4]) -> Option<Range<usize>> {
    chunks
        .iter()
        .find(|chunk| chunk.tag == tag)
        .map(|chunk| chunk.range.clone())
}

fn decode_strings(
    bytes: &[u8],
    range: &Range<usize>,
) -> Result<(StringsIndex, Range<usize>), CatalogError> {
    let body = bytes
        .get(range.clone())
        .ok_or(CatalogError::ChunkOutOfBounds)?;
    if body.len() < 4 {
        return Err(CatalogError::ChunkOutOfBounds);
    }
    let count = read_u32(body, 0)? as usize;
    let index_len = count.checked_mul(8).ok_or(CatalogError::ChunkOutOfBounds)?;
    let bytes_start = 4_usize
        .checked_add(index_len)
        .ok_or(CatalogError::ChunkOutOfBounds)?;
    if bytes_start > body.len() {
        return Err(CatalogError::ChunkOutOfBounds);
    }

    let mut result = vec![(0_u32, 0_u32); count];
    for (i, item) in result.iter_mut().enumerate() {
        let off_pos = 4 + i * 8;
        let off = read_u32(body, off_pos)?;
        let len = read_u32(body, off_pos + 4)?;
        let end = (off as usize)
            .checked_add(len as usize)
            .ok_or(CatalogError::ChunkOutOfBounds)?;
        if end > body.len() - bytes_start {
            return Err(CatalogError::ChunkOutOfBounds);
        }
        *item = (off, len);
    }

    Ok((result, bytes_start..body.len()))
}

fn decode_messages(bytes: &[u8], range: &Range<usize>) -> Result<Vec<MessageEntry>, CatalogError> {
    let body = bytes
        .get(range.clone())
        .ok_or(CatalogError::ChunkOutOfBounds)?;
    if body.len() < 4 {
        return Err(CatalogError::ChunkOutOfBounds);
    }
    let count = read_u32(body, 0)? as usize;
    let total = 4_usize
        .checked_add(count.checked_mul(8).ok_or(CatalogError::ChunkOutOfBounds)?)
        .ok_or(CatalogError::ChunkOutOfBounds)?;
    if total > body.len() {
        return Err(CatalogError::ChunkOutOfBounds);
    }

    let mut messages = Vec::with_capacity(count);
    for i in 0..count {
        let pos = 4 + i * 8;
        messages.push(MessageEntry {
            name_str_id: read_u32(body, pos)?,
            entry_pc: read_u32(body, pos + 4)?,
        });
    }
    Ok(messages)
}

fn decode_funcs(bytes: &[u8], range: &Range<usize>) -> Result<Vec<FuncEntry>, CatalogError> {
    let body = bytes
        .get(range.clone())
        .ok_or(CatalogError::ChunkOutOfBounds)?;
    if body.len() < 4 {
        return Err(CatalogError::ChunkOutOfBounds);
    }
    let count = read_u32(body, 0)? as usize;
    let mut pos = 4_usize;
    let mut funcs = Vec::with_capacity(count);
    for _ in 0..count {
        if pos + 8 > body.len() {
            return Err(CatalogError::ChunkOutOfBounds);
        }
        let name_str_id = read_u32(body, pos)?;
        let opt_count = read_u32(body, pos + 4)? as usize;
        pos += 8;
        let opts_len = opt_count
            .checked_mul(8)
            .ok_or(CatalogError::ChunkOutOfBounds)?;
        if pos
            .checked_add(opts_len)
            .ok_or(CatalogError::ChunkOutOfBounds)?
            > body.len()
        {
            return Err(CatalogError::ChunkOutOfBounds);
        }
        let mut static_options = Vec::with_capacity(opt_count);
        for _ in 0..opt_count {
            let key_str_id = read_u32(body, pos)?;
            let value_str_id = read_u32(body, pos + 4)?;
            static_options.push((key_str_id, value_str_id));
            pos += 8;
        }
        funcs.push(FuncEntry {
            name_str_id,
            static_options,
        });
    }
    Ok(funcs)
}

fn decode_code_range(bytes: &[u8], range: &Range<usize>) -> Result<Range<usize>, CatalogError> {
    let body = bytes
        .get(range.clone())
        .ok_or(CatalogError::ChunkOutOfBounds)?;
    if body.len() < 4 {
        return Err(CatalogError::ChunkOutOfBounds);
    }
    let code_len = read_u32(body, 0)? as usize;
    let code_start: usize = 4;
    let code_end = code_start
        .checked_add(code_len)
        .ok_or(CatalogError::ChunkOutOfBounds)?;
    if code_end > body.len() {
        return Err(CatalogError::ChunkOutOfBounds);
    }
    Ok(code_start..code_end)
}

fn validate_message_table(
    bytes: &[u8],
    strings: &[(u32, u32)],
    strings_bytes: &Range<usize>,
    messages: &[MessageEntry],
) -> Result<(), CatalogError> {
    let mut previous = None::<&str>;
    for (index, message) in messages.iter().enumerate() {
        let name =
            Catalog::string_slice_from_parts(bytes, strings, strings_bytes, message.name_str_id)
                .map_err(|_| CatalogError::InvalidMessageNameRef {
                    index,
                    id: message.name_str_id,
                })?;
        let name = str::from_utf8(name).map_err(|_| CatalogError::InvalidUtf8)?;
        if previous.is_some_and(|prev| prev >= name) {
            return Err(CatalogError::InvalidMessageOrder { index });
        }
        previous = Some(name);
    }
    Ok(())
}

fn validate_func_table(strings: &[(u32, u32)], funcs: &[FuncEntry]) -> Result<(), CatalogError> {
    for (index, func) in funcs.iter().enumerate() {
        if func.name_str_id as usize >= strings.len() {
            return Err(CatalogError::InvalidFunctionNameRef {
                index,
                id: func.name_str_id,
            });
        }
        for &(key_id, value_id) in &func.static_options {
            if key_id as usize >= strings.len() {
                return Err(CatalogError::InvalidFunctionOptionKeyRef { index, id: key_id });
            }
            if value_id as usize >= strings.len() {
                return Err(CatalogError::InvalidFunctionOptionValueRef {
                    index,
                    id: value_id,
                });
            }
        }
    }
    Ok(())
}

fn verify_code(
    code: &[u8],
    messages: &[MessageEntry],
    string_count: usize,
    literal_len: usize,
    func_count: usize,
) -> Result<Vec<u32>, CatalogError> {
    let mut pcs = Vec::new();
    let mut pc = 0_u32;
    while (pc as usize) < code.len() {
        pcs.push(pc);
        let decoded = vm::decode(code, pc)?;
        if decoded.next_pc as usize > code.len() {
            return Err(CatalogError::TruncatedInstruction { pc });
        }
        validate_instruction_operands(code, decoded, string_count, literal_len, func_count)?;
        pc = decoded.next_pc;
    }
    if pc as usize != code.len() {
        return Err(CatalogError::TruncatedInstruction { pc });
    }

    for start in &pcs {
        let decoded = vm::decode(code, *start)?;
        if let Some(target) = decoded.jump_target() {
            if target < 0 {
                return Err(CatalogError::BadJump {
                    from_pc: *start,
                    to_pc: target,
                });
            }
            let target_u32 = u32::try_from(target).map_err(|_| CatalogError::BadJump {
                from_pc: *start,
                to_pc: target,
            })?;
            if target_u32 as usize >= code.len() {
                return Err(CatalogError::BadJump {
                    from_pc: *start,
                    to_pc: target,
                });
            }
            if pcs.binary_search(&target_u32).is_err() {
                return Err(CatalogError::BadJump {
                    from_pc: *start,
                    to_pc: target,
                });
            }
        }
    }

    let mut halts_scratch = HaltsScratch::new(pcs.len());
    for message in messages {
        if pcs.binary_search(&message.entry_pc).is_err() {
            return Err(CatalogError::BadPc {
                pc: message.entry_pc,
            });
        }
        if !halts_reachable(code, &pcs, message.entry_pc, &mut halts_scratch)? {
            return Err(CatalogError::UnterminatedEntry {
                entry_pc: message.entry_pc,
            });
        }
    }

    verify_stack_safety(code, &pcs, messages)?;
    verify_control_state(code, &pcs, messages)?;

    Ok(pcs)
}

fn validate_instruction_operands(
    code: &[u8],
    decoded: vm::Decoded,
    string_count: usize,
    literal_len: usize,
    func_count: usize,
) -> Result<(), CatalogError> {
    let base = decoded.pc as usize;
    match decoded.opcode {
        vm::OP_PUSH_CONST
        | vm::OP_LOAD_ARG
        | vm::OP_OUT_LIT
        | vm::OP_OUT_ARG
        | vm::OP_SELECT_ARG
        | vm::OP_CASE_STR
        | vm::OP_EXPR_FALLBACK => {
            let id = read_u32(code, base + 1)?;
            if id as usize >= string_count {
                return Err(CatalogError::InvalidStringRef { pc: decoded.pc, id });
            }
        }
        vm::OP_OUT_SLICE | vm::OP_OUT_EXPR => {
            let offset = read_u32(code, base + 1)?;
            let len = read_u32(code, base + 5)?;
            validate_literal_ref(decoded.pc, offset, len, literal_len)?;
        }
        vm::OP_MARKUP_OPEN | vm::OP_MARKUP_CLOSE => {
            let id = read_u32(code, base + 1)?;
            if id as usize >= string_count {
                return Err(CatalogError::InvalidStringRef { pc: decoded.pc, id });
            }
        }
        vm::OP_CALL_FUNC | vm::OP_CALL_SELECT => {
            let fn_id = read_u16(code, base + 1)?;
            if usize::from(fn_id) >= func_count {
                return Err(CatalogError::InvalidFunctionRef {
                    pc: decoded.pc,
                    fn_id,
                });
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_literal_ref(
    pc: u32,
    offset: u32,
    len: u32,
    literal_len: usize,
) -> Result<(), CatalogError> {
    let start = offset as usize;
    let Some(end) = start.checked_add(len as usize) else {
        return Err(CatalogError::InvalidLiteralRef { pc, offset, len });
    };
    if end > literal_len {
        return Err(CatalogError::InvalidLiteralRef { pc, offset, len });
    }
    Ok(())
}

fn verify_stack_safety(
    code: &[u8],
    pcs: &[u32],
    messages: &[MessageEntry],
) -> Result<(), CatalogError> {
    let mut min_depths = vec![None::<u32>; pcs.len()];
    let mut work = Vec::new();
    for message in messages {
        if let Ok(idx) = pcs.binary_search(&message.entry_pc)
            && min_depths[idx].is_none()
        {
            min_depths[idx] = Some(0);
            work.push(message.entry_pc);
        }
    }

    while let Some(pc) = work.pop() {
        let idx = pcs
            .binary_search(&pc)
            .map_err(|_| CatalogError::BadPc { pc })?;
        let depth = min_depths[idx].ok_or(CatalogError::BadPc { pc })?;
        let decoded = vm::decode(code, pc)?;
        let (pops, pushes) = stack_effect(code, decoded)?;
        if depth < pops {
            return Err(CatalogError::BadPc { pc });
        }
        let out_depth = depth - pops + pushes;
        for succ in successor_pcs(decoded, code.len())?.into_iter().flatten() {
            let succ_idx = pcs
                .binary_search(&succ)
                .map_err(|_| CatalogError::BadPc { pc: succ })?;
            let old = min_depths[succ_idx];
            if old.is_none_or(|it| out_depth < it) {
                min_depths[succ_idx] = Some(out_depth);
                work.push(succ);
            }
        }
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ControlState {
    select_depth: u8,
    expr_fallback_pending: bool,
}

fn verify_control_state(
    code: &[u8],
    pcs: &[u32],
    messages: &[MessageEntry],
) -> Result<(), CatalogError> {
    let mut work = Vec::new();
    let mut visited = vec![Vec::<ControlState>::new(); pcs.len()];
    for message in messages {
        if let Ok(idx) = pcs.binary_search(&message.entry_pc) {
            let state = ControlState {
                select_depth: 0,
                expr_fallback_pending: false,
            };
            if !visited[idx].contains(&state) {
                visited[idx].push(state);
                work.push((message.entry_pc, state));
            }
        }
    }

    while let Some((pc, state)) = work.pop() {
        let decoded = vm::decode(code, pc)?;
        let next_state = advance_control_state(decoded, state)?;
        for succ in successor_pcs(decoded, code.len())?.into_iter().flatten() {
            let succ_idx = pcs
                .binary_search(&succ)
                .map_err(|_| CatalogError::BadPc { pc: succ })?;
            if !visited[succ_idx].contains(&next_state) {
                visited[succ_idx].push(next_state);
                work.push((succ, next_state));
            }
        }
    }

    Ok(())
}

fn advance_control_state(
    decoded: vm::Decoded,
    mut state: ControlState,
) -> Result<ControlState, CatalogError> {
    if state.expr_fallback_pending
        && decoded.opcode != vm::OP_CALL_FUNC
        && decoded.opcode != vm::OP_CALL_SELECT
    {
        return Err(CatalogError::InvalidExprFallbackSequence { pc: decoded.pc });
    }

    match decoded.opcode {
        vm::OP_SELECT_ARG | vm::OP_SELECT_BEGIN => {
            state.select_depth =
                state
                    .select_depth
                    .checked_add(1)
                    .ok_or(CatalogError::InvalidSelectSequence {
                        pc: decoded.pc,
                        opcode: decoded.opcode,
                    })?;
        }
        vm::OP_CASE_STR | vm::OP_CASE_DEFAULT => {
            if state.select_depth == 0 {
                return Err(CatalogError::InvalidSelectSequence {
                    pc: decoded.pc,
                    opcode: decoded.opcode,
                });
            }
        }
        vm::OP_SELECT_END => {
            if state.select_depth == 0 {
                return Err(CatalogError::InvalidSelectSequence {
                    pc: decoded.pc,
                    opcode: decoded.opcode,
                });
            }
            state.select_depth -= 1;
        }
        vm::OP_EXPR_FALLBACK => {
            state.expr_fallback_pending = true;
            return Ok(state);
        }
        vm::OP_CALL_FUNC | vm::OP_CALL_SELECT => {
            state.expr_fallback_pending = false;
        }
        _ => {}
    }

    Ok(state)
}

fn stack_effect(code: &[u8], decoded: vm::Decoded) -> Result<(u32, u32), CatalogError> {
    let base = decoded.pc as usize;
    let effect = match decoded.opcode {
        vm::OP_JMP_IF_FALSE | vm::OP_OUT_VAL | vm::OP_SELECT_BEGIN => (1, 0),
        vm::OP_PUSH_CONST | vm::OP_LOAD_ARG => (0, 1),
        vm::OP_OUT_ARG | vm::OP_SELECT_ARG => (0, 0),
        vm::OP_CALL_FUNC | vm::OP_CALL_SELECT => {
            let arg_count = u32::from(code[base + 3]);
            let optc = u32::from(code[base + 4]);
            let pops = arg_count + optc.saturating_mul(2);
            (pops, 1)
        }
        vm::OP_MARKUP_OPEN | vm::OP_MARKUP_CLOSE => {
            let optc = u32::from(code[base + 5]);
            let pops = optc.saturating_mul(2);
            (pops, 0)
        }
        _ => (0, 0),
    };
    Ok(effect)
}

fn successor_pcs(decoded: vm::Decoded, code_len: usize) -> Result<[Option<u32>; 2], CatalogError> {
    // VM bytecode only has linear, jump-only, or conditional flow, so verifier
    // successor sets are bounded at two edges. Keep this fixed-size to avoid a
    // heap allocation for every decoded instruction during verification.
    let mut out = [None, None];
    match decoded.flow_kind() {
        vm::FlowKind::Stop => {}
        vm::FlowKind::Linear => {
            if (decoded.next_pc as usize) < code_len {
                out[0] = Some(decoded.next_pc);
            }
        }
        vm::FlowKind::JumpOnly => {
            if let Some(target) = decoded.jump_target() {
                if target < 0 {
                    return Err(CatalogError::BadJump {
                        from_pc: decoded.pc,
                        to_pc: target,
                    });
                }
                out[0] = Some(u32::try_from(target).map_err(|_| CatalogError::BadJump {
                    from_pc: decoded.pc,
                    to_pc: target,
                })?);
            }
        }
        vm::FlowKind::Conditional => {
            if (decoded.next_pc as usize) < code_len {
                out[0] = Some(decoded.next_pc);
            }
            if let Some(target) = decoded.jump_target() {
                if target < 0 {
                    return Err(CatalogError::BadJump {
                        from_pc: decoded.pc,
                        to_pc: target,
                    });
                }
                out[1] = Some(u32::try_from(target).map_err(|_| CatalogError::BadJump {
                    from_pc: decoded.pc,
                    to_pc: target,
                })?);
            }
        }
    }
    Ok(out)
}

fn has_overlapping_chunks(chunks: &[ChunkRef]) -> bool {
    let mut ranges = chunks
        .iter()
        .map(|chunk| chunk.range.clone())
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start);
    for pair in ranges.windows(2) {
        let left = &pair[0];
        let right = &pair[1];
        if left.end > right.start {
            return true;
        }
    }
    false
}

/// Reusable scratch for `halts_reachable`'s per-message DFS.
///
/// The visited buffer is a generational epoch table: each message bumps `epoch`
/// and marks `visited[idx] = epoch`. A slot is "visited for the current pass"
/// iff `visited[idx] == epoch`. This avoids re-zeroing `visited.len()` bytes
/// between messages — the dominant cost for catalogs with many small messages.
struct HaltsScratch {
    visited: Vec<u32>,
    stack: Vec<u32>,
    epoch: u32,
}

impl HaltsScratch {
    fn new(pc_count: usize) -> Self {
        Self {
            visited: vec![0; pc_count],
            stack: Vec::new(),
            epoch: 0,
        }
    }

    /// Begin a new DFS pass. Bumps the epoch; on `u32` wrap, re-zeroes the
    /// buffer so a stale slot can never collide with the new epoch value.
    fn begin_pass(&mut self) -> u32 {
        if let Some(next) = self.epoch.checked_add(1) {
            self.epoch = next;
        } else {
            self.visited.fill(0);
            self.epoch = 1;
        }
        self.stack.clear();
        self.epoch
    }
}

fn halts_reachable(
    code: &[u8],
    pcs: &[u32],
    entry_pc: u32,
    scratch: &mut HaltsScratch,
) -> Result<bool, CatalogError> {
    let epoch = scratch.begin_pass();
    scratch.stack.push(entry_pc);

    while let Some(pc) = scratch.stack.pop() {
        let idx = match pcs.binary_search(&pc) {
            Ok(index) => index,
            Err(_) => {
                return Err(CatalogError::BadPc { pc });
            }
        };
        if scratch.visited[idx] == epoch {
            continue;
        }
        scratch.visited[idx] = epoch;

        let decoded = vm::decode(code, pc)?;
        if decoded.opcode == vm::OP_HALT {
            return Ok(true);
        }

        match decoded.flow_kind() {
            vm::FlowKind::Stop => {}
            vm::FlowKind::Linear => {
                if (decoded.next_pc as usize) < code.len() {
                    scratch.stack.push(decoded.next_pc);
                }
            }
            vm::FlowKind::JumpOnly => {
                if let Some(target) = decoded.jump_target() {
                    scratch.stack.push(u32::try_from(target).map_err(|_| {
                        CatalogError::BadJump {
                            from_pc: pc,
                            to_pc: target,
                        }
                    })?);
                }
            }
            vm::FlowKind::Conditional => {
                if (decoded.next_pc as usize) < code.len() {
                    scratch.stack.push(decoded.next_pc);
                }
                if let Some(target) = decoded.jump_target() {
                    scratch.stack.push(u32::try_from(target).map_err(|_| {
                        CatalogError::BadJump {
                            from_pc: pc,
                            to_pc: target,
                        }
                    })?);
                }
            }
        }
    }

    Ok(false)
}

fn read_u16(bytes: &[u8], pos: usize) -> Result<u16, CatalogError> {
    let raw = bytes
        .get(pos..pos + 2)
        .ok_or(CatalogError::ChunkOutOfBounds)?;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

fn read_u32(bytes: &[u8], pos: usize) -> Result<u32, CatalogError> {
    let raw = bytes
        .get(pos..pos + 4)
        .ok_or(CatalogError::ChunkOutOfBounds)?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

pub(crate) fn read_i32(bytes: &[u8], pos: usize) -> Result<i32, CatalogError> {
    let raw = bytes
        .get(pos..pos + 4)
        .ok_or(CatalogError::ChunkOutOfBounds)?;
    Ok(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

/// Build a deterministic minimal catalog for internal tests or bootstrap paths.
///
/// Hidden from normal docs because general callers should obtain catalogs from
/// the compiler, not assemble them through the runtime crate.
#[doc(hidden)]
#[must_use]
pub fn build_catalog(
    strings: &[&str],
    literals: &str,
    messages: &[MessageEntry],
    code: &[u8],
) -> Vec<u8> {
    build_catalog_with_funcs(strings, literals, messages, code, &[])
}

/// Build a catalog with an optional `FUNC` chunk for structured function entries.
///
/// Hidden from normal docs because general callers should obtain catalogs from
/// the compiler, not assemble them through the runtime crate.
#[doc(hidden)]
#[must_use]
pub fn build_catalog_with_funcs(
    strings: &[&str],
    literals: &str,
    messages: &[MessageEntry],
    code: &[u8],
    funcs: &[FuncEntry],
) -> Vec<u8> {
    let mut strs_index = Vec::with_capacity(strings.len());
    let mut strs_bytes = Vec::new();
    for value in strings {
        let off = usize_to_u32(strs_bytes.len());
        strs_bytes.extend_from_slice(value.as_bytes());
        strs_index.push((off, usize_to_u32(value.len())));
    }

    let mut strs_chunk = Vec::new();
    strs_chunk.extend_from_slice(&usize_to_u32(strings.len()).to_le_bytes());
    for (off, len) in &strs_index {
        strs_chunk.extend_from_slice(&off.to_le_bytes());
        strs_chunk.extend_from_slice(&len.to_le_bytes());
    }
    strs_chunk.extend_from_slice(&strs_bytes);

    let mut lits_chunk = Vec::new();
    lits_chunk.extend_from_slice(literals.as_bytes());

    let mut msgs_chunk = Vec::new();
    msgs_chunk.extend_from_slice(&usize_to_u32(messages.len()).to_le_bytes());
    for message in messages {
        msgs_chunk.extend_from_slice(&message.name_str_id.to_le_bytes());
        msgs_chunk.extend_from_slice(&message.entry_pc.to_le_bytes());
    }

    let mut code_chunk = Vec::new();
    code_chunk.extend_from_slice(&usize_to_u32(code.len()).to_le_bytes());
    code_chunk.extend_from_slice(code);

    let mut chunks = vec![
        (TAG_STRS, strs_chunk),
        (TAG_LITS, lits_chunk),
        (TAG_MSGS, msgs_chunk),
        (TAG_CODE, code_chunk),
    ];

    if !funcs.is_empty() {
        let mut func_chunk = Vec::new();
        func_chunk.extend_from_slice(&usize_to_u32(funcs.len()).to_le_bytes());
        for entry in funcs {
            func_chunk.extend_from_slice(&entry.name_str_id.to_le_bytes());
            func_chunk.extend_from_slice(&usize_to_u32(entry.static_options.len()).to_le_bytes());
            for (key_id, val_id) in &entry.static_options {
                func_chunk.extend_from_slice(&key_id.to_le_bytes());
                func_chunk.extend_from_slice(&val_id.to_le_bytes());
            }
        }
        chunks.push((TAG_FUNC, func_chunk));
    }

    let chunk_count = usize_to_u32(chunks.len());
    let chunk_table_offset = usize_to_u32(HEADER_LEN);
    let chunk_table_len = usize_to_u32(CHUNK_ENTRY_LEN) * chunk_count;
    let mut body_offset = usize_to_u32(HEADER_LEN) + chunk_table_len;

    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION_MAJOR.to_le_bytes());
    out.extend_from_slice(&VERSION_MINOR.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&chunk_count.to_le_bytes());
    out.extend_from_slice(&chunk_table_offset.to_le_bytes());

    for (tag, chunk) in &chunks {
        out.extend_from_slice(tag);
        out.extend_from_slice(&body_offset.to_le_bytes());
        out.extend_from_slice(&usize_to_u32(chunk.len()).to_le_bytes());
        out.extend_from_slice(&0_u32.to_le_bytes());
        body_offset += usize_to_u32(chunk.len());
    }

    for (_, chunk) in chunks {
        out.extend_from_slice(&chunk);
    }

    out
}

fn usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("catalog data exceeds u32 addressable limits")
}

/// Deterministic ordering helper for compiler-emitted message entries.
#[doc(hidden)]
pub fn sort_messages(entries: &mut [MessageEntry]) {
    entries.sort_by(
        |left, right| match left.name_str_id.cmp(&right.name_str_id) {
            Ordering::Equal => left.entry_pc.cmp(&right.entry_pc),
            other => other,
        },
    );
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;
    use crate::schema::TestOps;
    use crate::vm::{OP_CASE_STR, OP_HALT};

    fn chunk_entry_offset(index: usize) -> usize {
        HEADER_LEN + (index * CHUNK_ENTRY_LEN)
    }

    #[test]
    fn catalog_round_trip_minimal() {
        let code = TestOps::new().out_slice(0, 5).halt().build();
        let bytes = build_catalog(
            &["hello"],
            "Hello",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let catalog = Catalog::from_bytes(&bytes).expect("valid catalog");
        assert_eq!(catalog.message_pc("hello"), Some(0));
        assert_eq!(catalog.string(0).expect("string 0"), "hello");
        assert_eq!(catalog.literal(0, 5).expect("lit"), "Hello");
        assert!(catalog.is_instruction_boundary(0));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let code = [OP_HALT];
        let mut bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        bytes[8..10].copy_from_slice(&2_u16.to_le_bytes());
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(err, CatalogError::UnsupportedVersion { major: 2, minor: 0 });
    }

    #[test]
    fn missing_required_chunk_is_rejected() {
        let code = [OP_HALT];
        let mut bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        bytes[chunk_entry_offset(0)..chunk_entry_offset(0) + 4].copy_from_slice(b"JUNK");
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(err, CatalogError::MissingChunk("STRS"));
    }

    #[test]
    fn bad_jump_fails_verification() {
        let code = TestOps::new().halt().jmp_rel(99).build();
        let bytes = build_catalog(
            &["hello"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert!(matches!(err, CatalogError::BadJump { .. }));
    }

    #[test]
    fn overlapping_chunks_fail_verification() {
        let code = [OP_HALT];
        let mut bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let strs_off = read_u32(&bytes, chunk_entry_offset(0) + 4).expect("strs offset");
        bytes[chunk_entry_offset(1) + 4..chunk_entry_offset(1) + 8]
            .copy_from_slice(&strs_off.to_le_bytes());
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert!(matches!(err, CatalogError::ChunkOutOfBounds));
    }

    #[test]
    fn duplicate_required_chunk_fails_verification() {
        let code = [OP_HALT];
        let mut bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        bytes[chunk_entry_offset(1)..chunk_entry_offset(1) + 4].copy_from_slice(b"STRS");
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert!(matches!(err, CatalogError::ChunkOutOfBounds));
    }

    #[test]
    fn truncated_code_chunk_fails_decode() {
        let code = [OP_HALT];
        let mut bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let code_len_pos = read_u32(&bytes, chunk_entry_offset(3) + 4).expect("code off") as usize;
        bytes[code_len_pos..code_len_pos + 4].copy_from_slice(&10_u32.to_le_bytes());
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert!(matches!(err, CatalogError::ChunkOutOfBounds));
    }

    #[test]
    fn invalid_utf8_string_table_is_rejected() {
        let code = [OP_HALT];
        let mut bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let strs_off = read_u32(&bytes, chunk_entry_offset(0) + 4).expect("strs off") as usize;
        let data_off = strs_off + 4 + 8;
        bytes[data_off] = 0xFF;
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert!(matches!(err, CatalogError::InvalidUtf8));
    }

    #[test]
    fn invalid_utf8_literals_table_is_rejected() {
        let code = [OP_HALT];
        let mut bytes = build_catalog(
            &["main"],
            "ok",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let lits_off = read_u32(&bytes, chunk_entry_offset(1) + 4).expect("lits off") as usize;
        bytes[lits_off] = 0xFF;
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert!(matches!(err, CatalogError::InvalidUtf8));
    }

    #[test]
    fn invalid_message_name_ref_is_rejected() {
        let code = [OP_HALT];
        let bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 99,
                entry_pc: 0,
            }],
            &code,
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(
            err,
            CatalogError::InvalidMessageNameRef { index: 0, id: 99 }
        );
    }

    #[test]
    fn unsorted_message_table_is_rejected() {
        let code = [OP_HALT];
        let bytes = build_catalog(
            &["main", "z", "a"],
            "",
            &[
                MessageEntry {
                    name_str_id: 1,
                    entry_pc: 0,
                },
                MessageEntry {
                    name_str_id: 2,
                    entry_pc: 0,
                },
            ],
            &code,
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(err, CatalogError::InvalidMessageOrder { index: 1 });
    }

    #[test]
    fn unterminated_entry_is_rejected() {
        let code = TestOps::new().out_slice(0, 1).build();
        let bytes = build_catalog(
            &["main"],
            "x",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(err, CatalogError::UnterminatedEntry { entry_pc: 0 });
    }

    #[test]
    fn stack_underflowing_bytecode_is_rejected() {
        let code = TestOps::new().out_val().halt().build();
        let bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert!(matches!(err, CatalogError::BadPc { pc: 0 }));
    }

    #[test]
    fn out_arg_bytecode_is_verified() {
        let code = TestOps::new().out_arg(1).halt().build();
        let bytes = build_catalog(
            &["main", "name"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let catalog = Catalog::from_bytes(&bytes).expect("valid catalog");
        assert_eq!(catalog.code(), code.as_slice());
    }

    #[test]
    fn invalid_output_string_ref_is_rejected() {
        let code = TestOps::new().out_lit(99).halt().build();
        let bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(err, CatalogError::InvalidStringRef { pc: 0, id: 99 });
    }

    #[test]
    fn invalid_literal_slice_ref_is_rejected() {
        let code = TestOps::new().out_slice(4, 4).halt().build();
        let bytes = build_catalog(
            &["main"],
            "abc",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(
            err,
            CatalogError::InvalidLiteralRef {
                pc: 0,
                offset: 4,
                len: 4
            }
        );
    }

    #[test]
    fn select_arg_bytecode_is_verified() {
        let code = TestOps::new()
            .select_arg(1)
            .case_default("end")
            .out_slice(0, 1)
            .label("end")
            .select_end()
            .halt()
            .build();
        let bytes = build_catalog(
            &["main", "sel"],
            "x",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let catalog = Catalog::from_bytes(&bytes).expect("valid catalog");
        assert_eq!(catalog.code(), code.as_slice());
    }

    #[test]
    fn case_without_active_selector_is_rejected() {
        let code = TestOps::new().case_str_rel(1, 0).halt().build();
        let bytes = build_catalog(
            &["main", "one"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(
            err,
            CatalogError::InvalidSelectSequence {
                pc: 0,
                opcode: OP_CASE_STR
            }
        );
    }

    #[test]
    fn expr_fallback_without_immediate_call_is_rejected() {
        let code = TestOps::new().expr_fallback(1).halt().build();
        let bytes = build_catalog(
            &["main", "{$value}"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(err, CatalogError::InvalidExprFallbackSequence { pc: 5 });
    }

    #[test]
    fn invalid_function_ref_is_rejected() {
        let code = TestOps::new().call_func(9, 0, 0).out_val().halt().build();
        let bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(err, CatalogError::InvalidFunctionRef { pc: 0, fn_id: 9 });
    }

    #[test]
    fn call_func_stack_requirements_are_verified() {
        let code = TestOps::new()
            .load_arg(0)
            .call_func(0, 2, 0)
            .out_val()
            .halt()
            .build();
        let bytes = build_catalog_with_funcs(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
            &[FuncEntry {
                name_str_id: 0,
                static_options: vec![],
            }],
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert!(matches!(err, CatalogError::BadPc { pc: 5 }));
    }

    #[test]
    fn func_chunk_round_trip() {
        let code = [OP_HALT];
        let funcs = [
            FuncEntry {
                name_str_id: 1,
                static_options: vec![(2, 3)],
            },
            FuncEntry {
                name_str_id: 4,
                static_options: vec![],
            },
        ];
        // strings: 0=main, 1=number, 2=select, 3=plural, 4=integer
        let bytes = build_catalog_with_funcs(
            &["main", "number", "select", "plural", "integer"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
            &funcs,
        );
        let catalog = Catalog::from_bytes(&bytes).expect("valid catalog");
        assert_eq!(catalog.func_count(), 2);

        let f0 = catalog.func(0).expect("func 0");
        assert_eq!(f0.name_str_id, 1);
        assert_eq!(f0.static_options, vec![(2, 3)]);

        let f1 = catalog.func(1).expect("func 1");
        assert_eq!(f1.name_str_id, 4);
        assert!(f1.static_options.is_empty());

        assert!(catalog.func(2).is_none());
    }

    #[test]
    fn invalid_function_name_ref_is_rejected() {
        let code = [OP_HALT];
        let bytes = build_catalog_with_funcs(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
            &[FuncEntry {
                name_str_id: 99,
                static_options: vec![],
            }],
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(
            err,
            CatalogError::InvalidFunctionNameRef { index: 0, id: 99 }
        );
    }

    #[test]
    fn invalid_function_option_refs_are_rejected() {
        let code = [OP_HALT];
        let bytes = build_catalog_with_funcs(
            &["main", "number"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
            &[FuncEntry {
                name_str_id: 1,
                static_options: vec![(99, 100)],
            }],
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(
            err,
            CatalogError::InvalidFunctionOptionKeyRef { index: 0, id: 99 }
        );
    }

    #[test]
    fn invalid_function_option_value_ref_is_rejected() {
        let code = [OP_HALT];
        let bytes = build_catalog_with_funcs(
            &["main", "number", "style"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
            &[FuncEntry {
                name_str_id: 1,
                static_options: vec![(2, 100)],
            }],
        );
        let err = Catalog::from_bytes(&bytes).expect_err("must fail");
        assert_eq!(
            err,
            CatalogError::InvalidFunctionOptionValueRef { index: 0, id: 100 }
        );
    }

    #[test]
    fn catalog_without_func_chunk_has_empty_funcs() {
        let code = [OP_HALT];
        let bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );
        let catalog = Catalog::from_bytes(&bytes).expect("valid catalog");
        assert_eq!(catalog.func_count(), 0);
        assert!(catalog.func(0).is_none());
    }

    #[test]
    fn unknown_chunk_tag_is_ignored() {
        let code = [OP_HALT];
        let bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &code,
        );

        // Rebuild with an extra unknown chunk by patching the binary:
        // 1. Parse the original to get chunk table structure.
        // 2. Insert a new chunk entry + payload.
        let orig_chunk_count = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let extra_payload = b"unknown data";
        let extra_tag = *b"XTRA";

        // New binary: header + (orig_chunk_count+1) chunk entries + original payloads + extra payload.
        let new_chunk_count = orig_chunk_count + 1;
        let old_table_end = HEADER_LEN + (orig_chunk_count as usize * CHUNK_ENTRY_LEN);
        let shift = CHUNK_ENTRY_LEN; // extra entry shifts payloads
        let shift_u32 = u32::try_from(shift).expect("shift must fit into u32");

        let mut patched = Vec::new();
        // Copy header, update chunk_count.
        patched.extend_from_slice(&bytes[..16]);
        patched.extend_from_slice(&new_chunk_count.to_le_bytes());
        patched.extend_from_slice(&bytes[20..24]);

        // Copy original chunk table entries, shifting each payload offset.
        for i in 0..orig_chunk_count as usize {
            let entry_start = HEADER_LEN + i * CHUNK_ENTRY_LEN;
            let tag = &bytes[entry_start..entry_start + 4];
            let orig_off =
                u32::from_le_bytes(bytes[entry_start + 4..entry_start + 8].try_into().unwrap());
            let len = &bytes[entry_start + 8..entry_start + 12];
            let reserved = &bytes[entry_start + 12..entry_start + 16];
            patched.extend_from_slice(tag);
            patched.extend_from_slice(&(orig_off + shift_u32).to_le_bytes());
            patched.extend_from_slice(len);
            patched.extend_from_slice(reserved);
        }

        // Append extra chunk table entry pointing after all original payloads.
        let extra_offset =
            u32::try_from(bytes.len() + shift).expect("catalog bytes must fit into u32");
        patched.extend_from_slice(&extra_tag);
        patched.extend_from_slice(&extra_offset.to_le_bytes());
        patched.extend_from_slice(
            &u32::try_from(extra_payload.len())
                .expect("extra payload length must fit into u32")
                .to_le_bytes(),
        );
        patched.extend_from_slice(&0_u32.to_le_bytes());

        // Copy original payloads.
        patched.extend_from_slice(&bytes[old_table_end..]);
        // Append extra payload.
        patched.extend_from_slice(extra_payload);

        let catalog = Catalog::from_bytes(&patched).expect("unknown chunk should be ignored");
        assert_eq!(catalog.message_pc("main"), Some(0));
    }

    #[test]
    fn mutated_and_garbage_catalog_bytes_do_not_panic() {
        let base = build_catalog(
            &["main", "name"],
            "Hello",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &TestOps::new().out_slice(0, 5).halt().build(),
        );

        let mut seed = 0x4d46_4341_545f_4655_u64;
        for _ in 0..512 {
            let mut candidate = base.clone();
            mutate_bytes(&mut candidate, &mut seed);
            let _ = Catalog::from_bytes(&candidate);
        }

        for len in 0..128 {
            let mut garbage = vec![0_u8; len];
            for byte in &mut garbage {
                *byte = next_u8(&mut seed);
            }
            let _ = Catalog::from_bytes(&garbage);
        }
    }

    /// Three messages share the `HALT` at pc 0. If `HaltsScratch` failed to
    /// reset its visited buffer between messages, `msg_b` and `msg_c`'s DFS
    /// would see pc 0 as already-visited (from `main`) and report `UnterminatedEntry`.
    #[test]
    fn halts_reachable_buffer_is_reset_between_messages() {
        // pc 0:  OP_HALT          (entry: "main")
        // pc 1:  OP_JMP rel=-6    (entry: "msg_b")  next_pc=6,  target=0
        // pc 6:  OP_JMP rel=-11   (entry: "msg_c")  next_pc=11, target=0
        // pc 11: OP_HALT          (trailing sentinel so the code ends in HALT)
        let code = TestOps::new()
            .halt()
            .jmp_rel(-6)
            .jmp_rel(-11)
            .halt()
            .build();

        let bytes = build_catalog(
            &["main", "msg_b", "msg_c"],
            "",
            &[
                MessageEntry {
                    name_str_id: 0,
                    entry_pc: 0,
                },
                MessageEntry {
                    name_str_id: 1,
                    entry_pc: 1,
                },
                MessageEntry {
                    name_str_id: 2,
                    entry_pc: 6,
                },
            ],
            &code,
        );
        let catalog = Catalog::from_bytes(&bytes).expect("all three messages must verify");
        assert_eq!(catalog.message_pc("main"), Some(0));
        assert_eq!(catalog.message_pc("msg_b"), Some(1));
        assert_eq!(catalog.message_pc("msg_c"), Some(6));
    }

    /// Direct test of `HaltsScratch` epoch sequencing: each `begin_pass`
    /// observes a fresh epoch, and on `u32` wrap the visited buffer is
    /// re-zeroed so a stale slot can never coincide with the new epoch.
    #[test]
    fn halts_scratch_epoch_sequencing_and_wrap() {
        let mut scratch = HaltsScratch::new(4);
        // First few passes produce strictly increasing epoch values.
        let e1 = scratch.begin_pass();
        let e2 = scratch.begin_pass();
        let e3 = scratch.begin_pass();
        assert_eq!(e1, 1);
        assert_eq!(e2, 2);
        assert_eq!(e3, 3);
        // Stack is reset on each pass.
        assert!(scratch.stack.is_empty());

        // Plant a stale mark from a prior pass, simulating real DFS work.
        scratch.visited[2] = e3;
        // Force the epoch to the brink of overflow.
        scratch.epoch = u32::MAX;
        let after_wrap = scratch.begin_pass();
        assert_eq!(after_wrap, 1, "epoch must reset to 1 after u32 saturation");
        // The wrap path must zero the buffer so the stale mark cannot collide
        // with any future epoch value.
        assert!(
            scratch.visited.iter().all(|&v| v == 0),
            "visited buffer must be re-zeroed on wrap"
        );
    }

    fn mutate_bytes(bytes: &mut Vec<u8>, seed: &mut u64) {
        if bytes.is_empty() {
            return;
        }
        let flips = 1 + usize::from(next_u8(seed) % 8);
        for _ in 0..flips {
            let idx = next_usize(seed) % bytes.len();
            let delta = next_u8(seed).wrapping_add(1);
            bytes[idx] = bytes[idx].wrapping_add(delta);
        }

        match next_u64(seed) % 4 {
            0 => {
                let trim = next_usize(seed) % bytes.len();
                bytes.truncate(bytes.len().saturating_sub(trim));
            }
            1 => {
                let extra = usize::from(next_u8(seed) % 16);
                for _ in 0..extra {
                    bytes.push(next_u8(seed));
                }
            }
            _ => {}
        }
    }

    fn next_u8(seed: &mut u64) -> u8 {
        next_u64(seed).to_le_bytes()[0]
    }

    fn next_usize(seed: &mut u64) -> usize {
        let bytes = next_u64(seed).to_le_bytes();
        let mut out = [0_u8; size_of::<usize>()];
        out.copy_from_slice(&bytes[..size_of::<usize>()]);
        usize::from_le_bytes(out)
    }

    fn next_u64(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }
}
