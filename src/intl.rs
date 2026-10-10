//! `Intl` for r8r's QuickJS scripts (Code nodes and `{{ }}` expressions).
//! QuickJS has none, so the locale work is done natively with ICU4X and a
//! small JavaScript layer (`PRELUDE`) shapes it into the standard classes.
//! Values cross the boundary as strings and JSON only.

use fixed_decimal::{FixedDecimal, FloatPrecision};
use icu::calendar::{DateTime as IcuDateTime, Gregorian};
use icu::datetime::options::{components, length, preferences, DateTimeFormatterOptions};
use icu::datetime::TypedDateTimeFormatter;
use icu::collator::{CaseLevel, Collator, CollatorOptions, Numeric, Strength};
use icu::decimal::options::{FixedDecimalFormatterOptions, GroupingStrategy};
use icu::decimal::FixedDecimalFormatter;
use icu::locid::Locale;
use icu::plurals::{PluralCategory, PluralRuleType, PluralRules};
use icu::segmenter::{GraphemeClusterSegmenter, SentenceSegmenter, WordSegmenter};
use icu_experimental::dimension::currency::formatter::{CurrencyCode, CurrencyFormatter};
use icu_experimental::dimension::currency::options::Width;

/// The JavaScript side: the `Intl` classes over the `__r8r_intl` natives.
const PRELUDE: &str = include_str!("intl_prelude.js");

/// Whether `script` may use `Intl` (directly or through the locale-aware
/// built-ins): setting it up costs more than most expressions themselves.
pub fn is_used_by(script: &str) -> bool {
    script.contains("Intl") || script.contains("toLocale") || script.contains("localeCompare")
}

/// Installs `Intl` into a fresh QuickJS context.
pub fn install(js: &rquickjs::Ctx<'_>) -> rquickjs::Result<()> {
    let natives = rquickjs::Object::new(js.clone())?;
    natives.set("canonicalLocale", rquickjs::Function::new(js.clone(), canonical_locale_js)?)?;
    natives.set("segment", rquickjs::Function::new(js.clone(), segment_js)?)?;
    natives.set("formatNumber", rquickjs::Function::new(js.clone(), format_number_js)?)?;
    natives.set("pluralSelect", rquickjs::Function::new(js.clone(), plural_select_js)?)?;
    natives.set("collate", rquickjs::Function::new(js.clone(), collate_js)?)?;
    natives.set("formatDate", rquickjs::Function::new(js.clone(), format_date_js)?)?;
    natives.set("formatRelative", rquickjs::Function::new(js.clone(), format_relative_js)?)?;
    natives.set("formatList", rquickjs::Function::new(js.clone(), format_list_js)?)?;
    natives.set("localTimeZone", rquickjs::Function::new(js.clone(), local_time_zone)?)?;
    natives.set("isTimeZone", rquickjs::Function::new(js.clone(), |name: String| name.parse::<chrono_tz::Tz>().is_ok())?)?;
    js.globals().set("__r8r_intl", natives)?;
    js.eval::<(), _>(PRELUDE)
}

/// The canonical form of a BCP 47 tag ("EN-us" -> "en-US"), or "" if invalid.
pub fn canonical_locale(tag: &str) -> String {
    tag.parse::<Locale>().map(|l| l.to_string()).unwrap_or_default()
}

fn canonical_locale_js(tag: String) -> String {
    canonical_locale(&tag)
}

/// The most text one call may segment (UTF-16 units). The natives allocate
/// outside QuickJS's memory cap, so their inputs are capped here instead.
pub const MAX_SEGMENT_TEXT: usize = 1_000_000;
/// The most items, and characters in all, one list may format.
pub const MAX_LIST_ITEMS: usize = 100_000;
pub const MAX_LIST_TEXT: usize = 2_000_000;

/// Segments of `text` as JSON `[[start, end, isWordLike], ...]`, with
/// UTF-16 offsets as JavaScript strings use. `isWordLike` is null except
/// for words. Written straight into one string to stay small.
pub fn segment(text: &str, granularity: &str) -> Result<String, String> {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    if utf16.len() > MAX_SEGMENT_TEXT {
        return Err(format!("text too long for Intl.Segmenter in r8r ({} characters, at most {MAX_SEGMENT_TEXT})", utf16.len()));
    }
    use std::fmt::Write;
    let mut out = String::from("[");
    let mut push = |start: usize, end: usize, word_like: Option<bool>| {
        if end > start {
            if out.len() > 1 {
                out.push(',');
            }
            let w = match word_like {
                Some(true) => "true",
                Some(false) => "false",
                None => "null",
            };
            let _ = write!(out, "[{start},{end},{w}]");
        }
    };
    match granularity {
        "word" => {
            let segmenter = WordSegmenter::new_auto();
            let mut breaks = segmenter.segment_utf16(&utf16);
            let mut start = breaks.next().unwrap_or(0);
            while let Some(end) = breaks.next() {
                push(start, end, Some(breaks.is_word_like()));
                start = end;
            }
        }
        "sentence" => {
            let segmenter = SentenceSegmenter::new();
            let mut breaks = segmenter.segment_utf16(&utf16);
            let mut start = breaks.next().unwrap_or(0);
            for end in breaks {
                push(start, end, None);
                start = end;
            }
        }
        _ => {
            let segmenter = GraphemeClusterSegmenter::new();
            let mut breaks = segmenter.segment_utf16(&utf16);
            let mut start = breaks.next().unwrap_or(0);
            for end in breaks {
                push(start, end, None);
                start = end;
            }
        }
    }
    out.push(']');
    Ok(out)
}

