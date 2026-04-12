// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! ICU4X-backed built-in function host.

use alloc::{
    borrow::Cow, collections::BTreeMap, format, string::String, string::ToString, vec::Vec,
};
use core::array;
use core::str::FromStr;

use fixed_decimal::Decimal;
use icu_calendar::Date;
use icu_datetime::fieldsets;
use icu_datetime::input::{DateTime, Time};
use icu_datetime::options::Length;
use icu_datetime::{DateTimeFormatter, NoCalendarFormatter};
use icu_locale_core::Locale;
use icu_plurals::{PluralCategory, PluralRules};

use crate::{
    catalog::Catalog,
    error::{
        FormatError, HostCallError, ImplementationFailure, MessageFunctionError, Trap,
        UnsupportedOperation,
    },
    value::Value,
    vm::Host,
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
enum BuiltinSignDisplay {
    Auto,
    Always,
    Never,
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
        opts: &[(u32, Value)],
    ) -> Result<Value, FormatError> {
        let Some(raw_arg) = args.first() else {
            return Err(bad_operand());
        };
        let options =
            EffectiveOptions::new(&entry.options, opts, catalog, &index.option_keys_by_str_id);
        options.validate_keys()?;
        validate_builtin_option_values(entry.func, &options)?;

        match entry.func {
            BuiltinFn::String => Ok(Value::Str(format_string(catalog, raw_arg, &options))),
            BuiltinFn::Number | BuiltinFn::Integer => {
                let integer_only = entry.func == BuiltinFn::Integer;
                match options
                    .get(BuiltinOptionKey::Select)
                    .as_ref()
                    .map(OptionValue::as_str)
                {
                    Some("plural") => Ok(Value::Str(format_plural(
                        raw_arg,
                        catalog,
                        cardinal_rules,
                        &options,
                    )?)),
                    Some("ordinal") => Ok(Value::Str(format_plural(
                        raw_arg,
                        catalog,
                        ordinal_rules,
                        &options,
                    )?)),
                    _ if options
                        .get(BuiltinOptionKey::Style)
                        .as_ref()
                        .map(OptionValue::as_str)
                        == Some("percent") =>
                    {
                        Ok(Value::Str(format_percent(raw_arg, catalog, &options)?))
                    }
                    _ => Ok(Value::Str(format_number(
                        catalog,
                        raw_arg,
                        integer_only,
                        &options,
                    )?)),
                }
            }
            BuiltinFn::Percent => Ok(Value::Str(format_percent(raw_arg, catalog, &options)?)),
            BuiltinFn::Currency => Ok(Value::Str(format_currency(raw_arg, catalog, &options)?)),
            BuiltinFn::Offset => Ok(Value::Str(format_offset(raw_arg, catalog, &options)?)),
            BuiltinFn::TestSelect => {
                Ok(Value::Str(format_test_select(raw_arg, catalog, &options)?))
            }
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
        opts: &[(u32, Value)],
    ) -> Option<&PluralRules> {
        if entry
            .options
            .iter()
            .enumerate()
            .any(|(index, value)| value.is_some() && index != BuiltinOptionKey::Select.index())
        {
            return None;
        }
        if opts.is_empty() {
            return match entry.select_mode {
                BuiltinSelectMode::Plural => Some(&self.cardinal_rules),
                BuiltinSelectMode::Ordinal => Some(&self.ordinal_rules),
                BuiltinSelectMode::None => None,
            };
        }
        if !matches!(entry.func, BuiltinFn::Number | BuiltinFn::Integer) {
            return None;
        }
        if opts.iter().any(|(key_id, _)| {
            index
                .option_keys_by_str_id
                .get(key_id)
                .is_none_or(|key| *key != BuiltinOptionKey::Select)
        }) {
            return None;
        }
        let options =
            EffectiveOptions::new(&entry.options, opts, catalog, &index.option_keys_by_str_id);
        match options
            .get(BuiltinOptionKey::Select)
            .as_ref()
            .map(OptionValue::as_str)
        {
            Some("plural") => Some(&self.cardinal_rules),
            Some("ordinal") => Some(&self.ordinal_rules),
            _ => None,
        }
    }
}

/// Build a locale fallback candidate chain by progressively truncating subtags.
///
/// Example: `fr-CA-x-private` -> `fr-CA-x` -> `fr-CA` -> `fr`.
#[must_use]
pub fn locale_fallback_candidates(locale: &Locale) -> Vec<Locale> {
    let mut out = Vec::new();
    let mut current = locale.to_string();
    if current.is_empty() {
        return out;
    }
    while !current.is_empty() {
        if let Ok(parsed) = Locale::from_str(&current) {
            out.push(parsed);
        }
        let Some(idx) = current.rfind('-') else {
            break;
        };
        current.truncate(idx);
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
        opts: &[(u32, Value)],
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
        )
        .map_err(into_host_call_error)
    }

    fn call_select(
        &mut self,
        catalog: &Catalog,
        index: &BuiltinHostCatalogIndex,
        fn_id: u16,
        args: &[Value],
        opts: &[(u32, Value)],
    ) -> Result<Value, HostCallError> {
        let Some(entry) = index.by_id.get(&fn_id) else {
            return Err(HostCallError::UnknownFunction { fn_id });
        };
        // For number/integer with select=plural|ordinal, compute category and
        // return a StrRef into the string pool instead of allocating.
        if let Some(rules) = self.plural_rules_for(catalog, index, entry, opts) {
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
        )
        .map_err(into_host_call_error)
    }

    fn format_default(
        &mut self,
        _catalog: &Catalog,
        _index: &BuiltinHostCatalogIndex,
        value: &Value,
    ) -> Option<String> {
        match value {
            Value::Float(v) => Some(format_number_default_locale(*v, &self.locale)),
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
        Value::StrRef(id) => catalog
            .pool_string_opt(*id)
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(id.to_string())),
        Value::LitRef { off, len } => catalog
            .literal_opt(*off, *len)
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(format!("{off}:{len}"))),
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

fn format_string(catalog: &Catalog, value: &Value, options: &EffectiveOptions<'_>) -> String {
    let dir = options.get(BuiltinOptionKey::UDir);
    apply_bidi_dir(
        plain_text(catalog, value),
        dir.as_ref().map(OptionValue::as_str),
    )
}

fn value_text<'a>(catalog: &'a Catalog, value: &'a Value) -> Option<&'a str> {
    match value {
        Value::Str(value) => Some(value),
        Value::StrRef(id) => catalog.pool_string_opt(*id),
        Value::LitRef { off, len } => catalog.literal_opt(*off, *len),
        _ => None,
    }
}

fn format_number(
    catalog: &Catalog,
    value: &Value,
    integer_only: bool,
    options: &EffectiveOptions<'_>,
) -> Result<String, FormatError> {
    let notation = options.get(BuiltinOptionKey::Notation);
    if notation
        .as_ref()
        .is_some_and(|n| n.as_str() == "scientific")
    {
        let num = numeric_operand(value, catalog)?;
        return Ok(format_scientific(num));
    }

    let minimum_fraction_digits = parse_minimum_fraction_digits(options)?;
    let maximum_fraction_digits = parse_maximum_fraction_digits(options)?;
    let minimum_integer_digits = parse_minimum_integer_digits(options)?;
    validate_digit_range_relationship(minimum_fraction_digits, maximum_fraction_digits)?;
    let sign_display = parse_sign_display(options);
    let use_grouping = parse_use_grouping(options);

    let result = match value {
        Value::Null | Value::Bool(_) => Err(bad_operand()),
        Value::Int(v) => {
            if integer_only {
                Ok(format_signed_string(sign_display, v.to_string()))
            } else if let Some(min) = minimum_fraction_digits {
                // Keep integer operands exact here instead of routing them
                // through f64 formatting, which would silently round large i64s.
                Ok(format_signed_string(
                    sign_display,
                    format_int_with_min_fraction_digits(*v, min),
                ))
            } else {
                Ok(format_signed_string(sign_display, v.to_string()))
            }
        }
        Value::Str(v) => {
            let parsed = parse_number(v).ok_or_else(bad_operand)?;
            if integer_only {
                Ok(format_signed_string(sign_display, format_trunc(parsed)))
            } else if let Some(min) = minimum_fraction_digits {
                Ok(format_signed_string(
                    sign_display,
                    format!("{parsed:.min$}"),
                ))
            } else {
                Ok(format_signed_number(sign_display, parsed))
            }
        }
        Value::Float(v) => {
            if integer_only {
                Ok(format_signed_string(sign_display, format_trunc(*v)))
            } else if let Some(min) = minimum_fraction_digits {
                Ok(format_signed_string(
                    sign_display,
                    format_float_with_min_fraction_digits(*v, min),
                ))
            } else {
                Ok(format_signed_number(sign_display, *v))
            }
        }
        _ => {
            let text = value_text(catalog, value).ok_or_else(bad_operand)?;
            let parsed = parse_number(text).ok_or_else(bad_operand)?;
            if integer_only {
                Ok(format_signed_string(sign_display, format_trunc(parsed)))
            } else if let Some(min) = minimum_fraction_digits {
                Ok(format_signed_string(
                    sign_display,
                    format!("{parsed:.min$}"),
                ))
            } else {
                Ok(format_signed_number(sign_display, parsed))
            }
        }
    }?;

    let result = apply_maximum_fraction_digits(result, maximum_fraction_digits);
    let result = apply_minimum_integer_digits(result, minimum_integer_digits);
    let result = apply_grouping_strategy(result, use_grouping);
    Ok(result)
}

fn plural_category(
    value: &Value,
    catalog: &Catalog,
    rules: &PluralRules,
    options: &EffectiveOptions<'_>,
) -> Result<PluralCategory, FormatError> {
    let minimum_fraction_digits = parse_minimum_fraction_digits(options)?;
    let maximum_fraction_digits = parse_maximum_fraction_digits(options)?;
    validate_digit_range_relationship(minimum_fraction_digits, maximum_fraction_digits)?;

    if minimum_fraction_digits.is_none() && maximum_fraction_digits.is_none() {
        return match value {
            Value::Int(v) => Ok(rules.category_for(*v)),
            Value::Float(v) => {
                let decimal = Decimal::from_str(&v.to_string()).map_err(|_| bad_operand())?;
                Ok(rules.category_for(&decimal))
            }
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

fn format_plural_operand(
    value: &Value,
    catalog: &Catalog,
    minimum_fraction_digits: Option<usize>,
    maximum_fraction_digits: Option<usize>,
) -> Result<String, FormatError> {
    match value {
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
            let parsed = parse_number(text).ok_or_else(bad_operand)?;
            let digits = minimum_fraction_digits.unwrap_or(0);
            let rendered = format!("{parsed:.digits$}");
            Ok(apply_maximum_fraction_digits(
                rendered,
                maximum_fraction_digits,
            ))
        }
    }
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

fn format_plural(
    value: &Value,
    catalog: &Catalog,
    rules: &PluralRules,
    options: &EffectiveOptions<'_>,
) -> Result<String, FormatError> {
    plural_category(value, catalog, rules, options).map(|c| category_name(c).to_string())
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

fn strip_bidi_controls(value: &str) -> String {
    value
        .chars()
        .filter(|ch| {
            !matches!(
                ch,
                '\u{061C}'
                    | '\u{200E}'
                    | '\u{200F}'
                    | '\u{2066}'
                    | '\u{2067}'
                    | '\u{2068}'
                    | '\u{2069}'
            )
        })
        .collect()
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
    let Some(value) = options.get(key) else {
        return Ok(());
    };
    if allowed.iter().any(|candidate| *candidate == value.as_str()) {
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
    let value = raw.as_str().parse::<usize>().map_err(|_| bad_option())?;
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

fn parse_sign_display(options: &EffectiveOptions<'_>) -> BuiltinSignDisplay {
    match options
        .get(BuiltinOptionKey::SignDisplay)
        .as_ref()
        .map(OptionValue::as_str)
    {
        Some("always") => BuiltinSignDisplay::Always,
        Some("never") => BuiltinSignDisplay::Never,
        Some("auto") | None => BuiltinSignDisplay::Auto,
        Some(other) => unreachable!("unexpected validated signDisplay value: {other}"),
    }
}

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

fn parse_use_grouping(options: &EffectiveOptions<'_>) -> BuiltinGrouping {
    match options
        .get(BuiltinOptionKey::UseGrouping)
        .as_ref()
        .map(OptionValue::as_str)
    {
        Some("always") => BuiltinGrouping::Always,
        Some("never") => BuiltinGrouping::Never,
        Some("min2") => BuiltinGrouping::Min2,
        Some("auto") | None => BuiltinGrouping::Auto,
        Some(other) => unreachable!("unexpected validated useGrouping value: {other}"),
    }
}

fn parse_number(value: &str) -> Option<f64> {
    if !is_valid_number_literal(value) {
        return None;
    }
    value.parse::<f64>().ok()
}

fn is_valid_number_literal(value: &str) -> bool {
    let bytes = value.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return false;
    }

    let mut idx = 0_usize;
    if bytes[idx] == b'-' {
        idx += 1;
    }
    if idx >= len {
        return false;
    }

    if bytes[idx] == b'0' {
        idx += 1;
        if idx < len && bytes[idx].is_ascii_digit() {
            return false;
        }
    } else if bytes[idx].is_ascii_digit() {
        idx += 1;
        while idx < len && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
    } else {
        return false;
    }

    if idx < len && bytes[idx] == b'.' {
        idx += 1;
        let frac_start = idx;
        while idx < len && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        if frac_start == idx {
            return false;
        }
    }

    if idx < len && (bytes[idx] == b'e' || bytes[idx] == b'E') {
        idx += 1;
        if idx < len && (bytes[idx] == b'+' || bytes[idx] == b'-') {
            idx += 1;
        }
        let exp_start = idx;
        while idx < len && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        if exp_start == idx {
            return false;
        }
    }

    idx == len
}

fn format_trunc(value: f64) -> String {
    let text = value.to_string();
    truncate_decimal_text(&text).unwrap_or(text)
}

fn numeric_operand(value: &Value, catalog: &Catalog) -> Result<f64, FormatError> {
    match value {
        Value::Int(v) => exact_i64_to_f64(*v),
        Value::Float(v) => Ok(*v),
        _ => value_text(catalog, value)
            .and_then(parse_number)
            .ok_or_else(bad_operand),
    }
}

fn format_percent(
    value: &Value,
    catalog: &Catalog,
    options: &EffectiveOptions<'_>,
) -> Result<String, FormatError> {
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
        return Ok(format!("{} {v}", currency.as_str()));
    }
    let number = numeric_operand(value, catalog)?;
    Ok(format!("{} {number}", currency.as_str()))
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
    parse_number(number).is_some()
}

fn format_offset(
    value: &Value,
    catalog: &Catalog,
    options: &EffectiveOptions<'_>,
) -> Result<String, FormatError> {
    let number = parse_offset_operand(value, catalog)?;
    let preserve_plus = value_text(catalog, value).is_some_and(|raw| raw.starts_with('+'));
    let add = options
        .get(BuiltinOptionKey::Add)
        .map(|raw| parse_number(raw.as_str()));
    let subtract = options
        .get(BuiltinOptionKey::Subtract)
        .map(|raw| parse_number(raw.as_str()));

    if add.is_none() && subtract.is_none() {
        return Err(bad_option());
    }
    if add.is_some() && subtract.is_some() {
        return Err(bad_option());
    }
    if matches!(add, Some(None)) || matches!(subtract, Some(None)) {
        return Err(bad_option());
    }

    let adjusted = if let Some(Some(value)) = add {
        number + value
    } else if let Some(Some(value)) = subtract {
        number - value
    } else {
        number
    };
    let sign_display = if preserve_plus {
        BuiltinSignDisplay::Always
    } else {
        parse_sign_display(options)
    };
    Ok(format_signed_number(sign_display, adjusted))
}

fn parse_offset_operand(value: &Value, catalog: &Catalog) -> Result<f64, FormatError> {
    match value {
        Value::Int(v) => exact_i64_to_f64(*v),
        Value::Float(v) => Ok(*v),
        _ => {
            let raw = value_text(catalog, value).ok_or_else(bad_operand)?;
            if let Some(parsed) = parse_number(raw) {
                return Ok(parsed);
            }
            let Some(stripped) = raw.strip_prefix('+') else {
                return Err(bad_operand());
            };
            parse_number(stripped).ok_or_else(bad_operand)
        }
    }
}

fn format_test_select(
    value: &Value,
    catalog: &Catalog,
    options: &EffectiveOptions<'_>,
) -> Result<String, FormatError> {
    if options
        .get(BuiltinOptionKey::Fails)
        .is_some_and(|it| it.as_str() == "select")
    {
        return Err(implementation_failure(ImplementationFailure::TestSelect));
    }
    let number = numeric_operand(value, catalog)?;
    if let Some(raw) = options.get(BuiltinOptionKey::DecimalPlaces) {
        let dp = raw.as_str().parse::<usize>().map_err(|_| bad_option())?;
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
        .is_some_and(|it| it.as_str() == "format")
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
    has_invalid_runtime_key: bool,
    catalog: &'a Catalog,
}

#[derive(Debug)]
enum OptionValue<'a> {
    Borrowed(&'a str),
    Owned(String),
}

impl OptionValue<'_> {
    fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(value) => value,
            Self::Owned(value) => value,
        }
    }
}

impl<'a> EffectiveOptions<'a> {
    fn new(
        base: &'a [Option<String>; BUILTIN_OPTION_KEY_COUNT],
        runtime: &'a [(u32, Value)],
        catalog: &'a Catalog,
        option_keys_by_str_id: &'a BTreeMap<u32, BuiltinOptionKey>,
    ) -> Self {
        let mut runtime_values = array::from_fn(|_| None);
        let mut has_invalid_runtime_key = false;
        for (key_id, value) in runtime {
            if catalog.pool_string_opt(*key_id).is_none() {
                has_invalid_runtime_key = true;
                continue;
            }
            let Some(runtime_key) = option_keys_by_str_id.get(key_id) else {
                continue;
            };
            runtime_values[runtime_key.index()] = Some(value);
        }
        Self {
            base,
            runtime: runtime_values,
            has_invalid_runtime_key,
            catalog,
        }
    }

    fn validate_keys(&self) -> Result<(), FormatError> {
        if self.has_invalid_runtime_key {
            return Err(bad_option());
        }
        Ok(())
    }

    fn get(&self, key: BuiltinOptionKey) -> Option<OptionValue<'a>> {
        if let Some(value) = self.runtime[key.index()] {
            return Some(match value {
                Value::Str(value) => OptionValue::Borrowed(value.as_str()),
                Value::StrRef(id) => {
                    let value = self.catalog.pool_string_opt(*id)?;
                    OptionValue::Borrowed(value)
                }
                Value::LitRef { off, len } => {
                    let value = self.catalog.literal_opt(*off, *len)?;
                    OptionValue::Borrowed(value)
                }
                _ => OptionValue::Owned(plain_text(self.catalog, value).into_owned()),
            });
        }
        self.base[key.index()].as_deref().map(OptionValue::Borrowed)
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
    match style_str.as_ref().map(OptionValue::as_str) {
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
    match style_str.as_ref().map(OptionValue::as_str) {
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

fn format_signed_number(sign_display: BuiltinSignDisplay, value: f64) -> String {
    format_signed_string(sign_display, value.to_string())
}

fn format_number_default_locale(value: f64, locale: &Locale) -> String {
    let mut rendered = value.to_string();
    let locale_tag = locale.to_string();
    if locale_tag.starts_with("fr") {
        rendered = rendered.replace('.', ",");
    }
    rendered
}

fn format_signed_string(sign_display: BuiltinSignDisplay, value: String) -> String {
    match sign_display {
        BuiltinSignDisplay::Auto => value,
        BuiltinSignDisplay::Always => {
            if value.starts_with('-') || value.starts_with('+') {
                value
            } else {
                format!("+{value}")
            }
        }
        BuiltinSignDisplay::Never => {
            if let Some(stripped) = value.strip_prefix('-').or_else(|| value.strip_prefix('+')) {
                stripped.to_string()
            } else {
                value
            }
        }
    }
}

fn format_float_with_min_fraction_digits(value: f64, min: usize) -> String {
    // Fast path: integral values can be rendered once with zero fractional
    // digits and then padded directly. This avoids an extra `to_string()`
    // pass before the precision-formatting path for non-integral values.
    if value.is_finite() && value % 1.0 == 0.0 {
        let raw = format!("{value:.0}");
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
    use crate::{
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
            opts: &[(u32, Value)],
        ) -> Result<Value, HostCallError> {
            Host::call(&mut self.host, self.catalog, &self.index, fn_id, args, opts)
        }

        fn call_select(
            &mut self,
            fn_id: u16,
            args: &[Value],
            opts: &[(u32, Value)],
        ) -> Result<Value, HostCallError> {
            Host::call_select(&mut self.host, self.catalog, &self.index, fn_id, args, opts)
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
            &[vm::OP_HALT],
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
    fn builtin_host_applies_number_minimum_fraction_digits() {
        let mut host = builtin_host(&["number minimumFractionDigits=2"]);
        let out = host
            .call(0, &[Value::Str("4.2".to_string())], &[])
            .expect("formatted");
        assert_eq!(out, Value::Str("4.20".to_string()));
    }

    #[test]
    fn builtin_host_rejects_bad_minimum_fraction_digits() {
        let mut host = builtin_host(&["number minimumFractionDigits=foo"]);
        let err = host
            .call(0, &[Value::Str("4.2".to_string())], &[])
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_rejects_invalid_sign_display_literal() {
        let mut host = builtin_host(&["number signDisplay=bogus"]);
        let err = host.call(0, &[Value::Int(5)], &[]).expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_rejects_invalid_use_grouping_literal() {
        let mut host = builtin_host(&["number useGrouping=bogus"]);
        let err = host.call(0, &[Value::Int(5)], &[]).expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_formats_integral_float_minimum_fraction_digits() {
        let mut host = builtin_host(&["number minimumFractionDigits=3"]);
        let out = host.call(0, &[Value::Float(42.0)], &[]).expect("formatted");
        assert_eq!(out, Value::Str("42.000".to_string()));
    }

    #[test]
    fn builtin_host_formats_large_integer_minimum_fraction_digits_exactly() {
        let mut host = builtin_host(&["number minimumFractionDigits=2"]);
        let out = host
            .call(0, &[Value::Int(i64::MAX)], &[])
            .expect("formatted");
        assert_eq!(out, Value::Str(format!("{}.00", i64::MAX)));
    }

    #[test]
    fn builtin_host_applies_maximum_fraction_digits_rounding() {
        let mut host = builtin_host(&["number maximumFractionDigits=2"]);
        let out = host
            .call(0, &[Value::Float(4.256)], &[])
            .expect("formatted");
        assert_eq!(out, Value::Str("4.26".to_string()));
    }

    #[test]
    fn builtin_host_rejects_invalid_min_then_max_fraction_digit_range() {
        let mut host = builtin_host(&["number minimumFractionDigits=4 maximumFractionDigits=2"]);
        let err = host
            .call(0, &[Value::Float(4.2)], &[])
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_rejects_out_of_range_digit_options() {
        let mut host = builtin_host(&["number minimumFractionDigits=21"]);
        let err = host
            .call(0, &[Value::Float(4.2)], &[])
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);

        let mut host = builtin_host(&["number minimumIntegerDigits=22"]);
        let err = host.call(0, &[Value::Int(42)], &[]).expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_rejects_minimum_fraction_digits_greater_than_maximum() {
        let mut host = builtin_host(&["number minimumFractionDigits=3 maximumFractionDigits=2"]);
        let err = host
            .call(0, &[Value::Float(4.2)], &[])
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn builtin_host_preserves_negative_zero_fraction_formatting() {
        let mut host = builtin_host(&["number minimumFractionDigits=2"]);
        let out = host.call(0, &[Value::Float(-0.0)], &[]).expect("formatted");
        assert_eq!(out, Value::Str("-0.00".to_string()));
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
                &[(mfd_str_id, Value::Str("3".to_string()))],
            )
            .expect("formatted");
        assert_eq!(out, Value::Str("4.200".to_string()));
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
                &[(mystery_str_id, Value::Str("7".to_string()))],
            )
            .expect("formatted");
        assert_eq!(out, Value::Str("4.20".to_string()));
    }

    #[test]
    fn runtime_option_key_out_of_range_is_error() {
        let mut host = builtin_host(&["number minimumFractionDigits=2"]);
        let err = host
            .call(
                0,
                &[Value::Float(4.2)],
                &[(99, Value::Str("3".to_string()))],
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
                "\u{2066}\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}\u{2069}",
            ),
            ("string u:dir=rtl", "hello", "\u{2067}hello\u{2069}"),
            (
                "string u:dir=auto",
                "\u{05E9}\u{05DC}\u{05D5}\u{05DD} world",
                "\u{2068}\u{05E9}\u{05DC}\u{05D5}\u{05DD} world\u{2069}",
            ),
        ];
        for (func, input, expected) in cases {
            let mut host = builtin_host(&[func]);
            let out = host
                .call(0, &[Value::Str(input.to_string())], &[])
                .expect("formatted");
            assert_eq!(out, Value::Str(expected.to_string()));
        }
    }

    #[test]
    fn string_u_dir_ignores_bidi_controls_in_option_value() {
        let mut host = builtin_host(&["string u:dir=|\u{2067}rtl\u{2069}|"]);
        let out = host
            .call(0, &[Value::Str("abc".to_string())], &[])
            .expect("formatted");
        assert_eq!(out, Value::Str("\u{2067}abc\u{2069}".to_string()));
    }

    #[test]
    fn number_option_key_with_bidi_controls_is_recognized() {
        let mut host = builtin_host(&["number \u{2068}minimumFractionDigits\u{2069}=2"]);
        let out = host.call(0, &[Value::Float(4.2)], &[]).expect("formatted");
        assert_eq!(out, Value::Str("4.20".to_string()));
    }

    #[test]
    fn string_u_dir_does_not_double_wrap_existing_isolates() {
        let mut host = builtin_host(&["string u:dir=auto"]);
        let input = Value::Str("\u{2066}world\u{2069}".to_string());
        let out = host.call(0, &[input], &[]).expect("formatted");
        assert_eq!(out, Value::Str("\u{2066}world\u{2069}".to_string()));
    }

    #[test]
    fn builtin_host_resolves_string_pool_refs_before_formatting() {
        let mut host = builtin_host_with_funcs(&["string", "number"], &["hello", "42.5"]);
        let hello_id = host.catalog.string_id("hello").expect("hello in pool");
        let number_id = host.catalog.string_id("42.5").expect("number in pool");

        let string_out = host
            .call(0, &[Value::StrRef(hello_id)], &[])
            .expect("formatted");
        assert_eq!(string_out, Value::Str("\u{2068}hello\u{2069}".to_string()));

        let number_out = host
            .call(1, &[Value::StrRef(number_id)], &[])
            .expect("formatted");
        assert_eq!(number_out, Value::Str("42.5".to_string()));
    }

    #[test]
    fn builtin_host_resolves_literal_refs_before_formatting() {
        let mut host = builtin_host_with_catalog_parts(&["string", "number"], &[], "hello42.5");

        let string_out = host
            .call(0, &[Value::LitRef { off: 0, len: 5 }], &[])
            .expect("formatted");
        assert_eq!(string_out, Value::Str("\u{2068}hello\u{2069}".to_string()));

        let number_out = host
            .call(1, &[Value::LitRef { off: 5, len: 4 }], &[])
            .expect("formatted");
        assert_eq!(number_out, Value::Str("42.5".to_string()));
    }

    #[test]
    fn number_select_plural_returns_cardinal_category() {
        let mut host = builtin_host(&["number select=plural"]);
        let one = host.call(0, &[Value::Int(1)], &[]).expect("formatted");
        let other = host.call(0, &[Value::Int(2)], &[]).expect("formatted");
        assert_eq!(one, Value::Str("one".to_string()));
        assert_eq!(other, Value::Str("other".to_string()));
    }

    #[test]
    fn number_select_ordinal_returns_ordinal_category() {
        let mut host = builtin_host(&["number select=ordinal"]);
        let one = host.call(0, &[Value::Int(1)], &[]).expect("formatted");
        let two = host.call(0, &[Value::Int(2)], &[]).expect("formatted");
        let few = host.call(0, &[Value::Int(3)], &[]).expect("formatted");
        let other = host.call(0, &[Value::Int(11)], &[]).expect("formatted");
        assert_eq!(one, Value::Str("one".to_string()));
        assert_eq!(two, Value::Str("two".to_string()));
        assert_eq!(few, Value::Str("few".to_string()));
        assert_eq!(other, Value::Str("other".to_string()));
    }

    #[test]
    fn number_call_select_static_plural_returns_pool_ref() {
        let mut host = builtin_host(&["number select=plural"]);
        let out = host
            .call_select(0, &[Value::Int(1)], &[])
            .expect("formatted");
        assert_selector_result(host.catalog, out, "one");
    }

    #[test]
    fn number_call_select_runtime_override_still_uses_dynamic_select() {
        let mut host = builtin_host_with_funcs(&["number select=plural"], &["select", "ordinal"]);
        let select_id = host.catalog.string_id("select").expect("select in pool");
        let out = host
            .call_select(
                0,
                &[Value::Int(2)],
                &[(select_id, Value::Str("ordinal".to_string()))],
            )
            .expect("formatted");
        assert_selector_result(host.catalog, out, "two");
    }

    #[test]
    fn number_call_select_rejects_invalid_runtime_select_override() {
        let mut host = builtin_host_with_funcs(&["number select=plural"], &["select", "bogus"]);
        let select_id = host.catalog.string_id("select").expect("select in pool");
        let err = host
            .call_select(
                0,
                &[Value::Int(1)],
                &[(select_id, Value::Str("bogus".to_string()))],
            )
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOption);
    }

    #[test]
    fn number_call_select_with_fraction_digit_options_uses_dynamic_path() {
        let mut host = builtin_host(&["number select=plural minimumFractionDigits=1"]);
        let out = host
            .call_select(0, &[Value::Int(1)], &[])
            .expect("formatted");
        assert_eq!(out, Value::Str("other".to_string()));
    }

    #[test]
    fn number_select_exact_returns_formatted_number() {
        let mut host = builtin_host(&["number select=exact"]);
        let out = host.call(0, &[Value::Int(42)], &[]).expect("formatted");
        assert_eq!(out, Value::Str("42".to_string()));
    }

    #[test]
    fn integer_select_plural_returns_cardinal_category() {
        let mut host = builtin_host(&["integer select=plural"]);
        let one = host.call(0, &[Value::Int(1)], &[]).expect("formatted");
        let other = host.call(0, &[Value::Int(2)], &[]).expect("formatted");
        assert_eq!(one, Value::Str("one".to_string()));
        assert_eq!(other, Value::Str("other".to_string()));
    }

    #[test]
    fn number_without_select_returns_formatted_number() {
        let mut host = builtin_host(&["number"]);
        let out = host.call(0, &[Value::Int(42)], &[]).expect("formatted");
        assert_eq!(out, Value::Str("42".to_string()));
    }

    #[test]
    fn number_style_percent_multiplies_by_100() {
        let mut host = builtin_host(&["number style=percent"]);
        let out = host.call(0, &[Value::Float(0.5)], &[]).expect("formatted");
        assert_eq!(out, Value::Str("50%".to_string()));
    }

    #[test]
    fn number_style_percent_rejects_large_integer_that_would_lose_precision() {
        let mut host = builtin_host(&["number style=percent"]);
        let err = host
            .call(0, &[Value::Int(i64::MAX)], &[])
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOperand);
    }

    #[test]
    fn number_style_percent_with_fraction_digits() {
        let mut host = builtin_host(&["number style=percent minimumFractionDigits=1"]);
        let out = host
            .call(0, &[Value::Float(0.123)], &[])
            .expect("formatted");
        assert_eq!(out, Value::Str("12.3%".to_string()));
    }

    #[test]
    fn integer_style_percent_multiplies_by_100() {
        let mut host = builtin_host(&["integer style=percent"]);
        let out = host.call(0, &[Value::Float(0.42)], &[]).expect("formatted");
        assert_eq!(out, Value::Str("42%".to_string()));
    }

    #[test]
    fn builtin_host_caches_icu_formatters_after_first_use() {
        let mut date_host = builtin_host(&["date style=short"]);
        assert!(date_host.icu_formatters.date.short.is_none());
        let _ = date_host
            .call(0, &[Value::Str("2024-05-01".to_string())], &[])
            .expect("formatted");
        assert!(date_host.icu_formatters.date.short.is_some());

        let mut time_host = builtin_host(&["time style=short"]);
        assert!(time_host.icu_formatters.time.short.is_none());
        let _ = time_host
            .call(0, &[Value::Str("2024-05-01T14:30:00".to_string())], &[])
            .expect("formatted");
        assert!(time_host.icu_formatters.time.short.is_some());

        let mut datetime_host = builtin_host(&["datetime"]);
        assert!(datetime_host.icu_formatters.datetime.medium.short.is_none());
        let _ = datetime_host
            .call(0, &[Value::Str("2024-05-01T14:30:00".to_string())], &[])
            .expect("formatted");
        assert!(datetime_host.icu_formatters.datetime.medium.short.is_some());
    }

    #[test]
    fn offset_rejects_large_integer_that_would_lose_precision() {
        let mut host = builtin_host(&["offset add=1"]);
        let err = host
            .call(0, &[Value::Int(i64::MAX)], &[])
            .expect_err("must fail");
        assert_function_error(err, MessageFunctionError::BadOperand);
    }

    #[test]
    fn datetime_time_style_changes_output_and_cache_slot() {
        let mut short_host = builtin_host(&["datetime dateStyle=short timeStyle=short"]);
        let short = short_host
            .call(0, &[Value::Str("2024-05-01T14:30:45".to_string())], &[])
            .expect("formatted");

        let mut long_host = builtin_host(&["datetime dateStyle=short timeStyle=long"]);
        let long = long_host
            .call(0, &[Value::Str("2024-05-01T14:30:45".to_string())], &[])
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
            &[vm::OP_HALT],
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
            .call(0, &[Value::Str("2024-05-01T14:30".to_string())], &[])
            .expect("formatted");
        let with_seconds = host
            .call(0, &[Value::Str("2024-05-01T14:30:00".to_string())], &[])
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
}
