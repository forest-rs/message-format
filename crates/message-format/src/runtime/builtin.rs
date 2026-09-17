// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! ICU4X-backed built-in function host.

#[cfg(test)]
use alloc::vec;
use alloc::{
    borrow::Cow, boxed::Box, collections::BTreeMap, format, string::String, string::ToString,
    vec::Vec,
};
use core::str::FromStr;
use core::{array, fmt};

use fixed_decimal::{Decimal, Sign, SignedRoundingMode, UnsignedRoundingMode};
use icu_calendar::Date;
use icu_datetime::fieldsets;
use icu_datetime::fieldsets::builder::{DateFields, FieldSetBuilder, ZoneStyle};
use icu_datetime::fieldsets::enums::CompositeFieldSet;
use icu_datetime::input::{DateTime, Time, TimeZone, UtcOffset, ZonedDateTime};
use icu_datetime::options::{Alignment, Length, SubsecondDigits, TimePrecision, YearStyle};
use icu_datetime::preferences::HourCycle;
use icu_datetime::{DateTimeFormatter, DateTimeFormatterPreferences, NoCalendarFormatter};
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu_decimal::preferences::NumberingSystem;
use icu_decimal::{DecimalFormatter, DecimalFormatterPreferences};
use icu_experimental::dimension::currency::CurrencyType;
use icu_experimental::dimension::currency::formatter::{
    CurrencyFormatter, CurrencyFormatterPreferences,
};
use icu_experimental::dimension::currency::options::{CurrencyFormatterOptions, CurrencyUsage};
use icu_experimental::dimension::percent::formatter::{
    PercentFormatter, PercentFormatterPreferences,
};
use icu_experimental::dimension::percent::options::{
    Display as PercentDisplay, PercentFormatterOptions,
};
use icu_locale::{Direction, LocaleDirectionality};
use icu_locale_core::Locale;
use icu_plurals::{PluralCategory, PluralRules};
use writeable::{Part, PartsWrite, Writeable};

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
        CurrencyDisplay, CurrencySign, NumberFormatOptions, NumberGrouping, NumberNotation,
        NumberSelection, NumberSignDisplay, NumberStyle, NumberValue, ResolvedCurrencyOptions,
        ResolvedDateTimeOptions, ResolvedFormatted, ResolvedNumber, ResolvedSelect, ResolvedString,
        StringDirection, Value,
    },
    vm::{
        FormatDirection, FormatField, FormatSink, FormattedValue, FormattedValueKind,
        FunctionOptions, Host, emit_resolved_string, format_i64,
    },
};

const MAX_EXACT_I64_IN_F64: i64 = 9_007_199_254_740_992;

struct PartCollector {
    category: &'static str,
    active: Vec<Part>,
    fields: Vec<FormatField<'static>>,
}

impl PartCollector {
    fn new(category: &'static str) -> Self {
        Self {
            category,
            active: Vec::new(),
            fields: Vec::new(),
        }
    }

    fn finish(self) -> Vec<FormatField<'static>> {
        self.fields
    }
}

impl fmt::Write for PartCollector {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if value.is_empty() {
            return Ok(());
        }
        let semantic = self
            .active
            .iter()
            .rev()
            .find(|part| part.category == self.category)
            .map(|part| part.value);
        let decimal = self
            .active
            .iter()
            .rev()
            .find(|part| part.category == "decimal")
            .map(|part| part.value);
        let kind = match (self.category, semantic, decimal) {
            ("datetime", Some("second"), Some("fraction")) => "fractionalSecond",
            ("datetime", Some("second"), Some("decimal")) => "literal",
            (_, Some(kind), _) => kind,
            _ => "literal",
        };
        if let Some(last) = self.fields.last_mut()
            && last.kind == kind
        {
            last.value.to_mut().push_str(value);
        } else {
            self.fields.push(FormatField {
                kind,
                value: Cow::Owned(value.to_string()),
            });
        }
        Ok(())
    }
}

impl PartsWrite for PartCollector {
    type SubPartsWrite = Self;

    fn with_part(
        &mut self,
        part: Part,
        mut write: impl FnMut(&mut Self::SubPartsWrite) -> fmt::Result,
    ) -> fmt::Result {
        self.active.push(part);
        let result = write(self);
        self.active.pop();
        result
    }
}

fn collect_icu_parts(
    value: &impl Writeable,
    category: &'static str,
) -> Result<Vec<FormatField<'static>>, FormatError> {
    let mut collector = PartCollector::new(category);
    value
        .write_to_parts(&mut collector)
        .map_err(|_| implementation_failure(ImplementationFailure::Host))?;
    Ok(collector.finish())
}

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
    UId,
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
    Length,
    DateLength,
    Precision,
    TimePrecision,
    CurrencyDisplay,
    CurrencySign,
    MinimumSignificantDigits,
    MaximumSignificantDigits,
    NumberingSystem,
    FractionalSecondDigits,
    HourCycle,
}

const BUILTIN_OPTION_KEY_COUNT: usize = 37;

