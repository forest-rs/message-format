// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::helpers::*;
use crate::runtime_helpers;
use message_format::Locale;
use message_format::compiler::CompileOptions;
use message_format::runtime::Value;

// ---------------------------------------------------------------------------
// TR35 §16 — Expression resolution
// ---------------------------------------------------------------------------

/// TR35 §16 — Declarations are evaluated before the message body.
#[test]
fn declarations_evaluated_before_body() {
    assert_format(
        ".local $x = { |hello| :string }\n{{{ $x } world}}",
        &[],
        "hello world",
    );
}

/// TR35 §16 — Multiple declarations resolve in order.
#[test]
fn declarations_resolve_in_order() {
    assert_format(
        ".local $a = { |1| :string }\n.local $b = { |2| :string }\n{{{ $a }{ $b }}}",
        &[],
        "12",
    );
}

// ---------------------------------------------------------------------------
// TR35 §17 — Markup
// ---------------------------------------------------------------------------

/// TR35 §17 — Markup elements produce empty string in formatted output.
#[test]
fn markup_produces_empty_output() {
    assert_format("{#bold}text{/bold}", &[], "text");
}

/// TR35 §17 — Self-closing markup produces empty string.
#[test]
fn self_closing_markup_empty() {
    assert_format("a{#br /}b", &[], "ab");
}

/// TR35 §17 — Nested markup produces empty in output (only text preserved).
#[test]
fn nested_markup() {
    assert_format("{#b}bold{#i}both{/i}{/b}", &[], "boldboth");
}

