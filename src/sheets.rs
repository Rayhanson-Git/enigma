//! Structured sheets (10): schemas, validation, row matching, upsert, query, check.
use crate::error::{Error, Kind, Result};
use crate::extractor::json_to_y;
use crate::text::{casefold, collapse_ws, display, minute_stamp, quoted, split_lines, strip};
use crate::vault::{Vault, get, json_safe, parse, read_lenient};
use crate::yaml::{Y, dump_block};
use chrono::NaiveDateTime;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

pub const SCHEMA_FILE: &str = "_schema.md";
const STANDARD: [&str; 4] = ["agent", "created", "project", "tags"];
const TYPES: [&str; 6] = ["bool", "date", "enum", "number", "string", "url"];
const OPS: [&str; 10] = ["contains", "eq", "gt", "gte", "in", "lt", "lte", "missing", "ne", "nin"];
const MAX_STEM: usize = 120;
/// Columns whose changes get a "## Log" line (10 §6).
const STATUS_COLUMNS: [&str; 2] = ["status", "stage"];

fn re(p: &str) -> Regex {
    Regex::new(p).expect("static regex")
}
static DATE: LazyLock<Regex> = LazyLock::new(|| re(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"));
static TRACKING: LazyLock<Regex> = LazyLock::new(|| re(r"^(utm_.*|ref|src|gh_src|lever-origin)$"));
static URL: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:([A-Za-z][A-Za-z0-9+.-]*):)?(?://([^/?#]*))?([^?#]*)(?:\?([^#]*))?(?:#(.*))?$"));
static TOP_KEY: LazyLock<Regex> = LazyLock::new(|| re(r"^([A-Za-z_][\w-]*):"));
static LOG_HEAD: LazyLock<Regex> = LazyLock::new(|| re(r"^## Log[ \t]*$"));
static HEADING: LazyLock<Regex> = LazyLock::new(|| re(r"^#{1,6} "));
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| re(r"\{([^{}]*)\}"));

fn err(msg: impl Into<String>) -> Error {
    Error::new(Kind::SheetError, msg)
}

fn s(v: &str) -> Y {
    Y::Str(v.to_string())
}

fn str_list_y(items: &[&str]) -> Y {
    Y::Seq(items.iter().map(|i| s(i)).collect())
}

#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub ty: String,
    pub required: bool,
    pub values: Option<Vec<String>>,
    pub max_length: Option<Y>,
}

#[derive(Debug, Clone)]
pub struct Schema {
    pub name: String,
    pub folder: String,
    pub filename: String,
    pub key: Vec<String>,
    pub identity: Vec<String>,
    pub columns: Vec<Column>,
}

impl Schema {
    fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }
}

fn in_folder(folder: &str, file: &str) -> String {
    if folder.is_empty() { file.to_string() } else { format!("{folder}/{file}") }
}

/// The text before the closing fence and the body after it (10 §6 step 6).
fn split_text(text: &str) -> (Option<&str>, &str) {
    if text.starts_with("---")
        && let Some(i) = text[3..].find("\n---")
    {
        let end = i + 3;
        let fm = text.get(4..end).unwrap_or("");
        let body = text[end + 4..].trim_start_matches('\r').trim_start_matches('\n');
        return (Some(fm), body);
    }
    (None, text)
}

fn truthy(v: Option<&Y>) -> bool {
    crate::graph::truthy(v)
}

