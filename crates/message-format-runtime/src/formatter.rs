// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! High-level formatting wrappers around the bytecode VM.

use alloc::{boxed::Box, string::ToString, vec::Vec};
use core::fmt;

use crate::{
    catalog::Catalog,
    error::{FormatError, Trap},
    value::{Args, MessageArgs, Value},
    vm::{FormatSink, Host, MessageHandle, VecDiagnostics, run_bytecode},
};

#[derive(Default)]
pub(crate) struct VmState {
    pub(crate) fuel: Option<u64>,
    pub(crate) stack: Vec<Value>,
    pub(crate) call_args: Vec<Value>,
    pub(crate) call_options: Vec<(u32, Value)>,
}

/// Formatter executes catalog messages with caller-provided arguments and host functions.
///
/// Catalogs are expected to come from the compiler or prebuilt assets. This
/// example assumes a loaded catalog whose `"main"` message invokes a host
/// function and formats its result.
///
/// ```rust,no_run
/// use message_format_runtime::{Catalog, FormatError, Formatter, HostFn, HostCallError, Value};
///
/// # fn render(catalog: &Catalog) -> Result<String, FormatError> {
/// let host = HostFn(|_fn_id, _args, _opts| Ok(Value::Str("called".to_string())));
/// let mut formatter = Formatter::new(&catalog, host)?;
/// let message = formatter.resolve("main")?;
/// struct StringSink<'a>(&'a mut String);
/// impl message_format_runtime::FormatSink for StringSink<'_> {
///     fn literal(&mut self, s: &str) { self.0.push_str(s); }
///     fn expression(&mut self, s: &str) { self.0.push_str(s); }
///     fn markup_open(&mut self, _name: &str, _options: &[message_format_runtime::FormatOption<'_>]) {}
///     fn markup_close(&mut self, _name: &str, _options: &[message_format_runtime::FormatOption<'_>]) {}
/// }
/// let mut out = String::new();
/// let mut sink = StringSink(&mut out);
/// let _errors = formatter.format_to(message, &[], &mut sink)?;
/// # Ok(out)
/// # }
/// ```
pub struct Formatter<'a, H: Host> {
    catalog: &'a Catalog,
    index: H::CatalogIndex,
    host: H,
    vm: VmState,
}

impl<H: Host> fmt::Debug for Formatter<'_, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Formatter")
            .field("catalog", &self.catalog)
            .finish_non_exhaustive()
    }
}

impl<'a, H: Host> Formatter<'a, H> {
    /// Create a formatter for a loaded catalog.
    ///
    /// Calls [`Host::index`] to pre-compute catalog-specific data.
    pub fn new(catalog: &'a Catalog, mut host: H) -> Result<Self, FormatError> {
        let index = host.index(catalog)?;
        Ok(Self {
            catalog,
            index,
            host,
            vm: VmState::default(),
        })
    }

    /// Set the maximum number of instructions the VM may execute per message.
    ///
    /// When the budget is exhausted, formatting returns
    /// [`FormatError::Trap`]. Pass `None` for unlimited execution (the
    /// default). Use this to defend against denial-of-service from untrusted
    /// catalogs that may contain infinite loops.
    pub fn set_fuel(&mut self, fuel: Option<u64>) {
        self.vm.fuel = fuel;
    }

    /// Resolve a message id to a reusable handle.
    pub fn resolve(&self, message_id: &str) -> Result<MessageHandle, FormatError> {
        MessageHandle::from_catalog(self.catalog, message_id)
    }

    /// Format one message from a previously resolved handle, dispatching events to a [`FormatSink`].
    ///
    /// Returns recoverable formatting diagnostics collected during fallback
    /// rendering. Fatal execution failures are returned as `Err`.
    ///
    /// This is the runtime API that preserves structured markup. In contrast,
    /// string-oriented convenience helpers flatten only literal/expression text
    /// and drop markup events.
    pub fn format_to<S: FormatSink + ?Sized>(
        &mut self,
        message: MessageHandle,
        args: &dyn Args,
        sink: &mut S,
    ) -> Result<Vec<FormatError>, FormatError> {
        let mut diagnostics = VecDiagnostics::default();
        run_bytecode(
            self.catalog,
            &mut self.host,
            &self.index,
            message.entry_pc,
            args,
            self.vm.fuel,
            &mut self.vm.stack,
            sink,
            Some(&mut diagnostics),
            &mut self.vm.call_args,
            &mut self.vm.call_options,
        )?;
        Ok(diagnostics.into_inner())
    }
}

/// Resolved message handle for a [`MultiFormatter`].
///
/// Carries both the catalog slot and the entry point within that catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiMessageHandle {
    catalog_idx: u32,
    entry_pc: u32,
}

/// Formatter that can operate on multiple catalogs while sharing a single
/// [`Host`] and VM scratch state.
///
/// Each catalog gets its own [`Host::CatalogIndex`], computed at construction time.
/// The catalog list is fixed after construction — messages are resolved by
/// searching catalogs in the order they were provided.
pub struct MultiFormatter<'a, H: Host> {
    catalogs: Box<[(&'a Catalog, H::CatalogIndex)]>,
    host: H,
    vm: VmState,
}

impl<H: Host> fmt::Debug for MultiFormatter<'_, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultiFormatter")
            .field("catalog_count", &self.catalogs.len())
            .finish_non_exhaustive()
    }
}

