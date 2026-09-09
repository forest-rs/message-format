// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! ICU4X-backed built-in function host.

#[cfg(test)]
use alloc::vec;
use alloc::{
    borrow::Cow, boxed::Box, collections::BTreeMap, format, string::String, string::ToString,
    vec::Vec,
};
use core::array;
use core::str::FromStr;

use fixed_decimal::{Decimal, SignedRoundingMode, UnsignedRoundingMode};
use icu_calendar::Date;
use icu_datetime::fieldsets;
use icu_datetime::input::{DateTime, Time};
use icu_datetime::options::Length;
use icu_datetime::{DateTimeFormatter, NoCalendarFormatter};
use icu_locale_core::Locale;
use icu_plurals::{PluralCategory, PluralRules};

use crate::common::text::{
    SignDisplay, format_signed_string, parse_number_literal, strip_bidi_controls,
};
use crate::runtime::{
    catalog::Catalog,
    error::{
        FormatError, HostCallError, ImplementationFailure, MessageFunctionError, Trap,
        UnsupportedOperation,
    },
    value::{
        NumberFormatOptions, NumberGrouping, NumberSelection, NumberSignDisplay, NumberValue,
        ResolvedNumber, ResolvedSelect, ResolvedString, StringDirection, Value,
    },
    vm::{FunctionOptions, Host},
};

const MAX_EXACT_I64_IN_F64: i64 = 9_007_199_254_740_992;

fn bad_operand() -> FormatError {
    FormatError::Function(MessageFunctionError::BadOperand)
}

fn bad_option() -> FormatError {
    FormatError::Function(MessageFunctionError::BadOption)
}

fn unsupported_operation(operation: UnsupportedOperation) -> FormatError {
    FormatError::Function(MessageFunctionError::UnsupportedOperation(operation))
}

fn implementation_failure(failure: ImplementationFailure) -> FormatError {
    FormatError::Function(MessageFunctionError::Implementation(failure))
}