fn segment_js(text: String, granularity: String) -> rquickjs::Result<String> {
    segment(&text, &granularity).map_err(|e| rquickjs::Error::new_from_js_message("text", "segments", e))
}

/// `tag` as an ICU locale; the prelude has already canonicalized it.
fn locale(tag: &str) -> Locale {
    tag.parse().unwrap_or_else(|_| "en-US".parse().expect("valid"))
}

/// `value` rounded half away from zero to at most `max` and padded to at
/// least `min` fraction digits, as JavaScript's default rounding does.
fn decimal(value: f64, min: i16, max: i16) -> FixedDecimal {
    let mut d = FixedDecimal::try_from_f64(value, FloatPrecision::Floating).unwrap_or_default();
    d.half_expand(-max);
    d.trim_end();
    d.pad_end(-min);
    d
}

/// Number options as the prelude resolves them.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct NumberOptions {
    style: String,
    currency: Option<String>,
    currency_display: Option<String>,
    minimum_fraction_digits: i16,
    maximum_fraction_digits: i16,
    use_grouping: bool,
}

/// Where a language puts the percent sign (CLDR), as `before`/`after` the number.
fn percent_affixes(language: &str) -> (&'static str, &'static str) {
    match language {
        "tr" => ("%", ""),
        "ar" => ("", "\u{200e}%\u{200e}"),
        "ru" | "de" | "fr" | "es" | "sv" | "fi" | "nb" | "no" | "nn" | "da" | "cs" | "be" | "ro" | "lt" | "sk" | "sl" | "hr" => ("", "\u{a0}%"),
        _ => ("", "%"),
    }
}

/// A finite number formatted for `tag` (NaN and infinities are the prelude's).
pub fn format_number(value: f64, tag: &str, options: &str) -> Result<String, String> {
    let o: NumberOptions = serde_json::from_str(options).map_err(|e| format!("bad number options: {e}"))?;
    let loc = locale(tag);
    let mut fdf_options = FixedDecimalFormatterOptions::default();
    fdf_options.grouping_strategy = if o.use_grouping { GroupingStrategy::Auto } else { GroupingStrategy::Never };
    // Percent and currency patterns are written for the number's size; a
    // negative one gets the locale's minus sign in front, as CLDR's default
    // negative pattern does ("-$1.00", Swedish "−1,00 €").
    if o.style != "decimal" && value.is_sign_negative() && value != 0.0 {
        let minus = FixedDecimalFormatter::try_new(&(&loc).into(), FixedDecimalFormatterOptions::default())
            .map_err(|e| e.to_string())?
            .format(&FixedDecimal::from(-1))
            .to_string()
            .replace('1', "");
        let positive = format_number(-value, tag, options)?;
        // Dutch currency puts the minus between symbol and number (CLDR "¤ -#,##0.00").
        if o.style == "currency" && loc.id.language.as_str() == "nl" {
            if let Some(at) = positive.find(|c: char| c.is_ascii_digit()) {
                return Ok(format!("{}{minus}{}", &positive[..at], &positive[at..]));
            }
        }
        // Bidi marks that open the pattern (Arabic, Hebrew) stay first.
        let marks = positive.len() - positive.trim_start_matches(['\u{200e}', '\u{200f}', '\u{61c}']).len();
        return Ok(format!("{}{minus}{}", &positive[..marks], &positive[marks..]));
    }
    match o.style.as_str() {
        "currency" => {
            let code = o.currency.unwrap_or_default().to_ascii_uppercase();
            let code = tinystr::TinyAsciiStr::<3>::from_str(&code).map_err(|_| format!("Invalid currency code : {code}"))?;
            let width = if o.currency_display.as_deref() == Some("narrowSymbol") { Width::Narrow } else { Width::Short };
            let formatter = CurrencyFormatter::try_new(&(&loc).into(), width.into()).map_err(|e| e.to_string())?;
            let d = decimal(value, o.minimum_fraction_digits, o.maximum_fraction_digits);
            let formatted = writeable::Writeable::write_to_string(&formatter.format_fixed_decimal(&d, CurrencyCode(code))).into_owned();
            // icu_experimental 0.1 writes the number itself unlocalized
            // ("1234.50" in every locale): it only places the symbol, so put
            // the number in as the stable formatter writes it for the locale.
            let localized = FixedDecimalFormatter::try_new(&(&loc).into(), fdf_options).map_err(|e| e.to_string())?.format(&d).to_string();
            let unlocalized = d.to_string();
            Ok(currency_spacing(&formatted.replacen(&unlocalized, &localized, 1)))
        }
        "percent" => {
            let formatter = FixedDecimalFormatter::try_new(&(&loc).into(), fdf_options).map_err(|e| e.to_string())?;
            let d = decimal(value * 100.0, o.minimum_fraction_digits, o.maximum_fraction_digits);
            let (before, after) = percent_affixes(loc.id.language.as_str());
            Ok(format!("{before}{}{after}", formatter.format(&d)))
        }
        _ => {
            let formatter = FixedDecimalFormatter::try_new(&(&loc).into(), fdf_options).map_err(|e| e.to_string())?;
            Ok(formatter.format(&decimal(value, o.minimum_fraction_digits, o.maximum_fraction_digits)).to_string())
        }
    }
}

