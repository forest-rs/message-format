// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::helpers::*;
use message_format::runtime::Value;

// ---------------------------------------------------------------------------
// TR35 §2.3 — .input declarations (S-14)
// ---------------------------------------------------------------------------

/// S-14 — `.input` binds a variable for use in the message body.
#[test]
fn input_declaration_binding() {
    assert_format(
        ".input { $name :string }\n{{Hello { $name }!}}",
        &[("name", Value::Str("World".into()))],
        "Hello World!",
    );
}

/// S-14 — `.input` with a function annotation.
#[test]
fn input_declaration_with_function() {
    assert_format(
        ".input { $count :number }\n{{You have { $count } items.}}",
        &[("count", Value::Int(3))],
        "You have 3 items.",
    );
}

// ---------------------------------------------------------------------------
// TR35 §2.3 — .local declarations
// ---------------------------------------------------------------------------

/// S-11 — `.local` binds a computed value.
#[test]
fn local_declaration_binding() {
    assert_format(".local $x = { |hello| :string }\n{{{ $x }}}", &[], "hello");
}

/// S-11 — `.local` can reference a prior `.local`.
#[test]
fn local_references_prior_local() {
    assert_format(
        ".local $a = { |hi| :string }\n.local $b = { $a }\n{{{ $b }}}",
        &[],
        "hi",
    );
}

/// S-11 — `.local` can reference an input variable.
#[test]
fn local_references_input_variable() {
    assert_format(
        ".local $greeting = { $name :string }\n{{{ $greeting }}}",
        &[("name", Value::Str("Alice".into()))],
        "Alice",
    );
}

/// S-12 — `.local` and body work together.
#[test]
fn local_and_body() {
    assert_format(
        ".local $x = { |world| :string }\n{{Hello { $x }!}}",
        &[],
        "Hello world!",
    );
}

/// S-12 — `.local` MAY overwrite an external input if not used in prior declaration.
#[test]
fn local_can_shadow_unused_external_input() {
    assert_format(
        ".local $x = { |override| :string }\n{{{ $x }}}",
        &[("x", Value::Str("original".into()))],
        "override",
    );
}

// ---------------------------------------------------------------------------
// TR35 §2.3 — Declaration errors (S-8, S-9, S-10, S-11, S-13)
// ---------------------------------------------------------------------------

/// S-8 — Duplicate declaration of the same variable is an error.
#[test]
fn duplicate_declaration_is_error() {
    assert_compile_err(
        ".local $x = { |a| :string }\n.local $x = { |b| :string }\n{{{ $x }}}",
        is_duplicate_declaration,
    );
}

/// S-11 — Self-referencing local declaration is an error.
#[test]
fn local_self_reference_is_error() {
    assert_compile_err(
        ".local $x = { $x :string }\n{{{ $x }}}",
        is_duplicate_declaration,
    );
}

/// S-13 — Implicitly declared variable cannot be explicitly declared afterward.
#[test]
fn implicit_var_explicit_redeclaration_is_error() {
    // $x is implicitly declared by its use in the first .local,
    // so a subsequent .input $x should be an error.
    assert_compile_err(
        ".local $y = { $x :string }\n.input { $x :string }\n{{{ $y }}}",
        is_duplicate_declaration,
    );
}

/// S-23 — Each selector MUST reference a declaration with a function.
#[test]
fn missing_selector_annotation_is_error() {
    // $x has no function annotation — using it as a selector should fail.
    assert_compile_err(
        ".input { $x }\n.match $x\na {{A}}\n* {{OTHER}}",
        is_missing_selector_annotation,
    );
}