fn into_host_call_error(error: FormatError) -> HostCallError {
    debug_assert!(
        matches!(
            error,
            FormatError::Function(_) | FormatError::UnknownFunction { .. }
        ),
        "builtin host must only surface function-shaped errors: {error:?}"
    );
    match error {
        FormatError::Function(error) => HostCallError::Function(error),
        FormatError::UnknownFunction { fn_id } => HostCallError::UnknownFunction { fn_id },
        // Keep a defensive fallback in release builds until the helper return
        // types are narrowed enough to make this structurally impossible.
        _ => HostCallError::Function(MessageFunctionError::Implementation(
            ImplementationFailure::Host,
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuiltinFn {
    String,
    Number,
    Integer,
    Percent,
    Currency,
    Offset,
    TestSelect,
    TestFunction,
    TestFormat,
    Date,
    Time,
    DateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuiltinOptionKey {
    UDir,
    MinimumFractionDigits,
    MaximumFractionDigits,
    SignDisplay,
    Currency,
    Add,
    Subtract,
    Fails,
    DecimalPlaces,
    Select,
    Style,
    Notation,
    UseGrouping,
    MinimumIntegerDigits,
    DateStyle,
    TimeStyle,
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Weekday,
    Era,
    TimeZoneName,
}

const BUILTIN_OPTION_KEY_COUNT: usize = 25;

impl BuiltinOptionKey {
    const fn index(self) -> usize {
        match self {
            Self::UDir => 0,
            Self::MinimumFractionDigits => 1,
            Self::MaximumFractionDigits => 2,
            Self::SignDisplay => 3,
            Self::Currency => 4,
            Self::Add => 5,
            Self::Subtract => 6,
            Self::Fails => 7,
            Self::DecimalPlaces => 8,
            Self::Select => 9,
            Self::Style => 10,
            Self::Notation => 11,
            Self::UseGrouping => 12,
            Self::MinimumIntegerDigits => 13,
            Self::DateStyle => 14,
            Self::TimeStyle => 15,
            Self::Year => 16,
            Self::Month => 17,
            Self::Day => 18,
            Self::Hour => 19,
            Self::Minute => 20,
            Self::Second => 21,
            Self::Weekday => 22,
            Self::Era => 23,
            Self::TimeZoneName => 24,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct BuiltinEntry {
    func: BuiltinFn,
    options: [Option<String>; BUILTIN_OPTION_KEY_COUNT],
    select_mode: BuiltinSelectMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuiltinSelectMode {
    None,
    Plural,
    Ordinal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuiltinGrouping {
    Auto,
    Always,
    Never,
    Min2,
}

#[derive(Debug, Default)]
struct IcuFormatterCache {
    date: DateFormatterCache,
    time: TimeFormatterCache,
    datetime: DateTimeFormatterCache,
}

#[derive(Debug, Default)]
struct DateFormatterCache {
    short: Option<DateTimeFormatter<fieldsets::YMD>>,
    medium: Option<DateTimeFormatter<fieldsets::YMD>>,
    long: Option<DateTimeFormatter<fieldsets::YMD>>,
}

#[derive(Debug, Default)]
struct TimeFormatterCache {
    short: Option<NoCalendarFormatter<fieldsets::T>>,
    medium: Option<NoCalendarFormatter<fieldsets::T>>,
    long: Option<NoCalendarFormatter<fieldsets::T>>,
}

#[derive(Debug, Default)]
struct DateTimeFormatterCache {
    short: TimeStyleDateTimeFormatterCache,
    medium: TimeStyleDateTimeFormatterCache,
    long: TimeStyleDateTimeFormatterCache,
}

#[derive(Debug, Default)]
struct TimeStyleDateTimeFormatterCache {
    short: Option<DateTimeFormatter<fieldsets::YMDT>>,
    medium: Option<DateTimeFormatter<fieldsets::YMDT>>,
    long: Option<DateTimeFormatter<fieldsets::YMDT>>,
}

/// Pre-parsed catalog data needed by the built-in host.
#[derive(Debug)]
pub struct BuiltinHostCatalogIndex {
    by_id: BTreeMap<u16, BuiltinEntry>,
    option_keys_by_str_id: BTreeMap<u32, BuiltinOptionKey>,
    /// Cached string pool IDs for plural category names, indexed by `category_index()`.
    category_pool_ids: [Option<u32>; 6],
}

impl BuiltinHostCatalogIndex {
    fn new(catalog: &Catalog) -> Result<Self, FormatError> {
        let mut by_id = BTreeMap::new();
        let mut option_keys_by_str_id = BTreeMap::new();
        for idx in 0..catalog.string_count() {
            let str_id =
                u32::try_from(idx).map_err(|_| FormatError::Trap(Trap::StringIdOverflow))?;
            let name = catalog
                .string(str_id)
                .map_err(|_| FormatError::Trap(Trap::InvalidStringId))?;
            if let Some(option_key) = parse_builtin_option_key(name) {
                option_keys_by_str_id.insert(str_id, option_key);
            }
        }

        // Build function entries from the FUNC chunk.
        for idx in 0..catalog.func_count() {
            let fn_id =
                u16::try_from(idx).map_err(|_| FormatError::Trap(Trap::FunctionIdOverflow))?;
            let entry = catalog
                .func(fn_id)
                .ok_or(FormatError::Trap(Trap::InvalidFunctionIndex))?;
            let func_name = catalog
                .string(entry.name_str_id)
                .map_err(|_| FormatError::Trap(Trap::InvalidFunctionNameStringId))?;
            let Some(builtin) = (match func_name {
                "string" => Some(BuiltinFn::String),
                "number" => Some(BuiltinFn::Number),
                "integer" => Some(BuiltinFn::Integer),
                "percent" => Some(BuiltinFn::Percent),
                "currency" => Some(BuiltinFn::Currency),
                "offset" => Some(BuiltinFn::Offset),
                "test:select" => Some(BuiltinFn::TestSelect),
                "test:function" => Some(BuiltinFn::TestFunction),
                "test:format" => Some(BuiltinFn::TestFormat),
                "date" => Some(BuiltinFn::Date),
                "time" => Some(BuiltinFn::Time),
                "datetime" => Some(BuiltinFn::DateTime),
                _ => None,
            }) else {
                continue;
            };
            let mut options = array::from_fn(|_| None);
            for &(key_str_id, value_str_id) in &entry.static_options {
                let key = catalog
                    .string(key_str_id)
                    .map_err(|_| FormatError::Trap(Trap::InvalidOptionKeyStringId))?;
                let value = catalog
                    .string(value_str_id)
                    .map_err(|_| FormatError::Trap(Trap::InvalidOptionValueStringId))?;
                let normalized_key = strip_bidi_controls(key);
                let Some(option_key) = parse_builtin_option_key(&normalized_key) else {
                    continue;
                };
                options[option_key.index()] = Some(strip_bidi_controls(value));
            }
            let select_mode = parse_static_select_mode(builtin, &options);
            by_id.insert(
                fn_id,
                BuiltinEntry {
                    func: builtin,
                    options,
                    select_mode,
                },
            );
        }

        // Pre-cache plural category string pool IDs for zero-alloc selection.
        let mut category_pool_ids = [None; 6];
        for (i, name) in CATEGORY_NAMES.iter().enumerate() {
            category_pool_ids[i] = catalog.string_id(name);
        }

        Ok(Self {
            by_id,
            option_keys_by_str_id,
            category_pool_ids,
        })
    }
}

/// Built-in host implementation for a subset of MF2 default functions.
#[derive(Debug)]
pub struct BuiltinHost {
    locale: Locale,
    cardinal_rules: PluralRules,
    ordinal_rules: PluralRules,
    icu_formatters: IcuFormatterCache,
}

impl BuiltinHost {
    /// Build a host for a given locale.
    ///
    /// Returns:
    /// - `FormatError::Trap(Trap::UnsupportedLocale)` when ICU plural rules are unavailable.
    pub fn new(locale: &Locale) -> Result<Self, FormatError> {
        #[cfg(feature = "profiling")]
        profiling::function_scope!();
        let cardinal_rules = PluralRules::try_new_cardinal(locale.into())
            .map_err(|_| FormatError::Trap(Trap::UnsupportedLocale))?;
        let ordinal_rules = PluralRules::try_new_ordinal(locale.into())
            .map_err(|_| FormatError::Trap(Trap::UnsupportedLocale))?;

        Ok(Self {
            locale: locale.clone(),
            cardinal_rules,
            ordinal_rules,
            icu_formatters: IcuFormatterCache::default(),
        })
    }

    fn apply(
        catalog: &Catalog,
        index: &BuiltinHostCatalogIndex,
        locale: &Locale,
        cardinal_rules: &PluralRules,
        ordinal_rules: &PluralRules,
        icu_formatters: &mut IcuFormatterCache,
        entry: &BuiltinEntry,
        args: &[Value],
        opts: FunctionOptions<'_>,
        on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, FormatError> {
        let Some(raw_arg) = args.first() else {
            return Err(bad_operand());
        };
        let options =
            EffectiveOptions::new(&entry.options, opts, catalog, &index.option_keys_by_str_id);
        options.validate_keys()?;
        validate_builtin_option_values(entry.func, &options)?;

        match entry.func {
            BuiltinFn::String => Ok(Value::String(format_string(catalog, raw_arg, &options))),
            BuiltinFn::Number | BuiltinFn::Integer => {
                let integer_only = entry.func == BuiltinFn::Integer;
                if opts.iter().any(|(key_id, _)| {
                    index.option_keys_by_str_id.get(&key_id) == Some(&BuiltinOptionKey::Select)
                }) && !index.option_keys_by_str_id.iter().any(|(key_id, key)| {
                    *key == BuiltinOptionKey::Select && opts.was_unresolved(*key_id)
                }) {
                    on_error(MessageFunctionError::BadOption);
                }
                if options.get(BuiltinOptionKey::Style).as_deref() == Some("percent") {
                    return Ok(Value::Str(format_percent(raw_arg, catalog, &options)?));
                }
                let minimum_fraction_digits = parse_minimum_fraction_digits(&options)?;
                let maximum_fraction_digits = parse_maximum_fraction_digits(&options)?;
                let _ = parse_minimum_integer_digits(&options)?;
                validate_digit_range_relationship(
                    minimum_fraction_digits,
                    maximum_fraction_digits,
                )?;
                let resolved = resolve_number(
                    raw_arg,
                    catalog,
                    integer_only,
                    &options,
                    cardinal_rules,
                    ordinal_rules,
                    on_error,
                )?;
                Ok(Value::Number(Box::new(resolved)))
            }
            BuiltinFn::Percent => Ok(Value::Str(format_percent(raw_arg, catalog, &options)?)),
            BuiltinFn::Currency => Ok(Value::Str(format_currency(raw_arg, catalog, &options)?)),
            BuiltinFn::Offset => Ok(Value::Number(Box::new(resolve_offset(
                raw_arg,
                catalog,
                &options,
                cardinal_rules,
                ordinal_rules,
            )?))),
            BuiltinFn::TestSelect => Ok(Value::ResolvedSelect(Box::new(ResolvedSelect::new(
                format_test_select(raw_arg, catalog, &options)?,
            )))),
            BuiltinFn::TestFunction => format_test_function(raw_arg, catalog, &options),
            BuiltinFn::TestFormat => Err(implementation_failure(ImplementationFailure::TestFormat)),
            BuiltinFn::Date => {
                let text = validate_date_operand(raw_arg, catalog)?;
                let (date, _) = parse_iso_datetime(text)?;
                let style = resolve_date_style(&options);
                Ok(Value::Str(format_icu_date_cached(
                    locale,
                    &mut icu_formatters.date,
                    date,
                    style,
                )?))
            }
            BuiltinFn::Time => {
                let time_str = validate_time_operand(raw_arg, catalog)?;
                let (_, time) = parse_iso_datetime(&time_str)?;
                let style = resolve_time_style(&options);
                Ok(Value::Str(format_icu_time_cached(
                    locale,
                    &mut icu_formatters.time,
                    time,
                    style,
                )?))
            }
            BuiltinFn::DateTime => {
                validate_datetime_style_field_exclusivity(&options)?;
                let text = validate_datetime_operand(raw_arg, catalog)?;
                let (date, time) = parse_iso_datetime(text)?;
                let date_style = resolve_date_style(&options);
                let time_style = resolve_time_style(&options);
                Ok(Value::Str(format_icu_datetime_cached(
                    locale,
                    &mut icu_formatters.datetime,
                    date,
                    time,
                    date_style,
                    time_style,
                )?))
            }
        }
    }

    /// Return the appropriate plural rules if `entry` is a number/integer
    /// function with `select=plural` or `select=ordinal`.
    fn plural_rules_for(
        &self,
        catalog: &Catalog,
        index: &BuiltinHostCatalogIndex,
        entry: &BuiltinEntry,
        args: &[Value],
        opts: FunctionOptions<'_>,
    ) -> Option<&PluralRules> {
        if !matches!(entry.func, BuiltinFn::Number | BuiltinFn::Integer) {
            return None;
        }
        if let Some(Value::Number(number)) = args.first() {
            match number.selection {
                NumberSelection::Plural => return Some(&self.cardinal_rules),
                NumberSelection::Ordinal => return Some(&self.ordinal_rules),
                NumberSelection::Exact | NumberSelection::None | NumberSelection::Invalid => {}
            }
        }
        let dynamic_select = index
            .option_keys_by_str_id
            .iter()
            .any(|(key_id, key)| *key == BuiltinOptionKey::Select && opts.was_dynamic(*key_id));
        if dynamic_select {
            return None;
        }
        if opts.is_empty() {
            return match entry.select_mode {
                BuiltinSelectMode::Plural => Some(&self.cardinal_rules),
                BuiltinSelectMode::Ordinal => Some(&self.ordinal_rules),
                // Number selectors use cardinal plural selection by default.
                // The function entry has no explicit mode when the annotation
                // omits `select`, but a selector still needs a category.
                BuiltinSelectMode::None => Some(&self.cardinal_rules),
            };
        }
        let options =
            EffectiveOptions::new(&entry.options, opts, catalog, &index.option_keys_by_str_id);
        match options.get(BuiltinOptionKey::Select).as_deref() {
            Some("plural") => Some(&self.cardinal_rules),
            Some("ordinal") => Some(&self.ordinal_rules),
            _ => None,
        }
    }
}

/// Build a locale fallback candidate chain using CLDR-aware locale fallback.
///
/// Uses ICU4X [`LocaleFallbacker`](icu_locale::fallback::LocaleFallbacker) with
/// compiled CLDR data for language-priority fallback. This produces
/// linguistically correct chains — e.g. `pt-MZ` → `pt-PT` → `pt` → `und`
/// (rather than naive subtag truncation which would skip `pt-PT`).
#[must_use]
pub fn locale_fallback_candidates(locale: &Locale) -> Vec<Locale> {
    use icu_locale::fallback::LocaleFallbacker;

    let fallbacker = LocaleFallbacker::new();
    let mut iter = fallbacker
        .for_config(icu_locale::fallback::LocaleFallbackConfig::default())
        .fallback_for(locale.into());

    let mut out = Vec::new();
    loop {
        out.push(iter.get().into_locale());
        if iter.get().is_unknown() {
            break;
        }
        iter.step();
    }
    out
}

impl Host for BuiltinHost {
    type CatalogIndex = BuiltinHostCatalogIndex;

    fn index(&mut self, catalog: &Catalog) -> Result<BuiltinHostCatalogIndex, FormatError> {
        BuiltinHostCatalogIndex::new(catalog)
    }

    fn call(
        &mut self,
        catalog: &Catalog,
        index: &BuiltinHostCatalogIndex,
        fn_id: u16,
        args: &[Value],
        opts: FunctionOptions<'_>,
        on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, HostCallError> {
        let Some(entry) = index.by_id.get(&fn_id) else {
            return Err(HostCallError::UnknownFunction { fn_id });
        };
        Self::apply(
            catalog,
            index,
            &self.locale,
            &self.cardinal_rules,
            &self.ordinal_rules,
            &mut self.icu_formatters,
            entry,
            args,
            opts,
            on_error,
        )
        .map_err(into_host_call_error)
    }

    fn call_select(
        &mut self,
        catalog: &Catalog,
        index: &BuiltinHostCatalogIndex,
        fn_id: u16,
        args: &[Value],
        opts: FunctionOptions<'_>,
        on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, HostCallError> {
        let Some(entry) = index.by_id.get(&fn_id) else {
            return Err(HostCallError::UnknownFunction { fn_id });
        };
        if index
            .option_keys_by_str_id
            .iter()
            .any(|(key_id, key)| *key == BuiltinOptionKey::Select && opts.was_dynamic(*key_id))
        {
            if !index.option_keys_by_str_id.iter().any(|(key_id, key)| {
                *key == BuiltinOptionKey::Select && opts.was_unresolved(*key_id)
            }) {
                on_error(MessageFunctionError::BadOption);
            }
            return Ok(Value::Null);
        }
        if matches!(entry.func, BuiltinFn::Number | BuiltinFn::Integer)
            && args.first().is_some_and(|value| {
                matches!(value, Value::Number(number) if number.selection == NumberSelection::Exact)
            })
        {
            return Ok(args.first().cloned().expect("checked above"));
        }
        // For number/integer with select=plural|ordinal, compute category and
        // return a StrRef into the string pool instead of allocating.
        if args.first().is_some_and(|value| {
            matches!(value, Value::Number(number) if number.selection == NumberSelection::Invalid)
        }) {
            return Ok(Value::Null);
        }
        if let Some(rules) = self.plural_rules_for(catalog, index, entry, args, opts) {
            let raw_arg = args
                .first()
                .ok_or_else(bad_operand)
                .map_err(into_host_call_error)?;
            let options =
                EffectiveOptions::new(&entry.options, opts, catalog, &index.option_keys_by_str_id);
            let category =
                plural_category(raw_arg, catalog, rules, &options).map_err(into_host_call_error)?;
            return if let Some(str_id) = index.category_pool_ids[category_index(category)] {
                Ok(Value::StrRef(str_id))
            } else {
                Ok(Value::Str(category_name(category).to_string()))
            };
        }
        Self::apply(
            catalog,
            index,
            &self.locale,
            &self.cardinal_rules,
            &self.ordinal_rules,
            &mut self.icu_formatters,
            entry,
            args,
            opts,
            on_error,
        )
        .map_err(into_host_call_error)
    }

    fn format_default(
        &mut self,
        catalog: &Catalog,
        _index: &BuiltinHostCatalogIndex,
        value: &Value,
    ) -> Option<String> {
        match value {
            Value::Float(v) => Some(format_number_default_locale(*v, &self.locale)),
            Value::Number(number) => render_resolved_number(catalog, number),
            Value::String(value) => Some(apply_bidi_dir(
                Cow::Borrowed(value.text()),
                Some(match value.direction {
                    StringDirection::Auto => "auto",
                    StringDirection::Ltr => "ltr",
                    StringDirection::Rtl => "rtl",
                }),
            )),
            _ => None,
        }
    }
}

fn plain_text<'a>(catalog: &'a Catalog, value: &'a Value) -> Cow<'a, str> {
    // Keep text-backed values borrowed here so string-formatting paths only
    // allocate once for the final result instead of cloning into a temporary.
    match value {
        Value::Null => Cow::Borrowed(""),
        Value::Bool(v) => Cow::Owned(v.to_string()),
        Value::Int(v) => Cow::Owned(v.to_string()),
        Value::Float(v) => Cow::Owned(v.to_string()),
        Value::Str(v) => Cow::Borrowed(v.as_str()),
        Value::String(v) => Cow::Borrowed(v.text()),
        Value::StrRef(id) => catalog
            .pool_string_opt(*id)
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(id.to_string())),
        Value::Fallback(id) => catalog
            .pool_string_opt(*id)
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(id.to_string())),
        Value::LitRef { off, len } => catalog
            .literal_opt(*off, *len)
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(format!("{off}:{len}"))),
        Value::Number(number) => Cow::Owned(number.text()),
        Value::ResolvedSelect(value) => Cow::Borrowed(value.text()),
    }
}

fn parse_static_select_mode(
    builtin: BuiltinFn,
    options: &[Option<String>; BUILTIN_OPTION_KEY_COUNT],
) -> BuiltinSelectMode {
    if !matches!(builtin, BuiltinFn::Number | BuiltinFn::Integer) {
        return BuiltinSelectMode::None;
    }
    match options[BuiltinOptionKey::Select.index()].as_deref() {
        Some("plural") => BuiltinSelectMode::Plural,
        Some("ordinal") => BuiltinSelectMode::Ordinal,
        _ => BuiltinSelectMode::None,
    }
}

fn format_string(
    catalog: &Catalog,
    value: &Value,
    options: &EffectiveOptions<'_>,
) -> ResolvedString {
    let dir = options.get(BuiltinOptionKey::UDir);
    let text = plain_text(catalog, value).into_owned();
    let direction = match dir.as_deref() {
        Some("ltr") => StringDirection::Ltr,
        Some("rtl") => StringDirection::Rtl,
        _ => StringDirection::Auto,
    };
    ResolvedString {
        text: text.into_boxed_str(),
        direction,
    }
}

fn value_text<'a>(catalog: &'a Catalog, value: &'a Value) -> Option<&'a str> {
    match value {
        Value::Str(value) => Some(value),
        Value::StrRef(id) => catalog.pool_string_opt(*id),
        Value::LitRef { off, len } => catalog.literal_opt(*off, *len),
        Value::ResolvedSelect(value) => Some(value.text()),
        Value::String(value) => Some(value.text()),
        _ => None,
    }
}

fn resolve_number(
    value: &Value,
    catalog: &Catalog,
    integer_only: bool,
    options: &EffectiveOptions<'_>,
    cardinal_rules: &PluralRules,
    ordinal_rules: &PluralRules,
    on_error: &mut dyn FnMut(MessageFunctionError),
) -> Result<ResolvedNumber, FormatError> {
    let (mut number, inherited_format, inherited_selection, inherited_select) = match value {
        Value::Number(number) => (
            number.value.clone(),
            number.format,
            number.selection,
            number.has_explicit_select,
        ),
        _ => (
            parse_number_value(value, catalog)?,
            NumberFormatOptions::DEFAULT,
            NumberSelection::None,
            false,
        ),
    };
    if integer_only {
        // Integer annotations intentionally discard precision inherited from a
        // preceding number annotation. The integer function itself emits no
        // fraction digits either.
        let integer_text = match &number {
            NumberValue::Integer(value) => value.to_string(),
            NumberValue::Decimal(value) => {
                truncate_decimal_text(&value.to_string()).ok_or_else(bad_operand)?
            }
            NumberValue::NonFinite(value) => value.to_string(),
        };
        number = if matches!(&number, NumberValue::NonFinite(_)) {
            NumberValue::NonFinite(integer_text.parse().map_err(|_| bad_operand())?)
        } else if let Ok(value) = integer_text.parse::<i64>() {
            NumberValue::Integer(value)
        } else {
            NumberValue::Decimal(Decimal::from_str(&integer_text).map_err(|_| bad_operand())?)
        };
    }
    let format = resolve_number_format_options(inherited_format, options, integer_only)?;
    let has_explicit_select = inherited_select || options.get(BuiltinOptionKey::Select).is_some();
    let selection = if inherited_select {
        if !options.has_runtime(BuiltinOptionKey::Select) {
            on_error(MessageFunctionError::BadOption);
        }
        NumberSelection::Invalid
    } else if options.has_runtime(BuiltinOptionKey::Select) {
        // The caller reports this option error before resolving the value.
        NumberSelection::Invalid
    } else if let Some(select) = options.get(BuiltinOptionKey::Select) {
        match select.as_ref() {
            "plural" => NumberSelection::Plural,
            "ordinal" => NumberSelection::Ordinal,
            "exact" => NumberSelection::Exact,
            _ => NumberSelection::Invalid,
        }
    } else {
        // A number selector uses cardinal plural rules when no explicit
        // selection mode is present. Retain that mode on the stored value so
        // direct local/input matching can use the resolved category.
        match inherited_selection {
            NumberSelection::None => NumberSelection::Plural,
            selection => selection,
        }
    };
    let mut resolved = ResolvedNumber::new(number, format, selection, has_explicit_select);
    if matches!(
        selection,
        NumberSelection::Plural | NumberSelection::Ordinal
    ) && !matches!(resolved.value, NumberValue::NonFinite(_))
    {
        let rules = if selection == NumberSelection::Ordinal {
            ordinal_rules
        } else {
            cardinal_rules
        };
        resolved.set_selection_category(Some(resolved_plural_category(&resolved, rules)?));
    }
    Ok(resolved)
}

fn parse_number_value(value: &Value, catalog: &Catalog) -> Result<NumberValue, FormatError> {
    if let Value::Float(value) = value
        && value.is_sign_negative()
        && *value == 0.0
    {
        return Ok(NumberValue::Decimal(
            Decimal::from_str("-0").map_err(|_| bad_operand())?,
        ));
    }
    if let Value::Float(value) = value
        && !value.is_finite()
    {
        return Ok(NumberValue::NonFinite(*value));
    }
    if let Value::Float(value) = value
        && let Some(value) = exact_integral_float(*value)
    {
        return Ok(NumberValue::Integer(value));
    }
    let text = match value {
        Value::Int(value) => return Ok(NumberValue::Integer(*value)),
        Value::Float(value) => value.to_string(),
        _ => value_text(catalog, value)
            .ok_or_else(bad_operand)?
            .to_string(),
    };
    // Parsing integers before any floating-point conversion is essential: an
    // i64 such as 9007199254740993 must remain exact.
    if text == "-0" {
        return Ok(NumberValue::Decimal(
            Decimal::from_str(&text).map_err(|_| bad_operand())?,
        ));
    }
    if let Ok(value) = text.parse::<i64>() {
        if parse_number_literal(&text).is_none() {
            return Err(bad_operand());
        }
        return Ok(NumberValue::Integer(value));
    }
    if parse_number_literal(&text).is_none() {
        return Err(bad_operand());
    }
    let decimal = match Decimal::from_str(&text) {
        Ok(decimal) => decimal,
        Err(_) => {
            let value = text.parse::<f64>().map_err(|_| bad_operand())?;
            Decimal::from_str(&value.to_string()).map_err(|_| bad_operand())?
        }
    };
    Ok(NumberValue::Decimal(trim_decimal_end(decimal)))
}

/// Convert an integral float to an integer only while every integer in its
/// range is exactly representable by `f64`. Larger integral floats retain the
/// existing decimal parsing path so their shortest decimal representation is
/// not changed by a narrowing conversion.
fn exact_integral_float(value: f64) -> Option<i64> {
    let limit = MAX_EXACT_I64_IN_F64 as f64;
    if libm::trunc(value) == value && (-limit..=limit).contains(&value) {
        // The range check above makes this cast exact and within i64 bounds.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the preceding exact range check proves this conversion is lossless"
        )]
        Some(value as i64)
    } else {
        None
    }
}

fn resolve_number_format_options(
    inherited: NumberFormatOptions,
    options: &EffectiveOptions<'_>,
    integer_only: bool,
) -> Result<NumberFormatOptions, FormatError> {
    let minimum_fraction_digits = if integer_only {
        None
    } else {
        parse_digit_option_or_inherited(
            options,
            BuiltinOptionKey::MinimumFractionDigits,
            inherited.minimum_fraction_digits,
            MAX_FRACTION_DIGITS,
        )?
    };
    let maximum_fraction_digits = if integer_only {
        None
    } else {
        parse_digit_option_or_inherited(
            options,
            BuiltinOptionKey::MaximumFractionDigits,
            inherited.maximum_fraction_digits,
            MAX_FRACTION_DIGITS,
        )?
    };
    validate_digit_range_relationship(minimum_fraction_digits, maximum_fraction_digits)?;
    let minimum_integer_digits = parse_digit_option_or_inherited(
        options,
        BuiltinOptionKey::MinimumIntegerDigits,
        inherited.minimum_integer_digits,
        MAX_INTEGER_DIGITS,
    )?;
    let sign_display = match options.get(BuiltinOptionKey::SignDisplay).as_deref() {
        None => inherited.sign_display,
        Some("auto") => NumberSignDisplay::Auto,
        Some("always") => NumberSignDisplay::Always,
        Some("never") => NumberSignDisplay::Never,
        Some(_) => return Err(bad_option()),
    };
    let notation_scientific = match options.get(BuiltinOptionKey::Notation).as_deref() {
        None => inherited.notation_scientific,
        Some("scientific") => true,
        Some(_) => return Err(bad_option()),
    };
    let grouping = match options.get(BuiltinOptionKey::UseGrouping).as_deref() {
        None => inherited.grouping,
        Some("auto") => NumberGrouping::Auto,
        Some("always") => NumberGrouping::Always,
        Some("never") => NumberGrouping::Never,
        Some("min2") => NumberGrouping::Min2,
        Some(_) => return Err(bad_option()),
    };
    Ok(NumberFormatOptions {
        minimum_fraction_digits,
        maximum_fraction_digits,
        minimum_integer_digits,
        sign_display,
        notation_scientific,
        grouping,
    })
}

fn parse_digit_option_or_inherited(
    options: &EffectiveOptions<'_>,
    key: BuiltinOptionKey,
    inherited: Option<usize>,
    max: usize,
) -> Result<Option<usize>, FormatError> {
    if let Some(value) = options.get(key) {
        let value = value.parse::<usize>().map_err(|_| bad_option())?;
        if value > max {
            return Err(bad_option());
        }
        return Ok(Some(value));
    }
    Ok(inherited)
}

fn render_resolved_number(_catalog: &Catalog, number: &ResolvedNumber) -> Option<String> {
    let format = number.format;
    if let NumberValue::NonFinite(value) = number.value {
        return Some(format_signed_string(
            match format.sign_display {
                NumberSignDisplay::Auto => SignDisplay::Auto,
                NumberSignDisplay::Always => SignDisplay::Always,
                NumberSignDisplay::Never => SignDisplay::Never,
            },
            value.to_string(),
        ));
    }
    if format.notation_scientific {
        return Some(format_signed_string(
            match format.sign_display {
                NumberSignDisplay::Auto => SignDisplay::Auto,
                NumberSignDisplay::Always => SignDisplay::Always,
                NumberSignDisplay::Never => SignDisplay::Never,
            },
            format_scientific_text(&number.text()),
        ));
    }
    let text = format_int_or_decimal_with_min_fraction_digits(
        number.text(),
        format.minimum_fraction_digits.unwrap_or(0),
    );
    let text = apply_maximum_fraction_digits(text, format.maximum_fraction_digits);
    let text = format_signed_string(
        match format.sign_display {
            NumberSignDisplay::Auto => SignDisplay::Auto,
            NumberSignDisplay::Always => SignDisplay::Always,
            NumberSignDisplay::Never => SignDisplay::Never,
        },
        text,
    );
    let text = apply_minimum_integer_digits(text, format.minimum_integer_digits);
    Some(apply_grouping_strategy(
        text,
        match format.grouping {
            NumberGrouping::Auto => BuiltinGrouping::Auto,
            NumberGrouping::Always => BuiltinGrouping::Always,
            NumberGrouping::Never => BuiltinGrouping::Never,
            NumberGrouping::Min2 => BuiltinGrouping::Min2,
        },
    ))
}

fn format_scientific_text(value: &str) -> String {
    let (sign, unsigned) = if let Some(value) = value.strip_prefix('-') {
        ("-", value)
    } else if let Some(value) = value.strip_prefix('+') {
        ("+", value)
    } else {
        ("", value)
    };
    let (integer, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned, ""), |parts| parts);
    let digits = format!("{integer}{fraction}");
    let leading = digits.len() - digits.trim_start_matches('0').len();
    if leading == digits.len() {
        return format!("{sign}0E0");
    }
    let significant = &digits[leading..];
    let exponent = integer.len().cast_signed() - leading.cast_signed() - 1;
    let mut mantissa = significant[..1].to_string();
    let rest = significant[1..].trim_end_matches('0');
    if !rest.is_empty() {
        mantissa.push('.');
        mantissa.push_str(rest);
    }
    format!("{sign}{mantissa}E{exponent}")
}

fn format_int_or_decimal_with_min_fraction_digits(mut text: String, minimum: usize) -> String {
    let current = text
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    if current >= minimum {
        return text;
    }
    if current == 0 {
        text.push('.');
    }
    for _ in current..minimum {
        text.push('0');
    }
    text
}

fn plural_category(
    value: &Value,
    catalog: &Catalog,
    rules: &PluralRules,
    options: &EffectiveOptions<'_>,
) -> Result<PluralCategory, FormatError> {
    let minimum_fraction_digits = parse_number_digit_option(value, options, true)?;
    let maximum_fraction_digits = parse_number_digit_option(value, options, false)?;
    validate_digit_range_relationship(minimum_fraction_digits, maximum_fraction_digits)?;

    if minimum_fraction_digits.is_none() && maximum_fraction_digits.is_none() {
        return match value {
            Value::Int(v) => Ok(rules.category_for(*v)),
            Value::Float(v) => {
                let decimal = Decimal::from_str(&v.to_string()).map_err(|_| bad_operand())?;
                Ok(rules.category_for(&decimal))
            }
            Value::Number(number) => match &number.value {
                NumberValue::Integer(value) => Ok(rules.category_for(*value)),
                NumberValue::Decimal(value) => Ok(rules.category_for(value)),
                NumberValue::NonFinite(_) => Err(bad_operand()),
            },
            _ => {
                let text = value_text(catalog, value).ok_or_else(bad_operand)?;
                let decimal = Decimal::from_str(text).map_err(|_| bad_operand())?;
                Ok(rules.category_for(&decimal))
            }
        };
    }

    let formatted = format_plural_operand(
        value,
        catalog,
        minimum_fraction_digits,
        maximum_fraction_digits,
    )?;
    let decimal = Decimal::from_str(&formatted).map_err(|_| bad_operand())?;
    Ok(rules.category_for(&decimal))
}

/// Compute the category for a resolved number using the precision that was
/// validated and retained on the value. This is intentionally separate from
/// `plural_category`: stored values must be selectable without reapplying
/// their function or resolving options a second time.
fn resolved_plural_category(
    number: &ResolvedNumber,
    rules: &PluralRules,
) -> Result<PluralCategory, FormatError> {
    let minimum_fraction_digits = number.format.minimum_fraction_digits;
    let maximum_fraction_digits = number.format.maximum_fraction_digits;
    validate_digit_range_relationship(minimum_fraction_digits, maximum_fraction_digits)?;
    if minimum_fraction_digits.is_none() && maximum_fraction_digits.is_none() {
        return match &number.value {
            NumberValue::Integer(value) => Ok(rules.category_for(*value)),
            NumberValue::Decimal(value) => Ok(rules.category_for(value)),
            NumberValue::NonFinite(_) => Err(bad_operand()),
        };
    }
    let mut decimal = match &number.value {
        NumberValue::Integer(value) => Decimal::from(*value),
        NumberValue::Decimal(value) => value.clone(),
        NumberValue::NonFinite(_) => return Err(bad_operand()),
    };

    if let Some(minimum) = minimum_fraction_digits {
        // The supported option range is small, but keep the conversion
        // checked because fixed-decimal positions are i16.
        let minimum = i16::try_from(minimum)
            .map_err(|_| bad_option())?
            .checked_neg()
            .ok_or_else(bad_option)?;
        if *decimal.magnitude_range().start() > minimum {
            decimal.pad_end(minimum);
        }
    }
    if let Some(maximum) = maximum_fraction_digits {
        let maximum = i16::try_from(maximum).map_err(|_| bad_option())?;
        let current_fraction_digits = decimal
            .magnitude_range()
            .start()
            .checked_neg()
            .unwrap_or(i16::MAX)
            .max(0);
        if current_fraction_digits > maximum {
            decimal.round_with_mode(
                -maximum,
                SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfExpand),
            );
        }
    }
    Ok(rules.category_for(&decimal))
}

fn format_plural_operand(
    value: &Value,
    catalog: &Catalog,
    minimum_fraction_digits: Option<usize>,
    maximum_fraction_digits: Option<usize>,
) -> Result<String, FormatError> {
    match value {
        Value::Number(number) => Ok(apply_maximum_fraction_digits(
            minimum_fraction_digits.map_or_else(
                || number.text(),
                |minimum| format_int_or_decimal_with_min_fraction_digits(number.text(), minimum),
            ),
            maximum_fraction_digits,
        )),
        Value::Int(v) => {
            let rendered = if let Some(min) = minimum_fraction_digits {
                format_int_with_min_fraction_digits(*v, min)
            } else {
                v.to_string()
            };
            Ok(apply_maximum_fraction_digits(
                rendered,
                maximum_fraction_digits,
            ))
        }
        Value::Float(v) => {
            let rendered = if let Some(min) = minimum_fraction_digits {
                format_float_with_min_fraction_digits(*v, min)
            } else {
                v.to_string()
            };
            Ok(apply_maximum_fraction_digits(
                rendered,
                maximum_fraction_digits,
            ))
        }
        _ => {
            let text = value_text(catalog, value).ok_or_else(bad_operand)?;
            if minimum_fraction_digits.is_none() {
                return Ok(apply_maximum_fraction_digits(
                    text.to_string(),
                    maximum_fraction_digits,
                ));
            }
            let parsed = parse_number_literal(text).ok_or_else(bad_operand)?;
            let digits = minimum_fraction_digits.unwrap_or(0);
            let rendered = format!("{parsed:.digits$}");
            Ok(apply_maximum_fraction_digits(
                rendered,
                maximum_fraction_digits,
            ))
        }
    }
}

fn parse_number_digit_option(
    value: &Value,
    options: &EffectiveOptions<'_>,
    minimum: bool,
) -> Result<Option<usize>, FormatError> {
    let key = if minimum {
        BuiltinOptionKey::MinimumFractionDigits
    } else {
        BuiltinOptionKey::MaximumFractionDigits
    };
    if let Some(found) = options.get(key) {
        let value = found.parse::<usize>().map_err(|_| bad_option())?;
        if value > MAX_FRACTION_DIGITS {
            return Err(bad_option());
        }
        return Ok(Some(value));
    }
    if let Value::Number(number) = value {
        return Ok(if minimum {
            number.format.minimum_fraction_digits
        } else {
            number.format.maximum_fraction_digits
        });
    }
    Ok(None)
}

const CATEGORY_NAMES: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];
const MAX_FRACTION_DIGITS: usize = 20;
const MAX_INTEGER_DIGITS: usize = 21;

fn category_index(category: PluralCategory) -> usize {
    match category {
        PluralCategory::Zero => 0,
        PluralCategory::One => 1,
        PluralCategory::Two => 2,
        PluralCategory::Few => 3,
        PluralCategory::Many => 4,
        PluralCategory::Other => 5,
    }
}

fn category_name(category: PluralCategory) -> &'static str {
    CATEGORY_NAMES[category_index(category)]
}

fn parse_builtin_option_key(value: &str) -> Option<BuiltinOptionKey> {
    Some(match value {
        "u:dir" => BuiltinOptionKey::UDir,
        "minimumFractionDigits" => BuiltinOptionKey::MinimumFractionDigits,
        "maximumFractionDigits" => BuiltinOptionKey::MaximumFractionDigits,
        "signDisplay" => BuiltinOptionKey::SignDisplay,
        "currency" => BuiltinOptionKey::Currency,
        "add" => BuiltinOptionKey::Add,
        "subtract" => BuiltinOptionKey::Subtract,
        "fails" => BuiltinOptionKey::Fails,
        "decimalPlaces" => BuiltinOptionKey::DecimalPlaces,
        "select" => BuiltinOptionKey::Select,
        "style" => BuiltinOptionKey::Style,
        "notation" => BuiltinOptionKey::Notation,
        "useGrouping" => BuiltinOptionKey::UseGrouping,
        "minimumIntegerDigits" => BuiltinOptionKey::MinimumIntegerDigits,
        "dateStyle" => BuiltinOptionKey::DateStyle,
        "timeStyle" => BuiltinOptionKey::TimeStyle,
        "year" => BuiltinOptionKey::Year,
        "month" => BuiltinOptionKey::Month,
        "day" => BuiltinOptionKey::Day,
        "hour" => BuiltinOptionKey::Hour,
        "minute" => BuiltinOptionKey::Minute,
        "second" => BuiltinOptionKey::Second,
        "weekday" => BuiltinOptionKey::Weekday,
        "era" => BuiltinOptionKey::Era,
        "timeZoneName" => BuiltinOptionKey::TimeZoneName,
        _ => return None,
    })
}

fn validate_builtin_option_values(
    func: BuiltinFn,
    options: &EffectiveOptions<'_>,
) -> Result<(), FormatError> {
    validate_enum_option(options, BuiltinOptionKey::UDir, &["ltr", "rtl", "auto"])?;
    match func {
        BuiltinFn::Number | BuiltinFn::Integer => {
            validate_enum_option(
                options,
                BuiltinOptionKey::SignDisplay,
                &["auto", "always", "never"],
            )?;
            validate_enum_option(options, BuiltinOptionKey::Style, &["percent"])?;
            validate_enum_option(
                options,
                BuiltinOptionKey::Select,
                &["exact", "plural", "ordinal"],
            )?;
            validate_enum_option(options, BuiltinOptionKey::Notation, &["scientific"])?;
            validate_enum_option(
                options,
                BuiltinOptionKey::UseGrouping,
                &["auto", "always", "never", "min2"],
            )?;
        }
        BuiltinFn::Date => {
            validate_enum_option(
                options,
                BuiltinOptionKey::Style,
                &["short", "medium", "long", "full"],
            )?;
            validate_enum_option(
                options,
                BuiltinOptionKey::DateStyle,
                &["short", "medium", "long", "full"],
            )?;
        }
        BuiltinFn::Time => {
            validate_enum_option(
                options,
                BuiltinOptionKey::Style,
                &["short", "medium", "long", "full"],
            )?;
            validate_enum_option(
                options,
                BuiltinOptionKey::TimeStyle,
                &["short", "medium", "long", "full"],
            )?;
        }
        BuiltinFn::DateTime => {
            validate_enum_option(
                options,
                BuiltinOptionKey::Style,
                &["short", "medium", "long", "full"],
            )?;
            validate_enum_option(
                options,
                BuiltinOptionKey::DateStyle,
                &["short", "medium", "long", "full"],
            )?;
            validate_enum_option(
                options,
                BuiltinOptionKey::TimeStyle,
                &["short", "medium", "long", "full"],
            )?;
        }
        BuiltinFn::String
        | BuiltinFn::Percent
        | BuiltinFn::Currency
        | BuiltinFn::Offset
        | BuiltinFn::TestSelect
        | BuiltinFn::TestFunction
        | BuiltinFn::TestFormat => {}
    }
    Ok(())
}

fn validate_enum_option(
    options: &EffectiveOptions<'_>,
    key: BuiltinOptionKey,
    allowed: &[&str],
) -> Result<(), FormatError> {
    if key == BuiltinOptionKey::Select && options.has_runtime(key) {
        return Ok(());
    }
    let Some(value) = options.get(key) else {
        return Ok(());
    };
    if allowed.iter().any(|candidate| *candidate == value) {
        return Ok(());
    }
    Err(bad_option())
}

fn parse_minimum_fraction_digits(
    options: &EffectiveOptions<'_>,
) -> Result<Option<usize>, FormatError> {
    parse_digit_option(
        options,
        BuiltinOptionKey::MinimumFractionDigits,
        MAX_FRACTION_DIGITS,
    )
}

fn parse_maximum_fraction_digits(
    options: &EffectiveOptions<'_>,
) -> Result<Option<usize>, FormatError> {
    parse_digit_option(
        options,
        BuiltinOptionKey::MaximumFractionDigits,
        MAX_FRACTION_DIGITS,
    )
}

fn parse_digit_option(
    options: &EffectiveOptions<'_>,
    key: BuiltinOptionKey,
    max: usize,
) -> Result<Option<usize>, FormatError> {
    let Some(raw) = options.get(key) else {
        return Ok(None);
    };
    let value = raw.parse::<usize>().map_err(|_| bad_option())?;
    if value > max {
        return Err(bad_option());
    }
    Ok(Some(value))
}

fn validate_digit_range_relationship(
    min: Option<usize>,
    max: Option<usize>,
) -> Result<(), FormatError> {
    if let (Some(min), Some(max)) = (min, max)
        && min > max
    {
        return Err(bad_option());
    }
    Ok(())
}

fn apply_maximum_fraction_digits(value: String, max: Option<usize>) -> String {
    let Some(max) = max else {
        return value;
    };
    let Some(dot_pos) = value.find('.') else {
        return value;
    };
    let frac_start = dot_pos + 1;
    let frac_len = value[frac_start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .count();
    if frac_len <= max {
        return value;
    }
    let numeric_end = frac_start + frac_len;
    let suffix = &value[numeric_end..];
    let numeric = &value[..numeric_end];
    let rounded = round_decimal_numeric_text(numeric, max).unwrap_or_else(|| numeric.to_string());
    format!("{rounded}{suffix}")
}

fn round_decimal_numeric_text(value: &str, max_fraction_digits: usize) -> Option<String> {
    let (sign, rest) = if let Some(stripped) = value.strip_prefix('-') {
        ("-", stripped)
    } else if let Some(stripped) = value.strip_prefix('+') {
        ("+", stripped)
    } else {
        ("", value)
    };
    let (integer, fraction) = rest.split_once('.')?;
    if !integer.chars().all(|ch| ch.is_ascii_digit())
        || !fraction.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    if fraction.len() <= max_fraction_digits {
        return Some(value.to_string());
    }

    let mut integer_digits = integer.as_bytes().to_vec();
    let mut kept_fraction = fraction.as_bytes()[..max_fraction_digits].to_vec();
    let round_up = fraction.as_bytes()[max_fraction_digits] >= b'5';
    if round_up && (max_fraction_digits == 0 || !carry_fraction_digits(&mut kept_fraction)) {
        carry_integer_digits(&mut integer_digits);
    }

    let mut out = String::with_capacity(value.len() + 1);
    out.push_str(sign);
    for digit in integer_digits {
        out.push(char::from(digit));
    }
    if max_fraction_digits > 0 {
        out.push('.');
        for digit in kept_fraction {
            out.push(char::from(digit));
        }
    }
    Some(out)
}

fn carry_fraction_digits(digits: &mut [u8]) -> bool {
    for digit in digits.iter_mut().rev() {
        if *digit == b'9' {
            *digit = b'0';
            continue;
        }
        *digit += 1;
        return true;
    }
    false
}

fn carry_integer_digits(digits: &mut Vec<u8>) {
    for digit in digits.iter_mut().rev() {
        if *digit == b'9' {
            *digit = b'0';
            continue;
        }
        *digit += 1;
        return;
    }
    digits.insert(0, b'1');
}

fn parse_minimum_integer_digits(
    options: &EffectiveOptions<'_>,
) -> Result<Option<usize>, FormatError> {
    parse_digit_option(
        options,
        BuiltinOptionKey::MinimumIntegerDigits,
        MAX_INTEGER_DIGITS,
    )
}

fn parse_sign_display(options: &EffectiveOptions<'_>) -> SignDisplay {
    match options.get(BuiltinOptionKey::SignDisplay).as_deref() {
        Some("always") => SignDisplay::Always,
        Some("never") => SignDisplay::Never,
        Some("auto") | None => SignDisplay::Auto,
        Some(other) => unreachable!("unexpected validated signDisplay value: {other}"),
    }
}

#[cfg(test)]
fn format_scientific(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    if value == 0.0 {
        return "0E0".to_string();
    }
    // Use scientific formatting to avoid lossy float->int casts when deriving exponent.
    let scientific = format!("{value:e}");
    let (mantissa_raw, exponent_raw) = scientific
        .split_once('e')
        .expect("scientific formatting must contain exponent separator");
    let exp = exponent_raw
        .parse::<i32>()
        .expect("scientific exponent must parse as i32");
    let raw = mantissa_raw.to_string();
    let trimmed = if raw.contains('.') {
        raw.trim_end_matches('0').trim_end_matches('.')
    } else {
        &raw
    };
    format!("{trimmed}E{exp}")
}

fn apply_minimum_integer_digits(value: String, min: Option<usize>) -> String {
    let Some(min) = min else {
        return value;
    };
    let (sign, rest) = if let Some(stripped) = value.strip_prefix('-') {
        ("-", stripped)
    } else if let Some(stripped) = value.strip_prefix('+') {
        ("+", stripped)
    } else {
        ("", value.as_str())
    };
    let (integer, suffix) = rest.split_once('.').map_or((rest, ""), |(i, f)| (i, f));
    let int_len = integer.len();
    if int_len >= min {
        return value;
    }
    let padding = min - int_len;
    let mut out = String::with_capacity(value.len() + padding);
    out.push_str(sign);
    for _ in 0..padding {
        out.push('0');
    }
    out.push_str(integer);
    if !suffix.is_empty() {
        out.push('.');
        out.push_str(suffix);
    }
    out
}

fn apply_grouping(value: String) -> String {
    let (sign, rest) = if let Some(stripped) = value.strip_prefix('-') {
        ("-", stripped)
    } else if let Some(stripped) = value.strip_prefix('+') {
        ("+", stripped)
    } else {
        ("", value.as_str())
    };
    let (integer, suffix) = rest.split_once('.').map_or((rest, ""), |(i, f)| (i, f));
    if integer.len() <= 3 {
        return value;
    }
    let mut grouped = String::new();
    for (i, ch) in integer.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let grouped: String = grouped.chars().rev().collect();
    let mut out = String::with_capacity(sign.len() + grouped.len() + 1 + suffix.len());
    out.push_str(sign);
    out.push_str(&grouped);
    if !suffix.is_empty() {
        out.push('.');
        out.push_str(suffix);
    }
    out
}

fn apply_grouping_strategy(value: String, grouping: BuiltinGrouping) -> String {
    match grouping {
        BuiltinGrouping::Auto | BuiltinGrouping::Never => value,
        BuiltinGrouping::Always => apply_grouping(value),
        BuiltinGrouping::Min2 => apply_grouping_min2(value),
    }
}

fn apply_grouping_min2(value: String) -> String {
    let rest = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value.as_str());
    let integer = rest.split_once('.').map_or(rest, |(integer, _)| integer);
    if integer.len() <= 4 {
        return value;
    }
    apply_grouping(value)
}

fn numeric_operand(value: &Value, catalog: &Catalog) -> Result<f64, FormatError> {
    match value {
        Value::Int(v) => exact_i64_to_f64(*v),
        Value::Float(v) => Ok(*v),
        Value::Number(number) => number.text().parse::<f64>().map_err(|_| bad_operand()),
        _ => value_text(catalog, value)
            .and_then(parse_number_literal)
            .ok_or_else(bad_operand),
    }
}

fn format_percent(
    value: &Value,
    catalog: &Catalog,
    options: &EffectiveOptions<'_>,
) -> Result<String, FormatError> {
    if let Value::Number(number) = value {
        if let NumberValue::NonFinite(value) = number.value {
            return Ok(format_signed_string(
                parse_sign_display(options),
                format!("{}%", value),
            ));
        }
        let rendered = multiply_decimal_by_100(&number.text())?;
        let minimum = parse_minimum_fraction_digits(options)?;
        let rendered = if let Some(minimum) = minimum {
            format_int_or_decimal_with_min_fraction_digits(rendered, minimum)
        } else {
            rendered
        };
        return Ok(format!(
            "{}%",
            format_signed_string(parse_sign_display(options), rendered)
        ));
    }
    let mut number = numeric_operand(value, catalog)? * 100.0;
    if number == -0.0 {
        number = 0.0;
    }
    let digits = parse_minimum_fraction_digits(options)?;
    let rendered = if let Some(min) = digits {
        format!("{number:.min$}")
    } else {
        number.to_string()
    };
    Ok(format!("{rendered}%"))
}

fn multiply_decimal_by_100(value: &str) -> Result<String, FormatError> {
    let (negative, value) = if let Some(value) = value.strip_prefix('-') {
        (true, value)
    } else {
        (false, value.strip_prefix('+').unwrap_or(value))
    };
    let (integer, fraction) = value.split_once('.').map_or((value, ""), |parts| parts);
    if integer.is_empty()
        || !integer.chars().all(|ch| ch.is_ascii_digit())
        || !fraction.chars().all(|ch| ch.is_ascii_digit())
    {
        return Err(bad_operand());
    }
    let digits = format!("{integer}{fraction}");
    let decimal_pos = integer.len().checked_add(2).ok_or_else(bad_operand)?;
    let mut out = if decimal_pos >= digits.len() {
        format!("{digits}{}", "0".repeat(decimal_pos - digits.len()))
    } else {
        let split = decimal_pos;
        format!("{}.{digits}", &digits[..split])
    };
    while out.contains('.') && out.ends_with('0') {
        out.pop();
    }
    if out.ends_with('.') {
        out.pop();
    }
    let integer_end = out.find('.').unwrap_or(out.len());
    let leading = out[..integer_end]
        .bytes()
        .take_while(|digit| *digit == b'0')
        .count();
    if leading >= integer_end {
        out.replace_range(..integer_end, "0");
    } else if leading > 0 {
        out.replace_range(..leading, "");
    }
    if negative && out != "0" {
        out.insert(0, '-');
    }
    Ok(out)
}

fn format_currency(
    value: &Value,
    catalog: &Catalog,
    options: &EffectiveOptions<'_>,
) -> Result<String, FormatError> {
    let Some(currency) = options.get(BuiltinOptionKey::Currency) else {
        if let Some(raw) = value_text(catalog, value)
            && looks_like_currency_literal(raw)
        {
            return Ok(raw.to_string());
        }
        return Err(bad_operand());
    };
    if let Value::Int(v) = value {
        return Ok(format!("{} {v}", currency));
    }
    let number = numeric_operand(value, catalog)?;
    Ok(format!("{} {number}", currency))
}

fn looks_like_currency_literal(value: &str) -> bool {
    let mut parts = value.splitn(2, ' ');
    let Some(code) = parts.next() else {
        return false;
    };
    let Some(number) = parts.next() else {
        return false;
    };
    if code.len() != 3 || !code.chars().all(|ch| ch.is_ascii_uppercase()) {
        return false;
    }
    parse_number_literal(number).is_some()
}

fn resolve_offset(
    value: &Value,
    catalog: &Catalog,
    options: &EffectiveOptions<'_>,
    cardinal_rules: &PluralRules,
    ordinal_rules: &PluralRules,
) -> Result<ResolvedNumber, FormatError> {
    let (mut number, inherited_format, selection, has_explicit_select) = match value {
        Value::Number(number) => (
            number.value.clone(),
            number.format,
            number.selection,
            number.has_explicit_select,
        ),
        _ => (
            parse_number_value(value, catalog)?,
            NumberFormatOptions::DEFAULT,
            NumberSelection::Plural,
            false,
        ),
    };
    let add = options
        .get(BuiltinOptionKey::Add)
        .map(|raw| parse_integer_adjustment(&raw).ok_or_else(bad_option))
        .transpose()?;
    let subtract = options
        .get(BuiltinOptionKey::Subtract)
        .map(|raw| parse_integer_adjustment(&raw).ok_or_else(bad_option))
        .transpose()?;

    if add.is_none() && subtract.is_none() {
        return Err(bad_option());
    }
    if add.is_some() && subtract.is_some() {
        return Err(bad_option());
    }
    let adjustment = add
        .map(|value| (value, false))
        .or_else(|| subtract.map(|value| (value, true)));
    if let Some((adjustment, subtract)) = adjustment {
        number = checked_offset(number, adjustment, subtract)?;
    }
    let format = resolve_number_format_options(inherited_format, options, false)?;
    let mut resolved = ResolvedNumber::new(number, format, selection, has_explicit_select);
    if matches!(
        selection,
        NumberSelection::Plural | NumberSelection::Ordinal
    ) && !matches!(resolved.value, NumberValue::NonFinite(_))
    {
        let rules = if selection == NumberSelection::Ordinal {
            ordinal_rules
        } else {
            cardinal_rules
        };
        resolved.set_selection_category(Some(resolved_plural_category(&resolved, rules)?));
    }
    Ok(resolved)
}

fn number_text(value: &NumberValue) -> String {
    match value {
        NumberValue::Integer(value) => value.to_string(),
        NumberValue::Decimal(value) => value.to_string(),
        NumberValue::NonFinite(value) => value.to_string(),
    }
}

fn parse_integer_adjustment(value: &str) -> Option<i64> {
    parse_number_literal(value)?;
    value.parse::<i64>().ok()
}

fn parse_number_text(value: &str) -> Result<NumberValue, FormatError> {
    if let Ok(value) = value.parse::<i64>() {
        return Ok(NumberValue::Integer(value));
    }
    Decimal::from_str(value)
        .map(|value| NumberValue::Decimal(trim_decimal_end(value)))
        .map_err(|_| bad_operand())
}

fn trim_decimal_end(mut value: Decimal) -> Decimal {
    value.absolute = value.absolute.trimmed_end();
    value
}

fn checked_offset(
    number: NumberValue,
    adjustment: i64,
    subtract: bool,
) -> Result<NumberValue, FormatError> {
    if let NumberValue::NonFinite(value) = number {
        return Ok(NumberValue::NonFinite(value));
    }
    let text = number_text(&number);
    let unsigned = text.trim_start_matches(['-', '+']);
    let scale = unsigned
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    if scale > 38 {
        return Err(unsupported_operation(
            UnsupportedOperation::NumericMagnitude,
        ));
    }
    let digits = unsigned.replace('.', "");
    let magnitude = digits
        .parse::<i128>()
        .map_err(|_| unsupported_operation(UnsupportedOperation::NumericMagnitude))?;
    let signed = if text.starts_with('-') {
        -magnitude
    } else {
        magnitude
    };
    let factor = 10_i128
        .checked_pow(
            u32::try_from(scale)
                .map_err(|_| unsupported_operation(UnsupportedOperation::NumericMagnitude))?,
        )
        .ok_or_else(|| unsupported_operation(UnsupportedOperation::NumericMagnitude))?;
    let adjustment = i128::from(adjustment)
        .checked_mul(factor)
        .ok_or_else(|| unsupported_operation(UnsupportedOperation::NumericMagnitude))?;
    let result = if subtract {
        signed
            .checked_sub(adjustment)
            .ok_or_else(|| unsupported_operation(UnsupportedOperation::NumericMagnitude))?
    } else {
        signed
            .checked_add(adjustment)
            .ok_or_else(|| unsupported_operation(UnsupportedOperation::NumericMagnitude))?
    };
    let magnitude = result.unsigned_abs();
    let mut rendered = magnitude.to_string();
    if scale != 0 {
        if rendered.len() <= scale {
            rendered = format!("{}{}", "0".repeat(scale + 1 - rendered.len()), rendered);
        }
        let split = rendered.len() - scale;
        rendered.insert(split, '.');
        while rendered.ends_with('0') {
            rendered.pop();
        }
        if rendered.ends_with('.') {
            rendered.pop();
        }
    }
    if result < 0 {
        rendered.insert(0, '-');
    }
    parse_number_text(&rendered)
}

fn format_test_select(
    value: &Value,
    catalog: &Catalog,
    options: &EffectiveOptions<'_>,
) -> Result<String, FormatError> {
    if options
        .get(BuiltinOptionKey::Fails)
        .is_some_and(|it| it == "select")
    {
        return Err(implementation_failure(ImplementationFailure::TestSelect));
    }
    // Preserve only an explicitly resolved test-select value. Raw strings
    // still follow the ordinary numeric conversion path below.
    if options.get(BuiltinOptionKey::DecimalPlaces).is_none()
        && let Value::ResolvedSelect(value) = value
    {
        return Ok(value.text().to_string());
    }
    let number = numeric_operand(value, catalog)?;
    if let Some(raw) = options.get(BuiltinOptionKey::DecimalPlaces) {
        let dp = raw.parse::<usize>().map_err(|_| bad_option())?;
        if dp > 3 {
            return Err(bad_option());
        }
        Ok(format!("{number:.dp$}"))
    } else {
        Ok(number.to_string())
    }
}

fn format_test_function(
    value: &Value,
    catalog: &Catalog,
    options: &EffectiveOptions<'_>,
) -> Result<Value, FormatError> {
    if options
        .get(BuiltinOptionKey::Fails)
        .is_some_and(|it| it == "format")
    {
        if numeric_operand(value, catalog).is_ok() {
            return Err(bad_option());
        }
        return Err(bad_operand());
    }
    Ok(Value::Str(plain_text(catalog, value).into_owned()))
}

#[derive(Debug)]
struct EffectiveOptions<'a> {
    // `base` carries static per-function options compiled from function strings.
    // Keys are normalized up front to avoid repeated string comparisons in hot paths.
    base: &'a [Option<String>; BUILTIN_OPTION_KEY_COUNT],
    runtime: [Option<&'a Value>; BUILTIN_OPTION_KEY_COUNT],
    runtime_options: FunctionOptions<'a>,
    has_invalid_runtime_key: bool,
    catalog: &'a Catalog,
    option_keys_by_str_id: &'a BTreeMap<u32, BuiltinOptionKey>,
}

impl<'a> EffectiveOptions<'a> {
    fn has_runtime(&self, key: BuiltinOptionKey) -> bool {
        self.option_keys_by_str_id
            .iter()
            .any(|(id, candidate)| *candidate == key && self.runtime_options.was_dynamic(*id))
    }

    fn new(
        base: &'a [Option<String>; BUILTIN_OPTION_KEY_COUNT],
        runtime: FunctionOptions<'a>,
        catalog: &'a Catalog,
        option_keys_by_str_id: &'a BTreeMap<u32, BuiltinOptionKey>,
    ) -> Self {
        let mut runtime_values = array::from_fn(|_| None);
        let mut has_invalid_runtime_key = false;
        for (key_id, value) in runtime.iter() {
            if catalog.pool_string_opt(key_id).is_none() {
                has_invalid_runtime_key = true;
                continue;
            }
            let Some(runtime_key) = option_keys_by_str_id.get(&key_id) else {
                continue;
            };
            runtime_values[runtime_key.index()] = Some(value);
        }
        Self {
            base,
            runtime: runtime_values,
            runtime_options: runtime,
            has_invalid_runtime_key,
            catalog,
            option_keys_by_str_id,
        }
    }

    fn validate_keys(&self) -> Result<(), FormatError> {
        if self.has_invalid_runtime_key {
            return Err(bad_option());
        }
        Ok(())
    }

    fn get(&self, key: BuiltinOptionKey) -> Option<Cow<'a, str>> {
        if let Some(value) = self.runtime[key.index()] {
            return Some(match value {
                Value::Str(value) => Cow::Borrowed(value.as_str()),
                Value::StrRef(id) => {
                    let value = self.catalog.pool_string_opt(*id)?;
                    Cow::Borrowed(value)
                }
                Value::LitRef { off, len } => {
                    let value = self.catalog.literal_opt(*off, *len)?;
                    Cow::Borrowed(value)
                }
                _ => Cow::Owned(plain_text(self.catalog, value).into_owned()),
            });
        }
        self.base[key.index()].as_deref().map(Cow::Borrowed)
    }
}

/// TR35 §15: dateStyle/timeStyle and field options (year, month, etc.) are
/// mutually exclusive for :datetime. Supplying both is a bad-option error.
fn validate_datetime_style_field_exclusivity(
    options: &EffectiveOptions<'_>,
) -> Result<(), FormatError> {
    let has_style = options.get(BuiltinOptionKey::DateStyle).is_some()
        || options.get(BuiltinOptionKey::TimeStyle).is_some();
    let has_field = options.get(BuiltinOptionKey::Year).is_some()
        || options.get(BuiltinOptionKey::Month).is_some()
        || options.get(BuiltinOptionKey::Day).is_some()
        || options.get(BuiltinOptionKey::Hour).is_some()
        || options.get(BuiltinOptionKey::Minute).is_some()
        || options.get(BuiltinOptionKey::Second).is_some()
        || options.get(BuiltinOptionKey::Weekday).is_some()
        || options.get(BuiltinOptionKey::Era).is_some()
        || options.get(BuiltinOptionKey::TimeZoneName).is_some();
    if has_style && has_field {
        return Err(bad_option());
    }
    Ok(())
}

fn validate_date_operand<'a>(
    value: &'a Value,
    catalog: &'a Catalog,
) -> Result<&'a str, FormatError> {
    let text = value_text(catalog, value).ok_or_else(bad_operand)?;
    if text.len() >= 10 && text.chars().nth(4) == Some('-') && text.chars().nth(7) == Some('-') {
        Ok(text)
    } else {
        Err(bad_operand())
    }
}

fn validate_time_operand(value: &Value, catalog: &Catalog) -> Result<String, FormatError> {
    let text = value_text(catalog, value).ok_or_else(bad_operand)?;
    if text.contains('T') && text.matches(':').count() >= 1 {
        Ok(text.to_string())
    } else if text.len() >= 10
        && text.chars().nth(4) == Some('-')
        && text.chars().nth(7) == Some('-')
    {
        // Date-only input: default time component to 00:00:00
        Ok(format!("{text}T00:00:00"))
    } else {
        Err(bad_operand())
    }
}

fn validate_datetime_operand<'a>(
    value: &'a Value,
    catalog: &'a Catalog,
) -> Result<&'a str, FormatError> {
    let text = value_text(catalog, value).ok_or_else(bad_operand)?;
    if text.contains('T') && text.chars().nth(4) == Some('-') {
        Ok(text)
    } else {
        Err(bad_operand())
    }
}