impl<'a, H: Host> MultiFormatter<'a, H> {
    /// Create a multi-catalog formatter.
    ///
    /// Calls [`Host::index`] for each catalog to build its catalog-specific
    /// index. Catalogs are searched in iterator order during [`Self::resolve`].
    pub fn new(
        catalogs: impl IntoIterator<Item = &'a Catalog>,
        mut host: H,
    ) -> Result<Self, FormatError> {
        let catalogs: Box<[_]> = catalogs
            .into_iter()
            .map(|catalog| {
                let index = host.index(catalog)?;
                Ok((catalog, index))
            })
            .collect::<Result<_, FormatError>>()?;
        if catalogs.len() > u32::MAX as usize {
            return Err(FormatError::Trap(Trap::InvalidCatalogIndex));
        }
        Ok(Self {
            catalogs,
            host,
            vm: VmState::default(),
        })
    }

    /// Set the maximum number of instructions the VM may execute per message.
    ///
    /// When the budget is exhausted, formatting returns
    /// [`FormatError::Trap`]. Pass `None` for unlimited execution (the
    /// default).
    pub fn set_fuel(&mut self, fuel: Option<u64>) {
        self.vm.fuel = fuel;
    }

    /// Resolve a message id by searching catalogs in order.
    ///
    /// Returns a handle to the first catalog that contains the message.
    pub fn resolve(&self, message_id: &str) -> Result<MultiMessageHandle, FormatError> {
        for (idx, (catalog, _)) in self.catalogs.iter().enumerate() {
            if let Some(entry_pc) = catalog.message_pc(message_id) {
                return Ok(MultiMessageHandle {
                    #[allow(
                        clippy::cast_possible_truncation,
                        reason = "catalog count is checked in the constructor"
                    )]
                    catalog_idx: idx as u32,
                    entry_pc,
                });
            }
        }
        Err(FormatError::UnknownMessageId(message_id.to_string()))
    }

    /// Returns the catalog that a resolved handle points into.
    ///
    /// Useful when building a custom [`Args`] implementation against the
    /// correct string pool. For the common case, prefer [`Self::args_for`].
    ///
    /// # Errors
    ///
    /// Returns [`FormatError::Trap`] if the catalog slot index is out of
    /// range. Note that handles do not carry formatter identity, so a handle
    /// from another `MultiFormatter` whose slot index happens to be in range
    /// will be accepted — the caller must ensure handles are used with the
    /// formatter that produced them.
    pub fn catalog_for(&self, handle: MultiMessageHandle) -> Result<&'a Catalog, FormatError> {
        self.catalogs
            .get(handle.catalog_idx as usize)
            .map(|(catalog, _)| *catalog)
            .ok_or(FormatError::Trap(Trap::InvalidCatalogIndex))
    }

    /// Returns an empty [`MessageArgs`] bound to the catalog that `handle`
    /// was resolved from.
    ///
    /// String-pool ids are per-catalog, so args must be built against the
    /// same catalog that owns the message.
    ///
    /// # Errors
    ///
    /// Returns [`FormatError::Trap`] if the catalog slot index is out of
    /// range. See [`Self::catalog_for`] for the handle-provenance caveat.
    pub fn args_for(&self, handle: MultiMessageHandle) -> Result<MessageArgs<'a>, FormatError> {
        self.catalog_for(handle).map(MessageArgs::new)
    }

    /// Format one message, dispatching events to a [`FormatSink`].
    ///
    /// The handle identifies both the catalog and the message entry point.
    ///
    /// # Args safety
    ///
    /// `args` must carry [`StrId`](crate::StrId) keys from the same catalog
    /// that owns the message (see [`Self::args_for`] / [`Self::catalog_for`]).
    /// Passing ids from a different catalog will not panic, but will silently
    /// misbind or miss arguments — the same contract as
    /// [`Formatter::format_to`].
    ///
    /// # Errors
    ///
    /// Returns [`FormatError::Trap`] if the catalog slot index is out of
    /// range. See [`Self::catalog_for`] for the handle-provenance caveat.
    pub fn format_to<S: FormatSink + ?Sized>(
        &mut self,
        message: MultiMessageHandle,
        args: &dyn Args,
        sink: &mut S,
    ) -> Result<Vec<FormatError>, FormatError> {
        let (catalog, index) = self
            .catalogs
            .get(message.catalog_idx as usize)
            .ok_or(FormatError::Trap(Trap::InvalidCatalogIndex))?;
        let mut diagnostics = VecDiagnostics::default();
        run_bytecode(
            catalog,
            &mut self.host,
            index,
            message.entry_pc,
            args,
            self.vm.fuel,
            &mut self.vm.stack,
            sink,
            Some(&mut diagnostics),
            &mut self.vm.call_args,
            &mut self.vm.call_options,
        )?;
        Ok(diagnostics.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use alloc::{string::String, vec};

    use super::*;
    use crate::catalog::{MessageEntry, build_catalog};
    use crate::vm::{FormatOption, NoopHost};

    /// Build a minimal catalog with one message named by `strings[0]`.
    fn one_message_catalog(strings: &[&str], literals: &str, code: &[u8]) -> Catalog {
        let bytes = build_catalog(
            strings,
            literals,
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            code,
        );
        Catalog::from_bytes(&bytes).expect("valid catalog")
    }

    #[derive(Default)]
    struct TestStringSink {
        out: String,
    }

    impl FormatSink for TestStringSink {
        fn literal(&mut self, s: &str) {
            self.out.push_str(s);
        }

        fn expression(&mut self, s: &str) {
            self.out.push_str(s);
        }

        fn markup_open(&mut self, _name: &str, _options: &[FormatOption<'_>]) {}

        fn markup_close(&mut self, _name: &str, _options: &[FormatOption<'_>]) {}
    }

    use crate::schema::TestOps;

    #[test]
    fn first_match_resolution_order() {
        // Both catalogs define "greet" — resolve should pick the first.
        let code = TestOps::new().out_lit(0).halt().build();
        let cat1 = one_message_catalog(&["greet"], "", &code);
        let cat2 = one_message_catalog(&["greet"], "", &code);

        let mf = MultiFormatter::new([&cat1, &cat2], NoopHost).unwrap();
        let handle = mf.resolve("greet").unwrap();
        assert_eq!(handle.catalog_idx, 0);
    }

    #[test]
    fn fallback_to_second_catalog() {
        // First has "greet", second has "farewell".
        let code = TestOps::new().out_lit(0).halt().build();
        let cat1 = one_message_catalog(&["greet"], "", &code);
        let cat2 = one_message_catalog(&["farewell"], "", &code);

        let mf = MultiFormatter::new([&cat1, &cat2], NoopHost).unwrap();
        let handle = mf.resolve("farewell").unwrap();
        assert_eq!(handle.catalog_idx, 1);
    }

    #[test]
    fn unknown_message_across_all_catalogs() {
        let code = TestOps::new().out_lit(0).halt().build();
        let cat1 = one_message_catalog(&["a"], "", &code);
        let cat2 = one_message_catalog(&["b"], "", &code);

        let mf = MultiFormatter::new([&cat1, &cat2], NoopHost).unwrap();
        let err = mf.resolve("missing").unwrap_err();
        assert_eq!(err, FormatError::UnknownMessageId("missing".into()));
    }

    /// Handles only carry a slot index, not formatter identity, so
    /// cross-formatter misuse with an in-range index is *not* detected
    /// (same contract as `MessageHandle` / `Formatter`). This test
    /// verifies the bounds check that *is* performed.
    #[test]
    fn out_of_range_catalog_index() {
        let code = TestOps::new().out_lit(0).halt().build();
        let cat = one_message_catalog(&["a"], "", &code);

        let mf = MultiFormatter::new([&cat], NoopHost).unwrap();

        let bad_handle = MultiMessageHandle {
            catalog_idx: 99,
            entry_pc: 0,
        };

        assert_eq!(
            mf.catalog_for(bad_handle).unwrap_err(),
            FormatError::Trap(Trap::InvalidCatalogIndex)
        );

        let mut mf = mf;
        let mut sink = TestStringSink::default();
        assert_eq!(
            mf.format_to(bad_handle, &vec![] as &Vec<(u32, Value)>, &mut sink)
                .unwrap_err(),
            FormatError::Trap(Trap::InvalidCatalogIndex)
        );
    }

    #[test]
    fn args_bound_to_matched_catalog() {
        // cat1 strings: ["greet"]         → "who" is not interned
        // cat2 strings: ["farewell", "who"] → "who" is str_id 1
        //
        // cat2's message is: OUT_ARG(str_id=1) HALT — it outputs arg "who".
        // If args_for returned a MessageArgs bound to cat1, inserting "who"
        // would fail (not interned), so we'd get the fallback "{$who}".
        let code_noop = TestOps::new().out_lit(0).halt().build();
        let cat1 = one_message_catalog(&["greet"], "", &code_noop);

        let code_arg = TestOps::new().out_arg(1).halt().build();
        let cat2 = one_message_catalog(&["farewell", "who"], "", &code_arg);

        let mut mf = MultiFormatter::new([&cat1, &cat2], NoopHost).unwrap();

        let handle = mf.resolve("farewell").unwrap();
        let mut args = mf.args_for(handle).unwrap();
        // This insert succeeds because args_for bound to cat2 where "who"
        // is interned. Against cat1 it would return ArgNameError.
        args.insert("who", "world").expect("arg interned in cat2");

        let mut sink = TestStringSink::default();
        let diagnostics = mf.format_to(handle, &args, &mut sink).unwrap();
        assert!(diagnostics.is_empty());
        assert_eq!(sink.out, "world");
    }
}