fn format_number_js(value: f64, tag: String, options: String) -> rquickjs::Result<String> {
    format_number(value, &tag, &options).map_err(|e| rquickjs::Error::new_from_js_message("number", "string", e))
}

/// CLDR currency spacing: a symbol ending (or starting) with a letter is
/// kept apart from the digits by a no-break space ("RUB 1.00", "1.00 RUB").
fn currency_spacing(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 2);
    for (i, c) in chars.iter().enumerate() {
        out.push(*c);
        if let Some(next) = chars.get(i + 1) {
            if (c.is_alphabetic() && next.is_ascii_digit()) || (c.is_ascii_digit() && next.is_alphabetic()) {
                out.push('\u{a0}');
            }
        }
    }
    out
}

/// Date options as the prelude resolves them (JavaScript's names and values).
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct DateOptions {
    date_style: Option<String>,
    time_style: Option<String>,
    era: Option<String>,
    weekday: Option<String>,
    year: Option<String>,
    month: Option<String>,
    day: Option<String>,
    hour: Option<String>,
    minute: Option<String>,
    second: Option<String>,
    hour_cycle: Option<String>,
    time_zone: Option<String>,
}

/// The server's time zone, as JavaScript reports it when none is given.
pub fn local_time_zone() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim_start_matches(':');
        if tz.parse::<chrono_tz::Tz>().is_ok() {
            return tz.to_string();
        }
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| p.to_str().and_then(|s| s.split("zoneinfo/").nth(1).map(String::from)))
        .filter(|tz| tz.parse::<chrono_tz::Tz>().is_ok())
        .unwrap_or_else(|| "UTC".into())
}

fn text(v: &str) -> components::Text {
    match v {
        "long" => components::Text::Long,
        "narrow" => components::Text::Narrow,
        _ => components::Text::Short,
    }
}

fn numeric(v: &str) -> components::Numeric {
    if v == "2-digit" { components::Numeric::TwoDigit } else { components::Numeric::Numeric }
}

fn style_date(v: &str) -> length::Date {
    match v {
        "full" => length::Date::Full,
        "long" => length::Date::Long,
        "medium" => length::Date::Medium,
        _ => length::Date::Short,
    }
}

fn style_time(v: &str) -> length::Time {
    match v {
        "full" => length::Time::Full,
        "long" => length::Time::Long,
        "medium" => length::Time::Medium,
        _ => length::Time::Short,
    }
}

