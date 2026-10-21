// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Model cx-catalog's pooled `PricingLocalizer`: the formatter stays warm, but
//! each money value constructs its decimal, arguments, output, and diagnostics.
//!
//! `named` resolves the message and inserts both arguments by name each time.
//! `cached` uses a cached handle and IDs with a fixed argument array, keeping
//! the same decimal clone as `named`. `cached_owned` also moves the decimal.
//! See `examples/cached_money.rs` for the corresponding caller pattern.
//!
//! Both locales cover zero, negative, grouped, and small amounts. `single`
//! keeps USD warm; `alternating` cycles USD, EUR, SEK, and JPY, exercising the
//! single-entry formatter cache. Setup and equivalence checks are outside timing.
//! Results are time per money value, excluding currency metadata lookup, `Money`
//! construction, and the rest of pricing; allocation counts are not measured.
//!
//! Record a baseline before changing the runtime, then compare on the same
//! machine without concurrent builds:
//!
//! ```text
//! cargo bench -p wind_tunnel --bench money -- --save-baseline before
//! cargo bench -p wind_tunnel --bench money -- --baseline before
//! cargo run -p wind_tunnel --example cached_money
//! ```

use std::hint::black_box;

use criterion::{Bencher, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use fixed_decimal::Decimal;
use icu_locale_core::Locale;
use message_format::compiler::{CompileOptions, ResourceInput, SourceKind};
use message_format::runtime::{
    BuiltinHost, Catalog, MultiFormatter, MultiMessageHandle, StrId, Value,
};

// Match the downstream catalog's 15-message shape for message/name lookups.
const MESSAGES: [(&str, &str); 15] = [
    ("pricing.breakdown.line.base-price", "Base price"),
    (
        "pricing.breakdown.line.conditioned-price",
        "Conditioned price",
    ),
    (
        "pricing.breakdown.line.option",
        "{ $characteristic }: { $value }",
    ),
    (
        "pricing.breakdown.line.option-unlabeled",
        "Option: { $value }",
    ),
    ("pricing.breakdown.line.kind.base", "Base"),
    ("pricing.breakdown.line.kind.add", "Add"),
    ("pricing.breakdown.line.kind.discount", "Discount"),
    ("pricing.breakdown.line.kind.surcharge", "Surcharge"),
    ("pricing.breakdown.line.kind.tier", "Tier"),
    (
        "pricing.breakdown.line.kind.override-adjustment",
        "Bundle override adjustment",
    ),
    ("pricing.breakdown.line.kind.subtotal", "Subtotal"),
    (
        "pricing.breakdown.line.kind.bundle-discount",
        "Bundle discount",
    ),
    (
        "pricing.breakdown.heading.name-and-code",
        "{ $name } ({ $code })",
    ),
    ("pricing.breakdown.heading.occurrence", "#{ $id }"),
    (
        "pricing.money",
        "{ $amount :currency currency=$currency currencyDisplay=code }",
    ),
];

fn catalog() -> Catalog {
    let input = MESSAGES.into_iter().fold(
        ResourceInput::new("pricing", SourceKind::Generated),
        |input, (id, source)| input.message(id, source),
    );
    Catalog::compile_resources([input], CompileOptions::default())
        .expect("pricing catalog")
        .0
}

#[derive(Clone, Copy)]
struct PostedMoney {
    minor_units: i64,
    currency: &'static str,
    scale: i16,
}

impl PostedMoney {
    fn decimal(self) -> Decimal {
        let mut decimal = Decimal::from(self.minor_units);
        decimal.absolute.multiply_pow10(-self.scale);
        decimal
    }
}

// Include zero, negative, grouped, and ordinary values at ISO storage scale.
const SINGLE: [PostedMoney; 4] = [
    PostedMoney {
        minor_units: 123_456,
        currency: "USD",
        scale: 2,
    },
    PostedMoney {
        minor_units: 0,
        currency: "USD",
        scale: 2,
    },
    PostedMoney {
        minor_units: -4_250,
        currency: "USD",
        scale: 2,
    },
    PostedMoney {
        minor_units: 42,
        currency: "USD",
        scale: 2,
    },
];
const ALTERNATING: [PostedMoney; 4] = [
    SINGLE[0],
    PostedMoney {
        minor_units: 0,
        currency: "EUR",
        scale: 2,
    },
    PostedMoney {
        minor_units: -4_250,
        currency: "SEK",
        scale: 2,
    },
    PostedMoney {
        minor_units: 42,
        currency: "JPY",
        scale: 0,
    },
];

#[derive(Clone, Copy)]
struct Binding {
    message: MultiMessageHandle,
    amount: StrId,
    currency: StrId,
}

impl Binding {
    fn new(formatter: &MultiFormatter<'_, BuiltinHost>) -> Self {
        let message = formatter.resolve("pricing.money").expect("money message");
        // IDs must belong to the catalog selected by this formatter's handle.
        let catalog = formatter.catalog_for(message).expect("selected catalog");
        Self {
            message,
            amount: catalog.string_id("amount").expect("amount id"),
            currency: catalog.string_id("currency").expect("currency id"),
        }
    }
}

fn named(formatter: &mut MultiFormatter<'_, BuiltinHost>, posted: PostedMoney) -> Option<String> {
    let decimal = posted.decimal();
    let message = formatter.resolve("pricing.money").ok()?;
    let mut args = formatter.args_for(message).ok()?;
    args.insert("amount", &decimal).ok()?;
    args.insert("currency", posted.currency).ok()?;
    let mut output = String::new();
    let mut diagnostics = Vec::new();
    formatter
        .format_to(message, &args, &mut output, Some(&mut diagnostics))
        .ok()?;
    diagnostics.is_empty().then_some(output)
}

fn cached(
    formatter: &mut MultiFormatter<'_, BuiltinHost>,
    binding: Binding,
    posted: PostedMoney,
    move_decimal: bool,
) -> Option<String> {
    let decimal = posted.decimal();
    let amount = if move_decimal {
        Value::from(decimal)
    } else {
        Value::from(&decimal)
    };
    let args = [
        (binding.amount, amount),
        (binding.currency, Value::from(posted.currency)),
    ];
    let mut output = String::new();
    let mut diagnostics = Vec::new();
    formatter
        .format_to(binding.message, &args, &mut output, Some(&mut diagnostics))
        .ok()?;
    diagnostics.is_empty().then_some(output)
}

fn measure(
    b: &mut Bencher<'_>,
    inputs: [PostedMoney; 4],
    mut format: impl FnMut(PostedMoney) -> Option<String>,
) {
    let mut next = 0;
    b.iter(|| {
        let input = black_box(inputs[next]);
        next = (next + 1) % inputs.len();
        black_box(format(input)).expect("money without diagnostics");
    });
}

fn money(c: &mut Criterion) {
    let root = catalog();
    let translated = catalog();
    let mut group = c.benchmark_group("money");
    group.throughput(Throughput::Elements(1));
    for tag in ["en-US", "sv"] {
        for (workload, inputs) in [("single", SINGLE), ("alternating", ALTERNATING)] {
            for strategy in ["named", "cached", "cached_owned"] {
                let locale = tag.parse::<Locale>().expect("locale");
                let host = BuiltinHost::new(&locale).expect("host");
                let mut formatter =
                    MultiFormatter::new([&translated, &root], host).expect("formatter");
                let binding = Binding::new(&formatter);
                // Verify behavior and warm the same retained host used in timing.
                for input in inputs {
                    let expected = named(&mut formatter, input).expect("named money");
                    assert_eq!(
                        cached(&mut formatter, binding, input, false),
                        Some(expected.clone()),
                        "cached binding must preserve money output for {tag}/{workload}"
                    );
                    assert_eq!(
                        cached(&mut formatter, binding, input, true),
                        Some(expected),
                        "moving the decimal must preserve money output for {tag}/{workload}"
                    );
                }
                group.bench_function(
                    BenchmarkId::new(format!("{tag}/{workload}"), strategy),
                    |b| match strategy {
                        "named" => measure(b, inputs, |input| named(&mut formatter, input)),
                        "cached" => measure(b, inputs, |input| {
                            cached(&mut formatter, binding, input, false)
                        }),
                        "cached_owned" => measure(b, inputs, |input| {
                            cached(&mut formatter, binding, input, true)
                        }),
                        _ => unreachable!(),
                    },
                );
            }
        }
    }
    group.finish();
}

criterion_group!(benches, money);
criterion_main!(benches);