/// Parse an ISO 8601 date/datetime string into `(Date<Iso>, Time)`.
/// Accepts "YYYY-MM-DD", "YYYY-MM-DDThh:mm:ss", and with timezone offsets.
fn parse_iso_datetime(text: &str) -> Result<(Date<icu_calendar::Iso>, Time), FormatError> {
    let bad = bad_operand;

    // Split into date and optional time parts at 'T'.
    let (date_str, time_str) = if let Some(pos) = text.find('T') {
        (&text[..pos], Some(&text[pos + 1..]))
    } else {
        // Strip trailing 'Z' from date-only strings (shouldn't normally occur).
        (text.trim_end_matches('Z'), None)
    };

    // Parse date: YYYY-MM-DD
    let date_parts: Vec<&str> = date_str.split('-').collect();
    if date_parts.len() < 3 {
        return Err(bad());
    }
    let year: i32 = date_parts[0].parse().map_err(|_| bad())?;
    let month: u8 = date_parts[1].parse().map_err(|_| bad())?;
    let day: u8 = date_parts[2].parse().map_err(|_| bad())?;

    let date = Date::try_new_iso(year, month, day).map_err(|_| bad())?;

    // Parse time: hh:mm:ss (default to midnight if absent).
    let time = if let Some(ts) = time_str {
        // Strip timezone offset: 'Z', '+HH:MM', or '-HH:MM' at end.
        let ts = ts.trim_end_matches('Z');
        // Find last '+' or '-' that looks like a timezone offset (not at position 0).
        let ts = if let Some(offset_pos) = ts.rfind(['+', '-']) {
            if offset_pos > 0 {
                &ts[..offset_pos]
            } else {
                ts
            }
        } else {
            ts
        };
        let time_parts: Vec<&str> = ts.split(':').collect();
        let hour: u8 = time_parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        let minute: u8 = time_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let (second, nanosecond) = time_parts
            .get(2)
            .map_or(Ok((0, 0)), |part| parse_seconds_component(part))?;
        Time::try_new(hour, minute, second, nanosecond).map_err(|_| bad())?
    } else {
        Time::try_new(0, 0, 0, 0).map_err(|_| bad())?
    };

    Ok((date, time))
}