/// The instant `epoch_ms` formatted for `tag`, in `timeZone` (default: the server's).
pub fn format_date(epoch_ms: f64, tag: &str, options: &str) -> Result<String, String> {
    let o: DateOptions = serde_json::from_str(options).map_err(|e| format!("bad date options: {e}"))?;
    let utc = chrono::DateTime::from_timestamp_millis(epoch_ms as i64).ok_or("Invalid time value")?;
    let zone = o.time_zone.clone().unwrap_or_else(local_time_zone);
    let tz: chrono_tz::Tz = zone.parse().map_err(|_| format!("Invalid time zone specified: {zone}"))?;
    let wall = utc.with_timezone(&tz).naive_local();
    use chrono::{Datelike, Timelike};
    let date = IcuDateTime::try_new_gregorian_datetime(wall.year(), wall.month() as u8, wall.day() as u8, wall.hour() as u8, wall.minute() as u8, wall.second() as u8)
        .map_err(|e| e.to_string())?;
    let formatter_options: DateTimeFormatterOptions = if o.date_style.is_some() || o.time_style.is_some() {
        let mut bag = length::Bag::empty();
        bag.date = o.date_style.as_deref().map(style_date);
        bag.time = o.time_style.as_deref().map(style_time);
        bag.into()
    } else {
        let mut bag = components::Bag::empty();
        bag.era = o.era.as_deref().map(text);
        bag.weekday = o.weekday.as_deref().map(text);
        bag.year = o.year.as_deref().map(|v| if v == "2-digit" { components::Year::TwoDigit } else { components::Year::Numeric });
        bag.month = o.month.as_deref().map(|v| match v {
            "2-digit" => components::Month::TwoDigit,
            "long" => components::Month::Long,
            "short" => components::Month::Short,
            "narrow" => components::Month::Narrow,
            _ => components::Month::Numeric,
        });
        bag.day = o.day.as_deref().map(|v| if v == "2-digit" { components::Day::TwoDigitDayOfMonth } else { components::Day::NumericDayOfMonth });
        bag.hour = o.hour.as_deref().map(numeric);
        bag.minute = o.minute.as_deref().map(numeric);
        bag.second = o.second.as_deref().map(numeric);
        // No hourCycle given: the locale's own clock (12h in en-US, ko, ar, ...).
        let hour_cycle = o.hour_cycle.clone().or_else(|| o.hour.as_ref().map(|_| locale_hour_cycle(tag)));
        bag.preferences = hour_cycle.as_deref().map(|h| {
            preferences::Bag::from_hour_cycle(match h {
                "h11" => preferences::HourCycle::H11,
                "h12" => preferences::HourCycle::H12,
                "h24" => preferences::HourCycle::H24,
                _ => preferences::HourCycle::H23,
            })
        });
        bag.into()
    };
    let formatter = TypedDateTimeFormatter::<Gregorian>::try_new_experimental(&(&locale(tag)).into(), formatter_options).map_err(|e| e.to_string())?;
    // V8 (Node, Chrome) writes the narrow no-break space of CLDR's date
    // patterns ("2:05 PM", "2026 г.") as a plain space, for web compatibility.
    Ok(formatter.format_to_string(&date).replace('\u{202f}', " "))
}

/// The locale's preferred clock: "h12" if its standard time pattern doesn't
/// show 14:00 as "14", else "h23".
fn locale_hour_cycle(tag: &str) -> String {
    let probe = IcuDateTime::try_new_gregorian_datetime(2000, 1, 1, 14, 0, 0);
    let formatter = TypedDateTimeFormatter::<Gregorian>::try_new(&(&locale(tag)).into(), length::Bag::from_time_style(length::Time::Short).into());
    match (probe, formatter) {
        (Ok(probe), Ok(formatter)) if !formatter.format_to_string(&probe).contains("14") => "h12".into(),
        _ => "h23".into(),
    }
}

fn format_date_js(epoch_ms: f64, tag: String, options: String) -> rquickjs::Result<String> {
    format_date(epoch_ms, &tag, &options).map_err(|e| rquickjs::Error::new_from_js_message("date", "string", e))
}

/// "in 3 days", "вчера": `value` `unit`s from now, as Intl.RelativeTimeFormat.
pub fn format_relative(value: f64, unit: &str, tag: &str, style: &str, auto: bool) -> Result<String, String> {
    use icu_experimental::relativetime::options::{Numeric, RelativeTimeFormatterOptions};
    use icu_experimental::relativetime::RelativeTimeFormatter as F;
    let loc: icu_provider::DataLocale = (&locale(tag)).into();
    let options = RelativeTimeFormatterOptions { numeric: if auto { Numeric::Auto } else { Numeric::Always } };
    let formatter = match (style, unit) {
        ("short", "second") => F::try_new_short_second(&loc, options),
        ("short", "minute") => F::try_new_short_minute(&loc, options),
        ("short", "hour") => F::try_new_short_hour(&loc, options),
        ("short", "day") => F::try_new_short_day(&loc, options),
        ("short", "week") => F::try_new_short_week(&loc, options),
        ("short", "month") => F::try_new_short_month(&loc, options),
        ("short", "quarter") => F::try_new_short_quarter(&loc, options),
        ("short", "year") => F::try_new_short_year(&loc, options),
        ("narrow", "second") => F::try_new_narrow_second(&loc, options),
        ("narrow", "minute") => F::try_new_narrow_minute(&loc, options),
        ("narrow", "hour") => F::try_new_narrow_hour(&loc, options),
        ("narrow", "day") => F::try_new_narrow_day(&loc, options),
        ("narrow", "week") => F::try_new_narrow_week(&loc, options),
        ("narrow", "month") => F::try_new_narrow_month(&loc, options),
        ("narrow", "quarter") => F::try_new_narrow_quarter(&loc, options),
        ("narrow", "year") => F::try_new_narrow_year(&loc, options),
        (_, "second") => F::try_new_long_second(&loc, options),
        (_, "minute") => F::try_new_long_minute(&loc, options),
        (_, "hour") => F::try_new_long_hour(&loc, options),
        (_, "day") => F::try_new_long_day(&loc, options),
        (_, "week") => F::try_new_long_week(&loc, options),
        (_, "month") => F::try_new_long_month(&loc, options),
        (_, "quarter") => F::try_new_long_quarter(&loc, options),
        _ => F::try_new_long_year(&loc, options),
    }
    .map_err(|e| e.to_string())?;
    let d = FixedDecimal::try_from_f64(value, FloatPrecision::Floating).map_err(|e| e.to_string())?;
    Ok(writeable::Writeable::write_to_string(&formatter.format(d)).into_owned())
}