/// load_schema(folder) (10 §2).
pub fn load_schema(v: &Vault, folder: &str) -> Result<Schema> {
    let rel = in_folder(folder, SCHEMA_FILE);
    if !v.resolve(&rel)?.is_file() {
        return Err(err(format!("No _schema.md in '{folder}' \u{2014} not a sheet.")));
    }
    let meta = parse(&v.read(&rel)?)?;
    for k in ["sheet", "filename", "key", "columns"] {
        if get(&meta, k).is_none() {
            return Err(err(format!("{rel}: missing '{k}'.")));
        }
    }
    let mut columns = Vec::new();
    if let Some(Y::Map(cols)) = get(&meta, "columns") {
        for (name, spec) in cols {
            let name = display(name);
            let ty = match spec {
                Y::Map(p) => get(p, "type").map(display),
                _ => None,
            };
            let Some(ty) = ty.filter(|t| TYPES.contains(&t.as_str())) else {
                let shown = match spec {
                    Y::Map(p) => get(p, "type").map_or("null".to_string(), display),
                    _ => "null".to_string(),
                };
                return Err(err(format!(
                    "{rel}: column '{name}' has invalid type '{shown}' (allowed: {}).",
                    quoted(&str_list_y(&TYPES))
                )));
            };
            let Y::Map(p) = spec else { unreachable!("type came from a mapping") };
            if ty == "enum" && !truthy(get(p, "values")) {
                return Err(err(format!("{rel}: enum column '{name}' needs 'values'.")));
            }
            let values = match get(p, "values") {
                Some(Y::Seq(items)) => Some(items.iter().map(display).collect()),
                Some(Y::Null) | None => None,
                Some(other) => Some(vec![display(other)]),
            };
            columns.push(Column {
                name,
                ty,
                required: truthy(get(p, "required")),
                values,
                max_length: get(p, "max_length").filter(|m| !matches!(m, Y::Null)).cloned(),
            });
        }
    }
    let list = |k: &str| -> Result<Vec<String>> {
        match get(&meta, k) {
            None => Ok(vec![]),
            Some(Y::Seq(items)) => Ok(items.iter().map(display).collect()),
            Some(other) => Err(err(format!("{rel}: field '{k}' expected list, got {}.", quoted(other)))),
        }
    };
    Ok(Schema {
        name: get(&meta, "sheet").map(display).unwrap_or_default(),
        folder: folder.to_string(),
        filename: get(&meta, "filename").map(display).unwrap_or_default(),
        key: list("key")?,
        identity: list("identity")?,
        columns,
    })
}

pub struct Registry {
    /// (name, folder) in discovery order
    pub sheets: Vec<(String, String)>,
    /// (folder, error)
    pub invalid: Vec<(String, String)>,
}

/// discovery (10 §2).
pub fn discover(v: &Vault) -> Result<Registry> {
    let mut reg = Registry { sheets: vec![], invalid: vec![] };
    for (abs, rel) in crate::search::walk(v, None)? {
        if !rel.ends_with(SCHEMA_FILE) || (rel != SCHEMA_FILE && !rel.ends_with(&format!("/{SCHEMA_FILE}"))) {
            continue;
        }
        let folder = rel.strip_suffix(SCHEMA_FILE).unwrap_or("").trim_end_matches('/').to_string();
        let dir = abs.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        if v.in_claude(&dir) {
            continue;
        }
        match load_schema(v, &folder) {
            Ok(sc) => {
                if let Some((_, first)) = reg.sheets.iter().find(|(n, _)| *n == sc.name) {
                    return Err(err(format!(
                        "Duplicate sheet name '{}': registered in both '{first}' and '{folder}'.",
                        sc.name
                    )));
                }
                reg.sheets.push((sc.name, folder));
            }
            Err(e) if e.kind == Kind::SheetError => reg.invalid.push((folder, e.msg)),
            Err(e) => return Err(e),
        }
    }
    Ok(reg)
}

/// get_schema(name) (10 §2).
pub fn get_schema(v: &Vault, name: &str) -> Result<Schema> {
    let reg = discover(v)?;
    match reg.sheets.iter().find(|(n, _)| n == name) {
        Some((_, folder)) => load_schema(v, folder),
        None => {
            let mut names: Vec<&str> = reg.sheets.iter().map(|(n, _)| n.as_str()).collect();
            names.sort();
            let listed = if names.is_empty() { "none".to_string() } else { quoted(&str_list_y(&names)) };
            Err(err(format!("Unknown sheet '{name}'. Registered: {listed}.")))
        }
    }
}

pub fn is_sheet_folder(v: &Vault, folder: &str) -> bool {
    v.resolve(&in_folder(folder, SCHEMA_FILE)).is_ok_and(|p| p.is_file())
}

// ------------------------------------------------------------ normalisation