fn parse_seconds_component(value: &str) -> Result<(u8, u32), FormatError> {
    let bad = bad_operand;
    let (seconds, fraction) = value
        .split_once('.')
        .map_or((value, ""), |(sec, frac)| (sec, frac));
    let second = seconds.parse::<u8>().map_err(|_| bad())?;
    if fraction.is_empty() {
        return Ok((second, 0));
    }
    if !fraction.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(bad());
    }

    let mut digits = fraction.as_bytes().to_vec();
    digits.truncate(9);
    while digits.len() < 9 {
        digits.push(b'0');
    }
    let nanosecond = core::str::from_utf8(&digits)
        .ok()
        .and_then(|raw| raw.parse::<u32>().ok())
        .ok_or_else(bad)?;
    Ok((second, nanosecond))
}

/// Resolve the date style from options (`dateStyle` or `style`), defaulting to `medium`.
fn resolve_date_style(options: &EffectiveOptions<'_>) -> Length {
    let style_str = options
        .get(BuiltinOptionKey::DateStyle)
        .or_else(|| options.get(BuiltinOptionKey::Style));
    match style_str.as_deref() {
        Some("short") => Length::Short,
        Some("long") | Some("full") => Length::Long,
        _ => Length::Medium,
    }
}

/// Resolve the time style from options (`timeStyle` or `style`), defaulting to `short`.
fn resolve_time_style(options: &EffectiveOptions<'_>) -> Length {
    let style_str = options
        .get(BuiltinOptionKey::TimeStyle)
        .or_else(|| options.get(BuiltinOptionKey::Style));
    match style_str.as_deref() {
        Some("medium") => Length::Medium,
        Some("long") | Some("full") => Length::Long,
        _ => Length::Short,
    }
}