impl BuiltinOptionKey {
    const fn index(self) -> usize {
        match self {
            Self::UDir => 0,
            Self::UId => 25,
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
            Self::Length => 26,
            Self::DateLength => 27,
            Self::Precision => 28,
            Self::TimePrecision => 29,
            Self::CurrencyDisplay => 30,
            Self::CurrencySign => 31,
            Self::MinimumSignificantDigits => 32,
            Self::MaximumSignificantDigits => 33,
            Self::NumberingSystem => 34,
            Self::FractionalSecondDigits => 35,
            Self::HourCycle => 36,
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
    Exact,
    Plural,
    Ordinal,
}

#[derive(Debug, Default)]
struct IcuFormatterCache {
    decimal: Option<CachedDecimalFormatter>,
    percent: Option<CachedPercentFormatter>,
    currency: Option<CachedCurrencyFormatter>,
    date: DateFormatterCache,
    time: TimeFormatterCache,
    datetime: DateTimeFormatterCache,
}

#[derive(Debug)]
struct CachedDecimalFormatter {
    grouping: NumberGrouping,
    numbering_system: Option<NumberingSystem>,
    formatter: DecimalFormatter,
}

#[derive(Debug)]
struct CachedPercentFormatter {
    grouping: NumberGrouping,
    numbering_system: Option<NumberingSystem>,
    display: PercentDisplay,
    formatter: PercentFormatter<DecimalFormatter>,
}

#[derive(Debug)]
struct CachedCurrencyFormatter {
    currency: CurrencyType,
    display: CurrencyDisplay,
    sign: CurrencySign,
    formatter: CurrencyFormatter<DecimalFormatter>,
}

#[derive(Debug, Default)]
struct DateFormatterCache {
    short: Option<DateTimeFormatter<fieldsets::YMD>>,
    medium: Option<DateTimeFormatter<fieldsets::YMD>>,
    long: Option<DateTimeFormatter<fieldsets::YMD>>,
}

#[derive(Debug, Default)]
struct TimeFormatterCache {
    hour: Option<NoCalendarFormatter<fieldsets::T>>,
    minute: Option<NoCalendarFormatter<fieldsets::T>>,
    second: Option<NoCalendarFormatter<fieldsets::T>>,
}

#[derive(Debug, Default)]
struct DateTimeFormatterCache {
    short: TimeStyleDateTimeFormatterCache,
    medium: TimeStyleDateTimeFormatterCache,
    long: TimeStyleDateTimeFormatterCache,
    fields: Option<CachedFieldDateTimeFormatter>,
}

#[derive(Debug)]
struct CachedFieldDateTimeFormatter {
    field_set: CompositeFieldSet,
    hour_cycle: Option<HourCycle>,
    formatter: DateTimeFormatter<CompositeFieldSet>,
}

#[derive(Debug, Default)]
struct TimeStyleDateTimeFormatterCache {
    hour: Option<DateTimeFormatter<fieldsets::YMDT>>,
    minute: Option<DateTimeFormatter<fieldsets::YMDT>>,
    second: Option<DateTimeFormatter<fieldsets::YMDT>>,
}

/// Pre-parsed catalog data needed by the built-in host.
#[derive(Debug)]
pub struct BuiltinHostCatalogIndex {
    by_id: Vec<Option<BuiltinEntry>>,
    option_keys_by_str_id: BTreeMap<u32, BuiltinOptionKey>,
    /// Cached string pool IDs for plural category names, indexed by `category_index()`.
    category_pool_ids: [Option<u32>; 6],
}

impl BuiltinHostCatalogIndex {
    fn new(catalog: &Catalog) -> Result<Self, FormatError> {
        let mut by_id = Vec::with_capacity(catalog.func_count());
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
                by_id.push(None);
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
            by_id.push(Some(BuiltinEntry {
                func: builtin,
                options,
                select_mode,
            }));
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
    direction: Option<FormatDirection>,
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
        let direction = match LocaleDirectionality::new_common().get(&locale.id) {
            Some(Direction::LeftToRight) => Some(FormatDirection::LeftToRight),
            Some(Direction::RightToLeft) => Some(FormatDirection::RightToLeft),
            _ => None,
        };

        Ok(Self {
            locale: locale.clone(),
            direction,
            cardinal_rules,
            ordinal_rules,
            icu_formatters: IcuFormatterCache::default(),
        })
    }

    fn apply(
        catalog: &Catalog,
        index: &BuiltinHostCatalogIndex,
        locale: &Locale,
        icu_formatters: &mut IcuFormatterCache,
        entry: &BuiltinEntry,
        args: &[Value],
        opts: FunctionOptions<'_>,
        selecting: bool,
        on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, FormatError> {
        let Some(raw_arg) = args.first() else {
            return Err(bad_operand());
        };
        if matches!(raw_arg, Value::FunctionFallback(_)) {
            return Err(bad_operand());
        }
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
                let resolved = resolve_number(raw_arg, catalog, integer_only, &options, on_error)?;
                Ok(Value::Number(resolved))
            }
            BuiltinFn::Percent => {
                let (source, selection) = resolve_percent(raw_arg, catalog, &options)?;
                let mut presentation = source.clone();
                presentation.format.style = NumberStyle::Percent;
                let formatted = render_resolved_number(locale, icu_formatters, &presentation)?;
                Ok(Value::Formatted(Box::new(ResolvedFormatted::selectable(
                    Value::Number(source),
                    formatted,
                    selection,
                ))))
            }
            BuiltinFn::Currency => {
                let (formatted, currency) = format_currency(
                    raw_arg,
                    catalog,
                    locale,
                    &mut icu_formatters.currency,
                    &options,
                )?;
                let source = match raw_arg {
                    Value::Formatted(value) => value.source.clone(),
                    value => value.clone(),
                };
                Ok(Value::Formatted(Box::new(ResolvedFormatted::currency(
                    source, formatted, currency,
                ))))
            }
            BuiltinFn::Offset => Ok(Value::Number(resolve_offset(raw_arg, catalog, &options)?)),
            BuiltinFn::TestSelect => Ok(Value::ResolvedSelect(Box::new(ResolvedSelect::new(
                format_test_select(raw_arg, catalog, &options, selecting)?,
            )))),
            BuiltinFn::TestFunction => format_test_function(raw_arg, catalog, &options),
            BuiltinFn::TestFormat if selecting => {
                Err(implementation_failure(ImplementationFailure::TestFormat))
            }
            BuiltinFn::TestFormat => Ok(Value::ResolvedSelect(Box::new(ResolvedSelect::new(
                format_test_select(raw_arg, catalog, &options, false)?,
            )))),
            BuiltinFn::Date => {
                let text = validate_date_operand(raw_arg, catalog)?;
                let (date, _) = parse_iso_datetime(text)?;
                let style = resolve_date_style(&options, false);
                let formatted =
                    format_icu_date_cached(locale, &mut icu_formatters.date, date, style)?;
                Ok(resolved_datetime(
                    raw_arg,
                    formatted,
                    ResolvedDateTimeOptions::Date(style),
                ))
            }
            BuiltinFn::Time => {
                let time_str = validate_time_operand(raw_arg, catalog)?;
                let (_, time) = parse_iso_datetime(&time_str)?;
                let precision = resolve_time_precision(&options, false);
                let formatted =
                    format_icu_time_cached(locale, &mut icu_formatters.time, time, precision)?;
                Ok(resolved_datetime(
                    raw_arg,
                    formatted,
                    ResolvedDateTimeOptions::Time(precision.icu()),
                ))
            }
            BuiltinFn::DateTime => {
                validate_datetime_style_field_exclusivity(&options)?;
                let text = validate_datetime_operand(raw_arg, catalog)?;
                let (date, time) = parse_iso_datetime(&text)?;
                let (formatted, presentation) = if has_datetime_field_options(&options) {
                    let offset = parse_iso_utc_offset(&text)?;
                    let (field_set, hour_cycle) = resolve_datetime_field_set(&options)?;
                    let formatted = format_icu_datetime_fields_cached(
                        locale,
                        &mut icu_formatters.datetime,
                        date,
                        time,
                        offset,
                        field_set,
                        hour_cycle,
                    )?;
                    (
                        formatted,
                        ResolvedDateTimeOptions::Fields {
                            field_set,
                            hour_cycle,
                        },
                    )
                } else {
                    let date_style = resolve_date_style(&options, true);
                    let time_precision = resolve_time_precision(&options, true);
                    let formatted = format_icu_datetime_cached(
                        locale,
                        &mut icu_formatters.datetime,
                        date,
                        time,
                        date_style,
                        time_precision,
                    )?;
                    (
                        formatted,
                        ResolvedDateTimeOptions::DateTime {
                            date: date_style,
                            time: time_precision.icu(),
                        },
                    )
                };
                Ok(resolved_datetime(raw_arg, formatted, presentation))
            }
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

    fn function_is_known(
        &self,
        _catalog: &Catalog,
        index: &BuiltinHostCatalogIndex,
        fn_id: u16,
    ) -> Option<bool> {
        Some(
            index
                .by_id
                .get(usize::from(fn_id))
                .is_some_and(Option::is_some),
        )
    }

    fn unresolved_operand_error(
        &self,
        _catalog: &Catalog,
        index: &BuiltinHostCatalogIndex,
        fn_id: u16,
    ) -> Option<MessageFunctionError> {
        let entry = index.by_id.get(usize::from(fn_id))?.as_ref()?;
        (entry.func != BuiltinFn::String).then_some(MessageFunctionError::BadOperand)
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
        let Some(entry) = index.by_id.get(usize::from(fn_id)).and_then(Option::as_ref) else {
            return Err(HostCallError::UnknownFunction { fn_id });
        };
        if !opts.has_raw_options()
            && matches!(entry.func, BuiltinFn::Number | BuiltinFn::Integer)
            && entry.options.iter().all(Option::is_none)
            && let Some(Value::Int(value)) = args.first()
        {
            return Ok(Value::Number(ResolvedNumber::new(
                NumberValue::Integer(*value),
                NumberFormatOptions::DEFAULT,
                NumberSelection::Plural,
                false,
            )));
        }
        Self::apply(
            catalog,
            index,
            &self.locale,
            &mut self.icu_formatters,
            entry,
            args,
            opts,
            false,
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
        let Some(entry) = index.by_id.get(usize::from(fn_id)).and_then(Option::as_ref) else {
            return Err(HostCallError::UnknownFunction { fn_id });
        };
        if !opts.has_raw_options()
            && matches!(entry.func, BuiltinFn::Number | BuiltinFn::Integer)
            && entry.options.iter().enumerate().all(|(option, value)| {
                value.is_none() || option == BuiltinOptionKey::Select.index()
            })
            && let Some(Value::Int(value)) = args.first()
        {
            let rules = match entry.select_mode {
                BuiltinSelectMode::Plural => Some(&self.cardinal_rules),
                BuiltinSelectMode::Ordinal => Some(&self.ordinal_rules),
                BuiltinSelectMode::None | BuiltinSelectMode::Exact => None,
            };
            if let Some(rules) = rules {
                let category = rules.category_for(*value);
                return Ok(
                    index.category_pool_ids[category_index(category)].map_or_else(
                        || Value::Str(category_name(category).to_string()),
                        Value::StrRef,
                    ),
                );
            }
        }
        if opts.has_raw_options()
            && index
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
        if entry.func == BuiltinFn::Percent {
            let Some(raw_arg) = args.first() else {
                return Err(into_host_call_error(bad_operand()));
            };
            let options =
                EffectiveOptions::new(&entry.options, opts, catalog, &index.option_keys_by_str_id);
            options.validate_keys().map_err(into_host_call_error)?;
            validate_builtin_option_values(entry.func, &options).map_err(into_host_call_error)?;
            let (_, number) =
                resolve_percent(raw_arg, catalog, &options).map_err(into_host_call_error)?;
            return project_resolved_number(self, index, &number).map_err(into_host_call_error);
        }
        if matches!(
            entry.func,
            BuiltinFn::Currency | BuiltinFn::Date | BuiltinFn::Time | BuiltinFn::DateTime
        ) {
            return Ok(Value::Null);
        }
        if matches!(
            entry.func,
            BuiltinFn::Number | BuiltinFn::Integer | BuiltinFn::Offset
        ) {
            let resolved = Self::apply(
                catalog,
                index,
                &self.locale,
                &mut self.icu_formatters,
                entry,
                args,
                opts,
                true,
                on_error,
            )
            .map_err(into_host_call_error)?;
            let Value::Number(number) = resolved else {
                return Ok(resolved);
            };
            return project_resolved_number(self, index, &number).map_err(into_host_call_error);
        }
        Self::apply(
            catalog,
            index,
            &self.locale,
            &mut self.icu_formatters,
            entry,
            args,
            opts,
            true,
            on_error,
        )
        .map_err(into_host_call_error)
    }

    fn project_select(
        &mut self,
        catalog: &Catalog,
        index: &BuiltinHostCatalogIndex,
        fn_id: u16,
        value: &Value,
        on_error: &mut dyn FnMut(MessageFunctionError),
    ) -> Result<Value, HostCallError> {
        let Some(entry) = index.by_id.get(usize::from(fn_id)).and_then(Option::as_ref) else {
            return Err(HostCallError::UnknownFunction { fn_id });
        };
        if entry.func == BuiltinFn::Percent
            && let Value::Formatted(value) = value
            && let Some(number) = &value.selection
        {
            return project_resolved_number(self, index, number).map_err(into_host_call_error);
        }
        if matches!(
            entry.func,
            BuiltinFn::Number | BuiltinFn::Integer | BuiltinFn::Offset
        ) {
            if value.is_fallback() {
                return Ok(Value::Null);
            }
            if let Value::Number(number) = value {
                return match number.selection {
                    NumberSelection::Exact
                    | NumberSelection::Plural
                    | NumberSelection::Ordinal
                    | NumberSelection::Invalid => {
                        project_resolved_number(self, index, number).map_err(into_host_call_error)
                    }
                    NumberSelection::None => self.call_select(
                        catalog,
                        index,
                        fn_id,
                        core::slice::from_ref(value),
                        FunctionOptions::new(&[]),
                        on_error,
                    ),
                };
            }
        }
        self.call_select(
            catalog,
            index,
            fn_id,
            core::slice::from_ref(value),
            FunctionOptions::new(&[]),
            on_error,
        )
    }

    fn format_default(
        &mut self,
        catalog: &Catalog,
        _index: &BuiltinHostCatalogIndex,
        value: &Value,
    ) -> Option<String> {
        match value {
            Value::Int(_) | Value::Float(_) => {
                let number = ResolvedNumber::new(
                    parse_number_value(value, catalog).ok()?,
                    NumberFormatOptions::DEFAULT,
                    NumberSelection::None,
                    false,
                );
                render_resolved_number(&self.locale, &mut self.icu_formatters, &number).ok()
            }
            Value::Number(number) => {
                render_resolved_number(&self.locale, &mut self.icu_formatters, number).ok()
            }
            Value::Formatted(value) => Some(value.text().to_string()),
            Value::String(value) => {
                let direction = match value.direction {
                    StringDirection::Unspecified => None,
                    StringDirection::Auto => Some("auto"),
                    StringDirection::Ltr => Some("ltr"),
                    StringDirection::Rtl => Some("rtl"),
                };
                Some(apply_bidi_dir(Cow::Borrowed(value.text()), direction))
            }
            _ => None,
        }
    }

    fn format_default_to(
        &mut self,
        catalog: &Catalog,
        index: &BuiltinHostCatalogIndex,
        value: &Value,
        sink: &mut dyn FormatSink,
    ) -> bool {
        if sink.wants_structured_output() {
            if let Value::Formatted(value) = value {
                let fields = if let Some(options) = value.datetime {
                    format_resolved_datetime_parts(
                        &self.locale,
                        &mut self.icu_formatters,
                        catalog,
                        value,
                        options,
                    )
                } else if let Some(options) = value.currency.as_ref() {
                    format_resolved_currency_parts(
                        &self.locale,
                        &mut self.icu_formatters.currency,
                        catalog,
                        value,
                        options,
                    )
                } else if value.selection.is_some()
                    && let Value::Number(source) = &value.source
                {
                    let mut presentation = source.clone();
                    presentation.format.style = NumberStyle::Percent;
                    render_resolved_number_inner(
                        &self.locale,
                        &mut self.icu_formatters,
                        &presentation,
                        true,
                    )
                    .map(|rendered| rendered.fields)
                } else {
                    Ok(Vec::new())
                };
                let Ok(fields) = fields else {
                    return false;
                };
                sink.formatted_value(&FormattedValue {
                    kind: value.kind,
                    value: Cow::Borrowed(value.text()),
                    locale: Some(Cow::Owned(self.locale.to_string())),
                    id: None,
                    direction: self.direction,
                    fields: &fields,
                });
                return true;
            }
            if let Value::String(value) = value {
                emit_resolved_string(sink, value, Some(Cow::Owned(self.locale.to_string())));
                return true;
            }
            if let Value::Number(number) = value {
                let Ok(rendered) = render_resolved_number_inner(
                    &self.locale,
                    &mut self.icu_formatters,
                    number,
                    true,
                ) else {
                    return false;
                };
                sink.formatted_value(&FormattedValue {
                    kind: FormattedValueKind::Number,
                    value: Cow::Borrowed(rendered.text.as_str()),
                    locale: Some(Cow::Owned(self.locale.to_string())),
                    id: None,
                    direction: self.direction,
                    fields: &rendered.fields,
                });
                return true;
            }
        }
        if let Value::Formatted(value) = value {
            sink.expression(value.text());
            return true;
        }
        if let Value::String(value) = value
            && value.direction == StringDirection::Unspecified
        {
            sink.expression(value.text());
            return true;
        }
        if let Value::Number(number) = value
            && number.format == NumberFormatOptions::DEFAULT
            && let NumberValue::Integer(value) = &number.value
        {
            let rendered = format_i64(*value);
            sink.expression(rendered.as_str());
            return true;
        }
        let Some(formatted) = self.format_default(catalog, index, value) else {
            return false;
        };
        sink.expression(&formatted);
        true
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
        Value::FunctionFallback(id) => catalog
            .pool_string_opt(*id)
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(id.to_string())),
        Value::LitRef { off, len } => catalog
            .literal_opt(*off, *len)
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(format!("{off}:{len}"))),
        Value::Number(number) => Cow::Owned(number.text()),
        Value::Formatted(value) => Cow::Borrowed(value.text()),
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
        Some("exact") => BuiltinSelectMode::Exact,
        _ => BuiltinSelectMode::None,
    }
}

fn format_string(
    catalog: &Catalog,
    value: &Value,
    options: &EffectiveOptions<'_>,
) -> ResolvedString {
    let dir = options.get(BuiltinOptionKey::UDir);
    if let Value::String(value) = value
        && dir.as_deref() == Some("\0inherit")
    {
        // Compiler-generated default isolation preserves explicit direction
        // already established by a stored resolved string.
        let mut resolved = value.clone();
        if resolved.direction == StringDirection::Unspecified {
            resolved.direction = StringDirection::Auto;
        }
        return resolved;
    }
    let direction = match dir.as_deref() {
        None => StringDirection::Unspecified,
        Some("ltr") => StringDirection::Ltr,
        Some("rtl") => StringDirection::Rtl,
        Some("auto" | "\0inherit") => StringDirection::Auto,
        Some(_) => StringDirection::Unspecified,
    };
    if let Value::Int(value) = value {
        let text = format_i64(*value);
        return ResolvedString::from_integer(text.as_str(), *value, direction);
    }
    let text = plain_text(catalog, value);
    match text {
        Cow::Borrowed(text) => ResolvedString::from_borrowed(text, direction),
        Cow::Owned(text) => ResolvedString::from_owned(text, direction),
    }
}

fn value_text<'a>(catalog: &'a Catalog, value: &'a Value) -> Option<&'a str> {
    match value {
        Value::Str(value) => Some(value),
        Value::StrRef(id) => catalog.pool_string_opt(*id),
        Value::LitRef { off, len } => catalog.literal_opt(*off, *len),
        Value::ResolvedSelect(value) => Some(value.text()),
        Value::String(value) => Some(value.text()),
        Value::Formatted(value) => value_text(catalog, &value.source),
        Value::Fallback(_) | Value::FunctionFallback(_) => None,
        _ => None,
    }
}

fn resolved_datetime(value: &Value, formatted: String, options: ResolvedDateTimeOptions) -> Value {
    let source = match value {
        Value::Formatted(value) => value.source.clone(),
        value => value.clone(),
    };
    Value::Formatted(Box::new(ResolvedFormatted::datetime(
        source, formatted, options,
    )))
}

fn resolve_number(
    value: &Value,
    catalog: &Catalog,
    integer_only: bool,
    options: &EffectiveOptions<'_>,
    on_error: &mut dyn FnMut(MessageFunctionError),
) -> Result<ResolvedNumber, FormatError> {
    let value = numeric_source(value);
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
            NumberValue::Decimal(Box::new(
                Decimal::from_str(&integer_text).map_err(|_| bad_operand())?,
            ))
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
            _ => return Err(bad_option()),
        }
    } else {
        // A number selector uses cardinal plural rules when no explicit
        // selection mode is present. Retain the mode so `call_select` can
        // project a stored value to its category without reapplying the call.
        match inherited_selection {
            NumberSelection::None => NumberSelection::Plural,
            selection => selection,
        }
    };
    Ok(ResolvedNumber::new(
        number,
        format,
        selection,
        has_explicit_select,
    ))
}