fn format_relative_js(value: f64, unit: String, tag: String, style: String, auto: bool) -> rquickjs::Result<String> {
    format_relative(value, &unit, &tag, &style, auto).map_err(|e| rquickjs::Error::new_from_js_message("relative time", "string", e))
}

/// "a, b, and c" / "a, b или c": `items` joined as Intl.ListFormat.
pub fn format_list(items: &[String], tag: &str, kind: &str, style: &str) -> Result<String, String> {
    use icu::list::{ListFormatter, ListLength};
    let chars: usize = items.iter().map(|i| i.len()).sum();
    if items.len() > MAX_LIST_ITEMS || chars > MAX_LIST_TEXT {
        return Err(format!("list too long for Intl.ListFormat in r8r (at most {MAX_LIST_ITEMS} items, {MAX_LIST_TEXT} characters)"));
    }
    let loc: icu_provider::DataLocale = (&locale(tag)).into();
    let length = match style {
        "short" => ListLength::Short,
        "narrow" => ListLength::Narrow,
        _ => ListLength::Wide,
    };
    let formatter = match kind {
        "disjunction" => ListFormatter::try_new_or_with_length(&loc, length),
        "unit" => ListFormatter::try_new_unit_with_length(&loc, length),
        _ => ListFormatter::try_new_and_with_length(&loc, length),
    }
    .map_err(|e| e.to_string())?;
    Ok(formatter.format_to_string(items.iter()))
}

fn format_list_js(items: String, tag: String, kind: String, style: String) -> rquickjs::Result<String> {
    if items.len() > MAX_LIST_TEXT * 2 {
        return Err(rquickjs::Error::new_from_js_message("list", "string", "list too long for Intl.ListFormat in r8r".to_string()));
    }
    let items: Vec<String> = serde_json::from_str(&items).map_err(|e| rquickjs::Error::new_from_js_message("list", "string", e.to_string()))?;
    format_list(&items, &tag, &kind, &style).map_err(|e| rquickjs::Error::new_from_js_message("list", "string", e))
}

/// The plural category of `value` in `tag`'s language (`ordinal` or cardinal).
pub fn plural_select(value: f64, tag: &str, ordinal: bool) -> String {
    let kind = if ordinal { PluralRuleType::Ordinal } else { PluralRuleType::Cardinal };
    let Ok(rules) = PluralRules::try_new(&(&locale(tag)).into(), kind) else { return "other".into() };
    let d = FixedDecimal::try_from_f64(value.abs(), FloatPrecision::Floating).unwrap_or_default();
    match rules.category_for(&d) {
        PluralCategory::Zero => "zero",
        PluralCategory::One => "one",
        PluralCategory::Two => "two",
        PluralCategory::Few => "few",
        PluralCategory::Many => "many",
        PluralCategory::Other => "other",
    }
    .into()
}

fn plural_select_js(value: f64, tag: String, ordinal: bool) -> String {
    plural_select(value, &tag, ordinal)
}

/// -1, 0 or 1: how `a` sorts against `b` in `tag`'s language.
pub fn collate(a: &str, b: &str, tag: &str, sensitivity: &str, numeric: bool) -> i32 {
    let mut options = CollatorOptions::new();
    options.strength = Some(match sensitivity {
        "base" | "case" => Strength::Primary,
        "accent" => Strength::Secondary,
        _ => Strength::Tertiary,
    });
    if sensitivity == "case" {
        options.case_level = Some(CaseLevel::On);
    }
    if numeric {
        options.numeric = Some(Numeric::On);
    }
    match Collator::try_new(&(&locale(tag)).into(), options) {
        Ok(collator) => collator.compare(a, b) as i32,
        Err(_) => a.cmp(b) as i32,
    }
}