impl DateFormatterCache {
    fn slot_mut(&mut self, style: Length) -> &mut Option<DateTimeFormatter<fieldsets::YMD>> {
        match style_bucket(style) {
            StyleBucket::Short => &mut self.short,
            StyleBucket::Medium => &mut self.medium,
            StyleBucket::Long => &mut self.long,
        }
    }
}

impl TimeFormatterCache {
    fn slot_mut(&mut self, style: Length) -> &mut Option<NoCalendarFormatter<fieldsets::T>> {
        match style_bucket(style) {
            StyleBucket::Short => &mut self.short,
            StyleBucket::Medium => &mut self.medium,
            StyleBucket::Long => &mut self.long,
        }
    }
}

impl DateTimeFormatterCache {
    fn slot_mut(
        &mut self,
        date_style: Length,
        time_style: Length,
    ) -> &mut Option<DateTimeFormatter<fieldsets::YMDT>> {
        let time_slots = match style_bucket(date_style) {
            StyleBucket::Short => &mut self.short,
            StyleBucket::Medium => &mut self.medium,
            StyleBucket::Long => &mut self.long,
        };
        match style_bucket(time_style) {
            StyleBucket::Short => &mut time_slots.short,
            StyleBucket::Medium => &mut time_slots.medium,
            StyleBucket::Long => &mut time_slots.long,
        }
    }
}