fn resolve_percent(
    value: &Value,
    catalog: &Catalog,
    options: &EffectiveOptions<'_>,
) -> Result<(ResolvedNumber, ResolvedNumber), FormatError> {
    let value = numeric_source(value);
    let (number, inherited_format) = match value {
        Value::Number(number) => (number.value.clone(), number.format),
        _ => (
            parse_number_value(value, catalog)?,
            NumberFormatOptions::DEFAULT,
        ),
    };
    let text = match &number {
        NumberValue::Integer(value) => value.to_string(),
        NumberValue::Decimal(value) => value.to_string(),
        NumberValue::NonFinite(value) => value.to_string(),
    };
    let scaled = match &number {
        NumberValue::NonFinite(value) => NumberValue::NonFinite(*value),
        NumberValue::Integer(_) | NumberValue::Decimal(_) => {
            parse_number_text(&multiply_decimal_by_100(&text)?)?
        }
    };

    // Percent selection applies number options after scaling. Unlike
    // `:number`, its fraction digit defaults are both zero.
    let inherited_format = NumberFormatOptions {
        minimum_fraction_digits: inherited_format.minimum_fraction_digits.or(Some(0)),
        maximum_fraction_digits: inherited_format.maximum_fraction_digits,
        minimum_integer_digits: None,
        ..inherited_format
    };
    let mut format = resolve_number_format_options(inherited_format, options, false)?;
    if format.maximum_fraction_digits.is_none() {
        format.maximum_fraction_digits = format.minimum_fraction_digits;
    }
    let source = ResolvedNumber::new(number, format, NumberSelection::Plural, false);
    let selection = ResolvedNumber::new(scaled, format, NumberSelection::Plural, false);
    Ok((source, selection))
}

fn parse_number_value(value: &Value, catalog: &Catalog) -> Result<NumberValue, FormatError> {
    let value = numeric_source(value);
    if let Value::Float(value) = value
        && value.is_sign_negative()
        && *value == 0.0
    {
        return Ok(NumberValue::Decimal(Box::new(
            Decimal::from_str("-0").map_err(|_| bad_operand())?,
        )));
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
    match value {
        Value::Int(value) => return Ok(NumberValue::Integer(*value)),
        Value::Number(value) => return Ok(value.value.clone()),
        Value::String(value) => {
            if let Some(value) = value.integer_hint() {
                return Ok(NumberValue::Integer(value));
            }
        }
        Value::Float(value) => {
            let text = value.to_string();
            return parse_number_text(&text);
        }
        _ => {}
    }
    let text = value_text(catalog, value).ok_or_else(bad_operand)?;
    parse_number_text(text)
}

fn parse_number_text(text: &str) -> Result<NumberValue, FormatError> {
    // Parsing integers before any floating-point conversion is essential: an
    // i64 such as 9007199254740993 must remain exact.
    if text == "-0" {
        return Ok(NumberValue::Decimal(Box::new(
            Decimal::from_str(text).map_err(|_| bad_operand())?,
        )));
    }
    if let Ok(value) = text.parse::<i64>() {
        if parse_number_literal(text).is_none() {
            return Err(bad_operand());
        }
        return Ok(NumberValue::Integer(value));
    }
    if parse_number_literal(text).is_none() {
        return Err(bad_operand());
    }
    let decimal = match Decimal::from_str(text) {
        Ok(decimal) => decimal,
        Err(_) => {
            let value = text.parse::<f64>().map_err(|_| bad_operand())?;
            Decimal::from_str(&value.to_string()).map_err(|_| bad_operand())?
        }
    };
    Ok(NumberValue::Decimal(Box::new(trim_decimal_end(decimal))))
}

/// Convert an integral float to an integer only while every integer in its
/// range is exactly representable by `f64`. Larger integral floats retain the
/// existing decimal parsing path so their shortest decimal representation is
/// not changed by a narrowing conversion.
fn exact_integral_float(value: f64) -> Option<i64> {
    let limit = MAX_EXACT_I64_IN_F64 as f64;
    if (-limit..=limit).contains(&value) {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the range and round-trip checks prove this conversion is exact"
        )]
        let integer = value as i64;
        if integer as f64 != value {
            return None;
        }
        Some(integer)
    } else {
        None
    }
}