fn collate_js(a: String, b: String, tag: String, sensitivity: String, numeric: bool) -> i32 {
    collate(&a, &b, &tag, &sensitivity, numeric)
}

#[cfg(test)]
mod tests {
    use crate::expr::{eval_js, EvalContext};
    use std::collections::HashMap;

    fn js(script: &str) -> serde_json::Value {
        let ctx = EvalContext { json: serde_json::json!({}), items: &[], node_items: &HashMap::new(), workflow_name: "t", args: None };
        eval_js(script, &ctx).unwrap_or_else(|e| panic!("{script}: {e}"))
    }

    fn js_err(script: &str) -> String {
        let ctx = EvalContext { json: serde_json::json!({}), items: &[], node_items: &HashMap::new(), workflow_name: "t", args: None };
        eval_js(script, &ctx).unwrap_err().to_string()
    }

    #[test]
    #[ignore = "timing probe, run by hand"]
    fn cost_per_expression() {
        let ctx = EvalContext { json: serde_json::json!({"a": 1}), items: &[], node_items: &HashMap::new(), workflow_name: "t", args: None };
        let start = std::time::Instant::now();
        for _ in 0..500 {
            eval_js("$json.a + 1", &ctx).unwrap();
        }
        println!("PER EVAL without Intl: {:?}", start.elapsed() / 500);
        let start = std::time::Instant::now();
        for _ in 0..500 {
            eval_js("typeof Intl", &ctx).unwrap();
        }
        println!("PER EVAL using Intl: {:?}", start.elapsed() / 500);
    }

    #[test]
    #[ignore = "compare with Node.js by hand: R8R_INTL_CASES=<json list of expressions>"]
    fn evaluate_cases_for_comparison() {
        let path = std::env::var("R8R_INTL_CASES").unwrap();
        let cases: Vec<String> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let ctx = EvalContext { json: serde_json::json!({}), items: &[], node_items: &HashMap::new(), workflow_name: "t", args: None };
        let out: Vec<serde_json::Value> = cases
            .iter()
            .map(|c| match eval_js(c, &ctx) {
                Ok(v) => v,
                Err(e) => serde_json::json!(format!("ERROR {e}")),
            })
            .collect();
        std::fs::write(format!("{path}.r8r.json"), serde_json::to_string(&out).unwrap()).unwrap();
    }

    #[test]
    fn only_scripts_that_mention_it_pay_for_intl() {
        assert!(super::is_used_by("new Intl.Segmenter()"));
        assert!(super::is_used_by("(1234).toLocaleString('ru')"));
        assert!(super::is_used_by("a.localeCompare(b)"));
        assert!(!super::is_used_by("$json.a + 1"));
    }

    #[test]
    fn segments_russian_words_like_the_users_code() {
        assert_eq!(
            js("[...new Intl.Segmenter('ru', {granularity: 'word'}).segment('Привет, мир!')].filter(s => s.isWordLike).map(s => s.segment)"),
            serde_json::json!(["Привет", "мир"])
        );
    }

    #[test]
    fn word_segments_carry_index_input_and_word_likeness() {
        assert_eq!(
            js("[...new Intl.Segmenter('en', {granularity: 'word'}).segment('Hi, you')].map(s => [s.segment, s.index, s.isWordLike, s.input])"),
            serde_json::json!([["Hi", 0, true, "Hi, you"], [",", 2, false, "Hi, you"], [" ", 3, false, "Hi, you"], ["you", 4, true, "Hi, you"]])
        );
    }

    #[test]
    fn graphemes_keep_emoji_together_with_javascript_string_indexes() {
        // 👍🏽 is one grapheme of four UTF-16 units; "a" therefore starts at 4.
        assert_eq!(
            js("[...new Intl.Segmenter().segment('👍🏽a')].map(s => [s.segment, s.index])"),
            serde_json::json!([["👍🏽", 0], ["a", 4]])
        );
        assert_eq!(js("[...new Intl.Segmenter().segment('ab')][0].isWordLike"), serde_json::Value::Null);
    }

    #[test]
    fn sentences_and_containing() {
        assert_eq!(
            js("[...new Intl.Segmenter('en', {granularity: 'sentence'}).segment('One. Two!')].map(s => s.segment)"),
            serde_json::json!(["One. ", "Two!"])
        );
        assert_eq!(js("new Intl.Segmenter('en', {granularity: 'word'}).segment('Hi there').containing(5).segment"), serde_json::json!("there"));
        assert_eq!(js("new Intl.Segmenter('en', {granularity: 'word'}).segment('Hi').containing(9) === undefined"), serde_json::json!(true));
    }