fn format_icu_date_cached(
    locale: &Locale,
    cache: &mut DateFormatterCache,
    date: Date<icu_calendar::Iso>,
    style: Length,
) -> Result<String, FormatError> {
    let slot = cache.slot_mut(style);
    if slot.is_none() {
        *slot = Some(
            DateTimeFormatter::try_new(locale.clone().into(), date_field_set(style)).map_err(
                |_| unsupported_operation(UnsupportedOperation::DateFormattingForLocale),
            )?,
        );
    }
    let formatter = slot.as_ref().expect("date formatter initialized");
    Ok(formatter.format(&date).to_string())
}

fn format_icu_time_cached(
    locale: &Locale,
    cache: &mut TimeFormatterCache,
    time: Time,
    style: Length,
) -> Result<String, FormatError> {
    let slot = cache.slot_mut(style);
    if slot.is_none() {
        *slot = Some(
            NoCalendarFormatter::try_new(locale.clone().into(), time_field_set(style)).map_err(
                |_| unsupported_operation(UnsupportedOperation::TimeFormattingForLocale),
            )?,
        );
    }
    let formatter = slot.as_ref().expect("time formatter initialized");
    Ok(formatter.format(&time).to_string())
}

fn format_icu_datetime_cached(
    locale: &Locale,
    cache: &mut DateTimeFormatterCache,
    date: Date<icu_calendar::Iso>,
    time: Time,
    date_style: Length,
    time_style: Length,
) -> Result<String, FormatError> {
    let slot = cache.slot_mut(date_style, time_style);
    if slot.is_none() {
        *slot = Some(
            DateTimeFormatter::try_new(
                locale.clone().into(),
                datetime_field_set(date_style, time_style),
            )
            .map_err(|_| {
                unsupported_operation(UnsupportedOperation::DateTimeFormattingForLocale)
            })?,
        );
    }
    let formatter = slot.as_ref().expect("datetime formatter initialized");
    let dt = DateTime { date, time };
    Ok(formatter.format(&dt).to_string())
}

#[derive(Clone, Copy)]
enum StyleBucket {
    Short,
    Medium,
    Long,
}

fn style_bucket(style: Length) -> StyleBucket {
    match style {
        Length::Short => StyleBucket::Short,
        Length::Long => StyleBucket::Long,
        _ => StyleBucket::Medium,
    }
}

fn date_field_set(style: Length) -> fieldsets::YMD {
    match style_bucket(style) {
        StyleBucket::Short => fieldsets::YMD::short(),
        StyleBucket::Medium => fieldsets::YMD::medium(),
        StyleBucket::Long => fieldsets::YMD::long(),
    }
}

fn time_field_set(style: Length) -> fieldsets::T {
    match style_bucket(style) {
        StyleBucket::Short => fieldsets::T::short(),
        StyleBucket::Medium => fieldsets::T::medium(),
        StyleBucket::Long => fieldsets::T::long(),
    }
}

fn datetime_field_set(date_style: Length, time_style: Length) -> fieldsets::YMDT {
    let date = date_field_set(date_style);
    match style_bucket(time_style) {
        StyleBucket::Short => date.with_time_hm(),
        StyleBucket::Medium | StyleBucket::Long => date.with_time_hms(),
    }
}

fn format_number_default_locale(value: f64, locale: &Locale) -> String {
    let mut rendered = value.to_string();
    let locale_tag = locale.to_string();
    if locale_tag.starts_with("fr") {
        rendered = rendered.replace('.', ",");
    }
    rendered
}

fn format_float_with_min_fraction_digits(value: f64, min: usize) -> String {
    // Fast path: integral values can be rendered once with zero fractional
    // digits and then padded directly. This avoids an extra `to_string()`
    // pass before the precision-formatting path for non-integral values.
    if value.is_finite() && value % 1.0 == 0.0 {
        let raw = value.to_string();
        if min == 0 {
            return raw;
        }
        let mut out = raw;
        out.push('.');
        for _ in 0..min {
            out.push('0');
        }
        return out;
    }
    format!("{value:.min$}")
}

fn format_int_with_min_fraction_digits(value: i64, min: usize) -> String {
    if min == 0 {
        return value.to_string();
    }
    let mut rendered = value.to_string();
    rendered.push('.');
    for _ in 0..min {
        rendered.push('0');
    }
    rendered
}

fn exact_i64_to_f64(value: i64) -> Result<f64, FormatError> {
    // Math-heavy builtin paths still use f64 internally. Reject integers that
    // would lose precision instead of silently rounding through the cast.
    if value.unsigned_abs() <= MAX_EXACT_I64_IN_F64 as u64 {
        Ok(value as f64)
    } else {
        Err(bad_operand())
    }
}

fn apply_bidi_dir(value: Cow<'_, str>, dir: Option<&str>) -> String {
    if is_bidi_isolated(value.as_ref()) {
        return value.into_owned();
    }
    let isolate_open = match dir.unwrap_or("auto") {
        "ltr" => '\u{2066}',
        "rtl" => '\u{2067}',
        _ => '\u{2068}',
    };
    format!("{isolate_open}{value}\u{2069}")
}

fn is_bidi_isolated(value: &str) -> bool {
    value.ends_with('\u{2069}')
        && value
            .chars()
            .next()
            .is_some_and(|it| matches!(it, '\u{2066}' | '\u{2067}' | '\u{2068}'))
}

