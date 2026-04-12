// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(
    missing_docs,
    reason = "Criterion-generated benchmark entry points are private harness glue."
)]

use core::hint::black_box;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use icu_locale_core::Locale;
use message_format_compiler::compile_str;
use message_format_runtime::{
    Args, BuiltinHost, Catalog, FormatError, FormatSink, Formatter, Host, HostFn, MessageArgs,
    MessageHandle, NoopHost, Value,
    catalog::{FuncEntry, MessageEntry, build_catalog, build_catalog_with_funcs},
    vm,
};

#[derive(Default)]
struct BenchStringSink {
    out: String,
}

impl FormatSink for BenchStringSink {
    fn literal(&mut self, value: &str) {
        self.out.push_str(value);
    }

    fn expression(&mut self, value: &str) {
        self.out.push_str(value);
    }

    fn markup_open(&mut self, _name: &str, _options: &[message_format_runtime::FormatOption<'_>]) {}

    fn markup_close(&mut self, _name: &str, _options: &[message_format_runtime::FormatOption<'_>]) {
    }
}

trait RuntimeFormatExt<H: Host> {
    fn format_for_bench(
        &mut self,
        message: MessageHandle,
        args: &dyn Args,
    ) -> Result<String, FormatError>;
    fn format_by_id_for_bench(
        &mut self,
        message_id: &str,
        args: &dyn Args,
    ) -> Result<String, FormatError>;
    fn format_to_by_id_for_bench<S: FormatSink + ?Sized>(
        &mut self,
        message_id: &str,
        args: &dyn Args,
        sink: &mut S,
    ) -> Result<Vec<FormatError>, FormatError>;
}

impl<H: Host> RuntimeFormatExt<H> for Formatter<'_, H> {
    fn format_for_bench(
        &mut self,
        message: MessageHandle,
        args: &dyn Args,
    ) -> Result<String, FormatError> {
        let mut sink = BenchStringSink::default();
        let _diagnostics = self.format_to(message, args, &mut sink)?;
        Ok(sink.out)
    }

    fn format_by_id_for_bench(
        &mut self,
        message_id: &str,
        args: &dyn Args,
    ) -> Result<String, FormatError> {
        let message = self.resolve(message_id)?;
        self.format_for_bench(message, args)
    }

    fn format_to_by_id_for_bench<S: FormatSink + ?Sized>(
        &mut self,
        message_id: &str,
        args: &dyn Args,
        sink: &mut S,
    ) -> Result<Vec<FormatError>, FormatError> {
        let message = self.resolve(message_id)?;
        self.format_to(message, args, sink)
    }
}

fn locale(tag: &str) -> Locale {
    tag.parse::<Locale>().expect("locale")
}

fn compile_catalog(source: &str) -> Catalog {
    let bytes = compile_str(source).expect("valid benchmark source");
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn arg_id(catalog: &Catalog, name: &str) -> u32 {
    catalog.string_id(name).expect("arg id")
}

fn message_args<'a>(catalog: &'a Catalog, entries: &[(&str, Value)]) -> MessageArgs<'a> {
    let mut args = MessageArgs::with_capacity(catalog, entries.len());
    for (name, value) in entries {
        args.insert(name, value.clone()).expect("arg name");
    }
    args
}

fn build_plain_catalog() -> Catalog {
    let strings = ["main", "name"];
    let code = vec![
        vm::OP_OUT_SLICE,
        0,
        0,
        0,
        0,
        7,
        0,
        0,
        0, // "Hello, "
        vm::OP_OUT_ARG,
        1,
        0,
        0,
        0, // $name
        vm::OP_OUT_SLICE,
        7,
        0,
        0,
        0,
        1,
        0,
        0,
        0, // "!"
        vm::OP_HALT,
    ];
    let bytes = build_catalog(
        &strings,
        "Hello, !",
        &[MessageEntry {
            name_str_id: 0,
            entry_pc: 0,
        }],
        &code,
    );
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn build_select_catalog() -> Catalog {
    let strings = ["main", "kind", "formal"];
    let code = vec![
        vm::OP_SELECT_ARG,
        1,
        0,
        0,
        0, // selector: $kind
        vm::OP_CASE_STR,
        2,
        0,
        0,
        0, // "formal"
        19,
        0,
        0,
        0, // jump to formal output
        vm::OP_CASE_DEFAULT,
        0,
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

fn build_call_catalog_no_opts() -> Catalog {
    let strings = ["main", "name"];
    let code = vec![
        vm::OP_LOAD_ARG,
        1,
        0,
        0,
        0, // arg $name
        vm::OP_CALL_FUNC,
        1,
        0,
        1,
        0, // fn=1 args=1 opts=0
        vm::OP_OUT_VAL,
        vm::OP_HALT,
    ];
    let bytes = build_catalog(
        &strings,
        "",
        &[MessageEntry {
            name_str_id: 0,
            entry_pc: 0,
        }],
        &code,
    );
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn build_call_catalog_with_opts() -> Catalog {
    let strings = ["main", "name", "style", "short", "currency", "USD"];
    let code = vec![
        vm::OP_LOAD_ARG,
        1,
        0,
        0,
        0, // arg $name
        vm::OP_PUSH_CONST,
        2,
        0,
        0,
        0, // key "style"
        vm::OP_PUSH_CONST,
        3,
        0,
        0,
        0, // val "short"
        vm::OP_PUSH_CONST,
        4,
        0,
        0,
        0, // key "currency"
        vm::OP_PUSH_CONST,
        5,
        0,
        0,
        0, // val "USD"
        vm::OP_CALL_FUNC,
        1,
        0,
        1,
        2, // fn=1 args=1 opts=2
        vm::OP_OUT_VAL,
        vm::OP_HALT,
    ];
    let bytes = build_catalog(
        &strings,
        "",
        &[MessageEntry {
            name_str_id: 0,
            entry_pc: 0,
        }],
        &code,
    );
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn build_many_segments_catalog() -> Catalog {
    let strings = ["main", "x"];
    let mut code = Vec::new();
    // Build a larger output with alternating literal slices and runtime value.
    for i in 0_u32..24_u32 {
        let lit_off = i * 2;
        code.push(vm::OP_OUT_SLICE);
        code.extend_from_slice(&lit_off.to_le_bytes());
        code.extend_from_slice(&2_u32.to_le_bytes());
        code.extend_from_slice(&[vm::OP_OUT_ARG, 1, 0, 0, 0]);
    }
    code.push(vm::OP_HALT);

    let mut lits = String::new();
    for _ in 0..24 {
        lits.push_str("..");
    }
    let bytes = build_catalog(
        &strings,
        &lits,
        &[MessageEntry {
            name_str_id: 0,
            entry_pc: 0,
        }],
        &code,
    );
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn build_markup_catalog() -> Catalog {
    // MARKUP_OPEN "b"(1) optc=0, OUT_SLICE "Hello "(0,6), LOAD_ARG "name"(2), OUT_VAL,
    // MARKUP_CLOSE "b"(1) optc=0, HALT
    let strings = ["main", "b", "name"];
    let code = vec![
        vm::OP_MARKUP_OPEN,
        1,
        0,
        0,
        0,
        0,
        vm::OP_OUT_SLICE,
        0,
        0,
        0,
        0,
        6,
        0,
        0,
        0,
        vm::OP_LOAD_ARG,
        2,
        0,
        0,
        0,
        vm::OP_OUT_VAL,
        vm::OP_MARKUP_CLOSE,
        1,
        0,
        0,
        0,
        0,
        vm::OP_HALT,
    ];
    let bytes = build_catalog(
        &strings,
        "Hello ",
        &[MessageEntry {
            name_str_id: 0,
            entry_pc: 0,
        }],
        &code,
    );
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn build_markup_option_literal_catalog() -> Catalog {
    // PUSH_CONST "href"(2), PUSH_CONST "https://example.com"(3),
    // MARKUP_OPEN "a"(1) optc=1, OUT_SLICE "link"(0,4), MARKUP_CLOSE "a"(1) optc=0, HALT
    let strings = ["main", "a", "href", "https://example.com"];
    let code = vec![
        vm::OP_PUSH_CONST,
        2,
        0,
        0,
        0,
        vm::OP_PUSH_CONST,
        3,
        0,
        0,
        0,
        vm::OP_MARKUP_OPEN,
        1,
        0,
        0,
        0,
        1,
        vm::OP_OUT_SLICE,
        0,
        0,
        0,
        0,
        4,
        0,
        0,
        0,
        vm::OP_MARKUP_CLOSE,
        1,
        0,
        0,
        0,
        0,
        vm::OP_HALT,
    ];
    let bytes = build_catalog(
        &strings,
        "link",
        &[MessageEntry {
            name_str_id: 0,
            entry_pc: 0,
        }],
        &code,
    );
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn build_markup_option_variable_catalog() -> Catalog {
    // PUSH_CONST "href"(2), LOAD_ARG "url"(3),
    // MARKUP_OPEN "a"(1) optc=1, OUT_SLICE "click"(0,5), MARKUP_CLOSE "a"(1) optc=0, HALT
    let strings = ["main", "a", "href", "url"];
    let code = vec![
        vm::OP_PUSH_CONST,
        2,
        0,
        0,
        0,
        vm::OP_LOAD_ARG,
        3,
        0,
        0,
        0,
        vm::OP_MARKUP_OPEN,
        1,
        0,
        0,
        0,
        1,
        vm::OP_OUT_SLICE,
        0,
        0,
        0,
        0,
        5,
        0,
        0,
        0,
        vm::OP_MARKUP_CLOSE,
        1,
        0,
        0,
        0,
        0,
        vm::OP_HALT,
    ];
    let bytes = build_catalog(
        &strings,
        "click",
        &[MessageEntry {
            name_str_id: 0,
            entry_pc: 0,
        }],
        &code,
    );
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn build_builtin_call_catalog_no_opts() -> Catalog {
    let strings = ["main", "amount", "number"];
    let funcs = [FuncEntry {
        name_str_id: 2,
        static_options: vec![],
    }];
    let code = vec![
        vm::OP_LOAD_ARG,
        1,
        0,
        0,
        0, // arg $amount
        vm::OP_CALL_FUNC,
        0,
        0,
        1,
        0, // fn=0 ("number"), args=1, opts=0
        vm::OP_OUT_VAL,
        vm::OP_HALT,
    ];
    let bytes = build_catalog_with_funcs(
        &strings,
        "",
        &[MessageEntry {
            name_str_id: 0,
            entry_pc: 0,
        }],
        &code,
        &funcs,
    );
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn build_builtin_call_catalog_with_opts() -> Catalog {
    let strings = ["main", "amount", "number", "minimumFractionDigits", "2"];
    let funcs = [FuncEntry {
        name_str_id: 2,
        static_options: vec![],
    }];
    let code = vec![
        vm::OP_LOAD_ARG,
        1,
        0,
        0,
        0, // arg $amount
        vm::OP_PUSH_CONST,
        3,
        0,
        0,
        0, // key "minimumFractionDigits"
        vm::OP_PUSH_CONST,
        4,
        0,
        0,
        0, // val "2"
        vm::OP_CALL_FUNC,
        0,
        0,
        1,
        1, // fn=0 ("number"), args=1, opts=1
        vm::OP_OUT_VAL,
        vm::OP_HALT,
    ];
    let bytes = build_catalog_with_funcs(
        &strings,
        "",
        &[MessageEntry {
            name_str_id: 0,
            entry_pc: 0,
        }],
        &code,
        &funcs,
    );
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn build_icu_percent_catalog() -> Catalog {
    compile_catalog("main = { $amount :number style=percent minimumFractionDigits=1 }")
}

fn build_icu_currency_catalog() -> Catalog {
    compile_catalog("main = { $amount :currency currency=USD }")
}

fn build_icu_date_catalog() -> Catalog {
    compile_catalog("main = { $date :date style=short }")
}

fn build_icu_time_catalog() -> Catalog {
    compile_catalog("main = { $date :time style=short }")
}

fn build_icu_datetime_catalog() -> Catalog {
    compile_catalog("main = { $date :datetime }")
}

#[derive(Default)]
struct CountingSink {
    events: usize,
    bytes: usize,
}

impl CountingSink {
    fn reset(&mut self) {
        self.events = 0;
        self.bytes = 0;
    }
}

impl FormatSink for CountingSink {
    fn literal(&mut self, value: &str) {
        self.events += 1;
        self.bytes += value.len();
    }

    fn expression(&mut self, value: &str) {
        self.events += 1;
        self.bytes += value.len();
    }

    fn markup_open(&mut self, name: &str, options: &[message_format_runtime::FormatOption<'_>]) {
        self.events += 1;
        self.bytes += name.len();
        for option in options {
            self.bytes += option.key.len() + option.value.len();
        }
    }

    fn markup_close(&mut self, name: &str, options: &[message_format_runtime::FormatOption<'_>]) {
        self.events += 1;
        self.bytes += name.len();
        for option in options {
            self.bytes += option.key.len() + option.value.len();
        }
    }
}

fn bench_formatting(c: &mut Criterion) {
    let plain_catalog = build_plain_catalog();
    let select_catalog = build_select_catalog();
    let call_catalog_no_opts = build_call_catalog_no_opts();
    let call_catalog_with_opts = build_call_catalog_with_opts();
    let many_segments_catalog = build_many_segments_catalog();
    let markup_catalog = build_markup_catalog();
    let markup_option_literal_catalog = build_markup_option_literal_catalog();
    let markup_option_variable_catalog = build_markup_option_variable_catalog();
    let builtin_catalog_no_opts = build_builtin_call_catalog_no_opts();
    let builtin_catalog_with_opts = build_builtin_call_catalog_with_opts();
    let icu_percent_catalog = build_icu_percent_catalog();
    let icu_currency_catalog = build_icu_currency_catalog();
    let icu_date_catalog = build_icu_date_catalog();
    let icu_time_catalog = build_icu_time_catalog();
    let icu_datetime_catalog = build_icu_datetime_catalog();

    let mut group = c.benchmark_group("runtime_formatting");

    let short_args = message_args(&plain_catalog, &[("name", Value::Str("Ada".to_string()))]);
    group.throughput(Throughput::Elements(1));
    group.bench_function("plain_short_arg", |b| {
        let mut formatter = Formatter::new(&plain_catalog, NoopHost).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&short_args))
                .expect("format");
            black_box(out);
        });
    });

    for len in [16_usize, 64, 256, 1024] {
        let dynamic = "x".repeat(len);
        let args = message_args(&plain_catalog, &[("name", Value::Str(dynamic))]);
        group.bench_with_input(
            BenchmarkId::new("plain_varying_arg_len", len),
            &args,
            |b, a| {
                let mut formatter = Formatter::new(&plain_catalog, NoopHost).expect("formatter");
                b.iter(|| {
                    let out = formatter
                        .format_by_id_for_bench("main", black_box(a))
                        .expect("format");
                    black_box(out);
                });
            },
        );
    }

    let formal_args = message_args(
        &select_catalog,
        &[("kind", Value::Str("formal".to_string()))],
    );
    group.bench_function("select_formal", |b| {
        let mut formatter = Formatter::new(&select_catalog, NoopHost).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&formal_args))
                .expect("format");
            black_box(out);
        });
    });

    let default_args = message_args(
        &select_catalog,
        &[("kind", Value::Str("other".to_string()))],
    );
    group.bench_function("select_default", |b| {
        let mut formatter = Formatter::new(&select_catalog, NoopHost).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&default_args))
                .expect("format");
            black_box(out);
        });
    });

    group.finish();

    let mut call_group = c.benchmark_group("runtime_call_paths");
    call_group.throughput(Throughput::Elements(1));
    let call_args_no_opts = message_args(&call_catalog_no_opts, &[("name", Value::Int(42))]);
    let call_args_with_opts = message_args(&call_catalog_with_opts, &[("name", Value::Int(42))]);
    call_group.bench_function("call_no_opts_passthrough", |b| {
        let mut formatter = Formatter::new(
            &call_catalog_no_opts,
            HostFn(|_fn_id, args, _opts| Ok(args.first().cloned().unwrap_or(Value::Null))),
        )
        .expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&call_args_no_opts))
                .expect("format");
            black_box(out);
        });
    });
    call_group.bench_function("call_with_opts_passthrough", |b| {
        let mut formatter = Formatter::new(
            &call_catalog_with_opts,
            HostFn(|_fn_id, args, _opts| Ok(args.first().cloned().unwrap_or(Value::Null))),
        )
        .expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&call_args_with_opts))
                .expect("format");
            black_box(out);
        });
    });
    call_group.bench_function("call_with_opts_formatting_host", |b| {
        let mut formatter = Formatter::new(
            &call_catalog_with_opts,
            HostFn(|_fn_id, args, opts| {
                let mut out = String::new();
                if let Some(first) = args.first() {
                    out.push_str(&format!("{first:?}"));
                }
                for (key, value) in opts {
                    out.push('|');
                    out.push_str(&key.to_string());
                    out.push('=');
                    out.push_str(&format!("{value:?}"));
                }
                Ok(Value::Str(out))
            }),
        )
        .expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&call_args_with_opts))
                .expect("format");
            black_box(out);
        });
    });
    call_group.finish();

    let mut lookup_group = c.benchmark_group("runtime_lookup_paths");
    lookup_group.throughput(Throughput::Elements(1));
    lookup_group.bench_function("format_by_id", |b| {
        let mut formatter = Formatter::new(&plain_catalog, NoopHost).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&short_args))
                .expect("format");
            black_box(out);
        });
    });
    lookup_group.bench_function("format_resolved_handle", |b| {
        let mut formatter = Formatter::new(&plain_catalog, NoopHost).expect("formatter");
        let message = formatter.resolve("main").expect("resolved message");
        b.iter(|| {
            let out = formatter
                .format_for_bench(message, black_box(&short_args))
                .expect("format");
            black_box(out);
        });
    });
    lookup_group.finish();

    let mut builtin_group = c.benchmark_group("runtime_builtin_paths");
    builtin_group.throughput(Throughput::Elements(1));
    let builtin_args_no_opts =
        message_args(&builtin_catalog_no_opts, &[("amount", Value::Float(42.0))]);
    let builtin_args_with_opts = message_args(
        &builtin_catalog_with_opts,
        &[("amount", Value::Float(42.0))],
    );
    let en_us = locale("en-US");
    builtin_group.bench_function("builtin_number_no_opts", |b| {
        let host = BuiltinHost::new(&en_us).expect("host");
        let mut formatter = Formatter::new(&builtin_catalog_no_opts, host).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&builtin_args_no_opts))
                .expect("format");
            black_box(out);
        });
    });
    builtin_group.bench_function("builtin_number_with_opts", |b| {
        let host = BuiltinHost::new(&en_us).expect("host");
        let mut formatter = Formatter::new(&builtin_catalog_with_opts, host).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&builtin_args_with_opts))
                .expect("format");
            black_box(out);
        });
    });
    builtin_group.finish();

    let mut icu_group = c.benchmark_group("runtime_icu4x_paths");
    icu_group.throughput(Throughput::Elements(1));
    let icu_percent_args = message_args(&icu_percent_catalog, &[("amount", Value::Float(42.125))]);
    let icu_currency_args =
        message_args(&icu_currency_catalog, &[("amount", Value::Float(42.125))]);
    let icu_date_args = message_args(
        &icu_date_catalog,
        &[("date", Value::Str("2024-05-01T14:30:00".to_string()))],
    );
    let icu_time_args = message_args(
        &icu_time_catalog,
        &[("date", Value::Str("2024-05-01T14:30:00".to_string()))],
    );
    let icu_datetime_args = message_args(
        &icu_datetime_catalog,
        &[("date", Value::Str("2024-05-01T14:30:00".to_string()))],
    );
    let ar_eg = locale("ar-EG");
    for (locale_name, locale) in [("en-US", &en_us), ("ar-EG", &ar_eg)] {
        icu_group.bench_with_input(
            BenchmarkId::new("number_percent", locale_name),
            locale,
            |b, locale| {
                let host = BuiltinHost::new(locale).expect("host");
                let mut formatter = Formatter::new(&icu_percent_catalog, host).expect("formatter");
                b.iter(|| {
                    let out = formatter
                        .format_by_id_for_bench("main", black_box(&icu_percent_args))
                        .expect("format");
                    black_box(out);
                });
            },
        );
        icu_group.bench_with_input(
            BenchmarkId::new("date_short", locale_name),
            locale,
            |b, locale| {
                let host = BuiltinHost::new(locale).expect("host");
                let mut formatter = Formatter::new(&icu_date_catalog, host).expect("formatter");
                b.iter(|| {
                    let out = formatter
                        .format_by_id_for_bench("main", black_box(&icu_date_args))
                        .expect("format");
                    black_box(out);
                });
            },
        );
    }
    icu_group.bench_function("currency_usd_en-US", |b| {
        let host = BuiltinHost::new(&en_us).expect("host");
        let mut formatter = Formatter::new(&icu_currency_catalog, host).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&icu_currency_args))
                .expect("format");
            black_box(out);
        });
    });
    icu_group.bench_function("time_short_en-US", |b| {
        let host = BuiltinHost::new(&en_us).expect("host");
        let mut formatter = Formatter::new(&icu_time_catalog, host).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&icu_time_args))
                .expect("format");
            black_box(out);
        });
    });
    icu_group.bench_function("datetime_default_en-US", |b| {
        let host = BuiltinHost::new(&en_us).expect("host");
        let mut formatter = Formatter::new(&icu_datetime_catalog, host).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&icu_datetime_args))
                .expect("format");
            black_box(out);
        });
    });
    icu_group.finish();

    let mut select_modes = c.benchmark_group("runtime_select_modes");
    select_modes.throughput(Throughput::Elements(1));
    select_modes.bench_function("plain_string_select_formal", |b| {
        let mut formatter = Formatter::new(&select_catalog, NoopHost).expect("formatter");
        let message = formatter.resolve("main").expect("resolved message");
        b.iter(|| {
            let out = formatter
                .format_for_bench(message, black_box(&formal_args))
                .expect("format");
            black_box(out);
        });
    });
    let plural_catalog = build_plural_select_catalog();
    let plural_one = message_args(&plural_catalog, &[("count", Value::Int(1))]);
    select_modes.bench_function("builtin_plural_select_one", |b| {
        let host = BuiltinHost::new(&en_us).expect("host");
        let mut formatter = Formatter::new(&plural_catalog, host).expect("formatter");
        let message = formatter.resolve("main").expect("resolved message");
        b.iter(|| {
            let out = formatter
                .format_for_bench(message, black_box(&plural_one))
                .expect("format");
            black_box(out);
        });
    });
    select_modes.finish();

    let mut output_group = c.benchmark_group("runtime_output_assembly");
    output_group.throughput(Throughput::Elements(1));
    for len in [8_usize, 64, 256] {
        let dynamic = "z".repeat(len);
        let args = message_args(&many_segments_catalog, &[("x", Value::Str(dynamic))]);
        output_group.bench_with_input(
            BenchmarkId::new("many_segments_arg_len", len),
            &args,
            |b, a| {
                let mut formatter =
                    Formatter::new(&many_segments_catalog, NoopHost).expect("formatter");
                b.iter(|| {
                    let out = formatter
                        .format_by_id_for_bench("main", black_box(a))
                        .expect("format");
                    black_box(out);
                });
            },
        );
    }
    output_group.finish();

    let mut sink_group = c.benchmark_group("runtime_sink_paths");
    sink_group.throughput(Throughput::Elements(1));
    let markup_args = message_args(
        &markup_catalog,
        &[("name", Value::Str("World".to_string()))],
    );
    sink_group.bench_function("format_to_markup_open_close", |b| {
        let mut formatter = Formatter::new(&markup_catalog, NoopHost).expect("formatter");
        let mut sink = CountingSink::default();
        b.iter(|| {
            sink.reset();
            let errors = formatter
                .format_to_by_id_for_bench("main", black_box(&markup_args), &mut sink)
                .expect("format_to");
            black_box((&errors, sink.events, sink.bytes));
        });
    });

    sink_group.bench_function("format_to_markup_option_literal", |b| {
        let mut formatter =
            Formatter::new(&markup_option_literal_catalog, NoopHost).expect("formatter");
        let mut sink = CountingSink::default();
        let empty_args: Vec<(u32, Value)> = Vec::new();
        b.iter(|| {
            sink.reset();
            let errors = formatter
                .format_to_by_id_for_bench("main", black_box(&empty_args), &mut sink)
                .expect("format_to");
            black_box((&errors, sink.events, sink.bytes));
        });
    });

    let markup_option_args = vec![(
        arg_id(&markup_option_variable_catalog, "url"),
        Value::Str("https://test.com".to_string()),
    )];
    sink_group.bench_function("format_to_markup_option_variable", |b| {
        let mut formatter =
            Formatter::new(&markup_option_variable_catalog, NoopHost).expect("formatter");
        let mut sink = CountingSink::default();
        b.iter(|| {
            sink.reset();
            let errors = formatter
                .format_to_by_id_for_bench("main", black_box(&markup_option_args), &mut sink)
                .expect("format_to");
            black_box((&errors, sink.events, sink.bytes));
        });
    });
    sink_group.finish();
}

