// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use message_format_runtime::schema;
use message_format_runtime::{Catalog, FormatError, Formatter, Host, HostFn, NoopHost, Value};

use crate::manifest::{
    FunctionManifest, FunctionOperandKind, FunctionOptionValueKind, FunctionSchema,
    FunctionSelectorKeyKind,
};
use crate::resource::{MessageResource, ResourceInput, ResourceSpan};
use crate::semantic::{DetachedSourceSpan, OperandLiteralKind, SelectorExpr};

use super::frontend::{lower_expression_node_to_part, parse_messages};
use super::*;

#[derive(Default)]
struct OutputStringSink {
    out: String,
}

impl message_format_runtime::FormatSink for OutputStringSink {
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

trait FormatterTestExt<H: Host> {
    fn format_by_id_for_test(
        &mut self,
        message_id: &str,
        args: &dyn message_format_runtime::Args,
    ) -> Result<String, FormatError>;
}

impl<H: Host> FormatterTestExt<H> for Formatter<'_, H> {
    fn format_by_id_for_test(
        &mut self,
        message_id: &str,
        args: &dyn message_format_runtime::Args,
    ) -> Result<String, FormatError> {
        let message = self.resolve(message_id)?;
        let mut sink = OutputStringSink::default();
        let _diagnostics = self.format_to(message, args, &mut sink)?;
        Ok(sink.out)
    }
}

fn expect_compiled(report: CompileReport) -> CompiledCatalog {
    assert!(!report.has_errors(), "{}", report.render());
    report.compiled.expect("compiled catalog")
}

fn expect_first_error(report: CompileReport) -> BuildError {
    assert!(report.has_errors(), "expected compile errors");
    report
        .diagnostics
        .into_iter()
        .next()
        .expect("first diagnostic")
}

fn expect_errors(report: CompileReport) -> Vec<BuildError> {
    assert!(report.has_errors(), "expected compile errors");
    report.diagnostics
}

fn arg_id(catalog: &Catalog, name: &str) -> u32 {
    catalog.string_id(name).expect("arg id")
}

fn arg(catalog: &Catalog, name: &str, value: Value) -> (u32, Value) {
    (arg_id(catalog, name), value)
}

#[derive(Default)]
struct MarkupOptionSink {
    options: Vec<(String, String)>,
}

impl message_format_runtime::FormatSink for MarkupOptionSink {
    fn literal(&mut self, _s: &str) {}

    fn expression(&mut self, _s: &str) {}