fn truncate_decimal_text(value: &str) -> Option<String> {
    if value.contains('e') || value.contains('E') {
        return None;
    }
    let (sign, rest) = if let Some(stripped) = value.strip_prefix('-') {
        ("-", stripped)
    } else {
        ("", value)
    };
    let integer = rest.split('.').next()?;
    if integer.is_empty() || !integer.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(format!("{sign}{integer}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        catalog::{FuncEntry, MessageEntry, build_catalog, build_catalog_with_funcs},
        vm,
    };
    use alloc::{boxed::Box, collections::BTreeSet};
    use core::ops::{Deref, DerefMut};

    struct TestBuiltinHost {
        catalog: &'static Catalog,
        index: BuiltinHostCatalogIndex,
        host: BuiltinHost,
    }

    impl TestBuiltinHost {
        fn call(
            &mut self,
            fn_id: u16,
            args: &[Value],
            opts: FunctionOptions<'_>,
        ) -> Result<Value, HostCallError> {
            Host::call(
                &mut self.host,
                self.catalog,
                &self.index,
                fn_id,
                args,
                opts,
                &mut |_| {},
            )
        }

        fn call_select(
            &mut self,
            fn_id: u16,
            args: &[Value],
            opts: FunctionOptions<'_>,
        ) -> Result<Value, HostCallError> {
            Host::call_select(
                &mut self.host,
                self.catalog,
                &self.index,
                fn_id,
                args,
                opts,
                &mut |_| {},
            )
        }
    }

    impl Deref for TestBuiltinHost {
        type Target = BuiltinHost;

        fn deref(&self) -> &Self::Target {
            &self.host
        }
    }

    impl DerefMut for TestBuiltinHost {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.host
        }
    }

    fn decode_spec_option_value(value: &str) -> String {
        let Some(inner) = value.strip_prefix('|').and_then(|it| it.strip_suffix('|')) else {
            return value.to_string();
        };
        let mut out = String::new();
        let mut escaped = false;
        for ch in inner.chars() {
            if escaped {
                out.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else {
                out.push(ch);
            }
        }
        out
    }

    /// Build a `BuiltinHost` from function spec strings (e.g. `"number minimumFractionDigits=2"`)
    /// and extra string pool entries. The first string is always the message name.
    /// Function spec strings are parsed into FUNC chunk entries; remaining strings
    /// (non-function) are added to the string pool for runtime option resolution.
    fn builtin_host_with_funcs(func_specs: &[&str], extra_strings: &[&str]) -> TestBuiltinHost {
        builtin_host_with_catalog_parts(func_specs, extra_strings, "")
    }

    fn builtin_host_with_catalog_parts(
        func_specs: &[&str],
        extra_strings: &[&str],
        literals: &str,
    ) -> TestBuiltinHost {
        // Collect all unique strings needed for the string pool.
        let mut pool = BTreeSet::new();
        pool.insert(String::from("msg"));
        for extra in extra_strings {
            pool.insert((*extra).to_string());
        }

        // Parse function specs and collect their component strings.
        let mut parsed_funcs = Vec::new();
        for spec in func_specs {
            let mut parts = spec.split_whitespace();
            let name = parts.next().unwrap();
            pool.insert(name.to_string());
            let mut opts = Vec::new();
            for token in parts {
                if let Some((key, value)) = token.split_once('=') {
                    pool.insert(key.to_string());
                    let value = decode_spec_option_value(value);
                    pool.insert(value.clone());
                    opts.push((key, value));
                }
            }
            parsed_funcs.push((name, opts));
        }

        let strings: Vec<String> = pool.into_iter().collect();
        let string_map: BTreeMap<&str, u32> = strings
            .iter()
            .enumerate()
            .map(|(i, s)| {
                (
                    s.as_str(),
                    u32::try_from(i).expect("string map index must fit into u32"),
                )
            })
            .collect();

        let func_entries: Vec<FuncEntry> = parsed_funcs
            .iter()
            .map(|(name, opts)| FuncEntry {
                name_str_id: string_map[name],
                static_options: opts
                    .iter()
                    .map(|(k, v)| (string_map[k], string_map[v.as_str()]))
                    .collect(),
            })
            .collect();

        let bytes = build_catalog_with_funcs(
            &strings.iter().map(String::as_str).collect::<Vec<_>>(),
            literals,
            &[MessageEntry {
                name_str_id: string_map["msg"],
                entry_pc: 0,
            }],
            &[vm::Opcode::Halt as u8],
            &func_entries,
        );
        let boxed_catalog = Box::new(Catalog::from_bytes(&bytes).expect("valid catalog"));
        let catalog: &'static Catalog = Box::leak(boxed_catalog);
        let locale = Locale::from_str("en-US").expect("locale");
        let mut host = BuiltinHost::new(&locale).expect("host");
        let index = host.index(catalog).expect("index");
        TestBuiltinHost {
            catalog,
            index,
            host,
        }
    }

    fn builtin_host(func_specs: &[&str]) -> TestBuiltinHost {
        builtin_host_with_funcs(func_specs, &[])
    }

    fn assert_function_error(err: HostCallError, expected: MessageFunctionError) {
        assert_eq!(err, HostCallError::Function(expected));
    }

    fn assert_selector_result(catalog: &Catalog, result: Value, expected: &str) {
        match result {
            Value::StrRef(id) => {
                let value = catalog.string(id).expect("category in pool");
                assert_eq!(value, expected);
            }
            Value::Str(value) => assert_eq!(value, expected),
            other => panic!("unexpected selector result: {other:?}"),
        }
    }

    fn assert_number_rendered(host: &mut TestBuiltinHost, value: Value, expected: &str) {
        assert!(
            matches!(value, Value::Number(_)),
            "expected resolved number"
        );
        let rendered = host
            .host
            .format_default(host.catalog, &host.index, &value)
            .expect("resolved number renders");
        assert_eq!(rendered, expected);
    }

    fn assert_string_resolved(value: Value, expected_text: &str) {
        match value {
            Value::String(value) => assert_eq!(value.text(), expected_text),
            other => panic!("expected resolved string, got {other:?}"),
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "builtin host must only surface function-shaped errors")]
    fn into_host_call_error_rejects_unexpected_runtime_errors_in_debug() {
        let _ = into_host_call_error(FormatError::MissingArg("value".to_string()));
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn into_host_call_error_collapses_unexpected_runtime_errors_in_release() {
        let err = into_host_call_error(FormatError::MissingArg("value".to_string()));
        assert_function_error(
            err,
            MessageFunctionError::Implementation(ImplementationFailure::Host),
        );
    }

    #[test]
    fn builtin_host_maps_number_function() {
        let host = builtin_host(&["number"]);
        assert_eq!(host.index.by_id.len(), 1);
    }

    #[test]
    fn test_select_preserves_only_explicitly_resolved_precision() {
        let mut host = builtin_host(&["test:select decimalPlaces=1", "test:select"]);
        let resolved = host
            .call(0, &[Value::Int(1)], FunctionOptions::new(&[]))
            .expect("resolved selector");
        let Value::ResolvedSelect(value) = &resolved else {
            panic!("test:select must return a resolved selector");
        };
        assert_eq!(value.text(), "1.0");

        let aliased = host
            .call(
                1,
                core::slice::from_ref(&resolved),
                FunctionOptions::new(&[]),
            )
            .expect("aliased selector");
        let Value::ResolvedSelect(value) = &aliased else {
            panic!("test:select must return a resolved selector");
        };
        assert_eq!(value.text(), "1.0");

        let raw = host
            .call(
                1,
                &[Value::Str("1.0".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("raw selector");
        let Value::ResolvedSelect(value) = &raw else {
            panic!("test:select must return a resolved selector");
        };
        assert_eq!(value.text(), "1");
    }

    #[test]
    fn resolved_number_exposes_exact_text_without_formatting_options() {
        let mut host = builtin_host(&["number minimumFractionDigits=2 useGrouping=always"]);
        let out = host
            .call(
                0,
                &[Value::Int(9_007_199_254_740_993)],
                FunctionOptions::new(&[]),
            )
            .expect("resolved");
        let Value::Number(number) = out else {
            panic!("number function must return a resolved number");
        };
        assert_eq!(number.text(), "9007199254740993");
        assert_eq!(
            host.host
                .format_default(host.catalog, &host.index, &Value::Number(number)),
            Some("9,007,199,254,740,993.00".to_string())
        );
    }

    #[test]
    fn dynamic_select_reports_error_and_marks_resolved_number_unselectable() {
        let mut host = builtin_host_with_funcs(&["number"], &["select", "exact"]);
        let select_id = host.catalog.string_id("select").expect("select in pool");
        let mut errors = Vec::new();
        let out = Host::call(
            &mut host.host,
            host.catalog,
            &host.index,
            0,
            &[Value::Int(1)],
            FunctionOptions::new(&[(select_id, Value::Str("exact".to_string()))]),
            &mut |error| errors.push(error),
        )
        .expect("formatting remains available");
        assert_eq!(errors, vec![MessageFunctionError::BadOption]);
        let Value::Number(number) = out else {
            panic!("number function must return a resolved number");
        };
        assert_eq!(number.selection, NumberSelection::Invalid);
    }

    #[test]
    fn static_select_keeps_selection_provenance_on_resolved_number() {
        let mut host = builtin_host(&["number select=plural"]);
        let out = host
            .call(0, &[Value::Int(1)], FunctionOptions::new(&[]))
            .expect("formatting remains available");
        let Value::Number(number) = out else {
            panic!("number function must return a resolved number");
        };
        assert_eq!(number.selection, NumberSelection::Plural);
    }

    #[test]
    fn inherited_select_reports_error_on_reannotation() {
        let mut host = builtin_host(&["number select=plural", "number"]);
        let first = host
            .call(0, &[Value::Int(1)], FunctionOptions::new(&[]))
            .expect("first annotation");
        let mut errors = Vec::new();
        let second = Host::call(
            &mut host.host,
            host.catalog,
            &host.index,
            1,
            &[first],
            FunctionOptions::new(&[]),
            &mut |error| errors.push(error),
        )
        .expect("formatting remains available");
        assert_eq!(errors, vec![MessageFunctionError::BadOption]);
        let Value::Number(number) = second else {
            panic!("number function must return a resolved number");
        };
        assert_eq!(number.selection, NumberSelection::Invalid);
    }

    #[test]
    fn call_select_does_not_recover_invalid_stored_selection() {
        let mut host =
            builtin_host_with_funcs(&["number", "number select=plural"], &["select", "exact"]);
        let select_id = host.catalog.string_id("select").expect("select in pool");
        let mut errors = Vec::new();
        let stored = Host::call(
            &mut host.host,
            host.catalog,
            &host.index,
            0,
            &[Value::Int(1)],
            FunctionOptions::new(&[(select_id, Value::Str("exact".to_string()))]),
            &mut |error| errors.push(error),
        )
        .expect("formatting remains available");
        assert_eq!(errors, vec![MessageFunctionError::BadOption]);
        let selected = host
            .call_select(1, &[stored], FunctionOptions::new(&[]))
            .expect("invalid selection uses default");
        assert_eq!(selected, Value::Null);
    }

    #[test]
    fn call_select_uses_valid_mode_from_stored_number() {
        let mut host = builtin_host(&["number select=plural", "number"]);
        let stored = host
            .call(0, &[Value::Int(1)], FunctionOptions::new(&[]))
            .expect("stored number");
        let selected = host
            .call_select(1, &[stored], FunctionOptions::new(&[]))
            .expect("stored selection");
        assert_selector_result(host.catalog, selected, "one");
    }

    #[test]
    fn reannotation_validates_inherited_fraction_options() {
        let mut host = builtin_host(&[
            "number minimumFractionDigits=3",
            "number maximumFractionDigits=2",
        ]);
        let resolved = host
            .call(0, &[Value::Float(4.2)], FunctionOptions::new(&[]))
            .expect("first annotation");
        let err = host
            .call(1, &[resolved], FunctionOptions::new(&[]))
            .expect_err("merged options must be validated");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_applies_number_minimum_fraction_digits() {
        let mut host = builtin_host(&["number minimumFractionDigits=2"]);
        let out = host
            .call(
                0,
                &[Value::Str("4.2".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        assert_number_rendered(&mut host, out, "4.20");
    }

    #[test]
    fn builtin_host_rejects_bad_minimum_fraction_digits() {
        let mut host = builtin_host(&["number minimumFractionDigits=foo"]);
        let err = host
            .call(
                0,
                &[Value::Str("4.2".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_rejects_invalid_sign_display_literal() {
        let mut host = builtin_host(&["number signDisplay=bogus"]);
        let err = host
            .call(0, &[Value::Int(5)], FunctionOptions::new(&[]))
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_rejects_invalid_use_grouping_literal() {
        let mut host = builtin_host(&["number useGrouping=bogus"]);
        let err = host
            .call(0, &[Value::Int(5)], FunctionOptions::new(&[]))
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_formats_integral_float_minimum_fraction_digits() {
        let mut host = builtin_host(&["number minimumFractionDigits=3"]);
        let out = host
            .call(0, &[Value::Float(42.0)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, out, "42.000");
    }

    #[test]
    fn builtin_host_keeps_exact_integral_float_payloads_integer() {
        let mut host = builtin_host(&["number"]);
        for (input, expected) in [
            (MAX_EXACT_I64_IN_F64 as f64, MAX_EXACT_I64_IN_F64),
            (-(MAX_EXACT_I64_IN_F64 as f64), -MAX_EXACT_I64_IN_F64),
        ] {
            let out = host
                .call(0, &[Value::Float(input)], FunctionOptions::new(&[]))
                .expect("formatted");
            let Value::Number(number) = out else {
                panic!("expected resolved number");
            };
            assert_eq!(number.value, NumberValue::Integer(expected));
        }
    }

    #[test]
    fn builtin_host_keeps_fractional_and_large_integral_float_decimals() {
        let mut host = builtin_host(&["number"]);
        let fractional = host
            .call(0, &[Value::Float(4.25)], FunctionOptions::new(&[]))
            .expect("formatted");
        let Value::Number(number) = fractional else {
            panic!("expected resolved number");
        };
        assert!(matches!(number.value, NumberValue::Decimal(_)));

        let large = host
            .call(0, &[Value::Float(1e23)], FunctionOptions::new(&[]))
            .expect("formatted");
        let Value::Number(number) = large else {
            panic!("expected resolved number");
        };
        assert!(matches!(number.value, NumberValue::Decimal(_)));
        assert_eq!(number_text(&number.value), "100000000000000000000000");
    }

    #[test]
    fn builtin_host_formats_large_integer_minimum_fraction_digits_exactly() {
        let mut host = builtin_host(&["number minimumFractionDigits=2"]);
        let out = host
            .call(0, &[Value::Int(i64::MAX)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, out, &format!("{}.00", i64::MAX));
    }

    #[test]
    fn builtin_host_applies_maximum_fraction_digits_rounding() {
        let mut host = builtin_host(&["number maximumFractionDigits=2"]);
        let out = host
            .call(0, &[Value::Float(4.256)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, out, "4.26");
    }

    #[test]
    fn builtin_host_rejects_invalid_min_then_max_fraction_digit_range() {
        let mut host = builtin_host(&["number minimumFractionDigits=4 maximumFractionDigits=2"]);
        let err = host
            .call(0, &[Value::Float(4.2)], FunctionOptions::new(&[]))
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_rejects_out_of_range_digit_options() {
        let mut host = builtin_host(&["number minimumFractionDigits=21"]);
        let err = host
            .call(0, &[Value::Float(4.2)], FunctionOptions::new(&[]))
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);

        let mut host = builtin_host(&["number minimumIntegerDigits=22"]);
        let err = host
            .call(0, &[Value::Int(42)], FunctionOptions::new(&[]))
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_rejects_minimum_fraction_digits_greater_than_maximum() {
        let mut host = builtin_host(&["number minimumFractionDigits=3 maximumFractionDigits=2"]);
        let err = host
            .call(0, &[Value::Float(4.2)], FunctionOptions::new(&[]))
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_preserves_negative_zero_fraction_formatting() {
        let mut host = builtin_host(&["number minimumFractionDigits=2"]);
        let out = host
            .call(0, &[Value::Float(-0.0)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, out, "-0.00");
    }

    #[test]
    fn builtin_host_preserves_negative_zero_string_sign() {
        let mut host = builtin_host(&["number"]);
        let out = host
            .call(
                0,
                &[Value::Str("-0".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        assert_number_rendered(&mut host, out, "-0");
    }

    #[test]
    fn builtin_host_preserves_nonfinite_number_rendering() {
        let mut host = builtin_host(&["number"]);
        let infinity = host
            .call(0, &[Value::Float(f64::INFINITY)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, infinity, "inf");

        let mut offset = builtin_host(&["offset add=1"]);
        let infinity = offset
            .call(0, &[Value::Float(f64::INFINITY)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut offset, infinity, "inf");
    }

    #[test]
    fn runtime_option_overrides_static_option() {
        let mut host = builtin_host_with_funcs(
            &["number minimumFractionDigits=2"],
            &["minimumFractionDigits", "3"],
        );
        let mfd_str_id = host
            .catalog
            .string_id("minimumFractionDigits")
            .expect("minimumFractionDigits in pool");
        let out = host
            .call(
                0,
                &[Value::Float(4.2)],
                FunctionOptions::new(&[(mfd_str_id, Value::Str("3".to_string()))]),
            )
            .expect("formatted");
        assert_number_rendered(&mut host, out, "4.200");
    }

    #[test]
    fn unknown_runtime_option_key_is_ignored() {
        let mut host =
            builtin_host_with_funcs(&["number minimumFractionDigits=2"], &["mystery", "7"]);
        let mystery_str_id = host.catalog.string_id("mystery").expect("mystery in pool");
        let out = host
            .call(
                0,
                &[Value::Float(4.2)],
                FunctionOptions::new(&[(mystery_str_id, Value::Str("7".to_string()))]),
            )
            .expect("formatted");
        assert_number_rendered(&mut host, out, "4.20");
    }

    #[test]
    fn runtime_option_key_out_of_range_is_error() {
        let mut host = builtin_host(&["number minimumFractionDigits=2"]);
        let err = host
            .call(
                0,
                &[Value::Float(4.2)],
                FunctionOptions::new(&[(99, Value::Str("3".to_string()))]),
            )
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn string_u_dir_wraps_text_with_expected_isolates() {
        let cases = [
            (
                "string u:dir=ltr",
                "\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}",
            ),
            ("string u:dir=rtl", "hello"),
            (
                "string u:dir=auto",
                "\u{05E9}\u{05DC}\u{05D5}\u{05DD} world",
            ),
        ];
        for (func, input) in cases {
            let mut host = builtin_host(&[func]);
            let out = host
                .call(
                    0,
                    &[Value::Str(input.to_string())],
                    FunctionOptions::new(&[]),
                )
                .expect("formatted");
            assert_string_resolved(out, input);
        }
    }

    #[test]
    fn string_u_dir_ignores_bidi_controls_in_option_value() {
        let mut host = builtin_host(&["string u:dir=|\u{2067}rtl\u{2069}|"]);
        let out = host
            .call(
                0,
                &[Value::Str("abc".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        assert_string_resolved(out, "abc");
    }

    #[test]
    fn number_option_key_with_bidi_controls_is_recognized() {
        let mut host = builtin_host(&["number \u{2068}minimumFractionDigits\u{2069}=2"]);
        let out = host
            .call(0, &[Value::Float(4.2)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, out, "4.20");
    }

    #[test]
    fn string_u_dir_does_not_double_wrap_existing_isolates() {
        let mut host = builtin_host(&["string u:dir=auto"]);
        let input = Value::Str("\u{2066}world\u{2069}".to_string());
        let out = host
            .call(0, &[input], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_string_resolved(out, "\u{2066}world\u{2069}");
    }

    #[test]
    fn builtin_host_resolves_string_pool_refs_before_formatting() {
        let mut host = builtin_host_with_funcs(&["string", "number"], &["hello", "42.5"]);
        let hello_id = host.catalog.string_id("hello").expect("hello in pool");
        let number_id = host.catalog.string_id("42.5").expect("number in pool");

        let string_out = host
            .call(0, &[Value::StrRef(hello_id)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_string_resolved(string_out, "hello");

        let number_out = host
            .call(1, &[Value::StrRef(number_id)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, number_out, "42.5");
    }

    #[test]
    fn builtin_host_resolves_literal_refs_before_formatting() {
        let mut host = builtin_host_with_catalog_parts(&["string", "number"], &[], "hello42.5");

        let string_out = host
            .call(
                0,
                &[Value::LitRef { off: 0, len: 5 }],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        assert_string_resolved(string_out, "hello");

        let number_out = host
            .call(
                1,
                &[Value::LitRef { off: 5, len: 4 }],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        assert_number_rendered(&mut host, number_out, "42.5");
    }

    #[test]
    fn number_select_plural_returns_cardinal_category() {
        let mut host = builtin_host(&["number select=plural"]);
        let one = host
            .call(0, &[Value::Int(1)], FunctionOptions::new(&[]))
            .expect("formatted");
        let other = host
            .call(0, &[Value::Int(2)], FunctionOptions::new(&[]))
            .expect("formatted");
        let Value::Number(one_number) = &one else {
            panic!("number function must return a resolved number");
        };
        assert_eq!(one_number.selection_category, Some(PluralCategory::One));
        let Value::Number(other_number) = &other else {
            panic!("number function must return a resolved number");
        };
        assert_eq!(other_number.selection_category, Some(PluralCategory::Other));
        assert_number_rendered(&mut host, one, "1");
        assert_number_rendered(&mut host, other, "2");
    }

    #[test]
    fn number_select_ordinal_returns_ordinal_category() {
        let mut host = builtin_host(&["number select=ordinal"]);
        let one = host
            .call(0, &[Value::Int(1)], FunctionOptions::new(&[]))
            .expect("formatted");
        let two = host
            .call(0, &[Value::Int(2)], FunctionOptions::new(&[]))
            .expect("formatted");
        let few = host
            .call(0, &[Value::Int(3)], FunctionOptions::new(&[]))
            .expect("formatted");
        let other = host
            .call(0, &[Value::Int(11)], FunctionOptions::new(&[]))
            .expect("formatted");
        let Value::Number(two_number) = &two else {
            panic!("number function must return a resolved number");
        };
        assert_eq!(two_number.selection_category, Some(PluralCategory::Two));
        assert_number_rendered(&mut host, one, "1");
        assert_number_rendered(&mut host, two, "2");
        assert_number_rendered(&mut host, few, "3");
        assert_number_rendered(&mut host, other, "11");
    }

    #[test]
    fn number_call_select_static_plural_returns_pool_ref() {
        let mut host = builtin_host(&["number select=plural"]);
        let out = host
            .call_select(0, &[Value::Int(1)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_selector_result(host.catalog, out, "one");
    }

    #[test]
    fn number_call_select_runtime_override_still_uses_dynamic_select() {
        let mut host = builtin_host_with_funcs(&["number select=plural"], &["select", "ordinal"]);
        let select_id = host.catalog.string_id("select").expect("select in pool");
        let mut errors = Vec::new();
        let out = Host::call_select(
            &mut host.host,
            host.catalog,
            &host.index,
            0,
            &[Value::Int(2)],
            FunctionOptions::new(&[(select_id, Value::Str("ordinal".to_string()))]),
            &mut |error| errors.push(error),
        )
        .expect("formatted");
        assert_eq!(out, Value::Null);
        assert_eq!(errors, vec![MessageFunctionError::BadOption]);
    }

    #[test]
    fn number_call_select_rejects_invalid_runtime_select_override() {
        let mut host = builtin_host_with_funcs(&["number select=plural"], &["select", "bogus"]);
        let select_id = host.catalog.string_id("select").expect("select in pool");
        let mut errors = Vec::new();
        let out = Host::call_select(
            &mut host.host,
            host.catalog,
            &host.index,
            0,
            &[Value::Int(1)],
            FunctionOptions::new(&[(select_id, Value::Str("bogus".to_string()))]),
            &mut |error| errors.push(error),
        )
        .expect("selector fallback");
        assert_eq!(out, Value::Null);
        assert_eq!(errors, vec![MessageFunctionError::BadOption]);
    }

    #[test]
    fn number_call_select_with_fraction_digit_options_uses_dynamic_path() {
        let mut host = builtin_host(&["number select=plural minimumFractionDigits=1"]);
        let out = host
            .call_select(0, &[Value::Int(1)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_selector_result(host.catalog, out, "other");
    }

    #[test]
    fn number_select_exact_returns_formatted_number() {
        let mut host = builtin_host(&["number select=exact"]);
        let out = host
            .call(0, &[Value::Int(42)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, out, "42");
    }

    #[test]
    fn integer_select_plural_returns_cardinal_category() {
        let mut host = builtin_host(&["integer select=plural"]);
        let one = host
            .call(0, &[Value::Int(1)], FunctionOptions::new(&[]))
            .expect("formatted");
        let other = host
            .call(0, &[Value::Int(2)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, one, "1");
        assert_number_rendered(&mut host, other, "2");
    }

    #[test]
    fn number_without_select_returns_formatted_number() {
        let mut host = builtin_host(&["number"]);
        let out = host
            .call(0, &[Value::Int(42)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, out, "42");
    }

    #[test]
    fn number_style_percent_multiplies_by_100() {
        let mut host = builtin_host(&["number style=percent"]);
        let out = host
            .call(0, &[Value::Float(0.5)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_eq!(out, Value::Str("50%".to_string()));
    }

    #[test]
    fn number_style_percent_rejects_large_integer_that_would_lose_precision() {
        let mut host = builtin_host(&["number style=percent"]);
        let err = host
            .call(0, &[Value::Int(i64::MAX)], FunctionOptions::new(&[]))
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOperand);
    }

    #[test]
    fn number_style_percent_with_fraction_digits() {
        let mut host = builtin_host(&["number style=percent minimumFractionDigits=1"]);
        let out = host
            .call(0, &[Value::Float(0.123)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_eq!(out, Value::Str("12.3%".to_string()));
    }

    #[test]
    fn integer_style_percent_multiplies_by_100() {
        let mut host = builtin_host(&["integer style=percent"]);
        let out = host
            .call(0, &[Value::Float(0.42)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_eq!(out, Value::Str("42%".to_string()));
    }

    #[test]
    fn builtin_host_caches_icu_formatters_after_first_use() {
        let mut date_host = builtin_host(&["date style=short"]);
        assert!(date_host.icu_formatters.date.short.is_none());
        let _ = date_host
            .call(
                0,
                &[Value::Str("2024-05-01".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        assert!(date_host.icu_formatters.date.short.is_some());

        let mut time_host = builtin_host(&["time style=short"]);
        assert!(time_host.icu_formatters.time.short.is_none());
        let _ = time_host
            .call(
                0,
                &[Value::Str("2024-05-01T14:30:00".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        assert!(time_host.icu_formatters.time.short.is_some());

        let mut datetime_host = builtin_host(&["datetime"]);
        assert!(datetime_host.icu_formatters.datetime.medium.short.is_none());
        let _ = datetime_host
            .call(
                0,
                &[Value::Str("2024-05-01T14:30:00".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        assert!(datetime_host.icu_formatters.datetime.medium.short.is_some());
    }

    #[test]
    fn offset_rejects_large_integer_that_exceeds_checked_range() {
        let mut host = builtin_host(&["offset add=1"]);
        let err = host
            .call(0, &[Value::Int(i64::MAX)], FunctionOptions::new(&[]))
            .expect("i128 range");
        assert_number_rendered(&mut host, err, "9223372036854775808");

        let mut huge = builtin_host(&["offset add=1"]);
        let err = huge
            .call(
                0,
                &[Value::Str(
                    "99999999999999999999999999999999999999999".to_string(),
                )],
                FunctionOptions::new(&[]),
            )
            .expect_err("must exceed checked range");
        assert_function_error(
            err,
            MessageFunctionError::UnsupportedOperation(UnsupportedOperation::NumericMagnitude),
        );
    }

    #[test]
    fn offset_rejects_missing_or_non_integer_adjustments() {
        let mut missing = builtin_host(&["offset"]);
        let err = missing
            .call(0, &[Value::Int(4)], FunctionOptions::new(&[]))
            .expect_err("missing adjustment must fail");
        assert_function_error(err, MessageFunctionError::BadOption);

        let mut fractional = builtin_host(&["offset add=1.5"]);
        let err = fractional
            .call(0, &[Value::Int(4)], FunctionOptions::new(&[]))
            .expect_err("fractional adjustment must fail");
        assert_function_error(err, MessageFunctionError::BadOption);

        let mut invalid = builtin_host(&["offset add=bogus"]);
        let err = invalid
            .call(0, &[Value::Int(4)], FunctionOptions::new(&[]))
            .expect_err("invalid adjustment must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn datetime_time_style_changes_output_and_cache_slot() {
        let mut short_host = builtin_host(&["datetime dateStyle=short timeStyle=short"]);
        let short = short_host
            .call(
                0,
                &[Value::Str("2024-05-01T14:30:45".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");

        let mut long_host = builtin_host(&["datetime dateStyle=short timeStyle=long"]);
        let long = long_host
            .call(
                0,
                &[Value::Str("2024-05-01T14:30:45".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");

        assert_ne!(short, long);
        assert!(short_host.icu_formatters.datetime.short.short.is_some());
        assert!(long_host.icu_formatters.datetime.short.long.is_some());
    }

    #[test]
    fn parse_iso_datetime_preserves_fractional_seconds() {
        let (date, time) = parse_iso_datetime("2024-05-01T14:30:45.123").expect("parsed");
        assert_eq!(date, Date::try_new_iso(2024, 5, 1).expect("date"));
        assert_eq!(time, Time::try_new(14, 30, 45, 123_000_000).expect("time"));
    }

    #[test]
    fn validate_time_operand_preserves_datetime_without_seconds() {
        let bytes = build_catalog(
            &["main"],
            "",
            &[MessageEntry {
                name_str_id: 0,
                entry_pc: 0,
            }],
            &[vm::Opcode::Halt as u8],
        );
        let catalog = Catalog::from_bytes(&bytes).expect("catalog");
        let validated =
            validate_time_operand(&Value::Str("2024-05-01T14:30".to_string()), &catalog)
                .expect("validated");
        assert_eq!(validated, "2024-05-01T14:30");

        let (_, time) = parse_iso_datetime(&validated).expect("parsed");
        assert_eq!(time, Time::try_new(14, 30, 0, 0).expect("time"));
    }

    #[test]
    fn time_formatting_keeps_hour_and_minute_for_datetime_without_seconds() {
        let mut host = builtin_host(&["time timeStyle=short"]);
        let without_seconds = host
            .call(
                0,
                &[Value::Str("2024-05-01T14:30".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        let with_seconds = host
            .call(
                0,
                &[Value::Str("2024-05-01T14:30:00".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        assert_eq!(without_seconds, with_seconds);
    }

    #[test]
    fn scientific_notation_handles_non_finite_values_without_panicking() {
        assert_eq!(format_scientific(f64::NAN), "NaN");
        assert_eq!(format_scientific(f64::INFINITY), "inf");
        assert_eq!(format_scientific(f64::NEG_INFINITY), "-inf");
    }

    #[test]
    fn maximum_fraction_digits_rounding_does_not_corrupt_large_integer_parts() {
        assert_eq!(
            apply_maximum_fraction_digits("9007199254740993.256".to_string(), Some(2)),
            "9007199254740993.26"
        );
    }

    fn resolved_plural_category_via_text(
        number: &ResolvedNumber,
        rules: &PluralRules,
    ) -> Result<PluralCategory, FormatError> {
        let minimum = number.format.minimum_fraction_digits;
        let maximum = number.format.maximum_fraction_digits;
        validate_digit_range_relationship(minimum, maximum)?;
        let text = minimum.map_or_else(
            || number.text(),
            |minimum| format_int_or_decimal_with_min_fraction_digits(number.text(), minimum),
        );
        let text = apply_maximum_fraction_digits(text, maximum);
        let decimal = Decimal::from_str(&text).map_err(|_| bad_operand())?;
        Ok(rules.category_for(&decimal))
    }

    #[test]
    fn resolved_plural_category_decimal_path_matches_text_reference() {
        let cases = [
            ("1.25", None, Some(1)),
            ("-1.25", None, Some(1)),
            ("9.995", None, Some(2)),
            ("-9.995", None, Some(2)),
            ("1.5", None, Some(0)),
            ("-1.5", None, Some(0)),
            ("1.2", Some(2), Some(2)),
            ("-0", Some(2), Some(2)),
            ("1.2300", Some(2), Some(4)),
            ("1.2300", Some(2), Some(3)),
        ];

        for locale_name in ["en", "ru"] {
            let locale = locale_name.parse().expect("locale");
            let host = BuiltinHost::new(&locale).expect("host");
            for (text, minimum, maximum) in cases {
                let number = ResolvedNumber::new(
                    NumberValue::Decimal(Decimal::from_str(text).expect("decimal")),
                    NumberFormatOptions {
                        minimum_fraction_digits: minimum,
                        maximum_fraction_digits: maximum,
                        ..NumberFormatOptions::DEFAULT
                    },
                    NumberSelection::Plural,
                    true,
                );
                assert_eq!(
                    resolved_plural_category(&number, &host.cardinal_rules),
                    resolved_plural_category_via_text(&number, &host.cardinal_rules),
                    "locale={locale_name} value={text} min={minimum:?} max={maximum:?}"
                );
            }

            let integer = ResolvedNumber::new(
                NumberValue::Integer(1),
                NumberFormatOptions {
                    maximum_fraction_digits: Some(2),
                    ..NumberFormatOptions::DEFAULT
                },
                NumberSelection::Plural,
                true,
            );
            assert_eq!(
                resolved_plural_category(&integer, &host.cardinal_rules),
                resolved_plural_category_via_text(&integer, &host.cardinal_rules),
                "locale={locale_name} integer"
            );
        }
    }

    #[test]
    fn offset_uses_checked_decimal_scaling_without_f64_rounding() {
        assert_eq!(
            number_text(
                &checked_offset(NumberValue::Integer(9_007_199_254_740_993), 1, false,)
                    .expect("sum")
            ),
            "9007199254740994"
        );
        assert_eq!(
            number_text(
                &checked_offset(
                    NumberValue::Decimal(Decimal::from_str("0.5").expect("decimal")),
                    1,
                    false,
                )
                .expect("sum")
            ),
            "1.5"
        );
        assert_eq!(
            number_text(&checked_offset(NumberValue::Integer(i64::MAX), 1, false).expect("sum")),
            "9223372036854775808"
        );
    }

    #[test]
    fn percent_resolved_decimal_shifts_without_f64_rounding() {
        assert_eq!(multiply_decimal_by_100("0.5").expect("percent"), "50");
        assert_eq!(
            multiply_decimal_by_100("9007199254740993").expect("percent"),
            "900719925474099300"
        );
    }
}