fn resolve_number_format_options(
    inherited: NumberFormatOptions,
    options: &EffectiveOptions<'_>,
    integer_only: bool,
) -> Result<NumberFormatOptions, FormatError> {
    let minimum_fraction_digits = parse_digit_option_or_inherited(
        options,
        BuiltinOptionKey::MinimumFractionDigits,
        inherited.minimum_fraction_digits,
        MAX_FRACTION_DIGITS,
    )?;
    let maximum_fraction_digits = parse_digit_option_or_inherited(
        options,
        BuiltinOptionKey::MaximumFractionDigits,
        inherited.maximum_fraction_digits,
        MAX_FRACTION_DIGITS,
    )?;
    // `:integer` ignores fraction formatting, but it must still validate
    // explicitly supplied fraction options to preserve the function's error
    // contract before discarding their resolved values below.
    validate_digit_range_relationship(
        minimum_fraction_digits.map(usize::from),
        maximum_fraction_digits.map(usize::from),
    )?;
    let minimum_integer_digits = parse_digit_option_or_inherited(
        options,
        BuiltinOptionKey::MinimumIntegerDigits,
        inherited.minimum_integer_digits,
        MAX_INTEGER_DIGITS,
    )?;
    let minimum_significant_digits = parse_significant_digit_option_or_inherited(
        options,
        BuiltinOptionKey::MinimumSignificantDigits,
        inherited.minimum_significant_digits,
    )?;
    let maximum_significant_digits = parse_significant_digit_option_or_inherited(
        options,
        BuiltinOptionKey::MaximumSignificantDigits,
        inherited.maximum_significant_digits,
    )?;
    validate_digit_range_relationship(
        minimum_significant_digits.map(usize::from),
        maximum_significant_digits.map(usize::from),
    )?;
    let sign_display = match options.get(BuiltinOptionKey::SignDisplay).as_deref() {
        None => inherited.sign_display,
        Some("auto") => NumberSignDisplay::Auto,
        Some("always") => NumberSignDisplay::Always,
        Some("exceptZero") => NumberSignDisplay::ExceptZero,
        Some("negative") => NumberSignDisplay::Negative,
        Some("never") => NumberSignDisplay::Never,
        Some(_) => return Err(bad_option()),
    };
    let notation = match options.get(BuiltinOptionKey::Notation).as_deref() {
        None => inherited.notation,
        Some("standard") => NumberNotation::Standard,
        Some("scientific") => NumberNotation::Scientific,
        Some("engineering") => NumberNotation::Engineering,
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
    let numbering_system = options
        .get(BuiltinOptionKey::NumberingSystem)
        .map(|value| {
            icu_locale_core::extensions::unicode::Value::from_str(&value)
                .map_err(|_| bad_option())
                .and_then(|value| NumberingSystem::try_from(value).map_err(|_| bad_option()))
        })
        .transpose()?
        .or(inherited.numbering_system);
    let style = match options.get(BuiltinOptionKey::Style).as_deref() {
        None => inherited.style,
        Some("decimal") => NumberStyle::Decimal,
        Some("percent") => NumberStyle::Percent,
        Some(_) => return Err(bad_option()),
    };
    Ok(NumberFormatOptions {
        minimum_fraction_digits: if integer_only {
            None
        } else {
            minimum_fraction_digits
        },
        maximum_fraction_digits: if integer_only {
            None
        } else {
            maximum_fraction_digits
        },
        minimum_significant_digits: if integer_only {
            None
        } else {
            minimum_significant_digits
        },
        maximum_significant_digits,
        minimum_integer_digits,
        numbering_system,
        style,
        sign_display,
        notation,
        grouping,
    })
}

fn parse_digit_option_or_inherited(
    options: &EffectiveOptions<'_>,
    key: BuiltinOptionKey,
    inherited: Option<u8>,
    max: usize,
) -> Result<Option<u8>, FormatError> {
    if let Some(value) = options.get(key) {
        let value = value.parse::<usize>().map_err(|_| bad_option())?;
        if value > max {
            return Err(bad_option());
        }
        return u8::try_from(value).map(Some).map_err(|_| bad_option());
    }
    Ok(inherited)
}

fn parse_significant_digit_option_or_inherited(
    options: &EffectiveOptions<'_>,
    key: BuiltinOptionKey,
    inherited: Option<u8>,
) -> Result<Option<u8>, FormatError> {
    let Some(value) = options.get(key) else {
        return Ok(inherited);
    };
    let value = value.parse::<u8>().map_err(|_| bad_option())?;
    if !(1..=21).contains(&value) {
        return Err(bad_option());
    }
    Ok(Some(value))
}

fn render_resolved_number(
    locale: &Locale,
    cache: &mut IcuFormatterCache,
    number: &ResolvedNumber,
) -> Result<String, FormatError> {
    render_resolved_number_inner(locale, cache, number, false).map(|rendered| rendered.text)
}

struct RenderedNumber {
    text: String,
    fields: Vec<FormatField<'static>>,
}

fn render_resolved_number_inner(
    locale: &Locale,
    cache: &mut IcuFormatterCache,
    number: &ResolvedNumber,
    structured: bool,
) -> Result<RenderedNumber, FormatError> {
    let format = number.format;
    if let NumberValue::NonFinite(value) = number.value {
        let mut rendered = format_signed_string(
            match format.sign_display {
                NumberSignDisplay::Auto => SignDisplay::Auto,
                NumberSignDisplay::Always => SignDisplay::Always,
                NumberSignDisplay::ExceptZero => SignDisplay::ExceptZero,
                NumberSignDisplay::Negative => SignDisplay::Negative,
                NumberSignDisplay::Never => SignDisplay::Never,
            },
            value.to_string(),
        );
        if format.style == NumberStyle::Percent {
            rendered.push('%');
        }
        let fields = if structured {
            split_number_parts(&rendered, ('.', ','))
        } else {
            Vec::new()
        };
        return Ok(RenderedNumber {
            text: rendered,
            fields,
        });
    }
    let number_text = number.text();
    let number_text = if format.style == NumberStyle::Percent {
        multiply_decimal_by_100(&number_text)?
    } else {
        number_text
    };
    if matches!(
        format.notation,
        NumberNotation::Scientific | NumberNotation::Engineering
    ) {
        let text = if format.minimum_significant_digits.is_some()
            || format.maximum_significant_digits.is_some()
        {
            let mut decimal = Decimal::from_str(&number_text)
                .map_err(|_| implementation_failure(ImplementationFailure::Host))?;
            apply_significant_digits(&mut decimal, format);
            decimal.to_string()
        } else {
            number_text
        };
        let mut rendered = format_signed_string(
            match format.sign_display {
                NumberSignDisplay::Auto => SignDisplay::Auto,
                NumberSignDisplay::Always => SignDisplay::Always,
                NumberSignDisplay::ExceptZero => SignDisplay::ExceptZero,
                NumberSignDisplay::Negative => SignDisplay::Negative,
                NumberSignDisplay::Never => SignDisplay::Never,
            },
            format_scientific_text(
                &text,
                format.minimum_significant_digits.is_some(),
                format.notation == NumberNotation::Engineering,
            ),
        );
        if format.style == NumberStyle::Percent {
            rendered.push('%');
        }
        let fields = if structured {
            split_number_parts(&rendered, ('.', ','))
        } else {
            Vec::new()
        };
        return Ok(RenderedNumber {
            text: rendered,
            fields,
        });
    }
    let text = if format.minimum_significant_digits.is_some()
        || format.maximum_significant_digits.is_some()
    {
        let mut decimal = Decimal::from_str(&number_text)
            .map_err(|_| implementation_failure(ImplementationFailure::Host))?;
        apply_significant_digits(&mut decimal, format);
        decimal.to_string()
    } else {
        let text = format_int_or_decimal_with_min_fraction_digits(
            number_text,
            format.minimum_fraction_digits.map_or(0, usize::from),
        );
        apply_maximum_fraction_digits(text, format.maximum_fraction_digits.map(usize::from))
    };
    let text = format_signed_string(
        match format.sign_display {
            NumberSignDisplay::Auto => SignDisplay::Auto,
            NumberSignDisplay::Always => SignDisplay::Always,
            NumberSignDisplay::ExceptZero => SignDisplay::ExceptZero,
            NumberSignDisplay::Negative => SignDisplay::Negative,
            NumberSignDisplay::Never => SignDisplay::Never,
        },
        text,
    );
    let text = apply_minimum_integer_digits(text, format.minimum_integer_digits.map(usize::from));
    let decimal = Decimal::from_str(&text)
        .map_err(|_| implementation_failure(ImplementationFailure::Host))?;
    let grouping_strategy = match format.grouping {
        NumberGrouping::Auto => GroupingStrategy::Auto,
        NumberGrouping::Always => GroupingStrategy::Always,
        NumberGrouping::Never => GroupingStrategy::Never,
        NumberGrouping::Min2 => GroupingStrategy::Min2,
    };
    if format.style == NumberStyle::Percent {
        let display = if text.starts_with('+') {
            PercentDisplay::ExplicitSign
        } else {
            PercentDisplay::Standard
        };
        if !cache.percent.as_ref().is_some_and(|cached| {
            cached.grouping == format.grouping
                && cached.numbering_system == format.numbering_system
                && cached.display == display
        }) {
            let mut decimal_preferences = DecimalFormatterPreferences::from(locale);
            decimal_preferences.numbering_system = format.numbering_system;
            let decimal_formatter = DecimalFormatter::try_new(
                decimal_preferences,
                DecimalFormatterOptions::from(grouping_strategy),
            )
            .map_err(|_| implementation_failure(ImplementationFailure::Host))?;
            let mut percent_preferences = PercentFormatterPreferences::from(locale);
            percent_preferences.numbering_system = format.numbering_system;
            let formatter = PercentFormatter::try_new_with_decimal_formatter(
                percent_preferences,
                decimal_formatter,
                PercentFormatterOptions::from(display),
            )
            .map_err(|_| implementation_failure(ImplementationFailure::Host))?;
            cache.percent = Some(CachedPercentFormatter {
                grouping: format.grouping,
                numbering_system: format.numbering_system,
                display,
                formatter,
            });
        }
        let formatted = cache
            .percent
            .as_ref()
            .expect("percent formatter initialized")
            .formatter
            .format(&decimal)
            .to_string();
        let fields = if structured {
            split_number_parts(
                &formatted,
                localized_number_separators(locale, format.numbering_system)?,
            )
        } else {
            Vec::new()
        };
        return Ok(RenderedNumber {
            text: formatted,
            fields,
        });
    }
    if !cache.decimal.as_ref().is_some_and(|cached| {
        cached.grouping == format.grouping && cached.numbering_system == format.numbering_system
    }) {
        let mut preferences = DecimalFormatterPreferences::from(locale);
        preferences.numbering_system = format.numbering_system;
        let formatter = DecimalFormatter::try_new(
            preferences,
            DecimalFormatterOptions::from(grouping_strategy),
        )
        .map_err(|_| implementation_failure(ImplementationFailure::Host))?;
        cache.decimal = Some(CachedDecimalFormatter {
            grouping: format.grouping,
            numbering_system: format.numbering_system,
            formatter,
        });
    }
    let formatted = cache
        .decimal
        .as_ref()
        .expect("decimal formatter initialized")
        .formatter
        .format(&decimal);
    let fields = if structured {
        collect_icu_parts(&formatted, "decimal")?
    } else {
        Vec::new()
    };
    Ok(RenderedNumber {
        text: formatted.to_string(),
        fields,
    })
}

fn localized_number_separators(
    locale: &Locale,
    numbering_system: Option<NumberingSystem>,
) -> Result<(char, char), FormatError> {
    let mut preferences = DecimalFormatterPreferences::from(locale);
    preferences.numbering_system = numbering_system;
    let formatter = DecimalFormatter::try_new(
        preferences,
        DecimalFormatterOptions::from(GroupingStrategy::Always),
    )
    .map_err(|_| implementation_failure(ImplementationFailure::Host))?;
    let probe = Decimal::from_str("12345.6")
        .map_err(|_| implementation_failure(ImplementationFailure::Host))?;
    let fields = collect_icu_parts(&formatter.format(&probe), "decimal")?;
    let separator = |kind| {
        fields
            .iter()
            .find(|field| field.kind == kind)
            .and_then(|field| field.value.chars().next())
            .ok_or_else(|| implementation_failure(ImplementationFailure::Host))
    };
    Ok((separator("decimal")?, separator("group")?))
}

fn split_number_parts(value: &str, separators: (char, char)) -> Vec<FormatField<'static>> {
    let (decimal_separator, group_separator) = separators;
    let lowercase = value.to_ascii_lowercase();
    let has_nan = lowercase.contains("nan");
    let has_infinity = lowercase.contains("inf") || value.contains('∞');
    let mut fields: Vec<FormatField<'static>> = Vec::new();
    let mut fraction = false;
    let mut exponent = false;
    for ch in value.chars() {
        let kind = if ch.is_numeric() {
            if exponent {
                "exponentInteger"
            } else if fraction {
                "fraction"
            } else {
                "integer"
            }
        } else {
            match ch {
                '+' | '＋' => {
                    if exponent {
                        "exponentPlusSign"
                    } else {
                        "plusSign"
                    }
                }
                '-' | '−' => {
                    if exponent {
                        "exponentMinusSign"
                    } else {
                        "minusSign"
                    }
                }
                'e' | 'E' | 'Ｅ' => {
                    exponent = true;
                    fraction = false;
                    "exponentSeparator"
                }
                _ if ch == decimal_separator => {
                    fraction = true;
                    "decimal"
                }
                _ if ch == group_separator => "group",
                '%' | '\u{66a}' => "percentSign",
                _ if has_nan && ch.is_alphabetic() => "nan",
                _ if has_infinity && (ch.is_alphabetic() || ch == '∞') => "infinity",
                _ => "literal",
            }
        };
        if let Some(last) = fields.last_mut()
            && last.kind == kind
        {
            last.value.to_mut().push(ch);
        } else {
            fields.push(FormatField {
                kind,
                value: Cow::Owned(ch.to_string()),
            });
        }
    }
    fields
}