pub fn norm_str(v: &Y) -> String {
    casefold(&collapse_ws(&display(v)))
}

fn unquote_plus(s: &str) -> String {
    let bytes = s.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn quote_plus(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// normalize_link(url) (10 §3).
pub fn normalize_link(url: &str) -> String {
    let u = display(&s(url));
    let u = strip(&u);
    let c = URL.captures(u);
    let part = |i: usize| c.as_ref().and_then(|c| c.get(i)).map_or("", |m| m.as_str());
    let mut pairs: Vec<(String, String)> = part(4)
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (unquote_plus(k), unquote_plus(v)),
            None => (unquote_plus(p), String::new()),
        })
        .filter(|(k, _)| !TRACKING.is_match(k))
        .collect();
    pairs.sort();
    let path = part(3).trim_end_matches('/');
    let mut out = format!("{}://{}{}", part(1).to_lowercase(), part(2).to_lowercase(), if path.is_empty() { "/" } else { path });
    if !pairs.is_empty() {
        out.push('?');
        out.push_str(&pairs.iter().map(|(k, v)| format!("{}={}", quote_plus(k), quote_plus(v))).collect::<Vec<_>>().join("&"));
    }
    out
}

/// normalize value (10 §3).
pub fn normalize(v: &Y) -> Y {
    match v {
        Y::Date(d) => Y::Str(crate::text::date_display(d)),
        Y::DateTime(dt, off) => {
            let mut t = dt.format("%Y-%m-%dT%H:%M:%S").to_string();
            let us = chrono::Timelike::nanosecond(dt) / 1000;
            if us != 0 {
                t.push_str(&format!(".{us:06}"));
            }
            if let Some(o) = off {
                let sign = if *o < 0 { '-' } else { '+' };
                t.push_str(&format!("{sign}{:02}:{:02}", o.abs() / 3600, (o.abs() % 3600) / 60));
            }
            Y::Str(t)
        }
        other => other.clone(),
    }
}

/// Numeric equality across types: 3 == 3.0.
fn same(a: &Y, b: &Y) -> bool {
    match (a, b) {
        (Y::Int(x), Y::Float(y)) | (Y::Float(y), Y::Int(x)) => (*x as f64) == *y,
        _ => a == b,
    }
}

fn blank(v: Option<&Y>) -> bool {
    matches!(v, None | Some(Y::Null)) || matches!(v, Some(Y::Str(s)) if s.is_empty())
}

// --------------------------------------------------------------- validation

fn url_parts(v: &str) -> (String, String) {
    let c = URL.captures(v);
    let part = |i: usize| c.as_ref().and_then(|c| c.get(i)).map_or("", |m| m.as_str()).to_string();
    (part(1).to_lowercase(), part(2))
}

/// check(name, column, value) (10 §4); value is non-null.
pub fn check(n: &str, col: &Column, value: &Y) -> Result<()> {
    let got_q = || format!("'{}'", quoted(value));
    let length = |text: &str| -> Result<()> {
        if let Some(m) = col.max_length.as_ref().filter(|m| truthy(Some(m))) {
            let max = match m {
                Y::Int(i) => *i as f64,
                Y::Float(f) => *f,
                _ => f64::INFINITY,
            };
            let len = text.chars().count();
            if len as f64 > max {
                return Err(err(format!("Field '{n}': exceeds max_length {} ({len} chars).", display(m))));
            }
        }
        Ok(())
    };
    match col.ty.as_str() {
        "enum" => {
            let ok = matches!(value, Y::Str(v) if col.values.as_ref().is_some_and(|vals| vals.contains(v)));
            if !ok {
                let vals: Vec<&str> = col.values.iter().flatten().map(String::as_str).collect();
                return Err(err(format!("Field '{n}': expected one of {}, got '{}'.", quoted(&str_list_y(&vals)), display(value))));
            }
        }
        "date" => {
            if !(matches!(value, Y::Date(_)) || matches!(value, Y::Str(v) if DATE.is_match(v))) {
                return Err(err(format!("Field '{n}': expected date YYYY-MM-DD, got '{}'.", display(value))));
            }
        }
        "bool" => {
            if !matches!(value, Y::Bool(_)) {
                return Err(err(format!("Field '{n}': expected bool, got {}.", got_q())));
            }
        }
        "number" => {
            if !matches!(value, Y::Int(_) | Y::Float(_)) {
                return Err(err(format!("Field '{n}': expected number, got {}.", got_q())));
            }
        }
        "url" => {
            let Y::Str(u) = value else { return Err(err(format!("Field '{n}': expected a URL with no whitespace, got {}.", got_q()))) };
            if u.is_empty() || u.chars().any(crate::text::is_ws) {
                return Err(err(format!("Field '{n}': expected a URL with no whitespace, got {}.", got_q())));
            }
            let (scheme, host) = url_parts(u);
            if !(scheme == "http" || scheme == "https") || host.is_empty() {
                return Err(err(format!("Field '{n}': expected an http(s) URL with a host, got {}.", got_q())));
            }
            length(u)?;
        }
        _ => {
            let Y::Str(v) = value else { return Err(err(format!("Field '{n}': expected string, got {}.", got_q()))) };
            length(v)?;
        }
    }
    Ok(())
}

