// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(
    missing_docs,
    reason = "Criterion-generated benchmark entry points are private harness glue."
)]

use core::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use message_format_compiler::{
    CompileOptions, MessageResource, ResourceInput, SourceKind, compile_resources, compile_str,
};
use message_format_runtime::Catalog;

fn build_simple_catalog(message_count: usize) -> String {
    let mut source = String::new();
    for index in 0..message_count {
        source.push_str("msg_");
        source.push_str(&index.to_string());
        source.push_str(" = Hello { $name }!\n");
    }
    source
}

fn build_function_catalog(message_count: usize) -> String {
    let mut source = String::new();
    for index in 0..message_count {
        source.push_str("price_");
        source.push_str(&index.to_string());
        source.push_str(" = Total: { $amount :number minimumFractionDigits=2 }\n");
    }
    source
}

fn build_mixed_catalog(message_count: usize) -> Vec<u8> {
    let mut input = ResourceInput::new("bench-mixed", SourceKind::Generated);
    for index in 0..message_count {
        let (id, source) = match index % 5 {
            0 => (
                format!("price_{index}"),
                format!("Total {index}: {{ $amount :number minimumFractionDigits=2 }}"),
            ),
            1 => (
                format!("greeting_{index}"),
                format!(
                    ".input {{ $tone :string }}\
                     \n.match $tone\
                     \nformal {{{{Dear colleague {index}}}}}\
                     \ncasual {{{{Hey there {index}}}}}\
                     \nfriendly {{{{Hi friend {index}}}}}\
                     \n* {{{{Hello {index}}}}}"
                ),
            ),
            2 => (
                format!("count_{index}"),
                format!(
                    ".input {{ $count :number select=plural }}\
                     \n.match $count\
                     \none {{{{{index} item}}}}\
                     \nother {{{{{index} items}}}}\
                     \n* {{{{{index} items}}}}"
                ),
            ),
            3 => (
                format!("invoice_{index}"),
                format!(
                    ".local $total = {{ $raw :number minimumFractionDigits=2 }}\
                     \n{{{{Invoice {index}: {{ $total }}}}}}"
                ),
            ),
            4 => (
                format!("status_{index}"),
                format!(
                    ".local $state = {{ $input :string }}\
                     \n.match $state\
                     \nactive {{{{Active {index}}}}}\
                     \ninactive {{{{Inactive {index}}}}}\
                     \n* {{{{Unknown {index}}}}}"
                ),
            ),
            _ => unreachable!(),
        };
        input.resources.push(MessageResource::new(id, source));
    }
    compile_resources([input], CompileOptions::default())
        .into_result()
        .expect("mixed catalog compiles")
        .bytes
}

fn bench_catalog_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("load/catalog_simple");
    for message_count in [10_usize, 100, 1_000, 10_000] {
        let source = build_simple_catalog(message_count);
        let bytes = compile_str(&source).expect("compile succeeds");
        group.throughput(Throughput::Elements(
            u64::try_from(message_count).expect("message count fits in u64"),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(message_count),
            &bytes,
            |b, bytes| {
                b.iter(|| Catalog::from_bytes(black_box(bytes)).expect("load succeeds"));
            },
        );
    }
    group.finish();
}

fn bench_function_catalog_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("load/catalog_functions");
    for message_count in [10_usize, 100, 1_000, 10_000] {
        let source = build_function_catalog(message_count);
        let bytes = compile_str(&source).expect("compile succeeds");
        group.throughput(Throughput::Elements(
            u64::try_from(message_count).expect("message count fits in u64"),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(message_count),
            &bytes,
            |b, bytes| {
                b.iter(|| Catalog::from_bytes(black_box(bytes)).expect("load succeeds"));
            },
        );
    }
    group.finish();
}

fn bench_mixed_catalog_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("load/catalog_mixed");
    for message_count in [10_usize, 100, 1_000, 10_000] {
        let bytes = build_mixed_catalog(message_count);
        group.throughput(Throughput::Elements(
            u64::try_from(message_count).expect("message count fits in u64"),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(message_count),
            &bytes,
            |b, bytes| {
                b.iter(|| Catalog::from_bytes(black_box(bytes)).expect("load succeeds"));
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_catalog_load,
    bench_function_catalog_load,
    bench_mixed_catalog_load
);
criterion_main!(benches);
