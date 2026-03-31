// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(
    missing_docs,
    reason = "Criterion-generated benchmark entry points are private harness glue."
)]

use core::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use message_format_compiler::compile_str;

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

fn build_raw_match_source(variant_count: usize) -> String {
    let mut source = String::from(".input {$count :number}\n.match $count\n");
    for index in 0..variant_count {
        source.push(' ');
        source.push_str(&index.to_string());
        source.push_str(" {{");
        source.push_str("variant ");
        source.push_str(&index.to_string());
        source.push_str("}}\n");
    }
    source.push_str(" * {{fallback}}\n");
    source
}

fn build_raw_message_declarations_source(local_count: usize) -> String {
    let mut source = String::new();
    source.push_str(".input {$name}\n");
    for index in 0..local_count {
        source.push_str(".local $alias_");
        source.push_str(&index.to_string());
        source.push_str(" = {$name}\n");
    }
    source.push_str("{{Hello {$alias_");
    source.push_str(&local_count.saturating_sub(1).to_string());
    source.push_str("}!}}\n");
    source
}

fn bench_catalog_compile(c: &mut Criterion) {
    let mut group = c.benchmark_group("compile/catalog_simple");
    for message_count in [10_usize, 100, 1_000, 10_000] {
        let source = build_simple_catalog(message_count);
        group.throughput(Throughput::Elements(
            u64::try_from(message_count).expect("message count fits in u64"),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(message_count),
            &source,
            |b, source| {
                b.iter(|| compile_str(black_box(source)).expect("compile succeeds"));
            },
        );
    }
    group.finish();
}

fn bench_function_catalog_compile(c: &mut Criterion) {
    let mut group = c.benchmark_group("compile/catalog_functions");
    for message_count in [10_usize, 100, 1_000, 10_000] {
        let source = build_function_catalog(message_count);
        group.throughput(Throughput::Elements(
            u64::try_from(message_count).expect("message count fits in u64"),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(message_count),
            &source,
            |b, source| {
                b.iter(|| compile_str(black_box(source)).expect("compile succeeds"));
            },
        );
    }
    group.finish();
}

fn bench_raw_match_compile(c: &mut Criterion) {
    let mut group = c.benchmark_group("compile/raw_match");
    for variant_count in [8_usize, 64, 256] {
        let source = build_raw_match_source(variant_count);
        group.throughput(Throughput::Elements(
            u64::try_from(variant_count).expect("variant count fits in u64"),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(variant_count),
            &source,
            |b, source| {
                b.iter(|| compile_str(black_box(source)).expect("compile succeeds"));
            },
        );
    }
    group.finish();
}

fn bench_raw_message_declarations_compile(c: &mut Criterion) {
    let mut group = c.benchmark_group("compile/raw_message_declarations");
    for local_count in [4_usize, 32, 128] {
        let source = build_raw_message_declarations_source(local_count);
        group.throughput(Throughput::Elements(
            u64::try_from(local_count).expect("local count fits in u64"),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(local_count),
            &source,
            |b, source| {
                b.iter(|| compile_str(black_box(source)).expect("compile succeeds"));
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_catalog_compile,
    bench_function_catalog_compile,
    bench_raw_match_compile,
    bench_raw_message_declarations_compile
);
criterion_main!(benches);