/// validate_fields(schema, fields, require_required) (10 §4).
pub fn validate_fields(sc: &Schema, fields: &[(Y, Y)], require_required: bool) -> Result<()> {
    for (k, val) in fields {
        let k = display(k);
        if STANDARD.contains(&k.as_str()) {
            continue;
        }
        let Some(col) = sc.column(&k) else {
            let mut names: Vec<&str> = sc.columns.iter().map(|c| c.name.as_str()).collect();
            names.sort();
            return Err(err(format!(
                "Field '{k}' is not declared in sheet '{}' (columns: {}; standard: {}).",
                sc.name,
                quoted(&str_list_y(&names)),
                quoted(&str_list_y(&STANDARD))
            )));
        };
        if !matches!(val, Y::Null) {
            check(&k, col, val)?;
        }
    }
    if require_required {
        let missing: Vec<&str> = sc
            .columns
            .iter()
            .filter(|c| c.required && matches!(get(fields, &c.name), None | Some(Y::Null)))
            .map(|c| c.name.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(err(format!("Missing required field(s): {}.", quoted(&str_list_y(&missing)))));
        }
    }
    Ok(())
}

// ---------------------------------------------------------- rows, matching

pub type Fields = Vec<(Y, Y)>;
pub type Row = (String, Fields);
/// A row's identity: the key values plus the optional identity column and value.
type RowKey = (Vec<String>, Option<(String, String)>);

/// rows of a sheet (10 §5).
pub fn rows(v: &Vault, sc: &Schema) -> Result<Vec<Row>> {
    let dir = v.resolve(if sc.folder.is_empty() { "" } else { &sc.folder })?;
    let Ok(rd) = std::fs::read_dir(&dir) else { return Ok(vec![]) };
    let mut names: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| crate::vault::is_md(n) && n != SCHEMA_FILE)
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|n| {
            let text = read_lenient(&dir.join(&n))?;
            Ok((in_folder(&sc.folder, &n), parse(&text)?))
        })
        .collect()
}

/// render_filename(schema, fields) (10 §5).
pub fn render_filename(sc: &Schema, fields: &[(Y, Y)]) -> Result<String> {
    let mut out = String::new();
    let mut last = 0;
    for c in PLACEHOLDER.captures_iter(&sc.filename) {
        let m = c.get(0).map_or(0..0, |m| m.range());
        let name = &c[1];
        if sc.column(name).is_none() {
            return Err(err(format!("Filename pattern of sheet '{}' names undeclared column '{name}'.", sc.name)));
        }
        out.push_str(&sc.filename[last..m.start]);
        if let Some(v) = get(fields, name).filter(|v| !matches!(v, Y::Null)) {
            out.push_str(&display(v));
        }
        last = m.end;
    }
    out.push_str(&sc.filename[last..]);
    let cleaned: String = out.chars().map(|c| if "\\/:*?\"<>|".contains(c) || (c as u32) < 0x20 { '-' } else { c }).collect();
    let cut: String = collapse_ws(&cleaned).chars().take(MAX_STEM).collect();
    Ok(cut.trim_end_matches(crate::text::is_ws).to_string())
}

