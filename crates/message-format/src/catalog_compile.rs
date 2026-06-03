// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{boxed::Box, format, string::String, vec::Vec};

use crate::{compiler, runtime::Catalog};

#[cfg_attr(docsrs, doc(cfg(feature = "compile")))]
impl Catalog {
    /// Compile one MF2 message source string and decode it into a loaded catalog.
    pub fn compile(
        source: &str,
        options: compiler::CompileOptions,
    ) -> Result<Self, compiler::CompileError> {
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
        let bytes = compiler::compile(source, options)?;
        from_compiler_bytes(&bytes)
    }

    /// Compile one MF2 message source string with a function manifest and decode it into a loaded catalog.
    pub fn compile_with_manifest(
        source: &str,
        options: compiler::CompileOptions,
        manifest: &compiler::FunctionManifest,
    ) -> Result<Self, compiler::CompileError> {
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
        let bytes = compiler::compile_with_manifest(source, options, manifest)?;
        from_compiler_bytes(&bytes)
    }

    /// Compile one MF2 message source string with default options and decode it into a loaded catalog.
    pub fn compile_str(source: &str) -> Result<Self, compiler::CompileError> {
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
        let bytes = compiler::compile_str(source)?;
        from_compiler_bytes(&bytes)
    }

    /// Compile multiple explicitly keyed MF2 inputs into one loaded catalog and return source-map metadata.
    pub fn compile_inputs<'a>(
        inputs: impl IntoIterator<Item = compiler::CompileInput<'a>>,
        options: compiler::CompileOptions,
    ) -> Result<(Self, compiler::SourceMap), compiler::BuildError> {
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
        let compiled = compiler::compile_inputs(inputs, options)
            .into_result()
            .map_err(first_build_error)?;
        let catalog = Self::from_bytes(&compiled.bytes).map_err(wrap_catalog_error)?;
        Ok((catalog, compiled.source_map))
    }

    /// Compile multiple resource/container inputs into one loaded catalog and return source-map metadata.
    pub fn compile_resources(
        inputs: impl IntoIterator<Item = compiler::ResourceInput>,
        options: compiler::CompileOptions,
    ) -> Result<(Self, compiler::SourceMap), compiler::BuildError> {
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
        let compiled = compiler::compile_resources(inputs, options)
            .into_result()
            .map_err(first_build_error)?;
        let catalog = Self::from_bytes(&compiled.bytes).map_err(wrap_catalog_error)?;
        Ok((catalog, compiled.source_map))
    }

    /// Compile multiple explicitly keyed MF2 inputs using a function manifest and return source-map metadata.
    pub fn compile_inputs_with_manifest<'a>(
        inputs: impl IntoIterator<Item = compiler::CompileInput<'a>>,
        options: compiler::CompileOptions,
        manifest: &compiler::FunctionManifest,
    ) -> Result<(Self, compiler::SourceMap), compiler::BuildError> {
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
        let compiled = compiler::compile_inputs_with_manifest(inputs, options, manifest)
            .into_result()
            .map_err(first_build_error)?;
        let catalog = Self::from_bytes(&compiled.bytes).map_err(wrap_catalog_error)?;
        Ok((catalog, compiled.source_map))
    }

    /// Compile multiple resource/container inputs using a function manifest and return source-map metadata.
    pub fn compile_resources_with_manifest(
        inputs: impl IntoIterator<Item = compiler::ResourceInput>,
        options: compiler::CompileOptions,
        manifest: &compiler::FunctionManifest,
    ) -> Result<(Self, compiler::SourceMap), compiler::BuildError> {
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
        let compiled = compiler::compile_resources_with_manifest(inputs, options, manifest)
            .into_result()
            .map_err(first_build_error)?;
        let catalog = Self::from_bytes(&compiled.bytes).map_err(wrap_catalog_error)?;
        Ok((catalog, compiled.source_map))
    }

    /// Compile a file from disk and decode it into a loaded catalog.
    #[cfg(feature = "std")]
    pub fn compile_file(path: &std::path::Path) -> Result<Self, compiler::CompileError> {
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
        let source =
            std::fs::read_to_string(path).map_err(|err| compiler::CompileError::IoError {
                path: path.to_path_buf(),
                source: err,
            })?;
        Self::compile(&source, compiler::CompileOptions::default())
    }
}

fn from_compiler_bytes(bytes: &[u8]) -> Result<Catalog, compiler::CompileError> {
    Catalog::from_bytes(bytes).map_err(|err| compiler::CompileError::InternalError {
        detail: format!("compiler emitted invalid catalog: {err:?}"),
    })
}

fn wrap_catalog_error(err: crate::runtime::CatalogError) -> compiler::BuildError {
    compiler::BuildError {
        severity: compiler::DiagnosticSeverity::Error,
        context: None,
        error: Box::new(compiler::CompileError::InternalError {
            detail: format!("compiler emitted invalid catalog: {err:?}"),
        }),
    }
}

fn first_build_error(errors: Vec<compiler::BuildError>) -> compiler::BuildError {
    errors.into_iter().next().unwrap_or(compiler::BuildError {
        severity: compiler::DiagnosticSeverity::Error,
        context: None,
        error: Box::new(compiler::CompileError::InternalError {
            detail: String::from("compile report unexpectedly empty"),
        }),
    })
}