    fn markup_open(&mut self, _name: &str, options: &[message_format_runtime::FormatOption<'_>]) {
        self.options.extend(
            options
                .iter()
                .map(|option| (option.key.to_string(), option.value.to_string())),
        );
    }

    fn markup_close(&mut self, _name: &str, _options: &[message_format_runtime::FormatOption<'_>]) {
    }
}

#[test]
fn bare_interpolation_lowers_to_out_arg() {
    let bytes = compile_str("Hello { $name }!").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    assert_eq!(
        catalog.code(),
        &[
            schema::OP_OUT_SLICE,
            0,
            0,
            0,
            0,
            6,
            0,
            0,
            0,
            schema::OP_OUT_ARG,
            1,
            0,
            0,
            0,
            schema::OP_OUT_SLICE,
            6,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            schema::OP_HALT,
        ]
    );
}

#[test]
fn simple_select_lowers_to_select_arg() {
    let bytes =
        compile_str(".input { $kind :string }\n.match $kind\nformal {{Good evening}}\n* {{Hi}}")
            .expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    assert_eq!(catalog.code()[0], schema::OP_SELECT_ARG);
    assert!(!catalog.code().contains(&schema::OP_SELECT_BEGIN));
}

#[test]
fn string_selector_with_options_does_not_lower_to_select_arg() {
    let mut builder = CatalogBuilder::new();
    builder
        .add_message(
            Message::builder("main")
                .select(
                    SelectExpr::builder(SelectorExpr::call(
                        Operand::var("kind"),
                        FunctionSpec::new("string").option_literal("u:dir", "ltr"),
                    ))
                    .arm("formal", vec![Part::text("Good evening")])
                    .default(vec![Part::text("Hi")])
                    .build(),
                )
                .build(),
        )
        .expect("message");

    let compiled = expect_compiled(builder.compile());
    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    assert_ne!(catalog.code()[0], schema::OP_SELECT_ARG);
    assert!(catalog.code().contains(&schema::OP_CALL_SELECT));
}

#[test]
fn markup_option_variables_are_canonicalized_like_other_variables() {
    let source = "main = {#link href=$na\u{308}me}{/link}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    assert!(catalog.string_id("näme").is_some());

    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let message = formatter.resolve("main").expect("message");
    let args = vec![arg(
        &catalog,
        "näme",
        Value::Str("https://example.test".to_string()),
    )];
    let mut sink = MarkupOptionSink::default();
    let diagnostics = formatter
        .format_to(message, &args, &mut sink)
        .expect("format");

    assert!(diagnostics.is_empty());
    assert_eq!(
        sink.options,
        vec![(String::from("href"), String::from("https://example.test"))]
    );
}

#[test]
fn exact_match_beats_default_even_if_default_appears_first() {
    let bytes =
        compile_str(".input { $kind :string }\n.match $kind\n* {{Hi}}\nformal {{Good evening}}")
            .expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");

    let out = formatter
        .format_by_id_for_test(
            "main",
            &vec![arg(&catalog, "kind", Value::Str("formal".to_string()))],
        )
        .expect("formatted");

    assert_eq!(out, "Good evening");
}

#[test]
fn function_catalog_keys_do_not_collide_on_spaces_or_equals() {
    let mut builder = CatalogBuilder::new();
    builder
        .add_message(
            Message::builder("one")
                .call(CallExpr::new(
                    Operand::literal("x"),
                    FunctionSpec::new("custom:format").option_literal("a", "b c=d"),
                ))
                .build(),
        )
        .expect("message");
    builder
        .add_message(
            Message::builder("two")
                .call(CallExpr::new(
                    Operand::literal("x"),
                    FunctionSpec::new("custom:format")
                        .option_literal("a", "b")
                        .option_literal("c", "d"),
                ))
                .build(),
        )
        .expect("message");

    let compiled = expect_compiled(builder.compile());
    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    assert_eq!(catalog.func_count(), 2);
}

#[test]
fn compile_inputs_merges_multiple_files_and_tracks_origins() {
    let compiled = expect_compiled(compile_inputs(
        [
            CompileInput {
                name: "a.mf2",
                message_id: "hello",
                source: "Hello",
                kind: SourceKind::MessageFormat,
            },
            CompileInput {
                name: "b.mf2",
                message_id: "bye",
                source: "Bye",
                kind: SourceKind::MessageFormat,
            },
        ],
        CompileOptions::default(),
    ));

    assert_eq!(compiled.source_map.sources.len(), 2);
    assert_eq!(compiled.source_map.messages.len(), 2);
    assert_eq!(compiled.source_map.messages[0].message_id, "hello");
    assert_eq!(
        compiled.source_map.messages[0]
            .origin
            .as_ref()
            .and_then(|origin| origin.line),
        Some(1)
    );

    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    assert_eq!(
        formatter
            .format_by_id_for_test("bye", &Vec::<(u32, Value)>::new())
            .expect("formatted"),
        "Bye"
    );
}

#[test]
fn compile_resources_merges_named_message_bodies() {
    let compiled = expect_compiled(compile_resources(
        [
            ResourceInput::new("app.toml", SourceKind::Other(String::from("resource-toml")))
                .message("hello", "Hello")
                .message("bye", "Bye"),
        ],
        CompileOptions::default(),
    ));

    assert_eq!(compiled.source_map.sources.len(), 1);
    assert_eq!(compiled.source_map.messages.len(), 2);
    assert_eq!(compiled.source_map.messages[0].message_id, "hello");
    assert_eq!(compiled.source_map.messages[0].origin, None);

    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    assert_eq!(
        formatter
            .format_by_id_for_test("hello", &Vec::<(u32, Value)>::new())
            .expect("formatted"),
        "Hello"
    );
}

#[test]
fn compile_resources_preserves_upstream_origin_in_source_map() {
    let compiled = expect_compiled(compile_resources(
        [
            ResourceInput::new("app.toml", SourceKind::Other(String::from("resource-toml")))
                .resource(MessageResource::new("hello", "Hello").upstream_origin(
                    DetachedSourceSpan {
                        name: String::from("src/app.rs"),
                        kind: SourceKind::Rust,
                        byte_start: Some(10),
                        byte_end: Some(18),
                        line: Some(3),
                        column: Some(7),
                    },
                )),
        ],
        CompileOptions::default(),
    ));

    assert_eq!(
        compiled.source_map.messages[0].upstream_origin,
        Some(DetachedSourceSpan {
            name: String::from("src/app.rs"),
            kind: SourceKind::Rust,
            byte_start: Some(10),
            byte_end: Some(18),
            line: Some(3),
            column: Some(7),
        })
    );
}

#[test]
fn compile_resources_maps_container_origin_into_registered_source() {
    let compiled = expect_compiled(compile_resources(
        [
            ResourceInput::new("app.toml", SourceKind::Other(String::from("resource-toml")))
                .resource(MessageResource::new("hello", "Hello").origin(ResourceSpan {
                    byte_start: 14,
                    byte_end: 21,
                    line: Some(2),
                    column: Some(13),
                })),
        ],
        CompileOptions::default(),
    ));

    assert_eq!(
        compiled.source_map.messages[0].origin,
        Some(SourceSpan {
            source_id: SourceId(0),
            byte_start: 14,
            byte_end: 21,
            line: Some(2),
            column: Some(13),
        })
    );
}

#[test]
fn compile_resources_render_nested_message_diagnostics() {
    let err = expect_first_error(compile_resources(
        [
            ResourceInput::new("app.toml", SourceKind::Other(String::from("resource-toml")))
                .resource(
                    MessageResource::new("cart.total", "Total: { $amount :number style=currency ")
                        .origin(ResourceSpan {
                            byte_start: 24,
                            byte_end: 67,
                            line: Some(6),
                            column: Some(9),
                        }),
                ),
        ],
        CompileOptions::default(),
    ));

    let rendered = err.render();
    assert!(rendered.contains("app.toml:6:9: invalid expression in message \"cart.total\""));
    assert!(rendered.contains("message \"cart.total\":1:41"));
    assert!(rendered.contains("expected matching '}'"));
    assert!(rendered.contains("found unclosed '{'"));
}

#[test]
fn compile_str_rejects_invalid_builtin_number_style_literal() {
    let err = compile_str("{ $amount :number style=curren }").expect_err("must fail");

    match err {
        CompileError::InvalidBuiltinOptionValue {
            function,
            option,
            expected,
            found,
            ..
        } => {
            assert_eq!(function, "number");
            assert_eq!(option, "style");
            assert_eq!(expected, "\"percent\"");
            assert_eq!(found, "curren");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_rejects_invalid_builtin_number_sign_display_literal() {
    let err = compile_str("{ $amount :number signDisplay=bogus }").expect_err("must fail");

    match err {
        CompileError::InvalidBuiltinOptionValue {
            function,
            option,
            expected,
            found,
            ..
        } => {
            assert_eq!(function, "number");
            assert_eq!(option, "signDisplay");
            assert_eq!(expected, "\"auto\", \"always\", or \"never\"");
            assert_eq!(found, "bogus");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_rejects_invalid_builtin_number_use_grouping_literal() {
    let err = compile_str("{ $amount :number useGrouping=bogus }").expect_err("must fail");

    match err {
        CompileError::InvalidBuiltinOptionValue {
            function,
            option,
            expected,
            found,
            ..
        } => {
            assert_eq!(function, "number");
            assert_eq!(option, "useGrouping");
            assert_eq!(expected, "\"auto\", \"always\", \"never\", or \"min2\"");
            assert_eq!(found, "bogus");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_rejects_invalid_plural_variant_key() {
    let err =
        compile_str(".input { $x :number select=plural }\n.match $x\nhorse {{HORSE}}\n* {{OTHER}}")
            .expect_err("must fail");

    match err {
        CompileError::InvalidVariantKey {
            line,
            column,
            expected,
            found,
            ..
        } => {
            assert_eq!(line, 3);
            assert_eq!(column, 1);
            assert_eq!(expected, Some("CLDR plural category or MF2 number literal"));
            assert_eq!(found.as_deref(), Some("horse"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_rejects_invalid_exact_variant_key() {
    let err =
        compile_str(".input { $x :number select=exact }\n.match $x\nhorse {{HORSE}}\n* {{OTHER}}")
            .expect_err("must fail");

    match err {
        CompileError::InvalidVariantKey {
            line,
            column,
            expected,
            found,
            ..
        } => {
            assert_eq!(line, 3);
            assert_eq!(column, 1);
            assert_eq!(expected, Some("MF2 number literal"));
            assert_eq!(found.as_deref(), Some("horse"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_rejects_invalid_ordinal_variant_key() {
    let err = compile_str(
        ".input { $x :number select=ordinal }\n.match $x\nhorse {{HORSE}}\n* {{OTHER}}",
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidVariantKey {
            line,
            column,
            expected,
            found,
            ..
        } => {
            assert_eq!(line, 3);
            assert_eq!(column, 1);
            assert_eq!(expected, Some("CLDR plural category or MF2 number literal"));
            assert_eq!(found.as_deref(), Some("horse"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_rejects_invalid_default_number_variant_key() {
    let err = compile_str(".input { $x :number }\n.match $x\nhorse {{HORSE}}\n* {{OTHER}}")
        .expect_err("must fail");

    match err {
        CompileError::InvalidVariantKey {
            line,
            column,
            expected,
            found,
            ..
        } => {
            assert_eq!(line, 3);
            assert_eq!(column, 1);
            assert_eq!(expected, Some("CLDR plural category or MF2 number literal"));
            assert_eq!(found.as_deref(), Some("horse"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_str_accepts_number_literal_variant_key_for_plural() {
    compile_str(".input { $x :number select=plural }\n.match $x\n1 {{ONE}}\n* {{OTHER}}")
        .expect("compiled");
}

#[test]
fn resource_backed_builtin_option_error_does_not_duplicate_inner_context() {
    let err = expect_first_error(compile_resources(
        [ResourceInput::new(
            "apps/message-format-lsp/test.mf2.json",
            SourceKind::Other(String::from("json-flat")),
        )
        .resource(
            MessageResource::new("price.display", "{ $amount :number style=currenc }").origin(
                ResourceSpan {
                    byte_start: 120,
                    byte_end: 155,
                    line: Some(5),
                    column: Some(20),
                },
            ),
        )],
        CompileOptions::default(),
    ));

    let rendered = err.render();
    assert!(rendered.contains(
        "apps/message-format-lsp/test.mf2.json:5:20: invalid builtin option value in message \"price.display\""
    ));
    assert!(rendered.contains("error: invalid builtin option value"));
    assert!(rendered.contains("{ $amount :number style=currenc }"));
    assert!(rendered.contains("expected \"percent\", found currenc"));
    assert!(!rendered.contains("at source#"));
}

#[test]
fn compile_resources_collects_multiple_message_diagnostics() {
    let errs = expect_errors(compile_resources(
        [
            ResourceInput::new("app.toml", SourceKind::Other(String::from("resource-toml")))
                .message("bad.style", "{ $amount :number style=curren }")
                .message("bad.expr", "Total: { $amount :number style=currency "),
        ],
        CompileOptions::default(),
    ));

    assert_eq!(errs.len(), 2);
    let rendered = CompileReport::failure(errs).render();
    assert!(rendered.contains("message \"bad.style\""));
    assert!(rendered.contains("message \"bad.expr\""));
    assert!(rendered.contains("curren"));
    assert!(rendered.contains("expected matching '}'"));
}

#[test]
fn compile_inputs_collects_multiple_input_diagnostics() {
    let errs = expect_errors(compile_inputs(
        [
            CompileInput {
                name: "a.mf2",
                message_id: "bad.style",
                source: "{ $amount :number style=curren }",
                kind: SourceKind::MessageFormat,
            },
            CompileInput {
                name: "b.mf2",
                message_id: "bad.expr",
                source: "{ $value :number style=currency ",
                kind: SourceKind::MessageFormat,
            },
        ],
        CompileOptions::default(),
    ));

    assert_eq!(errs.len(), 2);
    let rendered = CompileReport::failure(errs).render();
    assert!(rendered.contains("a.mf2"));
    assert!(rendered.contains("b.mf2"));
    assert!(rendered.contains("curren"));
    assert!(rendered.contains("unclosed '{'"));
}

#[test]
fn compile_resources_rejects_duplicate_message_ids() {
    let err = expect_first_error(compile_resources(
        [
            ResourceInput::new("a.toml", SourceKind::Other(String::from("resource-toml")))
                .message("main", "Hello"),
            ResourceInput::new("b.toml", SourceKind::Other(String::from("resource-toml")))
                .message("main", "Bye"),
        ],
        CompileOptions::default(),
    ));

    assert!(matches!(
        *err.error,
        CompileError::DuplicateMessageId { ref message_id, .. } if message_id == "main"
    ));
}

#[test]
fn catalog_builder_accepts_structured_messages_without_text_roundtrip() {
    let mut builder = CatalogBuilder::new();
    let source_id = builder
        .register_source("generated", SourceKind::Generated)
        .expect("source id");
    builder
        .add_message(
            Message::builder("hello")
                .origin(SourceSpan {
                    source_id,
                    byte_start: 10,
                    byte_end: 20,
                    line: Some(3),
                    column: Some(7),
                })
                .text("Hello from builder")
                .build(),
        )
        .expect("message");

    let compiled = expect_compiled(builder.compile());
    assert_eq!(compiled.source_map.sources[0].name, "generated");
    assert_eq!(
        compiled.source_map.messages[0]
            .origin
            .as_ref()
            .and_then(|origin| origin.line),
        Some(3)
    );

    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    assert_eq!(
        formatter
            .format_by_id_for_test("hello", &Vec::<(u32, Value)>::new())
            .expect("formatted"),
        "Hello from builder"
    );
}

#[test]
fn compile_inputs_rejects_duplicate_message_ids() {
    let err = expect_first_error(compile_inputs(
        [
            CompileInput {
                name: "a.mf2",
                message_id: "main",
                source: "Hello",
                kind: SourceKind::MessageFormat,
            },
            CompileInput {
                name: "b.mf2",
                message_id: "main",
                source: "Bye",
                kind: SourceKind::MessageFormat,
            },
        ],
        CompileOptions::default(),
    ));

    match *err.error {
        CompileError::DuplicateMessageId {
            message_id,
            first_origin,
            second_origin,
        } => {
            assert_eq!(message_id, "main");
            assert_eq!(first_origin.and_then(|origin| origin.line), Some(1));
            assert_eq!(second_origin.and_then(|origin| origin.line), Some(1));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_unknown_function() {
    let manifest = FunctionManifest::new();
    let err = compile_with_manifest(
        "main = { $value :custom:unknown }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::UnknownFunction {
            function,
            message_id,
            origin,
            ..
        } => {
            assert_eq!(function, "custom:unknown");
            assert_eq!(message_id.as_deref(), Some("main"));
            assert_eq!(origin.as_deref().and_then(|origin| origin.line), Some(1));
            assert!(
                origin
                    .as_deref()
                    .and_then(|origin| origin.column)
                    .is_some_and(|column| column > 1)
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_unknown_markup_name() {
    let manifest = FunctionManifest::new();
    let err = compile_with_manifest(
        "main = {#custom:link href=|/|}{/custom:link}",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::UnknownFunction {
            function,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:link");
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_unknown_option() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(FunctionSchema::new("custom:format").allow_format());

    let err = compile_with_manifest(
        "main = { $value :custom:format mode=fast }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::UnknownFunctionOption {
            function,
            option,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:format");
            assert_eq!(option, "mode");
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_unknown_markup_option() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(FunctionSchema::new("custom:link").allow_format());

    let err = compile_with_manifest(
        "main = {#custom:link mode=|fast|}{/custom:link}",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::UnknownFunctionOption {
            function,
            option,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:link");
            assert_eq!(option, "mode");
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_wrong_option_value_shape() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:format")
            .allow_format()
            .option("mode", FunctionOptionValueKind::Literal),
    );

    let err = compile_with_manifest(
        "main = { $value :custom:format mode=$kind }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidFunctionOptionValue {
            function,
            option,
            expected,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:format");
            assert_eq!(option, "mode");
            assert_eq!(expected, FunctionOptionValueKind::Literal);
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_missing_required_option() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:number")
            .allow_format()
            .required_option("style", FunctionOptionValueKind::Literal),
    );

    let err = compile_with_manifest(
        "main = { $value :custom:number }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::MissingFunctionOption {
            function,
            option,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:number");
            assert_eq!(option, "style");
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_bad_literal_number_operand() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:number")
            .allow_format()
            .operand_kind(FunctionOperandKind::Number),
    );

    let err = compile_with_manifest(
        "main = { |horse| :custom:number }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidFunctionOperand {
            function,
            expected,
            found,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:number");
            assert_eq!(expected, FunctionOperandKind::Number);
            assert!(found.contains("horse"));
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_allows_variable_operand_for_number_kind() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:number")
            .allow_format()
            .operand_kind(FunctionOperandKind::Number),
    );

    compile_with_manifest(
        "main = { $value :custom:number }",
        CompileOptions::default(),
        &manifest,
    )
    .expect("compiled");
}

#[test]
fn compile_with_manifest_rejects_numeric_literal_for_string_operand() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:string")
            .allow_format()
            .operand_kind(FunctionOperandKind::String),
    );

    let err = compile_with_manifest(
        "main = { 123 :custom:string }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidFunctionOperand {
            function,
            expected,
            found,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:string");
            assert_eq!(expected, FunctionOperandKind::String);
            assert!(found.contains("123"));
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_allows_quoted_numeric_literal_for_string_operand() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:string")
            .allow_format()
            .operand_kind(FunctionOperandKind::String),
    );

    compile_with_manifest(
        "main = { |123| :custom:string }",
        CompileOptions::default(),
        &manifest,
    )
    .expect("compiled");
}

#[test]
fn compile_with_manifest_allows_quoted_numeric_local_for_string_operand() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:string")
            .allow_format()
            .operand_kind(FunctionOperandKind::String),
    );

    compile_with_manifest(
        ".local $x = {|123|}\n{{ { $x :custom:string } }}",
        CompileOptions::default(),
        &manifest,
    )
    .expect("compiled");
}

#[test]
fn compile_with_manifest_rejects_quoted_numeric_literal_for_number_operand() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(
        FunctionSchema::new("custom:number")
            .allow_format()
            .operand_kind(FunctionOperandKind::Number),
    );

    let err = compile_with_manifest(
        "main = { |123| :custom:number }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidFunctionOperand {
            function,
            expected,
            found,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:number");
            assert_eq!(expected, FunctionOperandKind::Number);
            assert!(found.contains("123"));
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_selector_only_usage_in_format_position() {
    let mut manifest = FunctionManifest::new();
    manifest.insert(FunctionSchema::new("custom:select").allow_select());

    let err = compile_with_manifest(
        "main = { $value :custom:select }",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::UnsupportedFunctionUsage {
            function,
            usage,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:select");
            assert_eq!(usage, "format");
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

fn custom_select_manifest(schema: FunctionSchema) -> FunctionManifest {
    let mut manifest = FunctionManifest::new();
    manifest.insert(schema);
    manifest
}

fn custom_select_schema() -> FunctionSchema {
    FunctionSchema::new("custom:select").allow_select()
}

#[test]
fn compile_with_manifest_accepts_selector_annotation_when_allowed() {
    let manifest = custom_select_manifest(custom_select_schema());

    compile_with_manifest(
        ".input { $kind :custom:select }\n.match $kind\na {{A}}\n* {{OTHER}}",
        CompileOptions::default(),
        &manifest,
    )
    .expect("compiled");
}

#[test]
fn compile_with_manifest_rejects_selector_key_outside_declared_domain() {
    let manifest = custom_select_manifest(
        custom_select_schema().selector_keys(FunctionSelectorKeyKind::Number),
    );

    let err = compile_with_manifest(
        ".input { $kind :custom:select }\n.match $kind\nhorse {{HORSE}}\n* {{OTHER}}",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidVariantKey {
            line,
            column,
            expected,
            found,
            ..
        } => {
            assert_eq!(line, 3);
            assert_eq!(column, 1);
            assert_eq!(expected, Some("MF2 number literal"));
            assert_eq!(found.as_deref(), Some("horse"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_rejects_selector_key_outside_plural_category_domain() {
    let manifest = custom_select_manifest(
        custom_select_schema().selector_keys(FunctionSelectorKeyKind::PluralCategory),
    );

    let err = compile_with_manifest(
        ".input { $kind :custom:select }\n.match $kind\n1 {{ONE}}\n* {{OTHER}}",
        CompileOptions::default(),
        &manifest,
    )
    .expect_err("must fail");

    match err {
        CompileError::InvalidVariantKey {
            line,
            column,
            expected,
            found,
            ..
        } => {
            assert_eq!(line, 3);
            assert_eq!(column, 1);
            assert_eq!(expected, Some("CLDR plural category"));
            assert_eq!(found.as_deref(), Some("1"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn compile_with_manifest_accepts_plural_category_selector_key() {
    let manifest = custom_select_manifest(
        custom_select_schema().selector_keys(FunctionSelectorKeyKind::PluralCategory),
    );

    compile_with_manifest(
        ".input { $kind :custom:select }\n.match $kind\none {{ONE}}\n* {{OTHER}}",
        CompileOptions::default(),
        &manifest,
    )
    .expect("compiled");
}

#[test]
fn compile_with_manifest_accepts_selector_key_with_declared_domain() {
    let manifest = custom_select_manifest(
        custom_select_schema().selector_keys(FunctionSelectorKeyKind::Identifier),
    );

    compile_with_manifest(
        ".input { $kind :custom:select }\n.match $kind\nhorse {{HORSE}}\n* {{OTHER}}",
        CompileOptions::default(),
        &manifest,
    )
    .expect("compiled");
}

#[test]
fn compile_inputs_with_manifest_reports_precise_source() {
    let manifest = FunctionManifest::new();
    let err = expect_first_error(compile_inputs_with_manifest(
        [
            CompileInput {
                name: "ok.mf2",
                message_id: "first",
                source: "Hello",
                kind: SourceKind::MessageFormat,
            },
            CompileInput {
                name: "bad.mf2",
                message_id: "second",
                source: "{ $value :custom:unknown }",
                kind: SourceKind::MessageFormat,
            },
        ],
        CompileOptions::default(),
        &manifest,
    ));

    assert_eq!(
        err.context
            .as_ref()
            .map(|context| context.source.name.as_str()),
        Some("bad.mf2")
    );
    match *err.error {
        CompileError::UnknownFunction { origin, .. } => {
            assert_eq!(origin.as_deref().and_then(|origin| origin.line), Some(1));
            assert!(
                origin
                    .as_deref()
                    .and_then(|origin| origin.column)
                    .is_some_and(|column| column > 1)
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn render_with_source_highlights_unknown_function_annotation() {
    let err = compile_with_manifest(
        "main = { $value :custom:unknown }",
        CompileOptions::default(),
        &FunctionManifest::new(),
    )
    .expect_err("must fail");

    let rendered = err.render_with_source("main = { $value :custom:unknown }");
    assert!(rendered.contains("error: unknown function"));
    assert!(rendered.contains("--> <input>:1:"));
    assert!(rendered.contains("custom:unknown"));
}

#[test]
fn catalog_builder_uses_manifest_for_structured_messages() {
    let mut builder = CatalogBuilder::new();
    builder.set_function_manifest(FunctionManifest::new());
    builder
        .add_message(
            Message::builder("main")
                .call(CallExpr::new(
                    Operand::var("value"),
                    FunctionSpec::new("custom:missing"),
                ))
                .build(),
        )
        .expect("message");

    let err = expect_first_error(builder.compile());
    match *err.error {
        CompileError::UnknownFunction {
            function,
            message_id,
            ..
        } => {
            assert_eq!(function, "custom:missing");
            assert_eq!(message_id.as_deref(), Some("main"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn catalog_builder_builder_api_formats_structured_select() {
    let mut builder = CatalogBuilder::new();
    let source_id = builder
        .register_source("generated", SourceKind::Generated)
        .expect("source id");
    builder
        .add_message(
            Message::builder("main")
                .origin(SourceSpan {
                    source_id,
                    byte_start: 0,
                    byte_end: 42,
                    line: Some(1),
                    column: Some(1),
                })
                .select(
                    SelectExpr::builder(SelectorExpr::var("kind"))
                        .arm("formal", vec![Part::text("Good evening")])
                        .default(vec![Part::text("Hi")])
                        .build(),
                )
                .build(),
        )
        .expect("message");

    let compiled = expect_compiled(builder.compile());
    let catalog = Catalog::from_bytes(&compiled.bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");

    let formal = formatter
        .format_by_id_for_test(
            "main",
            &vec![arg(&catalog, "kind", Value::Str("formal".to_string()))],
        )
        .expect("formatted");
    let casual = formatter
        .format_by_id_for_test(
            "main",
            &vec![arg(&catalog, "kind", Value::Str("casual".to_string()))],
        )
        .expect("formatted");

    assert_eq!(formal, "Good evening");
    assert_eq!(casual, "Hi");
}

#[test]
fn compiles_and_formats_interpolation() {
    let bytes = compile_str("Hello { $name }!").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args = vec![arg(&catalog, "name", Value::Str("World".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello World!");
}

#[test]
fn default_bidi_isolation_rewrites_bare_interpolation_to_string_call() {
    let messages = parse_messages(
        "{ $name }",
        CompileOptions {
            default_bidi_isolation: true,
        },
    )
    .expect("parsed");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "main");
    match messages[0].parts.as_slice() {
        [
            Part::Call(CallExpr {
                operand: Operand::Var(var),
                func,
                ..
            }),
        ] => {
            assert_eq!(var, "name");
            assert_eq!(func.name, "string");
        }
        other => panic!("unexpected lowered parts: {other:?}"),
    }
}

#[test]
fn default_bidi_isolation_rewrites_bare_literal_expression_to_string_call() {
    let messages = parse_messages(
        "{ hello }",
        CompileOptions {
            default_bidi_isolation: true,
        },
    )
    .expect("parsed");
    assert_eq!(messages.len(), 1);
    match messages[0].parts.as_slice() {
        [
            Part::Call(CallExpr {
                operand: Operand::Literal { value, kind },
                func,
                ..
            }),
        ] => {
            assert_eq!(value, "hello");
            assert_eq!(*kind, OperandLiteralKind::String);
            assert_eq!(func.name, "string");
        }
        other => panic!("unexpected lowered parts: {other:?}"),
    }
}

#[test]
fn compiles_and_formats_call() {
    let source = "{ $n :double }";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(
        &catalog,
        HostFn(|_fn_id, _args, _opts| Ok(Value::Str("CALLED".to_string()))),
    )
    .expect("formatter");
    let args = vec![arg(&catalog, "n", Value::Int(12))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args)
            .expect("formatted"),
        "CALLED"
    );
}

#[test]
fn invalid_expr_fails() {
    let err = compile_str("{}").expect_err("must fail");
    assert!(matches!(err, CompileError::InvalidExpr { .. }));
}

#[test]
fn lower_expression_node_requires_typed_non_select_payload() {
    let expr = crate::syntax::ast::ExpressionNode {
        raw_span: 1..6,
        span: 1..6,
        kind: crate::syntax::ast::ExpressionKindNode::Var,

        payload: None,
        diag_hint: None,
    };
    let err = lower_expression_node_to_part(
        "{ $x }",
        &expr,
        crate::syntax::span::SourceContext::Line {
            line: 1,
            column_offset: 0,
        },
        None,
    )
    .expect_err("must fail");
    match err {
        CompileError::InvalidExpr {
            expected, found, ..
        } => {
            assert_eq!(expected, Some("typed expression payload"));
            assert_eq!(found.as_deref(), Some("missing expression payload"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn missing_selector_reports_expected_and_found() {
    let err = compile_str(".match").expect_err("must fail");
    match err {
        CompileError::InvalidExpr {
            line,
            column,
            expected,
            found,
        } => {
            assert_eq!(line, 1);
            assert_eq!(expected, Some("at least one selector variable"));
            assert_eq!(found.as_deref(), Some("none"));
            assert!(column >= 1);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn duplicate_variant_key_reports_collision_context() {
    let err = compile_str(".input {$x :string} .match $x Ḍ̇ {{a}} Ḍ̇ {{b}} * {{c}}")
        .expect_err("must fail");
    match err {
        CompileError::DuplicateVariant {
            expected,
            found,
            line,
            column,
        } => {
            assert_eq!(line, 1);
            assert!(column >= 1);
            assert_eq!(expected, Some("unique variant key (NFC-normalized)"));
            let found = found.expect("found");
            assert!(found.contains("duplicates key first seen at 1:"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn missing_function_name_after_colon_reports_expected_and_found() {
    let err = compile_str("{ $n : }").expect_err("must fail");
    match err {
        CompileError::InvalidExpr {
            expected, found, ..
        } => {
            assert_eq!(expected, Some("function name after ':'"));
            assert_eq!(found.as_deref(), Some("end of expression"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn malformed_option_token_reports_expected_and_found() {
    let err = compile_str("{ $n :number minimumFractionDigits }").expect_err("must fail");
    match err {
        CompileError::InvalidExpr {
            expected, found, ..
        } => {
            assert_eq!(
                expected,
                Some("option token 'key=value' or attribute '@name'")
            );
            assert!(
                found
                    .as_deref()
                    .is_some_and(|value| value.contains("minimumFractionDigits"))
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn invalid_var_reports_column_and_display() {
    let err = compile_str("{ $-bad }").expect_err("must fail");
    match &err {
        CompileError::InvalidVar {
            line,
            column,
            expected,
            found,
        } => {
            assert_eq!(*line, 1);
            assert_eq!(*column, 4);
            assert_eq!(*expected, Some("valid identifier after '$'"));
            assert!(found.as_ref().is_some_and(|it| it.contains("$-bad")));
        }
        other => panic!("unexpected error: {other:?}"),
    }
    let message = err.to_string();
    assert!(message.contains("1:4"));
    assert!(message.contains("expected valid identifier after '$'"));
}

#[test]
fn renders_diagnostic_with_source_snippet() {
    let source = "{ $-bad }";
    let err = compile_str(source).expect_err("must fail");
    let rendered = err.render_with_source(source);
    assert!(rendered.contains("error: invalid variable"));
    assert!(rendered.contains("--> <input>:1:4"));
    assert!(rendered.contains("1 | { $-bad }"));
    assert!(rendered.contains("^ expected valid identifier after '$'"));
}

#[test]
fn renders_diagnostic_with_custom_label() {
    let source = ".match";
    let err = compile_str(source).expect_err("must fail");
    let rendered = err.render_with_source_label(source, "demo.mf2");
    assert!(rendered.contains("--> demo.mf2:1:"));
}

#[test]
fn render_with_source_falls_back_for_non_context_errors() {
    let err = CompileError::TooManyStrings;
    let rendered = err.render_with_source("hello = world");
    assert_eq!(rendered, "too many interned strings");
}

#[test]
fn render_with_source_falls_back_when_line_is_out_of_range() {
    let err = CompileError::InvalidExpr {
        line: 5,
        column: 2,
        expected: Some("expression"),
        found: Some(String::from("eof")),
    };
    let rendered = err.render_with_source("one line");
    assert_eq!(
        rendered,
        "invalid expression at 5:2 (expected expression, found eof)"
    );
}

#[test]
fn render_with_source_handles_zero_column_without_panic() {
    let err = CompileError::InvalidExpr {
        line: 1,
        column: 0,
        expected: Some("expression"),
        found: Some(String::from("eof")),
    };
    let rendered = err.render_with_source("hello");
    assert!(rendered.contains("--> <input>:1:0"));
    assert!(rendered.contains("| ^ expected expression, found eof"));
}

#[test]
fn render_with_source_targets_specific_multiline_row() {
    let err = CompileError::MissingDefaultArm {
        line: 3,
        column: 1,
        expected: Some("default arm '*[key]...'"),
        found: Some(String::from("none")),
    };
    let source = "a = A\nb = B\ng = .match $x a {{A}}";
    let rendered = err.render_with_source(source);
    assert!(rendered.contains("--> <input>:3:1"));
    assert!(rendered.contains("3 | g = .match $x a {{A}}"));
    assert!(rendered.contains("^ expected default arm '*[key]...', found none"));
}

#[test]
fn match_keyword_spacing_error_uses_character_column() {
    let err = compile_str(".match\u{16A0} $x * {{ok}}").expect_err("must fail");
    let rendered = err.render_with_source(".match\u{16A0} $x * {{ok}}");
    assert!(rendered.contains("error: invalid expression"));
    assert!(rendered.contains("--> <input>:1:7"));
    assert!(rendered.contains("expected whitespace after '.match'"));
}

#[test]
fn literal_expression_formats() {
    let bytes = compile_str("hello {world}{|!|}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "hello world!");
}

#[test]
fn raw_message_source_uses_main_id() {
    let bytes = compile_str("Hello { $name }!").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args = vec![arg(&catalog, "name", Value::Str("WG".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello WG!");
}

#[test]
fn raw_quoted_pattern_is_extracted() {
    let bytes = compile_str(".input {$x} {{Hello}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "Hello");
}

#[test]
fn local_literal_declaration_is_substituted() {
    let bytes = compile_str(".local $x = {1} {{A {$x} B}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "A 1 B");
}

#[test]
fn leading_declarations_without_quoted_pattern_are_stripped() {
    let bytes = compile_str(".input {$name} .local $x = {ok} {{Hello {$x}!}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "Hello ok!");
}

#[test]
fn single_message_declarations_require_quoted_body() {
    let err = compile_str(".input {$name} Hello {$name}").expect_err("must fail");
    match err {
        CompileError::InvalidExpr {
            expected, found, ..
        } => {
            assert_eq!(expected, Some("message body '{{...}}' after declarations"));
            assert_eq!(found.as_deref(), Some("end of input"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn raw_match_statement_is_rewritten_and_formatted() {
    let source =
        ".input {$kind :string} .match $kind formal {{Good evening}} casual {{Hi}} * {{Hello}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");

    let args1 = vec![arg(&catalog, "kind", Value::Str("formal".to_string()))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args1)
            .expect("formatted"),
        "Good evening"
    );

    let args2 = vec![arg(&catalog, "kind", Value::Str("casual".to_string()))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args2)
            .expect("formatted"),
        "Hi"
    );

    let args3 = vec![arg(&catalog, "kind", Value::Str("other".to_string()))];
    assert_eq!(
        formatter
            .format_by_id_for_test("main", &args3)
            .expect("formatted"),
        "Hello"
    );
}

#[test]
fn raw_match_with_local_literal_selector_resolves_at_compile_time() {
    let source = ".local $kind = {formal :string} .match $kind formal {{Good evening}} * {{Hello}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "Good evening");
}

#[test]
fn raw_match_with_local_alias_selector_uses_input() {
    let source =
        ".input {$kind :string} .local $k = {$kind} .match $k formal {{Good evening}} * {{Hello}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args = vec![arg(&catalog, "kind", Value::Str("formal".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Good evening");
}

#[test]
fn local_alias_is_substituted() {
    let bytes = compile_str(".local $a = {$name} {{Hello {$a}!}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args = vec![arg(&catalog, "name", Value::Str("World".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello World!");
}

#[test]
fn chained_local_aliases_are_substituted() {
    let bytes =
        compile_str(".local $a = {$name} .local $b = {$a} {{Hello {$b}!}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args = vec![arg(&catalog, "name", Value::Str("Chain".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello Chain!");
}

#[test]
fn alias_cycle_reports_resolution_overflow() {
    let err = compile_str(
        ".input {$seed} \
         .local $a = {$seed} \
         .local $b = {$a} \
         .local $c = {$b} \
         .local $d = {$c} \
         .local $e = {$d} \
         .local $f = {$e} \
         .local $g = {$f} \
         .local $h = {$g} \
         .local $i = {$h} \
         .local $j = {$i} \
         {{Hello {$j}!}}",
    )
    .expect_err("must fail");

    assert!(matches!(err, CompileError::AliasResolutionOverflow { .. }));
}

#[test]
fn local_integer_function_is_evaluated() {
    let bytes = compile_str(".local $x = {4.2 :integer} {{X={$x}}}").expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "X=4");
}

#[test]
fn local_test_select_decimal_places_is_evaluated() {
    let source = ".local $x = {1 :test:select decimalPlaces=1} .match $x 1.0 {{A}} * {{B}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "A");
}

#[test]
fn raw_match_with_dynamic_select_option_uses_default_arm() {
    let source =
        ".input {$mode} .local $x = {1 :test:select select=$mode} .match $x 1 {{A}} * {{B}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "B");
}

#[test]
fn raw_match_with_unstable_selector_chain_uses_default_arm() {
    let source = ".input {$mode} .local $a = {1 :test:select select=$mode} .local $x = {$a :integer} .match $x 1 {{A}} * {{B}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "B");
}

#[test]
fn raw_match_with_two_local_selectors_resolves_at_compile_time() {
    let source = ".local $x = {1 :test:select} .local $y = {0 :test:select} .match $x $y 1 1 {{1,1}} 1 * {{1,*}} * 1 {{*,1}} * * {{*,*}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "1,*");
}

#[test]
fn raw_match_with_two_runtime_selectors_is_rewritten() {
    let source = ".input {$x :string} .input {$y :string} .match $x $y 1 1 {{1,1}} * * {{*,*}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args_11 = vec![
        arg(&catalog, "x", Value::Int(1)),
        arg(&catalog, "y", Value::Int(1)),
    ];
    let args_10 = vec![
        arg(&catalog, "x", Value::Int(1)),
        arg(&catalog, "y", Value::Int(0)),
    ];

    let out_11 = formatter
        .format_by_id_for_test("main", &args_11)
        .expect("formatted");
    assert_eq!(out_11, "1,1");

    let out_10 = formatter
        .format_by_id_for_test("main", &args_10)
        .expect("formatted");
    assert_eq!(out_10, "*,*");
}

#[test]
fn raw_match_with_escaped_quoted_key_selects_expected_arm() {
    let source = ".input {$kind :string} .match $kind |a\\|b| {{pipe}} * {{other}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args_hit = vec![arg(&catalog, "kind", Value::Str(String::from("a|b")))];
    let args_default = vec![arg(&catalog, "kind", Value::Str(String::from("x")))];

    let out_hit = formatter
        .format_by_id_for_test("main", &args_hit)
        .expect("formatted");
    assert_eq!(out_hit, "pipe");

    let out_default = formatter
        .format_by_id_for_test("main", &args_default)
        .expect("formatted");
    assert_eq!(out_default, "other");
}

#[test]
fn raw_match_with_integer_select_exact_formats_literal_arm() {
    let source = ".local $sel = {1 :integer select=exact} .match $sel 1 {{literal select {$sel}}} * {{OTHER}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "literal select 1");
}

#[test]
fn local_offset_result_can_be_used_in_match_selector() {
    let source = ".local $x = {10 :integer} .local $y = {$x :offset subtract=6} .match $y 10 {{=10}} 4 {{=4}} * {{other}}";
    let bytes = compile_str(source).expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let out = formatter
        .format_by_id_for_test("main", &Vec::<(u32, Value)>::new())
        .expect("formatted");
    assert_eq!(out, "=4");
}

#[test]
fn missing_default_match_fails() {
    let err = compile_str(".input { $x :string }\n.match $x\na {{A}}").expect_err("must fail");
    assert!(matches!(err, CompileError::MissingDefaultArm { .. }));
}

#[test]
fn rejects_private_use_annotation() {
    compile_str("{!.\\{}}").expect_err("private-use annotation must fail");
    compile_str("{!. \\{}}").expect_err("private-use annotation with space must fail");
}

#[test]
fn rejects_reserved_annotation_after_literal() {
    compile_str("hello {|4.2| %num\\{be\\|r}").expect_err("reserved annotation must fail");
}

#[test]
fn bare_plus_is_valid_literal() {
    compile_str("{+}").expect("bare + should compile");
}

// ─── Quoted literal charset (mf-yr4e) ────────────────────────────────

#[test]
fn quoted_literal_allows_braces_inside() {
    let bytes = compile_str("{|foo{bar}baz|}").expect("braces inside quoted literal");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args: Vec<(u32, Value)> = vec![];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "foo{bar}baz");
}

#[test]
fn quoted_literal_allows_dot_and_at() {
    let bytes = compile_str("{|a.b@c|}").expect("dot and at inside quoted literal");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args: Vec<(u32, Value)> = vec![];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "a.b@c");
}

// ─── MF2 whitespace strictness (mf-5h50) ─────────────────────────────

#[test]
fn nbsp_inside_expression_is_not_whitespace() {
    // NBSP (U+00A0) should NOT be treated as whitespace separator
    compile_str("{ $x\u{00A0}:number }").expect_err("NBSP must not act as whitespace");
}

#[test]
fn ideographic_space_is_valid_whitespace() {
    // Ideographic space (U+3000) IS valid MF2 whitespace
    compile_str("{\u{3000}$x :string }").expect("ideographic space should be valid ws");
}

// ─── Bidi-only whitespace (mf-v289) ──────────────────────────────────

#[test]
fn bidi_only_between_function_and_option_fails() {
    // Bidi-only between function and option must not satisfy mandatory ws
    compile_str("{ $x :number\u{200E}style=percent }")
        .expect_err("bidi-only must not satisfy mandatory ws");
}

#[test]
fn bidi_plus_space_between_function_and_option_succeeds() {
    compile_str("{ $x :number\u{200E} style=percent }").expect("bidi + space should satisfy ws");
}

// ─── Escape sequence validation (mf-1mha) ────────────────────────────

#[test]
fn text_escape_backslash_n_fails() {
    compile_str("hello\\nworld").expect_err("\\n is not a valid escape in text");
}

#[test]
fn text_escape_backslash_backslash_succeeds() {
    let bytes = compile_str("hello\\\\world").expect("\\\\");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args: Vec<(u32, Value)> = vec![];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "hello\\world");
}

#[test]
fn text_escape_open_brace_succeeds() {
    let bytes = compile_str("hello\\{world").expect("\\{");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args: Vec<(u32, Value)> = vec![];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "hello{world");
}

#[test]
fn quoted_escape_pipe_succeeds() {
    let bytes = compile_str("{|a\\|b|}").expect("\\| in quoted literal");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args: Vec<(u32, Value)> = vec![];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "a|b");
}

#[test]
fn quoted_escape_backslash_n_fails() {
    compile_str("{|a\\nb|}").expect_err("\\n is not valid in quoted literal");
}

// ─── Bidi around identifiers (mf-s5fv) ───────────────────────────────

#[test]
fn function_with_leading_bidi() {
    compile_str("{ $x :\u{200E}number }").expect("bidi before function name");
}

#[test]
fn markup_with_leading_bidi() {
    compile_str("{#\u{200E}bold}text{/bold}").expect("bidi before markup name");
}

#[test]
fn markup_duplicate_options_are_rejected() {
    let err = compile_str("{#link href=one href=two}text{/link}").expect_err("must fail");
    assert!(matches!(err, CompileError::DuplicateOptionName { .. }));
}

#[test]
fn local_literal_option_values_are_substituted_as_semantic_text() {
    let bytes =
        compile_str(".local $mode = {|fast path|} {{Value: { $name :custom:format mode=$mode }}}")
            .expect("compiled");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    assert!(catalog.string_id("fast path").is_some());
    assert!(catalog.string_id("|fast path|").is_none());
}

// ─── Attribute values (mf-1m8f) ──────────────────────────────────────

#[test]
fn attribute_with_literal_value_succeeds() {
    compile_str("{ $x :string @locale=en }").expect("literal attribute value");
}

#[test]
fn attribute_variable_value_rejected() {
    // Per ABNF, attribute values must be literals, not variables
    compile_str("{ $x :string @locale=$y }").expect_err("variable attribute value must fail");
}

#[test]
fn attribute_without_value_succeeds() {
    compile_str("{ $x :string @flag }").expect("attribute without value");
}

// ─── Standalone quoted-pattern (mf-fm5e) ─────────────────────────────

#[test]
fn standalone_quoted_pattern() {
    let bytes = compile_str("{{Hello}}").expect("standalone quoted pattern");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args: Vec<(u32, Value)> = vec![];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello");
}

#[test]
fn standalone_quoted_pattern_with_declarations() {
    compile_str(".input {$x} {{Hello {$x}}}").expect("declarations + quoted pattern");
}

#[test]
fn standalone_quoted_pattern_with_expression() {
    let bytes = compile_str("{{Hello {$name}}}").expect("quoted pattern with expression");
    let catalog = Catalog::from_bytes(&bytes).expect("catalog");
    let mut formatter = Formatter::new(&catalog, NoopHost).expect("formatter");
    let args = vec![arg(&catalog, "name", Value::Str("World".to_string()))];
    let out = formatter
        .format_by_id_for_test("main", &args)
        .expect("formatted");
    assert_eq!(out, "Hello World");
}