    #[test]
    fn options_and_locales_are_checked_like_javascript() {
        assert!(js_err("new Intl.Segmenter('en', {granularity: 'line'})").contains("RangeError"));
        assert!(js_err("new Intl.Segmenter('not a locale!!')").contains("RangeError"));
        assert_eq!(js("new Intl.Segmenter('RU', {granularity: 'word'}).resolvedOptions()"), serde_json::json!({"locale": "ru", "granularity": "word"}));
        assert_eq!(js("Intl.getCanonicalLocales(['EN-us', 'ru'])"), serde_json::json!(["en-US", "ru"]));
        assert_eq!(js("typeof Intl.Segmenter.supportedLocalesOf"), serde_json::json!("function"));
    }

    // Expected values below are what Node.js 26 prints for the same calls.

    #[test]
    fn number_format_follows_each_locale() {
        assert_eq!(js("new Intl.NumberFormat('en-US').format(1234567.891)"), serde_json::json!("1,234,567.891"));
        assert_eq!(js("new Intl.NumberFormat('ru').format(1234567.891)"), serde_json::json!("1\u{a0}234\u{a0}567,891"));
        assert_eq!(js("new Intl.NumberFormat('de').format(1234567.891)"), serde_json::json!("1.234.567,891"));
        assert_eq!(js("new Intl.NumberFormat('fr').format(1234.5)"), serde_json::json!("1\u{202f}234,5"));
        assert_eq!(js("new Intl.NumberFormat('en', {maximumFractionDigits: 0}).format(2.5)"), serde_json::json!("3"));
        assert_eq!(js("new Intl.NumberFormat('en', {minimumFractionDigits: 2}).format(3)"), serde_json::json!("3.00"));
        assert_eq!(js("new Intl.NumberFormat('en', {useGrouping: false}).format(12345.6)"), serde_json::json!("12345.6"));
        assert_eq!(js("new Intl.NumberFormat('en').format(-0.5)"), serde_json::json!("-0.5"));
        assert_eq!(js("[new Intl.NumberFormat('en').format(NaN), new Intl.NumberFormat('en').format(Infinity)]"), serde_json::json!(["NaN", "∞"]));
        // `format` is bound, as in JavaScript, so it can be passed around.
        assert_eq!(js("[1000, 2000].map(new Intl.NumberFormat('en').format)"), serde_json::json!(["1,000", "2,000"]));
    }

    #[test]
    fn percent_follows_each_locale() {
        let got = js("['en','ru','de','fr','es','tr','ja','it','uk'].map(l => new Intl.NumberFormat(l, {style: 'percent'}).format(0.256))");
        assert_eq!(got, serde_json::json!(["26%", "26\u{a0}%", "26\u{a0}%", "26\u{a0}%", "26\u{a0}%", "%26", "26%", "26%", "26%"]));
    }

    #[test]
    fn currency_follows_each_locale() {
        assert_eq!(js("new Intl.NumberFormat('en-US', {style: 'currency', currency: 'USD'}).format(1234.5)"), serde_json::json!("$1,234.50"));
        assert_eq!(js("new Intl.NumberFormat('ru', {style: 'currency', currency: 'RUB'}).format(1234.5)"), serde_json::json!("1\u{a0}234,50\u{a0}₽"));
        assert_eq!(js("new Intl.NumberFormat('de', {style: 'currency', currency: 'EUR'}).format(1234.5)"), serde_json::json!("1.234,50\u{a0}€"));
        assert_eq!(js("new Intl.NumberFormat('en-US', {style: 'currency', currency: 'EUR'}).format(3)"), serde_json::json!("€3.00"));
        assert!(js_err("new Intl.NumberFormat('en', {style: 'currency'})").contains("TypeError"));
    }

    #[test]
    fn number_to_locale_string_uses_it() {
        assert_eq!(js("(1234.5).toLocaleString('ru')"), serde_json::json!("1\u{a0}234,5"));
        assert_eq!(js("(0.5).toLocaleString('en', {style: 'percent'})"), serde_json::json!("50%"));
    }

    #[test]
    fn plural_rules_follow_each_language() {
        assert_eq!(
            js("[1, 3, 5, 21].map(n => new Intl.PluralRules('ru').select(n))"),
            serde_json::json!(["one", "few", "many", "one"])
        );
        assert_eq!(
            js("[2, 3, 11].map(n => new Intl.PluralRules('en', {type: 'ordinal'}).select(n))"),
            serde_json::json!(["two", "few", "other"])
        );
        assert_eq!(js("new Intl.PluralRules('en').select(1.5)"), serde_json::json!("other"));
    }

