//! The two JSON styles (16 §3): MCP JSON for tool results, File JSON for the
//! state dir and CLI output.
use crate::text::float_text;
use serde::Serialize;
use serde_json::Value;
use std::io::{self, Write};

/// Non-finite floats cannot live in a serde_json Value, so results carry them
/// as this marker string until rendering.
const NONFINITE: char = '\u{0}';

pub fn nonfinite(f: f64) -> Value {
    let token = if f.is_nan() { "NaN" } else if f > 0.0 { "Infinity" } else { "-Infinity" };
    Value::String(format!("{NONFINITE}{token}"))
}

fn is_marker(s: &str) -> bool {
    matches!(s.strip_prefix(NONFINITE), Some("NaN" | "Infinity" | "-Infinity"))
}

/// Turns the non-finite markers back into bare NaN / Infinity / -Infinity tokens.
fn bare_nonfinite(s: String) -> String {
    if !s.contains("\\u0000") {
        return s;
    }
    s.replace("\"\\u0000NaN\"", "NaN").replace("\"\\u0000Infinity\"", "Infinity").replace("\"\\u0000-Infinity\"", "-Infinity")
}

/// MCP JSON text (16 §3.1): serde_json pretty printing, bare NaN tokens.
pub fn mcp_text(v: &Value) -> String {
    bare_nonfinite(serde_json::to_string_pretty(v).unwrap_or_default())
}

/// The structuredContent form: non-finite markers become null.
pub fn structured(v: &Value) -> Value {
    match v {
        Value::String(s) if is_marker(s) => Value::Null,
        Value::Array(a) => Value::Array(a.iter().map(structured).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), structured(v))).collect()),
        other => other.clone(),
    }
}

/// serde_json formatter producing File JSON: ", " / ": " separators (or a
/// 2-space indent), repr-style float text, optional ASCII escaping.
struct FileFmt {
    indent: bool,
    ascii: bool,
    level: usize,
    has_value: bool,
}

impl FileFmt {
    fn sep<W: ?Sized + Write>(&self, w: &mut W, first: bool) -> io::Result<()> {
        if !first {
            w.write_all(if self.indent { b"," } else { b", " })?;
        }
        if self.indent {
            w.write_all(b"\n")?;
            w.write_all("  ".repeat(self.level).as_bytes())?;
        }
        Ok(())
    }
    fn close<W: ?Sized + Write>(&mut self, w: &mut W, c: &[u8]) -> io::Result<()> {
        self.level -= 1;
        if self.indent && self.has_value {
            w.write_all(b"\n")?;
            w.write_all("  ".repeat(self.level).as_bytes())?;
        }
        self.has_value = true;
        w.write_all(c)
    }
}

impl serde_json::ser::Formatter for FileFmt {
    fn begin_array<W: ?Sized + Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.level += 1;
        self.has_value = false;
        w.write_all(b"[")
    }
    fn end_array<W: ?Sized + Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.close(w, b"]")
    }
    fn begin_array_value<W: ?Sized + Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        self.sep(w, first)
    }
    fn end_array_value<W: ?Sized + Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }
    fn begin_object<W: ?Sized + Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.level += 1;
        self.has_value = false;
        w.write_all(b"{")
    }
    fn end_object<W: ?Sized + Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.close(w, b"}")
    }
    fn begin_object_key<W: ?Sized + Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        self.sep(w, first)
    }
    fn begin_object_value<W: ?Sized + Write>(&mut self, w: &mut W) -> io::Result<()> {
        w.write_all(b": ")
    }
    fn end_object_value<W: ?Sized + Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }
    fn write_f64<W: ?Sized + Write>(&mut self, w: &mut W, v: f64) -> io::Result<()> {
        w.write_all(float_text(v).as_bytes())
    }
    fn write_f32<W: ?Sized + Write>(&mut self, w: &mut W, v: f32) -> io::Result<()> {
        self.write_f64(w, v as f64)
    }
    fn write_string_fragment<W: ?Sized + Write>(&mut self, w: &mut W, frag: &str) -> io::Result<()> {
        if !self.ascii || frag.is_ascii() {
            return w.write_all(frag.as_bytes());
        }
        for c in frag.chars() {
            if c.is_ascii() {
                w.write_all(&[c as u8])?;
            } else {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    write!(w, "\\u{unit:04x}")?;
                }
            }
        }
        Ok(())
    }
}

fn file_json<T: Serialize + ?Sized>(v: &T, indent: bool, ascii: bool) -> String {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, FileFmt { indent, ascii, level: 0, has_value: false });
    v.serialize(&mut ser).expect("serialising to memory");
    bare_nonfinite(String::from_utf8(out).expect("serde_json writes UTF-8"))
}

/// Compact File JSON (16 §3.2).
pub fn compact<T: Serialize + ?Sized>(v: &T) -> String {
    file_json(v, false, true)
}

/// Indented File JSON (16 §3.3).
pub fn indented<T: Serialize + ?Sized>(v: &T) -> String {
    file_json(v, true, true)
}

/// Indented File JSON with non-ASCII written literally (16 §3.5).
pub fn indented_literal<T: Serialize + ?Sized>(v: &T) -> String {
    file_json(v, true, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn file_json_styles() {
        let v = json!({"rebuilt": true, "db": "/h/é", "v": [1.0, 1e-5, -0.5], "e": {}, "n": "😀"});
        assert_eq!(compact(&v), r#"{"rebuilt": true, "db": "/h/\u00e9", "v": [1.0, 1e-05, -0.5], "e": {}, "n": "\ud83d\ude00"}"#);
        assert_eq!(indented(&json!({"hashes": {"a": "b"}, "failures": {}})), "{\n  \"hashes\": {\n    \"a\": \"b\"\n  },\n  \"failures\": {}\n}");
        let odd = json!({"x": nonfinite(f64::NAN), "y": [nonfinite(f64::INFINITY), nonfinite(f64::NEG_INFINITY)]});
        assert_eq!(compact(&odd), r#"{"x": NaN, "y": [Infinity, -Infinity]}"#);
    }

    #[test]
    fn mcp_nonfinite() {
        let v = json!({"x": nonfinite(f64::NAN), "y": [nonfinite(f64::NEG_INFINITY)]});
        assert_eq!(mcp_text(&v), "{\n  \"x\": NaN,\n  \"y\": [\n    -Infinity\n  ]\n}");
        assert_eq!(structured(&v), json!({"x": null, "y": [null]}));
    }
}