fn format_scientific_text(value: &str, preserve_trailing_zeros: bool, engineering: bool) -> String {
    let (sign, unsigned) = if let Some(value) = value.strip_prefix('-') {
        ("-", value)
    } else if let Some(value) = value.strip_prefix('+') {
        ("+", value)
    } else {
        ("", value)
    };
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let digits = format!("{integer}{fraction}");
    let leading = digits.len() - digits.trim_start_matches('0').len();
    if leading == digits.len() {
        return format!("{sign}0E0");
    }
    let significant = &digits[leading..];
    let scientific_exponent = integer.len().cast_signed() - leading.cast_signed() - 1;
    let exponent = if engineering {
        scientific_exponent.div_euclid(3) * 3
    } else {
        scientific_exponent
    };
    let integer_digits = usize::try_from(scientific_exponent - exponent + 1)
        .expect("engineering mantissa has one to three integer digits");
    let mut padded = significant.to_string();
    if padded.len() < integer_digits {
        padded.push_str(&"0".repeat(integer_digits - padded.len()));
    }
    let mut mantissa = padded[..integer_digits].to_string();
    let rest = if preserve_trailing_zeros {
        &padded[integer_digits..]
    } else {
        padded[integer_digits..].trim_end_matches('0')
    };
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

/// Compute the category for a resolved number using the precision that was
/// validated and retained on the value. This is intentionally separate from
/// `plural_category`: stored values must be selectable without reapplying
/// their function or resolving options a second time.
fn resolved_plural_category(
    number: &ResolvedNumber,
    rules: &PluralRules,
) -> Result<PluralCategory, FormatError> {
    let minimum_significant_digits = number.format.minimum_significant_digits;
    let maximum_significant_digits = number.format.maximum_significant_digits;
    if minimum_significant_digits.is_some() || maximum_significant_digits.is_some() {
        let mut decimal = match &number.value {
            NumberValue::Integer(value) => Decimal::from(*value),
            NumberValue::Decimal(value) => value.as_ref().clone(),
            NumberValue::NonFinite(_) => return Err(bad_operand()),
        };
        apply_significant_digits(&mut decimal, number.format);
        return Ok(rules.category_for(&decimal));
    }
    let minimum_fraction_digits = number.format.minimum_fraction_digits;
    let maximum_fraction_digits = number.format.maximum_fraction_digits;
    validate_digit_range_relationship(
        minimum_fraction_digits.map(usize::from),
        maximum_fraction_digits.map(usize::from),
    )?;
    if minimum_fraction_digits.is_none() && maximum_fraction_digits.is_none() {
        return match &number.value {
            NumberValue::Integer(value) => Ok(rules.category_for(*value)),
            NumberValue::Decimal(value) => Ok(rules.category_for(value.as_ref())),
            NumberValue::NonFinite(_) => Err(bad_operand()),
        };
    }
    let mut decimal = match &number.value {
        NumberValue::Integer(value) => Decimal::from(*value),
        NumberValue::Decimal(value) => value.as_ref().clone(),
        NumberValue::NonFinite(_) => return Err(bad_operand()),
    };

    if let Some(minimum) = minimum_fraction_digits {
        let minimum = -i16::from(minimum);
        if *decimal.magnitude_range().start() > minimum {
            decimal.pad_end(minimum);
        }
    }
    if let Some(maximum) = maximum_fraction_digits {
        let maximum = i16::from(maximum);
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

fn apply_significant_digits(decimal: &mut Decimal, format: NumberFormatOptions) {
    if let Some(maximum) = format.maximum_significant_digits {
        let position = decimal
            .nonzero_magnitude_start()
            .saturating_sub(i16::from(maximum) - 1);
        decimal.round_with_mode(
            position,
            SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfExpand),
        );
    }
    if let Some(minimum) = format.minimum_significant_digits {
        let position = decimal
            .nonzero_magnitude_start()
            .saturating_sub(i16::from(minimum) - 1);
        decimal.pad_end(position);
    }
}

fn project_resolved_number(
    host: &BuiltinHost,
    index: &BuiltinHostCatalogIndex,
    number: &ResolvedNumber,
) -> Result<Value, FormatError> {
    match number.selection {
        NumberSelection::Exact | NumberSelection::None => Ok(Value::Number(number.clone())),
        NumberSelection::Plural | NumberSelection::Ordinal => {
            let rules = if number.selection == NumberSelection::Ordinal {
                &host.ordinal_rules
            } else {
                &host.cardinal_rules
            };
            let category = resolved_plural_category(number, rules)?;
            if let Some(str_id) = index.category_pool_ids[category_index(category)] {
                Ok(Value::StrRef(str_id))
            } else {
                Ok(Value::Str(category_name(category).to_string()))
            }
        }
        NumberSelection::Invalid => Ok(Value::Null),
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

fn parse_builtin_option_key(value: &str) -> Option<BuiltinOptionKey> {
    Some(match value {
        "u:dir" => BuiltinOptionKey::UDir,
        "u:id" => BuiltinOptionKey::UId,
        "minimumFractionDigits" => BuiltinOptionKey::MinimumFractionDigits,
        "maximumFractionDigits" => BuiltinOptionKey::MaximumFractionDigits,
        "minimumSignificantDigits" => BuiltinOptionKey::MinimumSignificantDigits,
        "maximumSignificantDigits" => BuiltinOptionKey::MaximumSignificantDigits,
        "numberingSystem" => BuiltinOptionKey::NumberingSystem,
        "fractionalSecondDigits" => BuiltinOptionKey::FractionalSecondDigits,
        "hourCycle" => BuiltinOptionKey::HourCycle,
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
        "length" => BuiltinOptionKey::Length,
        "dateLength" => BuiltinOptionKey::DateLength,
        "precision" => BuiltinOptionKey::Precision,
        "timePrecision" => BuiltinOptionKey::TimePrecision,
        "currencyDisplay" => BuiltinOptionKey::CurrencyDisplay,
        "currencySign" => BuiltinOptionKey::CurrencySign,
        _ => return None,
    })
}

fn validate_builtin_option_values(
    func: BuiltinFn,
    options: &EffectiveOptions<'_>,
) -> Result<(), FormatError> {
    validate_enum_option(
        options,
        BuiltinOptionKey::UDir,
        &["ltr", "rtl", "auto", "\0inherit"],
    )?;
    match func {
        BuiltinFn::Number | BuiltinFn::Integer => {}
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
            validate_enum_option(
                options,
                BuiltinOptionKey::Length,
                &["short", "medium", "long"],
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
            validate_enum_option(
                options,
                BuiltinOptionKey::Precision,
                &["hour", "minute", "second"],
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
            validate_enum_option(
                options,
                BuiltinOptionKey::DateLength,
                &["short", "medium", "long"],
            )?;
            validate_enum_option(
                options,
                BuiltinOptionKey::TimePrecision,
                &["hour", "minute", "second"],
            )?;
            validate_datetime_field_option_values(options)?;
        }
        BuiltinFn::Currency => {
            validate_enum_option(
                options,
                BuiltinOptionKey::CurrencyDisplay,
                &["narrowSymbol", "symbol", "name", "code", "never"],
            )?;
            validate_enum_option(
                options,
                BuiltinOptionKey::CurrencySign,
                &["accounting", "standard"],
            )?;
        }
        BuiltinFn::String
        | BuiltinFn::Percent
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

fn validate_datetime_field_option_values(
    options: &EffectiveOptions<'_>,
) -> Result<(), FormatError> {
    validate_enum_option(
        options,
        BuiltinOptionKey::Weekday,
        &["long", "short", "narrow"],
    )?;
    validate_enum_option(options, BuiltinOptionKey::Era, &["long", "short", "narrow"])?;
    validate_enum_option(options, BuiltinOptionKey::Year, &["numeric", "2-digit"])?;
    validate_enum_option(
        options,
        BuiltinOptionKey::Month,
        &["numeric", "2-digit", "long", "short", "narrow"],
    )?;
    for key in [
        BuiltinOptionKey::Day,
        BuiltinOptionKey::Hour,
        BuiltinOptionKey::Minute,
        BuiltinOptionKey::Second,
    ] {
        validate_enum_option(options, key, &["numeric", "2-digit"])?;
    }
    validate_enum_option(
        options,
        BuiltinOptionKey::FractionalSecondDigits,
        &["1", "2", "3"],
    )?;
    validate_enum_option(
        options,
        BuiltinOptionKey::HourCycle,
        &["h11", "h12", "h23", "h24"],
    )?;
    validate_enum_option(
        options,
        BuiltinOptionKey::TimeZoneName,
        &[
            "long",
            "short",
            "shortOffset",
            "longOffset",
            "shortGeneric",
            "longGeneric",
        ],
    )
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

fn numeric_operand(value: &Value, catalog: &Catalog) -> Result<f64, FormatError> {
    let value = numeric_source(value);
    match value {
        Value::Int(v) => exact_i64_to_f64(*v),
        Value::Float(v) => Ok(*v),
        Value::Number(number) => number.text().parse::<f64>().map_err(|_| bad_operand()),
        _ => value_text(catalog, value)
            .and_then(parse_number_literal)
            .ok_or_else(bad_operand),
    }
}

fn multiply_decimal_by_100(value: &str) -> Result<String, FormatError> {
    let (negative, value) = if let Some(value) = value.strip_prefix('-') {
        (true, value)
    } else {
        (false, value.strip_prefix('+').unwrap_or(value))
    };
    let (integer, fraction) = value.split_once('.').unwrap_or((value, ""));
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
        format!("{}.{}", &digits[..split], &digits[split..])
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
    locale: &Locale,
    cache: &mut Option<CachedCurrencyFormatter>,
    options: &EffectiveOptions<'_>,
) -> Result<(String, ResolvedCurrencyOptions), FormatError> {
    let inherited = match value {
        Value::Formatted(value) => value.currency.as_ref(),
        _ => None,
    };
    let currency = match options.get(BuiltinOptionKey::Currency) {
        Some(code) => code.parse::<CurrencyType>().map_err(|_| bad_option())?,
        None => inherited
            .map(|options| options.code)
            .ok_or_else(bad_operand)?,
    };
    let display = match options.get(BuiltinOptionKey::CurrencyDisplay).as_deref() {
        Some("narrowSymbol") => CurrencyDisplay::NarrowSymbol,
        Some("code") => CurrencyDisplay::Code,
        Some("name") => CurrencyDisplay::Name,
        Some("never") => CurrencyDisplay::Never,
        Some("symbol") => CurrencyDisplay::Symbol,
        None => inherited.map_or(CurrencyDisplay::Symbol, |options| options.display),
        Some(_) => return Err(bad_option()),
    };
    let sign = match options.get(BuiltinOptionKey::CurrencySign).as_deref() {
        Some("accounting") => CurrencySign::Accounting,
        Some("standard") => CurrencySign::Standard,
        None => inherited.map_or(CurrencySign::Standard, |options| options.sign),
        Some(_) => return Err(bad_option()),
    };
    let number = match parse_number_value(value, catalog)? {
        NumberValue::Integer(value) => Decimal::from(value),
        NumberValue::Decimal(value) => *value,
        NumberValue::NonFinite(_) => return Err(bad_operand()),
    };
    let resolved = ResolvedCurrencyOptions {
        code: currency,
        display,
        sign,
    };
    let formatter = cached_currency_formatter(locale, cache, &resolved)?;
    let formatted = formatter.format_fixed_decimal(&number).to_string();
    Ok((formatted, resolved))
}

fn cached_currency_formatter<'a>(
    locale: &Locale,
    cache: &'a mut Option<CachedCurrencyFormatter>,
    options: &ResolvedCurrencyOptions,
) -> Result<&'a CurrencyFormatter<DecimalFormatter>, FormatError> {
    if !cache.as_ref().is_some_and(|cached| {
        cached.currency == options.code
            && cached.display == options.display
            && cached.sign == options.sign
    }) {
        let usage = match options.sign {
            CurrencySign::Standard => CurrencyUsage::Standard,
            CurrencySign::Accounting => CurrencyUsage::Accounting,
        };
        let prefs = CurrencyFormatterPreferences::from(locale);
        let formatter_options = CurrencyFormatterOptions::from(usage);
        let formatter = match options.display {
            CurrencyDisplay::Symbol => {
                CurrencyFormatter::try_new_symbol(prefs, options.code, formatter_options)
            }
            CurrencyDisplay::NarrowSymbol => {
                CurrencyFormatter::try_new_symbol_narrow(prefs, options.code, formatter_options)
            }
            CurrencyDisplay::Code => {
                CurrencyFormatter::try_new_code(prefs, options.code, formatter_options)
            }
            CurrencyDisplay::Name => CurrencyFormatter::try_new_name(prefs, options.code),
            CurrencyDisplay::Never => {
                CurrencyFormatter::try_new_no_currency(prefs, options.code, formatter_options)
            }
        }
        .map_err(|_| implementation_failure(ImplementationFailure::Host))?;
        *cache = Some(CachedCurrencyFormatter {
            currency: options.code,
            display: options.display,
            sign: options.sign,
            formatter,
        });
    }
    Ok(&cache
        .as_ref()
        .expect("currency formatter initialized")
        .formatter)
}

fn format_resolved_currency_parts(
    locale: &Locale,
    cache: &mut Option<CachedCurrencyFormatter>,
    catalog: &Catalog,
    value: &ResolvedFormatted,
    options: &ResolvedCurrencyOptions,
) -> Result<Vec<FormatField<'static>>, FormatError> {
    let number = match parse_number_value(&value.source, catalog)? {
        NumberValue::Integer(value) => Decimal::from(value),
        NumberValue::Decimal(value) => *value,
        NumberValue::NonFinite(_) => return Err(bad_operand()),
    };
    let formatter = cached_currency_formatter(locale, cache, options)?;
    let formatted = formatter.format_fixed_decimal(&number);
    let fields = collect_icu_parts(&formatted, "decimal")?;
    Ok(label_currency_parts(fields, options.display))
}

fn label_currency_parts(
    fields: Vec<FormatField<'static>>,
    display: CurrencyDisplay,
) -> Vec<FormatField<'static>> {
    if display == CurrencyDisplay::Never {
        return fields;
    }
    let mut labeled = Vec::with_capacity(fields.len() + 2);
    for field in fields {
        if field.kind != "literal" {
            labeled.push(field);
            continue;
        }
        let value = field.value.into_owned();
        if display == CurrencyDisplay::Name {
            let start = value.len() - value.trim_start().len();
            let end = value.trim_end().len();
            if start >= end {
                push_owned_field(&mut labeled, "literal", &value);
                continue;
            }
            push_owned_field(&mut labeled, "literal", &value[..start]);
            push_owned_field(&mut labeled, "currency", &value[start..end]);
            push_owned_field(&mut labeled, "literal", &value[end..]);
            continue;
        }
        let mut current_kind = None;
        let mut current = String::new();
        for ch in value.chars() {
            let kind = match ch {
                '+' | '＋' => "plusSign",
                '-' | '−' => "minusSign",
                '(' | ')' | '\u{200e}' | '\u{200f}' | '\u{2066}' | '\u{2067}' | '\u{2068}'
                | '\u{2069}'
                    if !ch.is_whitespace() =>
                {
                    "literal"
                }
                _ if ch.is_whitespace() => "literal",
                _ => "currency",
            };
            if current_kind.is_some_and(|active| active != kind) {
                push_owned_field(&mut labeled, current_kind.expect("field kind"), &current);
                current.clear();
            }
            current_kind = Some(kind);
            current.push(ch);
        }
        if let Some(kind) = current_kind {
            push_owned_field(&mut labeled, kind, &current);
        }
    }
    labeled
}

fn push_owned_field(fields: &mut Vec<FormatField<'static>>, kind: &'static str, value: &str) {
    if value.is_empty() {
        return;
    }
    if let Some(last) = fields.last_mut()
        && last.kind == kind
    {
        last.value.to_mut().push_str(value);
    } else {
        fields.push(FormatField {
            kind,
            value: Cow::Owned(value.to_string()),
        });
    }
}

fn numeric_source(mut value: &Value) -> &Value {
    while let Value::Formatted(formatted) = value {
        value = &formatted.source;
    }
    value
}

fn resolve_offset(
    value: &Value,
    catalog: &Catalog,
    options: &EffectiveOptions<'_>,
) -> Result<ResolvedNumber, FormatError> {
    let value = numeric_source(value);
    let (mut number, inherited_format, inherited_selection, has_explicit_select) = match value {
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
    let selection = match inherited_selection {
        NumberSelection::None => NumberSelection::Plural,
        selection => selection,
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
    Ok(ResolvedNumber::new(
        number,
        format,
        selection,
        has_explicit_select,
    ))
}

#[cfg(test)]
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

fn trim_decimal_end(mut value: Decimal) -> Decimal {
    value.absolute = value.absolute.trimmed_end();
    value
}

fn checked_offset(
    number: NumberValue,
    adjustment: i64,
    subtract: bool,
) -> Result<NumberValue, FormatError> {
    let (signed, scale) = match number {
        NumberValue::Integer(value) => (i128::from(value), 0_u32),
        NumberValue::Decimal(value) => decimal_scaled_integer(&value)?,
        NumberValue::NonFinite(value) => return Ok(NumberValue::NonFinite(value)),
    };
    if scale > 38 {
        return Err(unsupported_operation(
            UnsupportedOperation::NumericMagnitude,
        ));
    }
    let factor = 10_i128
        .checked_pow(scale)
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
    if scale == 0
        && let Ok(integer) = i64::try_from(result)
    {
        return Ok(NumberValue::Integer(integer));
    }
    let mut decimal = Decimal::from(result);
    decimal.multiply_pow10(
        -i16::try_from(scale)
            .map_err(|_| unsupported_operation(UnsupportedOperation::NumericMagnitude))?,
    );
    Ok(NumberValue::Decimal(Box::new(trim_decimal_end(decimal))))
}

fn decimal_scaled_integer(value: &Decimal) -> Result<(i128, u32), FormatError> {
    let range = value.absolute.magnitude_range();
    let lower = (*range.start()).min(0);
    let upper = (*range.end()).max(0);
    let scale = u32::try_from(lower.checked_neg().unwrap_or(i16::MAX))
        .map_err(|_| unsupported_operation(UnsupportedOperation::NumericMagnitude))?;
    let mut magnitude = 0_i128;
    for position in (lower..=upper).rev() {
        magnitude = magnitude
            .checked_mul(10)
            .and_then(|accumulator| {
                accumulator.checked_add(i128::from(value.absolute.digit_at(position)))
            })
            .ok_or_else(|| unsupported_operation(UnsupportedOperation::NumericMagnitude))?;
    }
    let signed = if value.sign() == Sign::Negative {
        magnitude
            .checked_neg()
            .ok_or_else(|| unsupported_operation(UnsupportedOperation::NumericMagnitude))?
    } else {
        magnitude
    };
    Ok((signed, scale))
}

fn format_test_select(
    value: &Value,
    catalog: &Catalog,
    options: &EffectiveOptions<'_>,
    selecting: bool,
) -> Result<String, FormatError> {
    if selecting
        && options
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
        if !self.runtime_options.has_raw_options() {
            return false;
        }
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
        || options.get(BuiltinOptionKey::TimeZoneName).is_some()
        || options
            .get(BuiltinOptionKey::FractionalSecondDigits)
            .is_some()
        || options.get(BuiltinOptionKey::HourCycle).is_some();
    if has_style && has_field {
        return Err(bad_option());
    }
    Ok(())
}

fn has_datetime_field_options(options: &EffectiveOptions<'_>) -> bool {
    [
        BuiltinOptionKey::Year,
        BuiltinOptionKey::Month,
        BuiltinOptionKey::Day,
        BuiltinOptionKey::Hour,
        BuiltinOptionKey::Minute,
        BuiltinOptionKey::Second,
        BuiltinOptionKey::Weekday,
        BuiltinOptionKey::Era,
        BuiltinOptionKey::FractionalSecondDigits,
        BuiltinOptionKey::HourCycle,
        BuiltinOptionKey::TimeZoneName,
    ]
    .into_iter()
    .any(|key| options.get(key).is_some())
}

fn resolve_datetime_field_set(
    options: &EffectiveOptions<'_>,
) -> Result<(CompositeFieldSet, Option<HourCycle>), FormatError> {
    let year = options.get(BuiltinOptionKey::Year);
    let month = options.get(BuiltinOptionKey::Month);
    let day = options.get(BuiltinOptionKey::Day);
    let weekday = options.get(BuiltinOptionKey::Weekday);
    let era = options.get(BuiltinOptionKey::Era);
    let hour = options.get(BuiltinOptionKey::Hour);
    let minute = options.get(BuiltinOptionKey::Minute);
    let second = options.get(BuiltinOptionKey::Second);
    let date_fields = match (
        year.is_some(),
        month.is_some(),
        day.is_some(),
        weekday.is_some(),
    ) {
        (false, false, false, false) => None,
        (false, false, true, false) => Some(DateFields::D),
        (false, true, true, false) => Some(DateFields::MD),
        (true, true, true, false) => Some(DateFields::YMD),
        (false, false, true, true) => Some(DateFields::DE),
        (false, true, true, true) => Some(DateFields::MDE),
        (true, true, true, true) => Some(DateFields::YMDE),
        (false, false, false, true) => Some(DateFields::E),
        (false, true, false, false) => Some(DateFields::M),
        (true, true, false, false) => Some(DateFields::YM),
        (true, false, false, false) => Some(DateFields::Y),
        _ => {
            return Err(unsupported_operation(
                UnsupportedOperation::DateTimeFormattingForLocale,
            ));
        }
    };

    if era.is_some() && year.is_none() {
        return Err(unsupported_operation(
            UnsupportedOperation::DateTimeFormattingForLocale,
        ));
    }

    let fractional_digits = match options
        .get(BuiltinOptionKey::FractionalSecondDigits)
        .as_deref()
    {
        None => None,
        Some("1") => Some(SubsecondDigits::S1),
        Some("2") => Some(SubsecondDigits::S2),
        Some("3") => Some(SubsecondDigits::S3),
        Some(_) => return Err(bad_option()),
    };
    let time_precision = if let Some(digits) = fractional_digits {
        Some(TimePrecision::Subsecond(digits))
    } else if second.is_some() {
        Some(TimePrecision::Second)
    } else if minute.is_some() {
        Some(TimePrecision::Minute)
    } else if hour.is_some() {
        Some(TimePrecision::Hour)
    } else {
        None
    };

    if date_fields.is_none()
        && time_precision.is_none()
        && options.get(BuiltinOptionKey::TimeZoneName).is_none()
    {
        return Err(bad_option());
    }

    let requested_values = [
        year.as_deref(),
        month.as_deref(),
        day.as_deref(),
        weekday.as_deref(),
        era.as_deref(),
        hour.as_deref(),
        minute.as_deref(),
        second.as_deref(),
    ];
    let length = if requested_values.contains(&Some("long")) {
        Length::Long
    } else if requested_values.contains(&Some("short")) {
        Length::Medium
    } else {
        Length::Short
    };
    let alignment = requested_values
        .contains(&Some("2-digit"))
        .then_some(Alignment::Column);
    let year_style = if era.is_some() {
        Some(YearStyle::WithEra)
    } else if year.as_deref() == Some("numeric") {
        Some(YearStyle::Full)
    } else {
        None
    };
    let hour_cycle = match options.get(BuiltinOptionKey::HourCycle).as_deref() {
        None => None,
        Some("h11") => Some(HourCycle::H11),
        Some("h12") => Some(HourCycle::H12),
        Some("h23") => Some(HourCycle::H23),
        Some("h24") => {
            return Err(unsupported_operation(
                UnsupportedOperation::DateTimeFormattingForLocale,
            ));
        }
        Some(_) => return Err(bad_option()),
    };
    let zone_style = match options.get(BuiltinOptionKey::TimeZoneName).as_deref() {
        None => None,
        Some("long") => Some(ZoneStyle::SpecificLong),
        Some("short") => Some(ZoneStyle::SpecificShort),
        Some("shortOffset") => Some(ZoneStyle::LocalizedOffsetShort),
        Some("longOffset") => Some(ZoneStyle::LocalizedOffsetLong),
        Some("shortGeneric") => Some(ZoneStyle::GenericShort),
        Some("longGeneric") => Some(ZoneStyle::GenericLong),
        Some(_) => return Err(bad_option()),
    };

    let mut builder = FieldSetBuilder::new();
    builder.date_fields = date_fields;
    builder.time_precision = time_precision;
    builder.length = Some(length);
    builder.alignment = alignment;
    builder.year_style = year_style;
    builder.zone_style = zone_style;
    let field_set = builder
        .build_composite()
        .map_err(|_| unsupported_operation(UnsupportedOperation::DateTimeFormattingForLocale))?;
    Ok((field_set, hour_cycle))
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
) -> Result<Cow<'a, str>, FormatError> {
    let text = value_text(catalog, value).ok_or_else(bad_operand)?;
    if text.contains('T') && text.chars().nth(4) == Some('-') {
        Ok(Cow::Borrowed(text))
    } else if text.len() >= 10
        && text.chars().nth(4) == Some('-')
        && text.chars().nth(7) == Some('-')
    {
        Ok(Cow::Owned(format!("{text}T00:00:00")))
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

fn parse_iso_utc_offset(text: &str) -> Result<UtcOffset, FormatError> {
    let Some((_, time)) = text.split_once('T') else {
        return Ok(UtcOffset::zero());
    };
    if time.ends_with('Z') {
        return Ok(UtcOffset::zero());
    }
    let Some(position) = time.rfind(['+', '-']) else {
        return Ok(UtcOffset::zero());
    };
    let sign = if time.as_bytes()[position] == b'-' {
        -1
    } else {
        1
    };
    let (hours, minutes) = time[position + 1..]
        .split_once(':')
        .ok_or_else(bad_operand)?;
    let hours = hours.parse::<i32>().map_err(|_| bad_operand())?;
    let minutes = minutes.parse::<i32>().map_err(|_| bad_operand())?;
    let seconds = sign * (hours * 60 + minutes) * 60;
    UtcOffset::try_from_seconds(seconds).map_err(|_| bad_operand())
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

/// Resolve the date length, defaulting to `medium`.
fn resolve_date_style(options: &EffectiveOptions<'_>, datetime: bool) -> Length {
    let current_key = if datetime {
        BuiltinOptionKey::DateLength
    } else {
        BuiltinOptionKey::Length
    };
    let style_str = options
        .get(current_key)
        .or_else(|| options.get(BuiltinOptionKey::DateStyle))
        .or_else(|| options.get(BuiltinOptionKey::Style));
    match style_str.as_deref() {
        Some("short") => Length::Short,
        Some("long") | Some("full") => Length::Long,
        _ => Length::Medium,
    }
}

#[derive(Clone, Copy)]
enum TimePrecisionBucket {
    Hour,
    Minute,
    Second,
}

impl TimePrecisionBucket {
    fn icu(self) -> TimePrecision {
        match self {
            Self::Hour => TimePrecision::Hour,
            Self::Minute => TimePrecision::Minute,
            Self::Second => TimePrecision::Second,
        }
    }

    fn from_icu(precision: TimePrecision) -> Result<Self, FormatError> {
        match precision {
            TimePrecision::Hour => Ok(Self::Hour),
            TimePrecision::Minute => Ok(Self::Minute),
            TimePrecision::Second => Ok(Self::Second),
            _ => Err(implementation_failure(ImplementationFailure::Host)),
        }
    }
}

/// Resolve the current precision option, accepting the former style aliases.
fn resolve_time_precision(options: &EffectiveOptions<'_>, datetime: bool) -> TimePrecisionBucket {
    let current_key = if datetime {
        BuiltinOptionKey::TimePrecision
    } else {
        BuiltinOptionKey::Precision
    };
    let precision = options.get(current_key);
    match precision.as_deref() {
        Some("hour") => return TimePrecisionBucket::Hour,
        Some("second") => return TimePrecisionBucket::Second,
        Some("minute") => return TimePrecisionBucket::Minute,
        _ => {}
    }

    let former_style = options
        .get(BuiltinOptionKey::TimeStyle)
        .or_else(|| options.get(BuiltinOptionKey::Style));
    match former_style.as_deref() {
        Some("medium" | "long" | "full") => TimePrecisionBucket::Second,
        _ => TimePrecisionBucket::Minute,
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
    fn slot_mut(
        &mut self,
        precision: TimePrecisionBucket,
    ) -> &mut Option<NoCalendarFormatter<fieldsets::T>> {
        match precision {
            TimePrecisionBucket::Hour => &mut self.hour,
            TimePrecisionBucket::Minute => &mut self.minute,
            TimePrecisionBucket::Second => &mut self.second,
        }
    }
}

impl DateTimeFormatterCache {
    fn slot_mut(
        &mut self,
        date_style: Length,
        time_precision: TimePrecisionBucket,
    ) -> &mut Option<DateTimeFormatter<fieldsets::YMDT>> {
        let time_slots = match style_bucket(date_style) {
            StyleBucket::Short => &mut self.short,
            StyleBucket::Medium => &mut self.medium,
            StyleBucket::Long => &mut self.long,
        };
        match time_precision {
            TimePrecisionBucket::Hour => &mut time_slots.hour,
            TimePrecisionBucket::Minute => &mut time_slots.minute,
            TimePrecisionBucket::Second => &mut time_slots.second,
        }
    }
}

fn format_icu_date_cached(
    locale: &Locale,
    cache: &mut DateFormatterCache,
    date: Date<icu_calendar::Iso>,
    style: Length,
) -> Result<String, FormatError> {
    Ok(cached_date_formatter(locale, cache, style)?
        .format(&date)
        .to_string())
}

fn cached_date_formatter<'a>(
    locale: &Locale,
    cache: &'a mut DateFormatterCache,
    style: Length,
) -> Result<&'a DateTimeFormatter<fieldsets::YMD>, FormatError> {
    let slot = cache.slot_mut(style);
    if slot.is_none() {
        *slot = Some(
            DateTimeFormatter::try_new(locale.clone().into(), date_field_set(style)).map_err(
                |_| unsupported_operation(UnsupportedOperation::DateFormattingForLocale),
            )?,
        );
    }
    Ok(slot.as_ref().expect("date formatter initialized"))
}

fn format_icu_time_cached(
    locale: &Locale,
    cache: &mut TimeFormatterCache,
    time: Time,
    precision: TimePrecisionBucket,
) -> Result<String, FormatError> {
    Ok(cached_time_formatter(locale, cache, precision)?
        .format(&time)
        .to_string())
}

fn cached_time_formatter<'a>(
    locale: &Locale,
    cache: &'a mut TimeFormatterCache,
    precision: TimePrecisionBucket,
) -> Result<&'a NoCalendarFormatter<fieldsets::T>, FormatError> {
    let slot = cache.slot_mut(precision);
    if slot.is_none() {
        *slot = Some(
            NoCalendarFormatter::try_new(locale.clone().into(), time_field_set(precision))
                .map_err(|_| {
                    unsupported_operation(UnsupportedOperation::TimeFormattingForLocale)
                })?,
        );
    }
    Ok(slot.as_ref().expect("time formatter initialized"))
}

fn format_icu_datetime_cached(
    locale: &Locale,
    cache: &mut DateTimeFormatterCache,
    date: Date<icu_calendar::Iso>,
    time: Time,
    date_style: Length,
    time_precision: TimePrecisionBucket,
) -> Result<String, FormatError> {
    let formatter = cached_datetime_formatter(locale, cache, date_style, time_precision)?;
    let dt = DateTime { date, time };
    Ok(formatter.format(&dt).to_string())
}

fn cached_datetime_formatter<'a>(
    locale: &Locale,
    cache: &'a mut DateTimeFormatterCache,
    date_style: Length,
    time_precision: TimePrecisionBucket,
) -> Result<&'a DateTimeFormatter<fieldsets::YMDT>, FormatError> {
    let slot = cache.slot_mut(date_style, time_precision);
    if slot.is_none() {
        *slot = Some(
            DateTimeFormatter::try_new(
                locale.clone().into(),
                datetime_field_set(date_style, time_precision),
            )
            .map_err(|_| {
                unsupported_operation(UnsupportedOperation::DateTimeFormattingForLocale)
            })?,
        );
    }
    Ok(slot.as_ref().expect("datetime formatter initialized"))
}

fn format_icu_datetime_fields_cached(
    locale: &Locale,
    cache: &mut DateTimeFormatterCache,
    date: Date<icu_calendar::Iso>,
    time: Time,
    offset: UtcOffset,
    field_set: CompositeFieldSet,
    hour_cycle: Option<HourCycle>,
) -> Result<String, FormatError> {
    let formatter = cached_datetime_fields_formatter(locale, cache, field_set, hour_cycle)?;
    let datetime = DateTime { date, time };
    let zone = TimeZone::UNKNOWN
        .with_offset(Some(offset))
        .at_date_time(datetime);
    let datetime = ZonedDateTime { date, time, zone };
    Ok(formatter.format(&datetime).to_string())
}

fn cached_datetime_fields_formatter<'a>(
    locale: &Locale,
    cache: &'a mut DateTimeFormatterCache,
    field_set: CompositeFieldSet,
    hour_cycle: Option<HourCycle>,
) -> Result<&'a DateTimeFormatter<CompositeFieldSet>, FormatError> {
    if !cache
        .fields
        .as_ref()
        .is_some_and(|cached| cached.field_set == field_set && cached.hour_cycle == hour_cycle)
    {
        let mut preferences = DateTimeFormatterPreferences::from(locale);
        preferences.hour_cycle = hour_cycle;
        let formatter = DateTimeFormatter::try_new(preferences, field_set).map_err(|_| {
            unsupported_operation(UnsupportedOperation::DateTimeFormattingForLocale)
        })?;
        cache.fields = Some(CachedFieldDateTimeFormatter {
            field_set,
            hour_cycle,
            formatter,
        });
    }
    Ok(&cache
        .fields
        .as_ref()
        .expect("datetime field formatter initialized")
        .formatter)
}

fn format_resolved_datetime_parts(
    locale: &Locale,
    formatters: &mut IcuFormatterCache,
    catalog: &Catalog,
    value: &ResolvedFormatted,
    options: ResolvedDateTimeOptions,
) -> Result<Vec<FormatField<'static>>, FormatError> {
    let text = value_text(catalog, &value.source).ok_or_else(bad_operand)?;
    let (date, time) = parse_iso_datetime(text)?;
    match options {
        ResolvedDateTimeOptions::Date(style) => collect_icu_parts(
            &cached_date_formatter(locale, &mut formatters.date, style)?.format(&date),
            "datetime",
        ),
        ResolvedDateTimeOptions::Time(precision) => {
            let precision = TimePrecisionBucket::from_icu(precision)?;
            collect_icu_parts(
                &cached_time_formatter(locale, &mut formatters.time, precision)?.format(&time),
                "datetime",
            )
        }
        ResolvedDateTimeOptions::DateTime {
            date: style,
            time: precision,
        } => {
            let precision = TimePrecisionBucket::from_icu(precision)?;
            let datetime = DateTime { date, time };
            collect_icu_parts(
                &cached_datetime_formatter(locale, &mut formatters.datetime, style, precision)?
                    .format(&datetime),
                "datetime",
            )
        }
        ResolvedDateTimeOptions::Fields {
            field_set,
            hour_cycle,
        } => {
            let datetime = DateTime { date, time };
            let zone = TimeZone::UNKNOWN
                .with_offset(Some(parse_iso_utc_offset(text)?))
                .at_date_time(datetime);
            let datetime = ZonedDateTime { date, time, zone };
            collect_icu_parts(
                &cached_datetime_fields_formatter(
                    locale,
                    &mut formatters.datetime,
                    field_set,
                    hour_cycle,
                )?
                .format(&datetime),
                "datetime",
            )
        }
    }
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

fn icu_time_precision(precision: TimePrecisionBucket) -> TimePrecision {
    match precision {
        TimePrecisionBucket::Hour => TimePrecision::Hour,
        TimePrecisionBucket::Minute => TimePrecision::Minute,
        TimePrecisionBucket::Second => TimePrecision::Second,
    }
}

fn time_field_set(precision: TimePrecisionBucket) -> fieldsets::T {
    fieldsets::T::short().with_time_precision(icu_time_precision(precision))
}

fn datetime_field_set(date_style: Length, time_precision: TimePrecisionBucket) -> fieldsets::YMDT {
    date_field_set(date_style).with_time(icu_time_precision(time_precision))
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
    let Some(dir) = dir else {
        return value.into_owned();
    };
    if is_bidi_isolated(value.as_ref()) {
        return value.into_owned();
    }
    let isolate_open = match dir {
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

    #[derive(Default)]
    struct StructuredSink {
        kind: Option<FormattedValueKind>,
        value: String,
        fields: Vec<(String, String)>,
    }

    impl FormatSink for StructuredSink {
        fn wants_structured_output(&self) -> bool {
            true
        }

        fn literal(&mut self, value: &str) {
            self.value.push_str(value);
        }

        fn expression(&mut self, value: &str) {
            self.value.push_str(value);
        }

        fn markup_open(&mut self, _name: &str, _options: &[crate::runtime::FormatOption<'_>]) {}

        fn markup_close(&mut self, _name: &str, _options: &[crate::runtime::FormatOption<'_>]) {}

        fn formatted_value(&mut self, value: &FormattedValue<'_>) {
            self.kind = Some(value.kind);
            self.value.push_str(&value.value);
            self.fields.extend(
                value
                    .fields
                    .iter()
                    .map(|field| (field.kind.to_string(), field.value.to_string())),
            );
        }
    }

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

        fn project_select(&mut self, fn_id: u16, value: &Value) -> Result<Value, HostCallError> {
            Host::project_select(
                &mut self.host,
                self.catalog,
                &self.index,
                fn_id,
                value,
                &mut |_| {},
            )
        }

        fn structured(&mut self, value: &Value) -> StructuredSink {
            let mut sink = StructuredSink::default();
            assert!(Host::format_default_to(
                &mut self.host,
                self.catalog,
                &self.index,
                value,
                &mut sink,
            ));
            sink
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
    fn formatted_values_retain_their_source_for_reannotation() {
        let mut host = builtin_host(&[
            "datetime dateLength=long timePrecision=second",
            "date",
            "percent",
            "currency currency=EUR",
            "currency",
        ]);
        let datetime = host
            .call(
                0,
                &[Value::Str("2006-01-02T15:04:06".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("datetime resolves");
        host.call(1, &[datetime], FunctionOptions::new(&[]))
            .expect("datetime can be reannotated as a date");

        let percent = host
            .call(2, &[Value::Float(0.01)], FunctionOptions::new(&[]))
            .expect("percent resolves");
        host.call(2, &[percent], FunctionOptions::new(&[]))
            .expect("percent can be reannotated");

        let currency = host
            .call(3, &[Value::Int(42)], FunctionOptions::new(&[]))
            .expect("currency resolves");
        host.call(4, &[currency], FunctionOptions::new(&[]))
            .expect("currency inherits its required option");
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
    fn project_select_uses_valid_mode_from_stored_number() {
        let mut host = builtin_host(&["number select=plural", "number"]);
        let stored = host
            .call(0, &[Value::Int(1)], FunctionOptions::new(&[]))
            .expect("stored number");
        let selected = host.project_select(0, &stored).expect("stored selection");
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
        for function in [
            "number minimumFractionDigits=foo",
            "integer minimumFractionDigits=foo",
        ] {
            let mut host = builtin_host(&[function]);
            let err = host
                .call(
                    0,
                    &[Value::Str("4.2".to_string())],
                    FunctionOptions::new(&[]),
                )
                .expect_err("must fail");
            assert_function_error(err, MessageFunctionError::BadOption);
        }
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
    fn percent_paths_reject_invalid_sign_display_literal() {
        for function in [
            "number style=percent signDisplay=bogus",
            "percent signDisplay=bogus",
        ] {
            let mut host = builtin_host(&[function]);
            let err = host
                .call(0, &[Value::Int(5)], FunctionOptions::new(&[]))
                .expect_err("must fail");
            assert_function_error(err, MessageFunctionError::BadOption);
        }
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
        assert_number_rendered(&mut host, out, "9,223,372,036,854,775,807.00");
    }

    #[test]
    fn resolved_numbers_use_locale_decimal_symbols() {
        let locale = Locale::from_str("fr-FR").expect("locale");
        let number = ResolvedNumber::new(
            NumberValue::Integer(1_234),
            NumberFormatOptions::DEFAULT,
            NumberSelection::None,
            false,
        );
        assert_eq!(
            render_resolved_number(&locale, &mut IcuFormatterCache::default(), &number)
                .expect("formatted"),
            "1\u{202f}234"
        );
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
        let one_category = host.project_select(0, &one).expect("selected");
        let other_category = host.project_select(0, &other).expect("selected");
        assert_selector_result(host.catalog, one_category, "one");
        assert_selector_result(host.catalog, other_category, "other");
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
        let two_category = host.project_select(0, &two).expect("selected");
        assert_selector_result(host.catalog, two_category, "two");
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
    fn integer_call_select_resolves_string_operand_before_projection() {
        let mut host = builtin_host(&["integer"]);
        let out = host
            .call_select(
                0,
                &[Value::Str("1.5".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("selected");
        assert_selector_result(host.catalog, out, "one");
    }

    #[test]
    fn offset_call_select_applies_adjustment_before_projection() {
        let mut host = builtin_host(&["offset add=1"]);
        let out = host
            .call_select(0, &[Value::Str("0".to_string())], FunctionOptions::new(&[]))
            .expect("selected");
        assert_selector_result(host.catalog, out, "one");

        let decimal = Decimal::from_str("0.0").expect("decimal");
        let out = host
            .call_select(0, &[Value::from(decimal)], FunctionOptions::new(&[]))
            .expect("selected");
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
        assert_number_rendered(&mut host, out, "50%");
    }

    #[test]
    fn number_style_percent_preserves_large_integer_precision() {
        let mut host = builtin_host(&["number style=percent"]);
        let out = host
            .call(0, &[Value::Int(i64::MAX)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, out, "922,337,203,685,477,580,700%");
    }

    #[test]
    fn number_style_percent_with_fraction_digits() {
        let mut host = builtin_host(&["number style=percent minimumFractionDigits=1"]);
        let out = host
            .call(0, &[Value::Float(0.123)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, out, "12.3%");
    }

    #[test]
    fn number_style_percent_composes_number_options() {
        let mut host = builtin_host(&[
            "number style=percent maximumFractionDigits=1 useGrouping=never",
            "number style=percent minimumIntegerDigits=4",
            "number style=percent minimumSignificantDigits=4",
            "number style=percent numberingSystem=arab",
        ]);

        let cases = [
            (0, Value::Float(12.3456), "1234.6%"),
            (1, Value::Float(0.5), "0,050%"),
            (2, Value::Float(0.5), "50.00%"),
            (3, Value::Float(0.5), "٥٠%"),
        ];
        for (fn_id, input, expected) in cases {
            let out = host
                .call(fn_id, &[input], FunctionOptions::new(&[]))
                .expect("formatted");
            assert_number_rendered(&mut host, out, expected);
        }
    }

    #[test]
    fn number_style_percent_is_inherited_and_can_be_reset() {
        let mut host = builtin_host(&["number style=percent", "number", "number style=decimal"]);
        let percent = host
            .call(0, &[Value::Float(0.5)], FunctionOptions::new(&[]))
            .expect("percent");
        let inherited = host
            .call(
                1,
                core::slice::from_ref(&percent),
                FunctionOptions::new(&[]),
            )
            .expect("inherited");
        assert_number_rendered(&mut host, inherited, "50%");
        let decimal = host
            .call(2, &[percent], FunctionOptions::new(&[]))
            .expect("reset");
        assert_number_rendered(&mut host, decimal, "0.5");
    }

    #[test]
    fn integer_style_percent_formats_the_resolved_integer() {
        let mut host = builtin_host(&["integer style=percent"]);
        let out = host
            .call(0, &[Value::Float(0.42)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert_number_rendered(&mut host, out, "0%");
    }

    #[test]
    fn builtin_host_caches_icu_formatters_after_first_use() {
        let mut currency_host = builtin_host(&["currency currency=USD"]);
        assert!(currency_host.icu_formatters.currency.is_none());
        let _ = currency_host
            .call(0, &[Value::Int(42)], FunctionOptions::new(&[]))
            .expect("formatted");
        assert!(currency_host.icu_formatters.currency.is_some());

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
        assert!(time_host.icu_formatters.time.minute.is_none());
        let _ = time_host
            .call(
                0,
                &[Value::Str("2024-05-01T14:30:00".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        assert!(time_host.icu_formatters.time.minute.is_some());

        let mut datetime_host = builtin_host(&["datetime"]);
        assert!(
            datetime_host
                .icu_formatters
                .datetime
                .medium
                .minute
                .is_none()
        );
        let _ = datetime_host
            .call(
                0,
                &[Value::Str("2024-05-01T14:30:00".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        assert!(
            datetime_host
                .icu_formatters
                .datetime
                .medium
                .minute
                .is_some()
        );
    }

    #[test]
    fn offset_rejects_large_integer_that_exceeds_checked_range() {
        let mut host = builtin_host(&["offset add=1"]);
        let err = host
            .call(0, &[Value::Int(i64::MAX)], FunctionOptions::new(&[]))
            .expect("i128 range");
        assert_number_rendered(&mut host, err, "9,223,372,036,854,775,808");

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
        assert!(short_host.icu_formatters.datetime.short.minute.is_some());
        assert!(long_host.icu_formatters.datetime.short.second.is_some());
    }

    #[test]
    fn structured_builtin_values_expose_reconstructable_semantic_fields() {
        let mut host = builtin_host(&[
            "number minimumFractionDigits=2",
            "percent",
            "currency currency=USD",
            "datetime year=numeric month=long day=numeric",
            "datetime hour=2-digit minute=2-digit second=2-digit fractionalSecondDigits=2 hourCycle=h23",
        ]);
        let values = [
            host.call(
                0,
                &[Value::Str("12345.67".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("number"),
            host.call(
                1,
                &[Value::Str("0.42".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("percent"),
            host.call(
                2,
                &[Value::Str("12345.67".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("currency"),
            host.call(
                3,
                &[Value::Str("2024-05-01T14:30:45".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("datetime"),
            host.call(
                4,
                &[Value::Str("2024-05-01T14:30:45.123".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("datetime fraction"),
        ];

        let expected = [
            (
                FormattedValueKind::Number,
                &["integer", "group", "decimal", "fraction"][..],
            ),
            (FormattedValueKind::Number, &["integer", "percentSign"][..]),
            (
                FormattedValueKind::Number,
                &["currency", "integer", "group", "decimal", "fraction"][..],
            ),
            (FormattedValueKind::DateTime, &["month", "day", "year"][..]),
            (
                FormattedValueKind::DateTime,
                &["hour", "minute", "second", "fractionalSecond"][..],
            ),
        ];

        for (value, (kind, required_fields)) in values.iter().zip(expected) {
            let sink = host.structured(value);
            assert_eq!(sink.kind, Some(kind));
            let reconstructed: String = sink
                .fields
                .iter()
                .map(|(_, value)| value.as_str())
                .collect();
            assert_eq!(reconstructed, sink.value);
            for required in required_fields {
                assert!(
                    sink.fields.iter().any(|(actual, _)| actual == required),
                    "missing {required:?} in {:?}",
                    sink.fields,
                );
            }
        }
    }

    #[test]
    fn structured_percent_uses_locale_decimal_separator() {
        let locale = Locale::from_str("de-DE").expect("locale");
        let mut format = NumberFormatOptions::DEFAULT;
        format.style = NumberStyle::Percent;
        format.minimum_fraction_digits = Some(3);
        format.maximum_fraction_digits = Some(3);
        let number = ResolvedNumber::new(
            parse_number_text("0.42").expect("number"),
            format,
            NumberSelection::None,
            false,
        );
        let rendered =
            render_resolved_number_inner(&locale, &mut IcuFormatterCache::default(), &number, true)
                .expect("formatted");
        assert_eq!(rendered.text, "42,000 %");
        assert!(
            rendered
                .fields
                .iter()
                .any(|field| field.kind == "decimal" && field.value == ",")
        );
        assert!(!rendered.fields.iter().any(|field| field.kind == "group"));
    }

    #[test]
    fn datetime_field_options_control_the_dynamic_formatter() {
        let input = Value::Str("2024-05-01T14:30:45.123".to_string());
        let cases = [
            ("datetime year=numeric", "2024"),
            ("datetime month=long day=numeric", "May 1"),
            ("datetime weekday=long", "Wednesday"),
            (
                "datetime hour=2-digit minute=2-digit hourCycle=h23",
                "14:30",
            ),
        ];
        for (function, expected) in cases {
            let mut host = builtin_host(&[function]);
            let value = host
                .call(0, core::slice::from_ref(&input), FunctionOptions::new(&[]))
                .expect("formatted");
            let Value::Formatted(value) = value else {
                panic!("datetime must produce a resolved formatted value");
            };
            assert_eq!(value.text(), expected, "function={function}");
            assert!(host.icu_formatters.datetime.fields.is_some());
        }
    }

    #[test]
    fn datetime_fractional_second_digits_are_rendered() {
        let mut host = builtin_host(&[
            "datetime hour=numeric minute=2-digit second=2-digit fractionalSecondDigits=2 hourCycle=h23",
        ]);
        let value = host
            .call(
                0,
                &[Value::Str("2024-05-01T14:30:45.123".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        let Value::Formatted(value) = value else {
            panic!("datetime must produce a resolved formatted value");
        };
        assert_eq!(value.text(), "14:30:45.12");
    }

    #[test]
    fn datetime_rejects_invalid_or_unsupported_field_requests() {
        let input = [Value::Str("2024-05-01T14:30:45".to_string())];
        let mut invalid = builtin_host(&["datetime month=huge"]);
        let error = invalid
            .call(0, &input, FunctionOptions::new(&[]))
            .expect_err("invalid field value");
        assert_function_error(error, MessageFunctionError::BadOption);

        for function in [
            "datetime dateStyle=short year=numeric",
            "datetime year=numeric day=numeric",
            "datetime hour=numeric hourCycle=h24",
        ] {
            let mut host = builtin_host(&[function]);
            let error = host
                .call(0, &input, FunctionOptions::new(&[]))
                .expect_err("unsupported field request");
            assert!(matches!(
                error,
                HostCallError::Function(
                    MessageFunctionError::BadOption
                        | MessageFunctionError::UnsupportedOperation(
                            UnsupportedOperation::DateTimeFormattingForLocale
                        )
                )
            ));
        }
    }

    #[test]
    fn datetime_time_zone_name_uses_the_operand_offset() {
        let mut host = builtin_host(&[
            "datetime hour=2-digit minute=2-digit hourCycle=h23 timeZoneName=longOffset",
        ]);
        let value = host
            .call(
                0,
                &[Value::Str("2024-05-01T14:30:45+07:00".to_string())],
                FunctionOptions::new(&[]),
            )
            .expect("formatted");
        let Value::Formatted(value) = value else {
            panic!("datetime must produce a resolved formatted value");
        };
        assert!(value.text().contains("14:30"));
        assert!(value.text().contains("GMT+07:00"), "{}", value.text());
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
        let Value::Formatted(without_seconds) = without_seconds else {
            panic!("time must produce a resolved formatted value");
        };
        let Value::Formatted(with_seconds) = with_seconds else {
            panic!("time must produce a resolved formatted value");
        };
        assert_eq!(without_seconds.text(), with_seconds.text());
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
        validate_digit_range_relationship(minimum.map(usize::from), maximum.map(usize::from))?;
        let text = minimum.map_or_else(
            || number.text(),
            |minimum| {
                format_int_or_decimal_with_min_fraction_digits(number.text(), usize::from(minimum))
            },
        );
        let text = apply_maximum_fraction_digits(text, maximum.map(usize::from));
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
                    NumberValue::Decimal(Box::new(Decimal::from_str(text).expect("decimal"))),
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
                    NumberValue::Decimal(Box::new(Decimal::from_str("0.5").expect("decimal"))),
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
        assert_eq!(multiply_decimal_by_100("0.123").expect("percent"), "12.3");
        assert_eq!(
            multiply_decimal_by_100("-1.234").expect("percent"),
            "-123.4"
        );
        assert_eq!(
            multiply_decimal_by_100("9007199254740993").expect("percent"),
            "900719925474099300"
        );
    }
}