fn identity_value(sc: &Schema, meta: &[(Y, Y)]) -> Option<(String, String)> {
    sc.identity.iter().find_map(|c| {
        let v = get(meta, c).filter(|v| !blank(Some(v)))?;
        let is_url = sc.column(c).is_some_and(|col| col.ty == "url");
        Some((c.clone(), if is_url { normalize_link(&display(v)) } else { norm_str(v) }))
    })
}

/// match_row(fields) (10 §5) → (row path, backfill).
pub fn match_row(v: &Vault, sc: &Schema, fields: &[(Y, Y)]) -> Result<(Option<String>, Fields)> {
    let key_of = |m: &[(Y, Y)]| -> Vec<String> { sc.key.iter().map(|k| norm_str(get(m, k).unwrap_or(&s("")))).collect() };
    let want = key_of(fields);
    let all = rows(v, sc)?;
    let cands: Vec<&Row> = all.iter().filter(|(_, m)| key_of(m) == want).collect();
    if let Some(id) = identity_value(sc, fields) {
        if let Some((p, _)) = cands.iter().find(|(_, m)| identity_value(sc, m).as_ref() == Some(&id)) {
            return Ok((Some(p.clone()), vec![]));
        }
        if cands.len() == 1 && identity_value(sc, &cands[0].1).is_none() {
            let raw = get(fields, &id.0).cloned().unwrap_or(Y::Null);
            return Ok((Some(cands[0].0.clone()), vec![(s(&id.0), raw)]));
        }
        return Ok((None, vec![]));
    }
    match cands.len() {
        0 => Ok((None, vec![])),
        1 => Ok((Some(cands[0].0.clone()), vec![])),
        _ => {
            let ids: Vec<&str> = sc.identity.iter().map(String::as_str).collect();
            let paths: Vec<&str> = cands.iter().map(|(p, _)| p.as_str()).collect();
            Err(err(format!(
                "Ambiguous match: multiple rows share this key \u{2014} supply {} to disambiguate. Candidates: {}",
                quoted(&str_list_y(&ids)),
                quoted(&str_list_y(&paths))
            )))
        }
    }
}

// ------------------------------------------------------------------ writing

/// One frontmatter line (or a block) for key: value (10 §6).
fn fm_line(k: &str, v: &Y) -> String {
    match v {
        Y::Str(t) if DATE.is_match(t) => format!("{k}: {t}"),
        _ => dump_block(&[(s(k), v.clone())]).trim().to_string(),
    }
}

/// append to Log (10 §6).
pub fn append_log(body: &str, line: &str) -> String {
    let lines = crate::text::split_keep_ends(body);
    let Some(h) = lines.iter().position(|l| LOG_HEAD.is_match(crate::text::without_end(l))) else {
        return format!("{}\n\n## Log\n{line}\n", body.trim_end_matches('\n'));
    };
    let start: usize = lines[..=h].iter().map(|l| l.len()).sum();
    let Some(n) = lines.iter().skip(h + 1).position(|l| HEADING.is_match(l)).map(|i| i + h + 1) else {
        return format!("{}\n{line}\n", body.trim_end_matches('\n'));
    };
    let end: usize = lines[..n].iter().map(|l| l.len()).sum();
    let section = body[start..end].trim_end_matches('\n');
    let mid = if section.is_empty() { format!("\n{line}\n\n") } else { format!("{section}\n{line}\n\n") };
    format!("{}{mid}{}", &body[..start], &body[end..])
}

fn log_line(now: &NaiveDateTime, col: &str, from: Option<&Y>, to: &Y, agent: &str) -> String {
    let from = from.filter(|f| !matches!(f, Y::Null)).map_or("(new)".to_string(), display);
    format!("- {} {col}: {from} \u{2192} {} (agent: {agent})", crate::text::day(now), display(to))
}

fn set(meta: &mut Vec<(Y, Y)>, k: &str, v: Y) {
    crate::graph::set_key(meta, k, v);
}

