// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Long-running select workloads intended for external profilers (`sample`, etc.).
//!
//! Usage:
//! `cargo run --release -p wind_tunnel --bin profile_select -- plain`
//! `cargo run --release -p wind_tunnel --bin profile_select -- builtin-plural`
//! `cargo run --release -p wind_tunnel --bin profile_select -- builtin-number`
//! `cargo run --release -p wind_tunnel --bin profile_select -- builtin-datetime`
//!
//! Optional env var:
//! `ITERS` (default: 300000000)

use std::{env, hint::black_box};

use icu_locale_core::Locale;
use message_format_compiler::compile_str;
use message_format_runtime::{
    BuiltinHost, Catalog, FormatSink, Formatter, MessageArgs, Value,
    catalog::{FuncEntry, MessageEntry, build_catalog, build_catalog_with_funcs},
    vm,
};

struct OutputStringSink<'a> {
    out: &'a mut String,
}

impl FormatSink for OutputStringSink<'_> {
    fn literal(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn expression(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn markup_open(&mut self, _name: &str, _options: &[message_format_runtime::FormatOption<'_>]) {}

    fn markup_close(&mut self, _name: &str, _options: &[message_format_runtime::FormatOption<'_>]) {
    }
}

fn locale(tag: &str) -> Locale {
    tag.parse::<Locale>().expect("locale")
}

fn compile_catalog(source: &str) -> Catalog {
    let bytes = compile_str(source).expect("valid profiling source");
    Catalog::from_bytes(&bytes).expect("valid profiling catalog")
}

fn build_plain_select_catalog() -> Catalog {
    let strings = ["main", "kind", "formal"];
    let code = vec![
        vm::OP_LOAD_ARG,
        1,
        0,
        0,
        0, // selector: $kind
        vm::OP_SELECT_BEGIN,
        vm::OP_CASE_STR,
        2,
        0,
        0,
        0, // "formal"
        5,
        0,
        0,
        0, // jump to formal output
        vm::OP_CASE_DEFAULT,
        9,
        0,
        0,
        0, // jump to default output
        vm::OP_OUT_SLICE,
        0,
        0,
        0,
        0,
        2,
        0,
        0,
        0, // "Hi"
        vm::OP_JMP,
        9,
        0,
        0,
        0, // jump after formal branch
        vm::OP_OUT_SLICE,
        2,
        0,
        0,
        0,
        12,
        0,
        0,
        0, // "Good evening"
        vm::OP_SELECT_END,
        vm::OP_HALT,
    ];
    let bytes = build_catalog(
        &strings,
        "HiGood evening",
        &[MessageEntry {
            name_str_id: 0,
            entry_pc: 0,
        }],
        &code,
    );
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn build_plural_select_catalog() -> Catalog {
    let strings = [
        "count", "few", "main", "many", "number", "one", "other", "plural", "select", "two", "zero",
    ];
    let literals = "one itemmany items";
    let funcs = [FuncEntry {
        name_str_id: 4,
        static_options: vec![(8, 7)],
    }];
    let code = vec![
        vm::OP_LOAD_ARG,
        0,
        0,
        0,
        0,
        vm::OP_CALL_SELECT,
        0,
        0,
        1,
        0,
        vm::OP_SELECT_BEGIN,
        vm::OP_CASE_STR,
        5,
        0,
        0,
        0,
        5,
        0,
        0,
        0,
        vm::OP_CASE_DEFAULT,
        14,
        0,
        0,
        0,
        vm::OP_OUT_SLICE,
        0,
        0,
        0,
        0,
        8,
        0,
        0,
        0,
        vm::OP_JMP,
        9,
        0,
        0,
        0,
        vm::OP_OUT_SLICE,
        8,
        0,
        0,
        0,
        10,
        0,
        0,
        0,
        vm::OP_SELECT_END,
        vm::OP_HALT,
    ];
    let bytes = build_catalog_with_funcs(
        &strings,
        literals,
        &[MessageEntry {
            name_str_id: 2,
            entry_pc: 0,
        }],
        &code,
        &funcs,
    );
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn build_number_percent_catalog() -> Catalog {
    compile_catalog("main = { $amount :number style=percent minimumFractionDigits=1 }")
}

fn build_datetime_catalog() -> Catalog {
    compile_catalog("main = { $date :datetime }")
}

fn message_args<'a>(catalog: &'a Catalog, entries: &[(&str, Value)]) -> MessageArgs<'a> {
    let mut args = MessageArgs::with_capacity(catalog, entries.len());
    for (name, value) in entries {
        args.insert(name, value.clone()).expect("arg name");
    }
    args
}

fn main() {
    let mode = env::args().nth(1).unwrap_or_else(|| "plain".to_string());
    let iters: u64 = env::var("ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300_000_000);

    match mode.as_str() {
        "plain" => {
            let catalog = build_plain_select_catalog();
            let mut formatter =
                Formatter::new(&catalog, message_format_runtime::NoopHost).expect("formatter");
            let message = formatter.resolve("main").expect("resolved message");
            let args = message_args(&catalog, &[("kind", Value::Str("formal".to_string()))]);
            let mut checksum = 0_usize;
            let mut out = String::new();
            for _ in 0..iters {
                out.clear();
                let mut sink = OutputStringSink { out: &mut out };
                let _diagnostics = formatter
                    .format_to(message, &args, &mut sink)
                    .expect("format");
                checksum = checksum.wrapping_add(black_box(out.len()));
            }
            println!("mode=plain checksum={checksum}");
        }
        "builtin-plural" => {
            let catalog = build_plural_select_catalog();
            let locale = locale("en-US");
            let host = BuiltinHost::new(&locale).expect("host");
            let mut formatter = Formatter::new(&catalog, host).expect("formatter");
            let message = formatter.resolve("main").expect("resolved message");
            let args = message_args(&catalog, &[("count", Value::Int(1))]);
            let mut checksum = 0_usize;
            let mut out = String::new();
            for _ in 0..iters {
                out.clear();
                let mut sink = OutputStringSink { out: &mut out };
                let _diagnostics = formatter
                    .format_to(message, &args, &mut sink)
                    .expect("format");
                checksum = checksum.wrapping_add(black_box(out.len()));
            }
            println!("mode=builtin-plural checksum={checksum}");
        }
        "builtin-number" => {
            let catalog = build_number_percent_catalog();
            let locale = locale("en-US");
            let host = BuiltinHost::new(&locale).expect("host");
            let mut formatter = Formatter::new(&catalog, host).expect("formatter");
            let message = formatter.resolve("main").expect("resolved message");
            let args = message_args(&catalog, &[("amount", Value::Float(42.125))]);
            let mut checksum = 0_usize;
            let mut out = String::new();
            for _ in 0..iters {
                out.clear();
                let mut sink = OutputStringSink { out: &mut out };
                let _diagnostics = formatter
                    .format_to(message, &args, &mut sink)
                    .expect("format");
                checksum = checksum.wrapping_add(black_box(out.len()));
            }
            println!("mode=builtin-number checksum={checksum}");
        }
        "builtin-datetime" => {
            let catalog = build_datetime_catalog();
            let locale = locale("en-US");
            let host = BuiltinHost::new(&locale).expect("host");
            let mut formatter = Formatter::new(&catalog, host).expect("formatter");
            let message = formatter.resolve("main").expect("resolved message");
            let args = message_args(
                &catalog,
                &[("date", Value::Str("2024-05-01T14:30:00".to_string()))],
            );
            let mut checksum = 0_usize;
            let mut out = String::new();
            for _ in 0..iters {
                out.clear();
                let mut sink = OutputStringSink { out: &mut out };
                let _diagnostics = formatter
                    .format_to(message, &args, &mut sink)
                    .expect("format");
                checksum = checksum.wrapping_add(black_box(out.len()));
            }
            println!("mode=builtin-datetime checksum={checksum}");
        }
        other => panic!("unknown mode: {other}"),
    }
}