    #[test]
    fn collator_sorts_like_the_language() {
        assert_eq!(js("['b', 'a', 'ä', 'z'].sort(new Intl.Collator('de').compare)"), serde_json::json!(["a", "ä", "b", "z"]));
        assert_eq!(js("['item10', 'item2', 'item1'].sort(new Intl.Collator('en', {numeric: true}).compare)"), serde_json::json!(["item1", "item2", "item10"]));
        assert_eq!(js("new Intl.Collator('en', {sensitivity: 'base'}).compare('a', 'Á')"), serde_json::json!(0));
        assert_eq!(js("'ё'.localeCompare('е', 'ru')"), serde_json::json!(1));
        assert_eq!(js("['б', 'а', 'в'].sort((x, y) => x.localeCompare(y, 'ru'))"), serde_json::json!(["а", "б", "в"]));
    }

    const D: &str = "new Date(Date.UTC(2026, 9, 8, 14, 5, 9))";

    #[test]
    fn dates_follow_each_locale_and_time_zone() {
        assert_eq!(js(&format!("{D}.toLocaleDateString('ru', {{timeZone: 'UTC'}})")), serde_json::json!("08.10.2026"));
        assert_eq!(js(&format!("{D}.toLocaleTimeString('en-US', {{timeZone: 'UTC'}})")), serde_json::json!("2:05:09 PM"));
        assert_eq!(js(&format!("{D}.toLocaleString('de', {{timeZone: 'Europe/Berlin'}})")), serde_json::json!("8.10.2026, 16:05:09"));
        assert_eq!(
            js(&format!("new Intl.DateTimeFormat('ru', {{dateStyle: 'full', timeZone: 'UTC'}}).format({D})")),
            serde_json::json!("четверг, 8 октября 2026 г.")
        );
        assert_eq!(
            js(&format!("new Intl.DateTimeFormat('en-GB', {{weekday: 'short', day: '2-digit', month: 'short', hour: '2-digit', minute: '2-digit', timeZone: 'Asia/Tokyo'}}).format({D})")),
            serde_json::json!("Thu 08 Oct, 23:05")
        );
        assert_eq!(js(&format!("{D}.toLocaleTimeString('en-US', {{hour12: false, timeZone: 'America/New_York'}})")), serde_json::json!("10:05:09"));
    }

    #[test]
    fn date_options_are_checked_like_javascript() {
        assert!(js_err("new Intl.DateTimeFormat('en', {timeZone: 'Mars/Base'})").contains("RangeError"));
        assert!(js_err("new Intl.DateTimeFormat('en', {dateStyle: 'short', hour: 'numeric'})").contains("TypeError"));
        assert!(js_err("new Intl.DateTimeFormat('en').format(new Date(NaN))").contains("RangeError"));
        assert_eq!(js("new Date(NaN).toLocaleString('en')"), serde_json::json!("Invalid Date"));
        assert_eq!(js("new Intl.DateTimeFormat('en', {timeZone: 'utc'}).resolvedOptions().timeZone"), serde_json::json!("UTC"));
    }

    #[test]
    fn relative_times_and_lists_follow_each_locale() {
        assert_eq!(js("new Intl.RelativeTimeFormat('ru', {numeric: 'auto'}).format(-1, 'day')"), serde_json::json!("вчера"));
        assert_eq!(js("new Intl.RelativeTimeFormat('en').format(3, 'months')"), serde_json::json!("in 3 months"));
        assert!(js_err("new Intl.RelativeTimeFormat('en').format(1, 'fortnight')").contains("RangeError"));
        assert_eq!(js("new Intl.ListFormat('ru').format(['a', 'b', 'c'])"), serde_json::json!("a, b и c"));
        assert_eq!(js("new Intl.ListFormat('en', {type: 'disjunction'}).format(['x', 'y'])"), serde_json::json!("x or y"));
    }

    #[test]
    fn huge_inputs_are_refused_instead_of_escaping_the_memory_cap() {
        // QuickJS caps a script at 64 MB, but natives allocate outside it:
        // a near-limit string must not turn into gigabytes of segments.
        let err = js_err("new Intl.Segmenter().segment('a'.repeat(1_000_001))");
        assert!(err.contains("RangeError") && err.contains("too long"), "{err}");
        let err = js_err("new Intl.ListFormat('en').format(Array(200_001).fill('x'))");
        assert!(err.contains("RangeError") && err.contains("too long"), "{err}");
        // Large-but-sane inputs still work.
        assert_eq!(js("[...new Intl.Segmenter().segment('ab'.repeat(50_000))].length"), serde_json::json!(100_000));
    }

    #[test]
    fn scripts_cannot_reach_the_natives_directly() {
        assert_eq!(js("typeof Intl.Segmenter === 'function' && typeof globalThis.__r8r_intl"), serde_json::json!("undefined"));
    }

    #[test]
    fn segment_output_stays_compact() {
        // Built as one string, not a JSON value per segment.
        assert_eq!(super::segment("ab", "grapheme").unwrap(), "[[0,1,null],[1,2,null]]");
    }
}