fn create(v: &Vault, sc: &Schema, merged: &[(Y, Y)], body: Option<&str>, agent: &str, now: &NaiveDateTime, suffixed: bool) -> Result<Value> {
    validate_fields(sc, merged, true)?;
    let mut meta = merged.to_vec();
    set(&mut meta, "created", s(&minute_stamp(now)));
    set(&mut meta, "agent", s(agent));
    let mut body = body.unwrap_or("").to_string();
    for c in STATUS_COLUMNS {
        if let Some(to) = get(merged, c) {
            body = append_log(&body, &log_line(now, c, None, to, agent));
        }
    }
    let lines: Vec<String> = meta.iter().map(|(k, val)| fm_line(&display(k), val)).collect();
    let text = format!("---\n{}\n---\n\n{body}", lines.join("\n"));
    let mut changed = Map::new();
    for (k, val) in merged {
        changed.insert(display(k), json!({"from": null, "to": json_safe(val)}));
    }
    let stem = render_filename(sc, merged)?;
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{stem}.md") } else { format!("{stem} {n}.md") };
        let path = in_folder(&sc.folder, &name);
        match v.write(&path, &text, false, true) {
            Ok(_) => return Ok(json!({"result": "created", "path": path, "changed": changed})),
            Err(e) if e.kind == Kind::NoteExists && suffixed => n += 1,
            Err(e) => return Err(e),
        }
    }
}

/// sheet_upsert (10 §6).
pub fn upsert(v: &Vault, sheet: &str, fields: &[(Y, Y)], body: Option<&str>, agent: &str, now: &NaiveDateTime) -> Result<Value> {
    let sc = get_schema(v, sheet)?;
    let (mut row, mut backfill) = match_row(v, &sc, fields)?;
    if row.is_none() {
        let mut merged = fields.to_vec();
        merged.extend(backfill.iter().cloned());
        match create(v, &sc, &merged, body, agent, now, false) {
            Err(e) if e.kind == Kind::NoteExists => {
                (row, backfill) = match_row(v, &sc, fields)?;
                if row.is_none() {
                    return create(v, &sc, &merged, body, agent, now, true);
                }
            }
            r => return r,
        }
    }
    let path = row.unwrap_or_default();
    if body.is_some() {
        return Err(err("body may only be supplied on create; omit it when updating an existing row \u{2014} sheet_upsert never edits an existing body (the '## Log' status append is the sole exception)."));
    }
    let mut merged: Vec<(Y, Y)> = fields.iter().chain(backfill.iter()).filter(|(k, _)| !matches!(display(k).as_str(), "created" | "agent")).cloned().collect();
    merged.dedup_by(|a, b| a.0 == b.0);
    validate_fields(&sc, &merged, false)?;
    let text = v.read(&path)?;
    let stored = parse(&text)?;
    let (fm, body_text) = split_text(&text);
    let mut changed = Map::new();
    let mut changes: Vec<(String, Y)> = Vec::new();
    for (k, val) in &merged {
        let k = display(k);
        let old = get(&stored, &k).map(normalize);
        if old.as_ref().is_some_and(|o| same(o, val)) {
            continue;
        }
        changed.insert(k.clone(), json!({"from": old.as_ref().map_or(Value::Null, json_safe), "to": json_safe(val)}));
        changes.push((k, val.clone()));
    }
    if changed.is_empty() {
        return Ok(json!({"result": "updated", "path": path, "changed": {}}));
    }
    let mut lines: Vec<String> = fm.map(|f| split_lines(f).into_iter().map(String::from).collect()).unwrap_or_default();
    let mut sets = changes.clone();
    sets.push(("agent".into(), s(agent)));
    for (k, val) in &sets {
        let new = fm_line(k, val);
        match lines.iter().position(|l| TOP_KEY.captures(l).is_some_and(|c| &c[1] == k)) {
            Some(i) => {
                let end = lines.iter().skip(i + 1).position(|l| TOP_KEY.is_match(l)).map_or(lines.len(), |j| j + i + 1);
                lines.splice(i..end, new.lines().map(String::from));
            }
            None => lines.extend(new.lines().map(String::from)),
        }
    }
    let mut body = body_text.to_string();
    for (k, val) in &changes {
        if STATUS_COLUMNS.contains(&k.as_str()) {
            body = append_log(&body, &log_line(now, k, get(&stored, k), val, agent));
        }
    }
    v.write(&path, &format!("---\n{}\n---\n\n{body}", lines.join("\n")), true, true)?;
    Ok(json!({"result": "updated", "path": path, "changed": changed}))
}

