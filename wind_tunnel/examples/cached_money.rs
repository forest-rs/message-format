// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Run with `cargo run -p wind_tunnel --example cached_money`.
//! Cache this binding alongside a pooled formatter, once per catalog chain.

use fixed_decimal::Decimal;
use message_format::compiler::{CompileOptions, ResourceInput, SourceKind};
use message_format::runtime::{
    BuiltinHost, Catalog, MultiFormatter, MultiMessageHandle, StrId, Value,
};

#[derive(Debug)]
struct MoneyBinding {
    message: MultiMessageHandle,
    amount: StrId,
    currency: StrId,
}

impl MoneyBinding {
    fn new(formatter: &MultiFormatter<'_, BuiltinHost>) -> Self {
        let message = formatter.resolve("pricing.money").expect("money message");
        // Resolve IDs from the actual fallback catalog selected for the message.
        // A different formatter/catalog chain needs its own binding.
        let catalog = formatter.catalog_for(message).expect("selected catalog");
        Self {
            message,
            amount: catalog.string_id("amount").expect("amount id"),
            currency: catalog.string_id("currency").expect("currency id"),
        }
    }

    fn format(
        &self,
        formatter: &mut MultiFormatter<'_, BuiltinHost>,
        minor_units: i64,
        currency: &str,
        scale: i16,
    ) -> Option<String> {
        // The caller supplies the currency's ISO minor-unit scale.
        let mut decimal = Decimal::from(minor_units);
        decimal.absolute.multiply_pow10(-scale);
        // Move the decimal to avoid cloning it. Value still owns a boxed decimal
        // and an owned currency string; this is not a borrowed-input API.
        let args = [
            (self.amount, Value::from(decimal)),
            (self.currency, Value::from(currency)),
        ];
        let mut output = String::new();
        let mut diagnostics = Vec::new();
        formatter
            .format_to(self.message, &args, &mut output, Some(&mut diagnostics))
            .ok()?;
        // Preserve the downstream caller's strict fallback-on-any-diagnostic rule.
        diagnostics.is_empty().then_some(output)
    }
}

fn main() {
    let input = ResourceInput::new("pricing", SourceKind::Generated).message(
        "pricing.money",
        "{ $amount :currency currency=$currency currencyDisplay=code }",
    );
    let (root, _) =
        Catalog::compile_resources([input], CompileOptions::default()).expect("catalog");
    let locale = "sv".parse().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = MultiFormatter::new([&root], host).expect("formatter");
    let money = MoneyBinding::new(&formatter);

    for (minor_units, currency, scale) in [(123_456, "USD", 2), (-4_250, "SEK", 2), (42, "JPY", 0)]
    {
        let display = money
            .format(&mut formatter, minor_units, currency, scale)
            .expect("money");
        println!("{display}");
    }
}
