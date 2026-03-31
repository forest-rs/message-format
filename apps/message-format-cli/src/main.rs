// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! CLI for compiling explicit resource/container inputs into one binary catalog.
//!
//! Custom function manifests are currently used for compile-time validation
//! only. Manifest metadata such as docs/snippets is accepted and preserved by
//! the compiler types, but this CLI does not yet expose richer help or editor
//! features from that data.
//!
//! Minimal manifest example:
//!
//! ```toml
//! [functions."app:link"]
//! format = true
//! operand = "string"
//!
//! [functions."app:link".options.href]
//! value = "either"
//! required = true
//! ```

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use message_format_compiler::{
    BuildError, CompileOptions, CompileReport, DiagnosticSeverity, FunctionManifest, ResourceInput,
    SourceKind, compile_resources, compile_resources_with_manifest,
};
use message_format_resource_json::{JsonProfile, parse_json_resource};
use message_format_resource_toml::parse_resource_toml;

fn main() {
    match run(std::env::args().skip(1)) {
        Ok(RunOutcome::Done) => {}
        Ok(RunOutcome::Help(text)) => {
            println!("{text}");
        }
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}

fn run(args: impl IntoIterator<Item = String>) -> Result<RunOutcome, String> {
    let command = parse_args(args)?;
    match command {
        Command::Help(text) => Ok(RunOutcome::Help(text)),
        Command::Compile {
            inputs,
            input_format,
            output,
            source_map_output,
            functions_manifest,
            check_only,
        } => {
            compile_command(
                &inputs,
                input_format,
                output.as_deref(),
                source_map_output.as_deref(),
                functions_manifest.as_deref(),
                check_only,
            )?;
            Ok(RunOutcome::Done)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Help(String),
    Compile {
        inputs: Vec<PathBuf>,
        input_format: InputFormat,
        output: Option<PathBuf>,
        source_map_output: Option<PathBuf>,
        functions_manifest: Option<PathBuf>,
        check_only: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputFormat {
    ResourceToml,
    JsonFlat,
    JsonChrome,
}

#[derive(Debug, PartialEq, Eq)]
enum RunOutcome {
    Done,
    Help(String),
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(subcommand) = args.next() else {
        return Err(usage());
    };
    if matches!(subcommand.as_str(), "-h" | "--help") {
        return Ok(Command::Help(usage()));
    }
    if subcommand != "compile" {
        return Err(format!("unknown subcommand {subcommand:?}\n\n{}", usage()));
    }

    let mut inputs = Vec::new();
    let mut input_format = None;
    let mut output = None;
    let mut source_map_output = None;
    let mut functions_manifest = None;
    let mut check_only = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" | "--output" => {
                let Some(path) = args.next() else {
                    return Err(String::from("missing path after --output"));
                };
                output = Some(PathBuf::from(path));
            }
            "--source-map" => {
                let Some(path) = args.next() else {
                    return Err(String::from("missing path after --source-map"));
                };
                source_map_output = Some(PathBuf::from(path));
            }
            "--functions" => {
                let Some(path) = args.next() else {
                    return Err(String::from("missing path after --functions"));
                };
                functions_manifest = Some(PathBuf::from(path));
            }
            "--input-format" => {
                let Some(value) = args.next() else {
                    return Err(String::from("missing value after --input-format"));
                };
                input_format = Some(parse_input_format(&value)?);
            }
            "--check" => {
                check_only = true;
            }
            "-h" | "--help" => {
                return Ok(Command::Help(usage()));
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unknown flag {arg:?}"));
            }
            _ => inputs.push(PathBuf::from(arg)),
        }
    }

    if inputs.is_empty() {
        return Err(String::from("at least one input file is required"));
    }
    let Some(input_format) = input_format else {
        return Err(String::from(
            "missing --input-format; expected one of \"resource-toml\", \"json-flat\", or \"json-chrome\"",
        ));
    };

    Ok(Command::Compile {
        inputs,
        input_format,
        output,
        source_map_output,
        functions_manifest,
        check_only,
    })
}

fn compile_command(
    input_paths: &[PathBuf],
    input_format: InputFormat,
    output: Option<&Path>,
    source_map_output: Option<&Path>,
    functions_manifest: Option<&Path>,
    check_only: bool,
) -> Result<(), String> {
    let manifest = if let Some(path) = functions_manifest {
        Some(load_function_manifest(path)?)
    } else {
        None
    };

    let compiled = match input_format {
        InputFormat::ResourceToml => compile_toml_resource_inputs(input_paths, manifest.as_ref()),
        InputFormat::JsonFlat => {
            compile_json_resource_inputs(input_paths, manifest.as_ref(), JsonProfile::Flat)
        }
        InputFormat::JsonChrome => {
            compile_json_resource_inputs(input_paths, manifest.as_ref(), JsonProfile::Chrome)
        }
    };
    if compiled.has_errors() {
        return Err(render_compile_report(compiled));
    }
    let compiled = compiled
        .compiled
        .ok_or_else(|| String::from("compile report completed without catalog or errors"))?;

    if check_only {
        return Ok(());
    }

    if let Some(path) = source_map_output {
        fs::write(path, render_source_map_json(&compiled.source_map))
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    }

    if let Some(path) = output {
        fs::write(path, &compiled.bytes)
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
        return Ok(());
    }

    let mut stdout = io::stdout();
    if stdout.is_terminal() {
        return Err(String::from(
            "refusing to write binary catalog to a terminal; pass --output or --check",
        ));
    }
    stdout
        .write_all(&compiled.bytes)
        .map_err(|err| format!("failed to write catalog to stdout: {err}"))
}

fn compile_toml_resource_inputs(
    input_paths: &[PathBuf],
    manifest: Option<&FunctionManifest>,
) -> CompileReport {
    compile_resource_inputs(input_paths, manifest, parse_resource_toml, "resource-toml")
}

fn render_compile_report(report: CompileReport) -> String {
    report.render()
}

fn parse_input_format(value: &str) -> Result<InputFormat, String> {
    match value {
        "resource-toml" => Ok(InputFormat::ResourceToml),
        "json-flat" => Ok(InputFormat::JsonFlat),
        "json-chrome" => Ok(InputFormat::JsonChrome),
        _ => Err(format!(
            "unknown input format {value:?}; expected \"resource-toml\", \"json-flat\", or \"json-chrome\""
        )),
    }
}

fn compile_json_resource_inputs(
    input_paths: &[PathBuf],
    manifest: Option<&FunctionManifest>,
    profile: JsonProfile,
) -> CompileReport {
    let label = match profile {
        JsonProfile::Flat => "json-flat",
        JsonProfile::Chrome => "json-chrome",
    };
    compile_resource_inputs(
        input_paths,
        manifest,
        |name, source| parse_json_resource(name, source, profile),
        label,
    )
}

fn compile_resource_inputs<E>(
    input_paths: &[PathBuf],
    manifest: Option<&FunctionManifest>,
    parse: impl Fn(String, &str) -> Result<ResourceInput, E>,
    label: &str,
) -> CompileReport
where
    E: std::fmt::Display,
{
    let mut inputs = Vec::with_capacity(input_paths.len());
    let mut diagnostics = Vec::new();
    for path in input_paths {
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(err) => {
                diagnostics.push(BuildError {
                    severity: DiagnosticSeverity::Error,
                    context: None,
                    error: Box::new(message_format_compiler::CompileError::IoError {
                        path: path.clone(),
                        source: err,
                    }),
                });
                continue;
            }
        };
        match parse(path.display().to_string(), &source) {
            Ok(input) => inputs.push(input),
            Err(err) => diagnostics.push(BuildError {
                severity: DiagnosticSeverity::Error,
                context: None,
                error: Box::new(message_format_compiler::CompileError::ResourceInputError {
                    detail: format!("failed to parse {label} {}: {err}", path.display()),
                }),
            }),
        }
    }

    let mut report = if let Some(manifest) = manifest {
        compile_resources_with_manifest(inputs, CompileOptions::default(), manifest)
    } else {
        compile_resources(inputs, CompileOptions::default())
    };
    if !diagnostics.is_empty() {
        report.compiled = None;
        report.diagnostics.extend(diagnostics);
    }
    report
}

fn load_function_manifest(path: &Path) -> Result<FunctionManifest, String> {
    let source = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    FunctionManifest::parse(&source)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn render_source_map_json(source_map: &message_format_compiler::SourceMap) -> String {
    let mut out = String::new();
    out.push_str("{\n  \"sources\": [\n");
    for (index, source) in source_map.sources.iter().enumerate() {
        if index > 0 {
            out.push_str(",\n");
        }
        out.push_str("    {");
        out.push_str(&format!(
            "\"id\": {}, \"name\": \"{}\", \"kind\": \"{}\"",
            source.id.0,
            json_escape(&source.name),
            json_escape(source_kind_name(&source.kind))
        ));
        out.push('}');
    }
    out.push_str("\n  ],\n  \"messages\": [\n");
    for (index, message) in source_map.messages.iter().enumerate() {
        if index > 0 {
            out.push_str(",\n");
        }
        out.push_str("    {");
        out.push_str(&format!(
            "\"messageId\": \"{}\"",
            json_escape(&message.message_id)
        ));
        if let Some(origin) = &message.origin {
            out.push_str(&format!(
                ", \"origin\": {{\"sourceId\": {}, \"byteStart\": {}, \"byteEnd\": {}, \"line\": {}, \"column\": {}}}",
                origin.source_id.0,
                origin.byte_start,
                origin.byte_end,
                origin.line.map_or(String::from("null"), |line| line.to_string()),
                origin.column.map_or(String::from("null"), |column| column.to_string()),
            ));
        } else {
            out.push_str(", \"origin\": null");
        }
        if let Some(origin) = &message.upstream_origin {
            out.push_str(&format!(
                ", \"upstreamOrigin\": {{\"name\": \"{}\", \"kind\": \"{}\", \"byteStart\": {}, \"byteEnd\": {}, \"line\": {}, \"column\": {}}}",
                json_escape(&origin.name),
                json_escape(source_kind_name(&origin.kind)),
                origin.byte_start.map_or(String::from("null"), |byte| byte.to_string()),
                origin.byte_end.map_or(String::from("null"), |byte| byte.to_string()),
                origin.line.map_or(String::from("null"), |line| line.to_string()),
                origin.column.map_or(String::from("null"), |column| column.to_string()),
            ));
        } else {
            out.push_str(", \"upstreamOrigin\": null");
        }
        out.push('}');
    }
    out.push_str("\n  ]\n}\n");
    out
}

fn source_kind_name(kind: &SourceKind) -> &str {
    match kind {
        SourceKind::MessageFormat => "message-format",
        SourceKind::Generated => "generated",
        SourceKind::Rust => "rust",
        SourceKind::Xliff => "xliff",
        SourceKind::Other(name) => name.as_str(),
    }
}

fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

fn usage() -> String {
    String::from(
        "usage:\n  message-format-cli compile [--check] --input-format resource-toml|json-flat|json-chrome [-o OUTPUT] [--source-map PATH] [--functions PATH] INPUT...",
    )
}

#[cfg(test)]
mod tests {
    use super::{Command, InputFormat, parse_args, run};
    use std::path::{Path, PathBuf};
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_temp_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        path.push(format!(
            "message_format_cli_{name}_{}_{}",
            std::process::id(),
            counter
        ));
        path
    }

    fn write_temp_file(path: &Path, contents: &str) {
        fs::write(path, contents).expect("write temp file");
    }

    #[test]
    fn parse_compile_command_with_output_and_source_map() {
        let command = parse_args([
            String::from("compile"),
            String::from("--check"),
            String::from("--input-format"),
            String::from("resource-toml"),
            String::from("-o"),
            String::from("out.mfcat"),
            String::from("--source-map"),
            String::from("out.map.json"),
            String::from("--functions"),
            String::from("functions.toml"),
            String::from("a.toml"),
            String::from("b.toml"),
        ])
        .expect("parsed");

        assert_eq!(
            command,
            Command::Compile {
                inputs: vec![PathBuf::from("a.toml"), PathBuf::from("b.toml")],
                input_format: InputFormat::ResourceToml,
                output: Some(PathBuf::from("out.mfcat")),
                source_map_output: Some(PathBuf::from("out.map.json")),
                functions_manifest: Some(PathBuf::from("functions.toml")),
                check_only: true,
            }
        );
    }

    #[test]
    fn parse_compile_command_with_resource_toml_input_format() {
        let command = parse_args([
            String::from("compile"),
            String::from("--input-format"),
            String::from("resource-toml"),
            String::from("messages.toml"),
        ])
        .expect("parsed");

        assert_eq!(
            command,
            Command::Compile {
                inputs: vec![PathBuf::from("messages.toml")],
                input_format: InputFormat::ResourceToml,
                output: None,
                source_map_output: None,
                functions_manifest: None,
                check_only: false,
            }
        );
    }

    #[test]
    fn parse_compile_command_with_json_input_format() {
        let command = parse_args([
            String::from("compile"),
            String::from("--input-format"),
            String::from("json-flat"),
            String::from("messages.json"),
        ])
        .expect("parsed");

        assert_eq!(
            command,
            Command::Compile {
                inputs: vec![PathBuf::from("messages.json")],
                input_format: InputFormat::JsonFlat,
                output: None,
                source_map_output: None,
                functions_manifest: None,
                check_only: false,
            }
        );
    }

    #[test]
    fn help_returns_help_command() {
        assert_eq!(
            parse_args([String::from("--help")]).expect("parsed"),
            Command::Help(super::usage())
        );
    }

    #[test]
    fn compile_requires_explicit_input_format() {
        let err = parse_args([String::from("compile"), String::from("messages.toml")])
            .expect_err("must fail");
        assert!(err.contains("missing --input-format"));
    }

    #[test]
    fn check_only_does_not_write_source_map() {
        let input = unique_temp_path("input.toml");
        let source_map = unique_temp_path("source_map.json");
        write_temp_file(
            &input,
            r#"
[messages]
"main" = "Hello"
"#,
        );

        let result = run([
            String::from("compile"),
            String::from("--check"),
            String::from("--input-format"),
            String::from("resource-toml"),
            String::from("--source-map"),
            source_map.display().to_string(),
            input.display().to_string(),
        ]);

        let _ = fs::remove_file(&input);
        let _ = fs::remove_file(&source_map);

        assert!(result.is_ok());
        assert!(!source_map.exists());
    }

    #[test]
    fn check_only_accepts_resource_toml_inputs() {
        let input = unique_temp_path("messages.toml");
        write_temp_file(
            &input,
            r#"
[messages]
"app.title" = "Example App"
"#,
        );

        let result = run([
            String::from("compile"),
            String::from("--check"),
            String::from("--input-format"),
            String::from("resource-toml"),
            input.display().to_string(),
        ]);

        let _ = fs::remove_file(&input);

        assert!(result.is_ok());
    }

    #[test]
    fn check_only_accepts_flat_json_inputs() {
        let input = unique_temp_path("messages.json");
        write_temp_file(&input, r#"{ "app.title": "Example App" }"#);

        let result = run([
            String::from("compile"),
            String::from("--check"),
            String::from("--input-format"),
            String::from("json-flat"),
            input.display().to_string(),
        ]);

        let _ = fs::remove_file(&input);

        assert!(result.is_ok());
    }

    #[test]
    fn check_only_accepts_chrome_json_inputs() {
        let input = unique_temp_path("messages-chrome.json");
        write_temp_file(
            &input,
            r#"{ "app.title": { "message": "Example App", "description": "Title" } }"#,
        );

        let result = run([
            String::from("compile"),
            String::from("--check"),
            String::from("--input-format"),
            String::from("json-chrome"),
            input.display().to_string(),
        ]);

        let _ = fs::remove_file(&input);

        assert!(result.is_ok());
    }
}