// -------------------------------------------------------------------- query

fn compare(a: &Y, b: &Y) -> Result<std::cmp::Ordering> {
    let num = |y: &Y| match y {
        Y::Int(i) => Some(*i as f64),
        Y::Float(f) => Some(*f),
        _ => None,
    };
    match (num(a), num(b), a, b) {
        (Some(x), Some(y), _, _) => Ok(x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)),
        (None, None, Y::Str(x), Y::Str(y)) => Ok(x.cmp(y)),
        _ => Err(err(format!("Cannot compare {} with {}.", quoted(a), quoted(b)))),
    }
}

fn holds(op: &str, value: Option<&Y>, expected: &Y) -> Result<bool> {
    if op == "missing" {
        return Ok(blank(value) == truthy(Some(expected)));
    }
    let Some(value) = value.filter(|v| !blank(Some(v))) else { return Ok(op == "ne" || op == "nin") };
    let (a, b) = (normalize(value), normalize(expected));
    let member = || match &b {
        Y::Seq(items) => items.iter().any(|i| same(&normalize(i), &a)),
        _ => false,
    };
    Ok(match op {
        "eq" => same(&a, &b),
        "ne" => !same(&a, &b),
        "in" => member(),
        "nin" => !member(),
        "contains" => casefold(&display(&a)).contains(&casefold(&display(&b))),
        "lt" => compare(&a, &b)?.is_lt(),
        "lte" => compare(&a, &b)?.is_le(),
        "gt" => compare(&a, &b)?.is_gt(),
        _ => compare(&a, &b)?.is_ge(),
    })
}

fn row_json(path: &str, meta: &[(Y, Y)]) -> Value {
    let mut o = Map::new();
    o.insert("path".into(), json!(path));
    for (k, val) in meta {
        o.insert(display(k), json_safe(&normalize(val)));
    }
    Value::Object(o)
}

/// sheet_query (10 §7).
pub fn query(v: &Vault, sheet: &str, filters: Option<&Map<String, Value>>, sort: Option<&Map<String, Value>>, limit: i64) -> Result<Vec<Value>> {
    let sc = get_schema(v, sheet)?;
    let mut conds: Vec<(String, String, Y)> = Vec::new();
    for (col, ops) in filters.into_iter().flatten() {
        for (op, expected) in ops.as_object().into_iter().flatten() {
            if !OPS.contains(&op.as_str()) {
                return Err(err(format!("Unknown operator '{op}' (allowed: {}).", quoted(&str_list_y(&OPS)))));
            }
            if ["lt", "lte", "gt", "gte"].contains(&op.as_str()) {
                let ty = sc.column(col).map(|c| c.ty.clone());
                if !matches!(ty.as_deref(), Some("date" | "number")) {
                    return Err(err(format!(
                        "Column '{col}' ({}) does not support ordering operators; only ['date', 'number'] columns do.",
                        ty.unwrap_or_else(|| "null".into())
                    )));
                }
            }
            conds.push((col.clone(), op.clone(), json_to_y(expected)));
        }
    }
    let mut out: Vec<Row> = Vec::new();
    for (path, meta) in rows(v, &sc)? {
        let mut ok = true;
        for (col, op, expected) in &conds {
            if !holds(op, get(&meta, col), expected)? {
                ok = false;
                break;
            }
        }
        if ok {
            out.push((path, meta));
        }
    }
    if let Some(by) = sort.and_then(|s| s.get("by")).and_then(Value::as_str) {
        let desc = sort.and_then(|s| s.get("dir")).and_then(Value::as_str) == Some("desc");
        let (mut with, without): (Vec<Row>, Vec<Row>) = out.into_iter().partition(|(_, m)| !blank(get(m, by)));
        let mut failure = None;
        with.sort_by(|a, b| {
            let (x, y) = (normalize(get(&a.1, by).unwrap_or(&Y::Null)), normalize(get(&b.1, by).unwrap_or(&Y::Null)));
            let o = compare(&x, &y).unwrap_or_else(|e| {
                failure.get_or_insert(e);
                std::cmp::Ordering::Equal
            });
            if desc { o.reverse() } else { o }
        });
        if let Some(e) = failure {
            return Err(e);
        }
        with.extend(without);
        out = with;
    }
    Ok(crate::search::slice(out, limit).iter().map(|(p, m)| row_json(p, m)).collect())
}