/// Build a catalog for plural selection:
///
///     .input {$count :number select=plural}
///     .match $count
///     one {{one item}}
///     * {{many items}}
fn build_plural_select_catalog() -> Catalog {
    // String pool: 0="count", 1="few", 2="main", 3="many", 4="number", 5="one",
    //              6="other", 7="plural", 8="select", 9="two", 10="zero"
    let strings = [
        "count", "few", "main", "many", "number", "one", "other", "plural", "select", "two", "zero",
    ];
    let literals = "one itemmany items";
    // fn_id=0 is "number" (str_id=4) with static option select=plural (str_id=8 -> str_id=7)
    let funcs = [FuncEntry {
        name_str_id: 4,
        static_options: vec![(8, 7)],
    }];
    // Bytecode:
    //   pc  0: LOAD_ARG "count" (str_id=0)       (5 bytes)
    //   pc  5: CALL_SELECT fn=0 args=1 opts=0     (5 bytes)
    //   pc 10: SELECT_BEGIN                        (1 byte)
    //   pc 11: CASE_STR "one" (str_id=5) +5       (9 bytes) -> pc 25
    //   pc 20: CASE_DEFAULT +14                    (5 bytes) -> pc 39
    //   pc 25: OUT_SLICE 0,8 "one item"            (9 bytes)
    //   pc 34: JMP +9                              (5 bytes) -> pc 48
    //   pc 39: OUT_SLICE 8,10 "many items"         (9 bytes)
    //   pc 48: SELECT_END                          (1 byte)
    //   pc 49: HALT                                (1 byte)
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

/// Build a catalog for ordinal selection with more cases:
///
///     .input {$pos :number select=ordinal}
///     .match $pos
///     one {{1st}}
///     two {{2nd}}
///     few {{3rd}}
///     * {{Nth}}
fn build_ordinal_select_catalog() -> Catalog {
    // String pool: 0="few", 1="main", 2="many", 3="number", 4="one",
    //              5="ordinal", 6="other", 7="pos", 8="select", 9="two", 10="zero"
    let strings = [
        "few", "main", "many", "number", "one", "ordinal", "other", "pos", "select", "two", "zero",
    ];
    let literals = "1st2nd3rdNth";
    // fn_id=0 is "number" (str_id=3) with static option select=ordinal (str_id=8 -> str_id=5)
    let funcs = [FuncEntry {
        name_str_id: 3,
        static_options: vec![(8, 5)],
    }];
    // Bytecode layout:
    //   pc  0: LOAD_ARG "pos" (str_id=7)           (5)
    //   pc  5: CALL_SELECT fn=0 args=1 opts=0      (5)
    //   pc 10: SELECT_BEGIN                         (1)
    //   pc 11: CASE_STR "one" (str_id=4) +23       (9) -> pc 43
    //   pc 20: CASE_STR "two" (str_id=9) +28       (9) -> pc 57
    //   pc 29: CASE_STR "few" (str_id=0) +33       (9) -> pc 71
    //   pc 38: CASE_DEFAULT +42                     (5) -> pc 85
    //   pc 43: OUT_SLICE 0,3 "1st"                  (9)
    //   pc 52: JMP +37                              (5) -> pc 94
    //   pc 57: OUT_SLICE 3,3 "2nd"                  (9)
    //   pc 66: JMP +23                              (5) -> pc 94
    //   pc 71: OUT_SLICE 6,3 "3rd"                  (9)
    //   pc 80: JMP +9                               (5) -> pc 94
    //   pc 85: OUT_SLICE 9,3 "Nth"                  (9)
    //   pc 94: SELECT_END                           (1)
    //   pc 95: HALT                                 (1)
    let code = vec![
        vm::OP_LOAD_ARG,
        7,
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
        4,
        0,
        0,
        0,
        23,
        0,
        0,
        0, // "one" -> pc 43
        vm::OP_CASE_STR,
        9,
        0,
        0,
        0,
        28,
        0,
        0,
        0, // "two" -> pc 57
        vm::OP_CASE_STR,
        0,
        0,
        0,
        0,
        33,
        0,
        0,
        0, // "few" -> pc 71
        vm::OP_CASE_DEFAULT,
        42,
        0,
        0,
        0, // -> pc 85
        // "1st"
        vm::OP_OUT_SLICE,
        0,
        0,
        0,
        0,
        3,
        0,
        0,
        0,
        vm::OP_JMP,
        37,
        0,
        0,
        0,
        // "2nd"
        vm::OP_OUT_SLICE,
        3,
        0,
        0,
        0,
        3,
        0,
        0,
        0,
        vm::OP_JMP,
        23,
        0,
        0,
        0,
        // "3rd"
        vm::OP_OUT_SLICE,
        6,
        0,
        0,
        0,
        3,
        0,
        0,
        0,
        vm::OP_JMP,
        9,
        0,
        0,
        0,
        // "Nth"
        vm::OP_OUT_SLICE,
        9,
        0,
        0,
        0,
        3,
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
            name_str_id: 1,
            entry_pc: 0,
        }],
        &code,
        &funcs,
    );
    Catalog::from_bytes(&bytes).expect("valid benchmark catalog")
}

