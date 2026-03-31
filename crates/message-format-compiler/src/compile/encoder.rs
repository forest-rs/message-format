// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::cmp::Ordering;

use message_format_runtime::schema::{FuncEntry, MessageEntry};

use super::CompileError;

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

pub(super) fn encode_catalog(
    strings: &[&str],
    literals: &str,
    messages: &[MessageEntry],
    code: &[u8],
    funcs: &[FuncEntry],
) -> Result<Vec<u8>, CompileError> {
    let mut strs_index = Vec::with_capacity(strings.len());
    let mut strs_bytes = Vec::new();
    for value in strings {
        let off = usize_to_u32(strs_bytes.len())?;
        strs_bytes.extend_from_slice(value.as_bytes());
        strs_index.push((off, usize_to_u32(value.len())?));
    }

    let mut strs_chunk = Vec::new();
    strs_chunk.extend_from_slice(&usize_to_u32(strings.len())?.to_le_bytes());
    for (off, len) in &strs_index {
        strs_chunk.extend_from_slice(&off.to_le_bytes());
        strs_chunk.extend_from_slice(&len.to_le_bytes());
    }
    strs_chunk.extend_from_slice(&strs_bytes);

    let mut lits_chunk = Vec::new();
    lits_chunk.extend_from_slice(literals.as_bytes());

    let mut msgs_chunk = Vec::new();
    msgs_chunk.extend_from_slice(&usize_to_u32(messages.len())?.to_le_bytes());
    for message in messages {
        msgs_chunk.extend_from_slice(&message.name_str_id.to_le_bytes());
        msgs_chunk.extend_from_slice(&message.entry_pc.to_le_bytes());
    }

    let mut code_chunk = Vec::new();
    code_chunk.extend_from_slice(&usize_to_u32(code.len())?.to_le_bytes());
    code_chunk.extend_from_slice(code);

    let mut chunks = vec![
        (TAG_STRS, strs_chunk),
        (TAG_LITS, lits_chunk),
        (TAG_MSGS, msgs_chunk),
        (TAG_CODE, code_chunk),
    ];

    if !funcs.is_empty() {
        let mut func_chunk = Vec::new();
        func_chunk.extend_from_slice(&usize_to_u32(funcs.len())?.to_le_bytes());
        for entry in funcs {
            func_chunk.extend_from_slice(&entry.name_str_id.to_le_bytes());
            func_chunk.extend_from_slice(&usize_to_u32(entry.static_options.len())?.to_le_bytes());
            for (key_id, val_id) in &entry.static_options {
                func_chunk.extend_from_slice(&key_id.to_le_bytes());
                func_chunk.extend_from_slice(&val_id.to_le_bytes());
            }
        }
        chunks.push((TAG_FUNC, func_chunk));
    }

    let chunk_count = usize_to_u32(chunks.len())?;
    let chunk_table_offset = usize_to_u32(HEADER_LEN)?;
    let chunk_table_len = usize_to_u32(CHUNK_ENTRY_LEN)?
        .checked_mul(chunk_count)
        .ok_or(CompileError::size_overflow("catalog chunk table"))?;
    let mut body_offset = usize_to_u32(HEADER_LEN)?
        .checked_add(chunk_table_len)
        .ok_or(CompileError::size_overflow("catalog body offset"))?;

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
        out.extend_from_slice(&usize_to_u32(chunk.len())?.to_le_bytes());
        out.extend_from_slice(&0_u32.to_le_bytes());
        body_offset = body_offset
            .checked_add(usize_to_u32(chunk.len())?)
            .ok_or(CompileError::size_overflow("catalog body offset"))?;
    }

    for (_, chunk) in chunks {
        out.extend_from_slice(&chunk);
    }

    Ok(out)
}

fn usize_to_u32(value: usize) -> Result<u32, CompileError> {
    u32::try_from(value).map_err(|_| CompileError::size_overflow("catalog section"))
}

pub(super) fn sort_messages(entries: &mut [MessageEntry]) {
    entries.sort_by(
        |left, right| match left.name_str_id.cmp(&right.name_str_id) {
            Ordering::Equal => left.entry_pc.cmp(&right.entry_pc),
            other => other,
        },
    );
}