// ------------------------------------------------------------ info and check

fn columns_json(sc: &Schema) -> Value {
    let mut cols = Map::new();
    for c in &sc.columns {
        cols.insert(
            c.name.clone(),
            json!({"type": c.ty, "required": c.required, "values": c.values, "max_length": c.max_length.as_ref().map_or(Value::Null, json_safe)}),
        );
    }
    Value::Object(cols)
}

/// sheet_schema(name?) (10 §8).
pub fn schema_info(v: &Vault, name: Option<&str>) -> Result<Value> {
    if let Some(name) = name.filter(|n| !n.is_empty()) {
        let sc = get_schema(v, name)?;
        let text = v.read(&in_folder(&sc.folder, SCHEMA_FILE))?;
        let instructions = strip(split_text(&text).1).to_string();
        return Ok(json!({
            "sheet": sc.name, "folder": sc.folder, "filename": sc.filename, "key": sc.key,
            "identity": sc.identity, "columns": columns_json(&sc), "instructions": instructions,
        }));
    }
    let reg = discover(v)?;
    let mut out = Map::new();
    for (name, folder) in &reg.sheets {
        let sc = load_schema(v, folder)?;
        out.insert(name.clone(), json!({"folder": folder, "rows": rows(v, &sc)?.len()}));
    }
    if !reg.invalid.is_empty() {
        out.insert("invalid".into(), reg.invalid.iter().map(|(f, e)| json!({"folder": f, "error": e})).collect());
    }
    Ok(Value::Object(out))
}

/// check(vault) (10 §9).
pub fn check_vault(v: &Vault) -> Result<Value> {
    let reg = discover(v)?;
    let mut sheets = Map::new();
    let mut clean = true;
    for (name, folder) in &reg.sheets {
        let sc = load_schema(v, folder)?;
        let all = rows(v, &sc)?;
        let (mut invalid, mut dups) = (vec![], vec![]);
        let mut seen: Vec<(RowKey, String)> = vec![];
        for (path, meta) in &all {
            let fields: Vec<(Y, Y)> = meta.iter().filter(|(k, _)| !STANDARD.contains(&display(k).as_str())).cloned().collect();
            if let Err(e) = validate_fields(&sc, &fields, true) {
                invalid.push(json!({"path": path, "error": e.msg}));
            }
            let key = (sc.key.iter().map(|k| norm_str(get(meta, k).unwrap_or(&s("")))).collect(), identity_value(&sc, meta));
            match seen.iter().find(|(k, _)| *k == key) {
                Some((_, first)) => dups.push(json!({"paths": [first, path]})),
                None => seen.push((key, path.clone())),
            }
        }
        clean &= invalid.is_empty() && dups.is_empty();
        sheets.insert(name.clone(), json!({"rows": all.len(), "invalid": invalid, "duplicates": dups}));
    }
    Ok(json!({"sheets": sheets, "clean": clean}))
}

/// Librarian health: total invalid rows over registered sheets.
pub fn invalid_rows(v: &Vault) -> Result<usize> {
    let report = check_vault(v)?;
    Ok(report["sheets"].as_object().into_iter().flatten().map(|(_, s)| s["invalid"].as_array().map_or(0, Vec::len)).sum())
}

/// Fields from an MCP object.
pub fn fields_from(obj: &Map<String, Value>) -> Vec<(Y, Y)> {
    obj.iter().map(|(k, v)| (s(k), json_to_y(v))).collect()
}

