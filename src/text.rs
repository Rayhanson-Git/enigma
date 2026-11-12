//! Text conventions (00 §3): lines, whitespace, case, display/quoted, float text, stamps.
use crate::yaml::Y;
use chrono::{NaiveDate, NaiveDateTime, Timelike};
use unicode_general_category::{GeneralCategory as G, get_general_category};

/// Whitespace per 00 §3.3: Unicode White_Space plus U+001C..U+001F.
pub fn is_ws(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

pub fn strip(s: &str) -> &str {
    s.trim_matches(is_ws)
}

pub fn collapse_ws(s: &str) -> String {
    s.split(is_ws).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")
}

pub fn casefold(s: &str) -> String {
    caseless::default_case_fold_str(s)
}

fn is_line_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// The universal line split (00 §3.2), terminators dropped.
pub fn split_lines(s: &str) -> Vec<&str> {
    split_keep_ends(s).into_iter().map(without_end).collect()
}

/// Lines with their terminators kept.
pub fn split_keep_ends(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if is_line_break(c) {
            let mut end = i + c.len_utf8();
            if c == '\r' && matches!(it.peek(), Some((_, '\n'))) {
                it.next();
                end += 1;
            }
            out.push(&s[start..end]);
            start = end;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// Strips one trailing terminator from a line produced by split_keep_ends.
pub fn without_end(line: &str) -> &str {
    if let Some(l) = line.strip_suffix("\r\n") {
        return l;
    }
    match line.chars().last() {
        Some(c) if is_line_break(c) => &line[..line.len() - c.len_utf8()],
        _ => line,
    }
}

/// Normalise line endings of a text read (00 §3.1).
pub fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// Lenient decode: invalid UTF-8 byte sequences are dropped.
pub fn decode_lenient(bytes: &[u8]) -> String {
    bytes.utf8_chunks().map(|c| c.valid()).collect()
}

/// Strict decode with the DecodeError wording of 00 §5.
pub fn decode_strict(bytes: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(bytes).map_err(|e| {
        let pos = e.valid_up_to();
        let b = bytes[pos];
        let reason = match e.error_len() {
            None => "the text ends mid-character",
            Some(_) if !(0xC2..=0xF4).contains(&b) => "not a valid first byte",
            Some(_) => "not a valid continuation byte",
        };
        format!("invalid UTF-8: byte 0x{b:02x} at offset {pos}: {reason}")
    })
}

/// Float text (16 §3.4): shortest round-trip digits, repr layout.
pub fn float_text(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let sci = format!("{:e}", f.abs()); // e.g. "1.2345e-5", shortest round-trip
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let sign = if f < 0.0 { "-" } else { "" };
    if (-4..16).contains(&exp) {
        let s = if exp < 0 {
            format!("0.{}{}", "0".repeat((-exp - 1) as usize), digits)
        } else {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                format!("{}{}.0", digits, "0".repeat(int_len - digits.len()))
            } else {
                format!("{}.{}", &digits[..int_len], &digits[int_len..])
            }
        };
        format!("{sign}{s}")
    } else {
        let rest = if digits.len() > 1 { format!(".{}", &digits[1..]) } else { String::new() };
        let es = if exp < 0 { '-' } else { '+' };
        format!("{sign}{}{rest}e{es}{:02}", &digits[..1], exp.abs())
    }
}

fn offset_text(off: i32) -> String {
    let sign = if off < 0 { '-' } else { '+' };
    let a = off.abs();
    format!("{sign}{:02}:{:02}", a / 3600, (a % 3600) / 60)
}

pub fn datetime_display(dt: &NaiveDateTime, off: Option<i32>) -> String {
    let mut s = dt.format("%Y-%m-%d %H:%M:%S").to_string();
    let us = dt.nanosecond() / 1000;
    if us != 0 {
        s.push_str(&format!(".{us:06}"));
    }
    if let Some(o) = off {
        s.push_str(&offset_text(o));
    }
    s
}

pub fn date_display(d: &NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// display(v) of 00 §3.4.
pub fn display(v: &Y) -> String {
    match v {
        Y::Str(s) => s.clone(),
        Y::Seq(_) | Y::Map(_) => quoted(v),
        _ => scalar_text(v),
    }
}

fn scalar_text(v: &Y) -> String {
    match v {
        Y::Null => "null".into(),
        Y::Bool(b) => if *b { "true" } else { "false" }.into(),
        Y::Int(i) => i.to_string(),
        Y::Float(f) => float_text(*f),
        Y::Date(d) => date_display(d),
        Y::DateTime(dt, off) => datetime_display(dt, *off),
        Y::Str(s) => s.clone(),
        Y::Other(s) => s.clone(),
        Y::Seq(_) | Y::Map(_) => quoted(v),
    }
}

/// quoted(v) of 00 §3.4.
pub fn quoted(v: &Y) -> String {
    match v {
        Y::Str(s) => quote_str(s),
        Y::Date(d) => {
            use chrono::Datelike;
            format!("datetime.date({}, {}, {})", d.year(), d.month(), d.day())
        }
        Y::DateTime(dt, off) => {
            use chrono::Datelike;
            let mut s = format!("datetime.datetime({}, {}, {}, {}, {}", dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute());
            let us = dt.nanosecond() / 1000;
            if dt.second() != 0 || us != 0 {
                s.push_str(&format!(", {}", dt.second()));
            }
            if us != 0 {
                s.push_str(&format!(", {us}"));
            }
            match off {
                None => {}
                Some(0) => s.push_str(", tzinfo=datetime.timezone.utc"),
                Some(o) if *o > 0 => s.push_str(&format!(", tzinfo=datetime.timezone(datetime.timedelta(seconds={o}))")),
                Some(o) => s.push_str(&format!(
                    ", tzinfo=datetime.timezone(datetime.timedelta(days=-1, seconds={}))",
                    86400 - o.abs()
                )),
            }
            s.push(')');
            s
        }
        Y::Seq(items) => format!("[{}]", items.iter().map(quoted).collect::<Vec<_>>().join(", ")),
        Y::Map(pairs) => format!(
            "{{{}}}",
            pairs.iter().map(|(k, v)| format!("{}: {}", quoted(k), quoted(v))).collect::<Vec<_>>().join(", ")
        ),
        _ => scalar_text(v),
    }
}

fn printable(c: char) -> bool {
    if c == ' ' {
        return true;
    }
    !matches!(
        get_general_category(c),
        G::Control | G::Format | G::Surrogate | G::PrivateUse | G::Unassigned | G::LineSeparator | G::ParagraphSeparator | G::SpaceSeparator
    )
}

pub fn quote_str(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::from(q);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c if printable(c) => out.push(c),
            c if (c as u32) < 0x100 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c if (c as u32) < 0x10000 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push_str(&format!("\\U{:08x}", c as u32)),
        }
    }
    out.push(q);
    out
}

pub fn minute_stamp(t: &NaiveDateTime) -> String {
    t.format("%Y-%m-%d %H:%M").to_string()
}
pub fn day(t: &NaiveDateTime) -> String {
    t.format("%Y-%m-%d").to_string()
}
pub fn clock(t: &NaiveDateTime) -> String {
    t.format("%H:%M").to_string()
}
pub fn now() -> NaiveDateTime {
    chrono::Local::now().naive_local()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_text_matches_spec() {
        for (f, s) in [(1.0, "1.0"), (0.5, "0.5"), (1e-5, "1e-05"), (1.5e-5, "1.5e-05"), (1e16, "1e+16"), (0.0001, "0.0001"), (123.456, "123.456"), (1234567890123456.0, "1234567890123456.0"), (-0.0, "-0.0"), (0.1, "0.1")] {
            assert_eq!(float_text(f), s);
        }
    }

    #[test]
    fn lines_split_universally() {
        assert_eq!(split_lines("a\r\nb\rc\u{2028}d\n"), vec!["a", "b", "c", "d"]);
        assert!(split_lines("").is_empty());
        assert_eq!(split_keep_ends("a\r\nb"), vec!["a\r\n", "b"]);
    }

    #[test]
    fn quoted_strings() {
        assert_eq!(quote_str("weekly"), "'weekly'");
        assert_eq!(quote_str("it's"), "\"it's\"");
        assert_eq!(quote_str("a\tb\u{0}"), "'a\\tb\\x00'");
        assert_eq!(quoted(&Y::Seq(vec![Y::Str("a".into()), Y::Int(2)])), "['a', 2]");
    }

    #[test]
    fn decode_messages() {
        assert_eq!(decode_strict(b"ab\xff").unwrap_err(), "invalid UTF-8: byte 0xff at offset 2: not a valid first byte");
        assert_eq!(decode_strict(b"\xe4A").unwrap_err(), "invalid UTF-8: byte 0xe4 at offset 0: not a valid continuation byte");
        assert_eq!(decode_strict(b"a\xe4").unwrap_err(), "invalid UTF-8: byte 0xe4 at offset 1: the text ends mid-character");
        assert_eq!(decode_lenient(b"a\xffb"), "ab");
    }
}