fn bench_plural_ordinal(c: &mut Criterion) {
    let plural_catalog = build_plural_select_catalog();
    let ordinal_catalog = build_ordinal_select_catalog();
    let en_us = locale("en-US");

    let mut group = c.benchmark_group("runtime_plural_ordinal");
    group.throughput(Throughput::Elements(1));

    // Plural: hit "one" case (first case)
    let args_one = vec![(arg_id(&plural_catalog, "count"), Value::Int(1))];
    group.bench_function("plural_one", |b| {
        let host = BuiltinHost::new(&en_us).expect("host");
        let mut formatter = Formatter::new(&plural_catalog, host).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&args_one))
                .expect("format");
            black_box(out);
        });
    });

    // Plural: hit default "other" case
    let args_many = vec![(arg_id(&plural_catalog, "count"), Value::Int(5))];
    group.bench_function("plural_other", |b| {
        let host = BuiltinHost::new(&en_us).expect("host");
        let mut formatter = Formatter::new(&plural_catalog, host).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&args_many))
                .expect("format");
            black_box(out);
        });
    });

    // Ordinal: hit "one" case (1st — first case)
    let args_1st = vec![(arg_id(&ordinal_catalog, "pos"), Value::Int(1))];
    group.bench_function("ordinal_one", |b| {
        let host = BuiltinHost::new(&en_us).expect("host");
        let mut formatter = Formatter::new(&ordinal_catalog, host).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&args_1st))
                .expect("format");
            black_box(out);
        });
    });

    // Ordinal: hit "few" case (3rd — third case)
    let args_3rd = vec![(arg_id(&ordinal_catalog, "pos"), Value::Int(3))];
    group.bench_function("ordinal_few", |b| {
        let host = BuiltinHost::new(&en_us).expect("host");
        let mut formatter = Formatter::new(&ordinal_catalog, host).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&args_3rd))
                .expect("format");
            black_box(out);
        });
    });

    // Ordinal: hit default "other" case (5th — falls through all cases)
    let args_5th = vec![(arg_id(&ordinal_catalog, "pos"), Value::Int(5))];
    group.bench_function("ordinal_other", |b| {
        let host = BuiltinHost::new(&en_us).expect("host");
        let mut formatter = Formatter::new(&ordinal_catalog, host).expect("formatter");
        b.iter(|| {
            let out = formatter
                .format_by_id_for_bench("main", black_box(&args_5th))
                .expect("format");
            black_box(out);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_formatting, bench_plural_ordinal);
criterion_main!(benches);