/// Integration: compile markup source, `format_to` with collecting sink, assert events.
#[test]
fn format_to_markup_events_through_compiler() {
    use message_format::runtime::{BuiltinHost, Catalog, FormatSink, Formatter};

    #[derive(Default)]
    struct Events(Vec<String>);

    impl FormatSink for Events {
        fn literal(&mut self, s: &str) {
            self.0.push(format!("L:{s}"));
        }
        fn expression(&mut self, s: &str) {
            self.0.push(format!("E:{s}"));
        }
        fn markup_open(&mut self, name: &str, _opts: &[message_format::runtime::FormatOption<'_>]) {
            self.0.push(format!("O:{name}"));
        }
        fn markup_close(
            &mut self,
            name: &str,
            _opts: &[message_format::runtime::FormatOption<'_>],
        ) {
            self.0.push(format!("C:{name}"));
        }
    }

    let source = "{#b}Hello {$name}{/b}";
    let bytes = message_format::compiler::compile_str(source).expect("compile");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse::<Locale>().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let args = vec![(
        catalog.string_id("name").expect("arg id"),
        Value::Str("World".into()),
    )];
    let mut sink = Events::default();
    let errors = runtime_helpers::format_to_by_id(&mut formatter, "main", &args, &mut sink)
        .expect("format_to");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(
        sink.0,
        vec!["O:b", "L:Hello ", "E:World", "C:b"],
        "events mismatch"
    );
}

/// Integration: self-closing markup through compiler produces open+close events.
#[test]
fn format_to_self_closing_markup_through_compiler() {
    use message_format::runtime::{BuiltinHost, Catalog, FormatSink, Formatter};

    #[derive(Default)]
    struct Events(Vec<String>);

    impl FormatSink for Events {
        fn literal(&mut self, s: &str) {
            self.0.push(format!("L:{s}"));
        }
        fn expression(&mut self, s: &str) {
            self.0.push(format!("E:{s}"));
        }
        fn markup_open(&mut self, name: &str, _opts: &[message_format::runtime::FormatOption<'_>]) {
            self.0.push(format!("O:{name}"));
        }
        fn markup_close(
            &mut self,
            name: &str,
            _opts: &[message_format::runtime::FormatOption<'_>],
        ) {
            self.0.push(format!("C:{name}"));
        }
    }

    let source = "a{#br /}b";
    let bytes = message_format::compiler::compile_str(source).expect("compile");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let locale = "en".parse::<Locale>().expect("locale");
    let host = BuiltinHost::new(&locale).expect("host");
    let mut formatter = Formatter::new(&catalog, host).expect("formatter");
    let mut sink = Events::default();
    let errors = runtime_helpers::format_to_by_id(
        &mut formatter,
        "main",
        &Vec::<(u32, Value)>::new(),
        &mut sink,
    )
    .expect("format_to");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(
        sink.0,
        vec!["L:a", "O:br", "C:br", "L:b"],
        "events mismatch"
    );
}

// ---------------------------------------------------------------------------
// TR35 §18 — Bidi isolation
// ---------------------------------------------------------------------------

/// TR35 §18 — With `default_bidi_isolation`, bare placeholders get bidi wrapping.
#[test]
fn bidi_isolation_wraps_placeholders() {
    let out = format_with_options_raw(
        "Hello { $name :string }!",
        &[("name", Value::Str("World".into()))],
        CompileOptions {
            default_bidi_isolation: true,
        },
    );
    // FSI = U+2068, PDI = U+2069
    assert!(
        out.contains('\u{2068}') && out.contains('\u{2069}'),
        "expected bidi isolation markers, got: {out:?}",
    );
}

/// TR35 §18 — :string function always applies bidi isolation.
#[test]
fn string_function_applies_bidi() {
    let out = format_raw("{ $x :string }", &[("x", Value::Str("test".into()))]);
    assert!(
        out.contains('\u{2068}') && out.contains('\u{2069}'),
        "expected bidi isolation from :string, got: {out:?}",
    );
}

/// TR35 §18 — Without bidi isolation and without :string, no markers.
#[test]
fn no_bidi_without_string_function() {
    let out = format_with_options_raw(
        "Hello { $name }!",
        &[("name", Value::Str("World".into()))],
        CompileOptions {
            default_bidi_isolation: false,
        },
    );
    assert!(
        !out.contains('\u{2068}') && !out.contains('\u{2069}'),
        "unexpected bidi isolation markers in: {out:?}",
    );
}

// ---------------------------------------------------------------------------
// TR35 §16 — Fallback value rendering
// ---------------------------------------------------------------------------

/// FMT-21 — Unresolved variable renders as fallback `{$varname}` in output.
#[test]
fn fallback_value_for_missing_variable() {
    assert_format("{ $x :string }", &[], "{$x}");
}

/// FMT-22 — Unknown function renders fallback `{:funcname}` in output.
#[test]
fn fallback_value_for_unknown_function() {
    assert_format("{ :nonexistent }", &[], "{:nonexistent}");
}

/// FMT-19 — Literal operand failure renders fallback `{|literal|}` in output.
#[test]
fn fallback_value_for_literal_operand() {
    assert_format("{ |horse| :number }", &[], "{|horse|}");
}

// ---------------------------------------------------------------------------
// TR35 §16 — Literal and variable resolution (FMT-5, FMT-6, FMT-7)
// ---------------------------------------------------------------------------

/// FMT-5 — Resolved value of a text is the character sequence after escape conversion.
#[test]
fn literal_resolves_after_escape_conversion() {
    assert_format("{|a\\\\b| :string}", &[], "a\\b");
}

/// FMT-6 — Quoted and unquoted literals with same content are identical to functions.
#[test]
fn quoted_unquoted_identical_to_functions() {
    let a = format("{|42| :number}", &[]);
    let b = format("{42 :number}", &[]);
    assert_eq!(a, b);
}

/// FMT-4 — Declarations evaluated in order before body.
#[test]
fn declaration_order_matters() {
    assert_format(
        ".local $a = { |first| :string }\n.local $b = { $a }\n{{{ $b }}}",
        &[],
        "first",
    );
}

// ---------------------------------------------------------------------------
// TR35 §16 — Markup resolution (FMT-17, OUT-3)
// ---------------------------------------------------------------------------

/// OUT-3 — When formatting to a string, markup resolves to empty string.
#[test]
fn markup_resolves_to_empty_in_string_output() {
    assert_format("before{#tag}after", &[], "beforeafter");
}

/// FMT-17 — Resolution of markup MUST always succeed (no errors).
#[test]
fn markup_resolution_always_succeeds() {
    // Markup with options and attributes — should still succeed.
    assert_format("{#tag option=value @attr=x}text{/tag}", &[], "text");
}
