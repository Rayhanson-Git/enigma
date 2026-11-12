//! YAML 1.1 safe parsing and emission (16 §2). Syntax comes from yaml-rust2's
//! event parser; typing, construction and emission follow the spec's rules.
use crate::error::{Error, Result};
use crate::text::float_text;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::{Marker, TScalarStyle};

#[derive(Debug, Clone, PartialEq)]
pub enum Y {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Date(NaiveDate),
    /// Local date-time plus an optional UTC offset in seconds.
    DateTime(NaiveDateTime, Option<i32>),
    Seq(Vec<Y>),
    Map(Vec<(Y, Y)>),
    /// binary, set, omap, pairs: rendered text only.
    Other(String),
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Plain {
    Bool,
    Float,
    Int,
    Merge,
    Null,
    Timestamp,
    Value,
    Str,
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("static regex")
}

static BOOL: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:yes|Yes|YES|no|No|NO|true|True|TRUE|false|False|FALSE|on|On|ON|off|Off|OFF)$"));
static FLOAT: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^(?:[-+]?[0-9][0-9_]*\.[0-9_]*(?:[eE][-+][0-9]+)?|\.[0-9][0-9_]*(?:[eE][-+][0-9]+)?|[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$")
});
static INT: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^(?:[-+]?0b[0-1_]+|[-+]?0[0-7_]+|[-+]?(?:0|[1-9][0-9_]*)|[-+]?0x[0-9a-fA-F_]+|[-+]?[1-9][0-9_]*(?::[0-5]?[0-9])+)$")
});
static NULL: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:~|null|Null|NULL|)$"));
static TS: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^(?:[0-9]{4}-[0-9]{2}-[0-9]{2}|[0-9]{4}-[0-9]{1,2}-[0-9]{1,2}(?:[Tt]|[ \t]+)[0-9]{1,2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]*)?(?:[ \t]*(?:Z|[-+][0-9]{1,2}(?::[0-9]{2})?))?)$")
});
static TS_PARTS: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^([0-9]{4})-([0-9]{1,2})-([0-9]{1,2})(?:(?:[Tt]|[ \t]+)([0-9]{1,2}):([0-9]{2}):([0-9]{2})(?:\.([0-9]*))?(?:[ \t]*(Z|([-+])([0-9]{1,2})(?::([0-9]{2}))?))?)?$")
});

/// Implicit resolution of a plain scalar (16 §2.2).
pub fn resolve_plain(s: &str) -> Plain {
    if BOOL.is_match(s) {
        Plain::Bool
    } else if FLOAT.is_match(s) {
        Plain::Float
    } else if INT.is_match(s) {
        Plain::Int
    } else if s == "<<" {
        Plain::Merge
    } else if NULL.is_match(s) {
        Plain::Null
    } else if TS.is_match(s) {
        Plain::Timestamp
    } else if s == "=" {
        Plain::Value
    } else {
        Plain::Str
    }
}

/// Internal construction failure: a YAML error (frontmatter becomes {}) or an
/// edge-case Q5 calendar error that fails the whole operation.
enum Fail {
    Yaml,
    Calendar(Error),
}

type Built = std::result::Result<Y, Fail>;

fn base60(s: &str) -> f64 {
    s.split(':').fold(0.0, |acc, p| acc * 60.0 + p.parse::<f64>().unwrap_or(0.0))
}

fn to_int(s: &str) -> Built {
    let v: String = s.chars().filter(|c| *c != '_').collect();
    let (neg, v) = match v.strip_prefix('-') {
        Some(r) => (true, r.to_string()),
        None => (false, v.strip_prefix('+').unwrap_or(&v).to_string()),
    };
    let n: i128 = if v == "0" {
        0
    } else if let Some(b) = v.strip_prefix("0b") {
        i128::from_str_radix(b, 2).map_err(|_| Fail::Yaml)?
    } else if let Some(h) = v.strip_prefix("0x") {
        i128::from_str_radix(h, 16).map_err(|_| Fail::Yaml)?
    } else if v.starts_with('0') {
        i128::from_str_radix(&v, 8).map_err(|_| Fail::Yaml)?
    } else if v.contains(':') {
        v.split(':').try_fold(0i128, |acc, p| p.parse::<i128>().map(|d| acc * 60 + d)).map_err(|_| Fail::Yaml)?
    } else {
        v.parse::<i128>().map_err(|_| Fail::Yaml)?
    };
    let n = if neg { -n } else { n };
    // shortcut: integers beyond i64 become floats; upgrade if bigints ever matter
    Ok(i64::try_from(n).map(Y::Int).unwrap_or(Y::Float(n as f64)))
}

fn to_float(s: &str) -> Built {
    let v: String = s.chars().filter(|c| *c != '_').collect::<String>().to_lowercase();
    let (neg, body) = match v.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, v.strip_prefix('+').unwrap_or(&v)),
    };
    let f = if body == ".inf" {
        f64::INFINITY
    } else if body == ".nan" {
        f64::NAN
    } else if body.contains(':') {
        base60(body)
    } else {
        body.parse::<f64>().map_err(|_| Fail::Yaml)?
    };
    Ok(Y::Float(if neg { -f } else { f }))
}

fn cal(msg: &str) -> Fail {
    Fail::Calendar(Error::invalid(msg))
}

fn to_timestamp(s: &str) -> Built {
    let c = TS_PARTS.captures(s).ok_or(Fail::Yaml)?;
    let num = |i: usize| c.get(i).map(|m| m.as_str().parse::<u32>().unwrap_or(0));
    let (y, m, d) = (num(1).unwrap_or(0), num(2).unwrap_or(0), num(3).unwrap_or(0));
    if y == 0 {
        return Err(cal("the year must be 1 or later"));
    }
    if !(1..=12).contains(&m) {
        return Err(cal("the month must be between 1 and 12"));
    }
    let date = NaiveDate::from_ymd_opt(y as i32, m, d).ok_or_else(|| cal("that day does not exist in that month"))?;
    let Some(h) = num(4) else { return Ok(Y::Date(date)) };
    let (mi, se) = (num(5).unwrap_or(0), num(6).unwrap_or(0));
    if h > 23 {
        return Err(cal("the hour must be between 0 and 23"));
    }
    if mi > 59 {
        return Err(cal("the minute must be between 0 and 59"));
    }
    if se > 59 {
        return Err(cal("the second must be between 0 and 59"));
    }
    let frac = c.get(7).map(|m| m.as_str()).unwrap_or("");
    let us: u32 = format!("{:0<6}", &frac[..frac.len().min(6)]).parse().unwrap_or(0);
    let time = NaiveTime::from_hms_micro_opt(h, mi, se, us).ok_or(Fail::Yaml)?;
    let off = match c.get(8).map(|m| m.as_str()) {
        None => None,
        Some("Z") => Some(0),
        Some(_) => {
            let secs = (num(10).unwrap_or(0) * 3600 + num(11).unwrap_or(0) * 60) as i32;
            Some(if c.get(9).map(|m| m.as_str()) == Some("-") { -secs } else { secs })
        }
    };
    Ok(Y::DateTime(NaiveDateTime::new(date, time), off))
}

fn construct_plain(s: &str) -> Built {
    match resolve_plain(s) {
        Plain::Bool => Ok(Y::Bool(matches!(s.to_lowercase().as_str(), "yes" | "true" | "on"))),
        Plain::Float => to_float(s),
        Plain::Int => to_int(s),
        Plain::Null => Ok(Y::Null),
        Plain::Timestamp => to_timestamp(s),
        Plain::Merge | Plain::Value => Err(Fail::Yaml),
        Plain::Str => Ok(Y::Str(s.to_string())),
    }
}

fn std_tag(tag: &Tag) -> Option<&str> {
    (tag.handle == "tag:yaml.org,2002:" || tag.handle == "!!").then_some(tag.suffix.as_str())
}

fn construct_tagged_scalar(v: &str, tag: &Tag) -> Built {
    if tag.handle == "!" && tag.suffix.is_empty() {
        return Ok(Y::Str(v.to_string()));
    }
    match std_tag(tag).ok_or(Fail::Yaml)? {
        "str" => Ok(Y::Str(v.to_string())),
        "int" => to_int(v),
        "float" => to_float(v),
        "null" => Ok(Y::Null),
        "bool" => match v.to_lowercase().as_str() {
            "yes" | "true" | "on" => Ok(Y::Bool(true)),
            "no" | "false" | "off" => Ok(Y::Bool(false)),
            _ => Err(Fail::Yaml),
        },
        "timestamp" => to_timestamp(v),
        "binary" => Ok(Y::Other(format!("b'{}'", v.split_whitespace().collect::<String>()))),
        _ => Err(Fail::Yaml),
    }
}


/// Nesting deeper than this is treated as invalid YAML.
const MAX_DEPTH: usize = 1000;
/// Values built, alias copies included, before the document counts as invalid
/// (protects memory from alias bombs).
const MAX_VALUES: usize = 1_000_000;

/// An open collection while building (the builder keeps its own stack, so
/// deep nesting cannot overflow the thread stack).
enum Frame {
    Seq { aid: usize, tag: Option<Tag>, start: usize, items: Vec<Y> },
    Map { aid: usize, tag: Option<Tag>, start: usize, merged: Vec<Vec<(Y, Y)>>, own: Vec<(Y, Y)>, key: Option<Y>, merge_next: bool },
}

fn finish_seq(items: Vec<Y>, tag: Option<Tag>) -> Built {
    match tag.as_ref().map(|t| std_tag(t).ok_or(Fail::Yaml)).transpose()? {
        None | Some("seq") => Ok(Y::Seq(items)),
        Some("omap") | Some("pairs") => Ok(Y::Other(crate::text::quoted(&Y::Seq(items)))),
        _ => Err(Fail::Yaml),
    }
}

fn finish_map(merged: Vec<Vec<(Y, Y)>>, own: Vec<(Y, Y)>, tag: Option<Tag>) -> Built {
    let mut out: Vec<(Y, Y)> = Vec::new();
    for (k, v) in merged.into_iter().flatten().chain(own) {
        match out.iter_mut().find(|(ek, _)| *ek == k) {
            Some(slot) => slot.1 = v,
            None => out.push((k, v)),
        }
    }
    match tag.as_ref().map(|t| std_tag(t).ok_or(Fail::Yaml)).transpose()? {
        None | Some("map") => Ok(Y::Map(out)),
        Some("set") => Ok(Y::Other(format!("{{{}}}", out.iter().map(|(k, _)| crate::text::quoted(k)).collect::<Vec<_>>().join(", ")))),
        _ => Err(Fail::Yaml),
    }
}

/// Builds the first document's value from the parser events.
fn build(ev: &[Event]) -> Built {
    let mut stack: Vec<Frame> = Vec::new();
    let mut anchors: HashMap<usize, (Y, usize)> = HashMap::new();
    let mut built = 0usize;
    for e in ev {
        built += 1;
        if built > MAX_VALUES {
            return Err(Fail::Yaml);
        }
        // a completed value and the anchor/start it closes
        let (y, aid, start) = match e {
            Event::Scalar(v, TScalarStyle::Plain, _, None)
                if v == "<<" && matches!(stack.last(), Some(Frame::Map { key: None, merge_next: false, .. })) =>
            {
                if let Some(Frame::Map { merge_next, .. }) = stack.last_mut() {
                    *merge_next = true;
                }
                continue;
            }
            Event::Scalar(v, style, aid, tag) => {
                let y = match (tag, style) {
                    (Some(t), _) => construct_tagged_scalar(v, t)?,
                    (None, TScalarStyle::Plain) => construct_plain(v)?,
                    _ => Y::Str(v.clone()),
                };
                (y, *aid, built - 1)
            }
            Event::Alias(id) => {
                let (y, cost) = anchors.get(id).cloned().ok_or(Fail::Yaml)?;
                built += cost;
                if built > MAX_VALUES {
                    return Err(Fail::Yaml);
                }
                (y, 0, built)
            }
            Event::SequenceStart(aid, tag) | Event::MappingStart(aid, tag) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(Fail::Yaml);
                }
                let (aid, tag, start) = (*aid, tag.clone(), built - 1);
                stack.push(if matches!(e, Event::SequenceStart(..)) {
                    Frame::Seq { aid, tag, start, items: vec![] }
                } else {
                    Frame::Map { aid, tag, start, merged: vec![], own: vec![], key: None, merge_next: false }
                });
                continue;
            }
            Event::SequenceEnd | Event::MappingEnd => match stack.pop() {
                Some(Frame::Seq { aid, tag, start, items }) => (finish_seq(items, tag)?, aid, start),
                Some(Frame::Map { aid, tag, start, merged, own, key: None, .. }) => (finish_map(merged, own, tag)?, aid, start),
                _ => return Err(Fail::Yaml),
            },
            Event::DocumentEnd | Event::StreamEnd => break,
            _ => continue,
        };
        if aid > 0 {
            anchors.insert(aid, (y.clone(), built - start));
        }
        match stack.last_mut() {
            None => return Ok(y),
            Some(Frame::Seq { items, .. }) => items.push(y),
            Some(Frame::Map { merged, own, key, merge_next, .. }) => {
                if *merge_next {
                    *merge_next = false;
                    match y {
                        Y::Map(p) => merged.push(p),
                        Y::Seq(items) => {
                            let mut sub = Vec::new();
                            for it in items {
                                match it {
                                    Y::Map(p) => sub.push(p),
                                    _ => return Err(Fail::Yaml),
                                }
                            }
                            sub.reverse();
                            merged.extend(sub);
                        }
                        _ => return Err(Fail::Yaml),
                    }
                } else if let Some(k) = key.take() {
                    own.push((k, y));
                } else if matches!(y, Y::Seq(_) | Y::Map(_)) {
                    return Err(Fail::Yaml);
                } else {
                    *key = Some(y);
                }
            }
        }
    }
    Err(Fail::Yaml)
}

#[derive(Default)]
struct Events(Vec<Event>);
impl MarkedEventReceiver for Events {
    fn on_event(&mut self, ev: Event, _mark: Marker) {
        self.0.push(ev);
    }
}

/// An upper bound on the nesting depth of src: each level needs more
/// indentation, a compact "- "/"? " indicator or a flow bracket.
fn depth_bound(src: &str) -> usize {
    src.lines()
        .map(|l| {
            let t = l.trim_start_matches(' ');
            let mut rest = t;
            let mut compact = 0;
            while let Some(r) = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("? ")) {
                compact += 1;
                rest = r.trim_start_matches(' ');
            }
            2 * (l.len() - t.len()) + 2 + compact + l.matches(['[', '{']).count()
        })
        .max()
        .unwrap_or(0)
}

/// Depth up to which yaml-rust2's recursive parser runs on the caller's stack.
const SHALLOW: usize = 250;

fn load_events(src: &str) -> Option<Vec<Event>> {
    let mut rx = Events::default();
    Parser::new_from_str(src).load(&mut rx, true).ok()?;
    Some(rx.0)
}

/// The parser's message for a YAML syntax error, if the text has one.
pub fn syntax_error(src: &str) -> Option<String> {
    Parser::new_from_str(src).load(&mut Events::default(), true).err().map(|e| e.to_string())
}

/// Parses one YAML document. Ok(None) on any YAML syntax or construction
/// error, nesting beyond MAX_DEPTH or an empty stream; Err only for impossible
/// calendar values (Q5).
pub fn parse_doc(src: &str) -> Result<Option<Y>> {
    let bound = depth_bound(src);
    if bound > MAX_DEPTH {
        return Ok(None);
    }
    let events = if bound <= SHALLOW {
        load_events(src)
    } else {
        // deep but legal: parse on a thread with room for the parser's recursion
        let owned = src.to_string();
        std::thread::Builder::new().stack_size(64 << 20).spawn(move || load_events(&owned)).ok().and_then(|h| h.join().ok()).flatten()
    };
    let Some(events) = events else { return Ok(None) };
    let rx = Events(events);
    let docs = rx.0.iter().filter(|e| matches!(e, Event::DocumentStart)).count();
    if docs != 1 {
        return Ok(None);
    }
    let start = rx.0.iter().position(|e| matches!(e, Event::DocumentStart)).unwrap_or(0) + 1;
    match build(&rx.0[start..]) {
        Ok(y) => Ok(Some(y)),
        Err(Fail::Yaml) => Ok(None),
        Err(Fail::Calendar(e)) => Err(e),
    }
}

// ---------------------------------------------------------------- emission

const WIDTH: usize = 80;

struct Em {
    out: String,
    col: usize,
    ws: bool,
    indention: bool,
    /// flow-leaf style: all-scalar collections are written in flow style.
    flow_leaf: bool,
}

fn is_scalar(v: &Y) -> bool {
    !matches!(v, Y::Seq(_) | Y::Map(_))
}

fn is_leaf(v: &Y) -> bool {
    match v {
        Y::Seq(items) => items.iter().all(is_scalar),
        Y::Map(pairs) => pairs.iter().all(|(k, v)| is_scalar(k) && is_scalar(v)),
        _ => false,
    }
}

fn is_break(c: char) -> bool {
    matches!(c, '\n' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

fn is_ws_or_nul(c: char) -> bool {
    matches!(c, '\0' | ' ' | '\t' | '\r' | '\n' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

#[derive(PartialEq)]
enum Style {
    Plain,
    Single,
    Double,
}

const FIRST_INDICATORS: &str = "#,[]{}&*!|>'\"%@\u{60}";

/// String style choice of 16 §2.3.
fn choose_style(s: &str, flow: bool, key: bool) -> Style {
    let ch: Vec<char> = s.chars().collect();
    let n = ch.len();
    let (mut flow_ind, mut block_ind) = (false, false);
    let (mut line_breaks, mut special) = (false, false);
    let (mut lead_space, mut lead_break, mut trail_space, mut trail_break) = (false, false, false, false);
    let (mut break_space, mut space_break) = (false, false);
    if s.starts_with("---") || s.starts_with("...") {
        flow_ind = true;
        block_ind = true;
    }
    let mut preceded = true;
    let mut followed = n == 1 || (n > 1 && is_ws_or_nul(ch[1]));
    let (mut prev_space, mut prev_break) = (false, false);
    for (i, &c) in ch.iter().enumerate() {
        if i == 0 {
            if FIRST_INDICATORS.contains(c) {
                flow_ind = true;
                block_ind = true;
            }
            if c == '?' || c == ':' {
                flow_ind = true;
                block_ind |= followed;
            }
            if c == '-' && followed {
                flow_ind = true;
                block_ind = true;
            }
        } else {
            if ",?[]{}".contains(c) {
                flow_ind = true;
            }
            if c == ':' {
                flow_ind = true;
                block_ind |= followed;
            }
            if c == '#' && preceded {
                flow_ind = true;
                block_ind = true;
            }
        }
        if is_break(c) {
            line_breaks = true;
        }
        if !(c == '\n' || (' '..='~').contains(&c)) {
            special = true;
        }
        if c == ' ' {
            lead_space |= i == 0;
            trail_space |= i == n - 1;
            break_space |= prev_break;
            prev_space = true;
            prev_break = false;
        } else if is_break(c) {
            lead_break |= i == 0;
            trail_break |= i == n - 1;
            space_break |= prev_space;
            prev_space = false;
            prev_break = true;
        } else {
            prev_space = false;
            prev_break = false;
        }
        preceded = is_ws_or_nul(c);
        followed = i + 2 >= n || is_ws_or_nul(ch[i + 2]);
    }
    let (mut flow_plain, mut block_plain, mut single) = (true, true, true);
    if lead_space || lead_break || trail_space || trail_break {
        flow_plain = false;
        block_plain = false;
    }
    if break_space || space_break || special {
        flow_plain = false;
        block_plain = false;
        single = false;
    }
    if line_breaks {
        flow_plain = false;
        block_plain = false;
    }
    if flow_ind {
        flow_plain = false;
    }
    if block_ind {
        block_plain = false;
    }
    let plain_ok = if flow { flow_plain } else { block_plain };
    if !s.is_empty() && resolve_plain(s) == Plain::Str && plain_ok && !(key && line_breaks) {
        Style::Plain
    } else if single && !(key && line_breaks) {
        Style::Single
    } else {
        Style::Double
    }
}

fn chars(ch: &[char]) -> String {
    ch.iter().collect()
}

impl Em {
    fn new(flow_leaf: bool) -> Self {
        Em { out: String::new(), col: 0, ws: true, indention: true, flow_leaf }
    }
    fn write(&mut self, s: &str) {
        self.out.push_str(s);
        self.col += s.chars().count();
    }
    fn line_break(&mut self) {
        self.out.push('\n');
        self.col = 0;
        self.ws = true;
        self.indention = true;
    }
    fn indent(&mut self, indent: usize) {
        if !self.indention || self.col > indent || (self.col == indent && !self.ws) {
            self.line_break();
        }
        if self.col < indent {
            self.ws = true;
            let pad = " ".repeat(indent - self.col);
            self.write(&pad);
        }
    }
    fn indicator(&mut self, ind: &str, need_ws: bool, whitespace: bool, indention: bool) {
        if self.ws || !need_ws {
            self.write(ind);
        } else {
            self.write(&format!(" {ind}"));
        }
        self.ws = whitespace;
        self.indention = self.indention && indention;
    }

    fn plain(&mut self, text: &str, indent: usize, split: bool) {
        if text.is_empty() {
            return;
        }
        if !self.ws {
            self.write(" ");
        }
        self.ws = false;
        self.indention = false;
        let ch: Vec<char> = text.chars().collect();
        let (mut start, mut spaces) = (0, false);
        for end in 0..=ch.len() {
            let c = ch.get(end).copied();
            if spaces {
                if c != Some(' ') {
                    if start + 1 == end && self.col > WIDTH && split {
                        self.indent(indent);
                        self.ws = false;
                        self.indention = false;
                    } else {
                        self.write(&chars(&ch[start..end]));
                    }
                    start = end;
                }
            } else if c.is_none() || c == Some(' ') {
                self.write(&chars(&ch[start..end]));
                start = end;
            }
            spaces = c == Some(' ');
        }
    }

    fn single(&mut self, text: &str, indent: usize, split: bool) {
        self.indicator("'", true, false, false);
        let ch: Vec<char> = text.chars().collect();
        let n = ch.len();
        let (mut start, mut spaces, mut breaks) = (0, false, false);
        for end in 0..=n {
            let c = ch.get(end).copied();
            if spaces {
                if c != Some(' ') {
                    if start + 1 == end && self.col > WIDTH && split && start != 0 && end != n {
                        self.indent(indent);
                    } else {
                        self.write(&chars(&ch[start..end]));
                    }
                    start = end;
                }
            } else if breaks {
                if !c.is_some_and(is_break) {
                    if ch[start] == '\n' {
                        self.line_break();
                    }
                    for &b in &ch[start..end] {
                        if b == '\n' {
                            self.line_break();
                        } else {
                            self.out.push(b);
                            self.col = 0;
                        }
                    }
                    self.indent(indent);
                    start = end;
                }
            } else if c.is_none_or(|c| c == ' ' || is_break(c) || c == '\'') && start < end {
                self.write(&chars(&ch[start..end]));
                start = end;
            }
            if c == Some('\'') {
                self.write("''");
                start = end + 1;
            }
            if let Some(c) = c {
                spaces = c == ' ';
                breaks = is_break(c);
            }
        }
        self.indicator("'", false, false, false);
    }

    fn double(&mut self, text: &str, indent: usize, split: bool) {
        self.indicator("\"", true, false, false);
        let ch: Vec<char> = text.chars().collect();
        let n = ch.len();
        let mut start = 0;
        for end in 0..=n {
            let c = ch.get(end).copied();
            let escape = c.is_none_or(|c| matches!(c, '"' | '\\' | '\u{85}' | '\u{2028}' | '\u{2029}' | '\u{feff}') || !(' '..='~').contains(&c));
            if escape {
                if start < end {
                    self.write(&chars(&ch[start..end]));
                    start = end;
                }
                if let Some(c) = c {
                    let data = match c {
                        '\0' => "\\0".to_string(),
                        '\u{7}' => "\\a".into(),
                        '\u{8}' => "\\b".into(),
                        '\t' => "\\t".into(),
                        '\n' => "\\n".into(),
                        '\u{b}' => "\\v".into(),
                        '\u{c}' => "\\f".into(),
                        '\r' => "\\r".into(),
                        '\u{1b}' => "\\e".into(),
                        '"' => "\\\"".into(),
                        '\\' => "\\\\".into(),
                        '\u{85}' => "\\N".into(),
                        '\u{a0}' => "\\_".into(),
                        '\u{2028}' => "\\L".into(),
                        '\u{2029}' => "\\P".into(),
                        c if (c as u32) <= 0xff => format!("\\x{:02X}", c as u32),
                        c if (c as u32) <= 0xffff => format!("\\u{:04X}", c as u32),
                        c => format!("\\U{:08X}", c as u32),
                    };
                    self.write(&data);
                    start = end + 1;
                }
            }
            if 0 < end && end + 1 < n && (c == Some(' ') || start >= end) && self.col + end.saturating_sub(start) > WIDTH && split {
                let mut data = if start < end { chars(&ch[start..end]) } else { String::new() };
                data.push('\\');
                if start < end {
                    start = end;
                }
                self.write(&data);
                self.indent(indent);
                self.ws = false;
                self.indention = false;
                if ch[start] == ' ' {
                    self.write("\\");
                }
            }
        }
        self.indicator("\"", false, false, false);
    }

    /// A scalar; indent is the continuation indent, flow marks flow context.
    fn scalar(&mut self, v: &Y, indent: usize, flow: bool, key: bool) {
        let split = !key;
        match v {
            Y::Str(s) => match choose_style(s, flow, key) {
                Style::Plain => self.plain(s, indent, split),
                Style::Single => self.single(s, indent, split),
                Style::Double => self.double(s, indent, split),
            },
            other => self.plain(&scalar_repr(other), indent, split),
        }
    }

    fn block_map(&mut self, pairs: &[(Y, Y)], indent: usize) {
        for (k, v) in pairs {
            self.indent(indent);
            self.scalar(k, indent + 2, false, true);
            self.indicator(":", false, false, false);
            self.block_value(v, indent, true);
        }
    }

    /// A node as a mapping value (in_map) or a sequence item; parent_indent is
    /// the indent of the containing collection.
    fn block_value(&mut self, v: &Y, parent_indent: usize, in_map: bool) {
        if self.flow_leaf && !is_scalar(v) && is_leaf(v) {
            return self.flow(v, parent_indent + 2);
        }
        match v {
            Y::Seq(items) if items.is_empty() => self.indicator("[]", true, false, false),
            Y::Map(pairs) if pairs.is_empty() => self.indicator("{}", true, false, false),
            Y::Seq(items) => {
                // a sequence directly under a mapping key is written indentless
                let indent = if in_map && !self.indention { parent_indent } else { parent_indent + 2 };
                for it in items {
                    self.indent(indent);
                    self.indicator("-", true, false, true);
                    self.block_value(it, indent, false);
                }
            }
            Y::Map(pairs) => self.block_map(pairs, parent_indent + 2),
            scalar => self.scalar(scalar, parent_indent + 2, false, false),
        }
    }
}

impl Em {
    /// A flow collection of scalars; indent is its own indent (enclosing + 2).
    fn flow(&mut self, v: &Y, indent: usize) {
        let (open, close) = if matches!(v, Y::Seq(_)) { ("[", "]") } else { ("{", "}") };
        self.indicator(open, true, true, false);
        let items: Vec<(&Y, Option<&Y>)> = match v {
            Y::Seq(items) => items.iter().map(|i| (i, None)).collect(),
            Y::Map(pairs) => pairs.iter().map(|(k, v)| (k, Some(v))).collect(),
            _ => vec![],
        };
        for (i, (first, value)) in items.into_iter().enumerate() {
            if i > 0 {
                self.indicator(",", false, false, false);
            }
            if self.col > WIDTH {
                self.indent(indent);
            }
            self.scalar(first, indent + 2, true, value.is_some());
            if let Some(val) = value {
                self.indicator(":", false, false, false);
                self.scalar(val, indent + 2, true, false);
            }
        }
        self.indicator(close, false, false, false);
    }
}

fn scalar_repr(v: &Y) -> String {
    match v {
        Y::Null => "null".into(),
        Y::Bool(b) => b.to_string(),
        Y::Int(i) => i.to_string(),
        Y::Float(f) if f.is_nan() => ".nan".into(),
        Y::Float(f) if f.is_infinite() => if *f > 0.0 { ".inf" } else { "-.inf" }.into(),
        Y::Float(f) => {
            let t = float_text(*f).to_lowercase();
            if !t.contains('.') && t.contains('e') { t.replacen('e', ".0e", 1) } else { t }
        }
        Y::Date(d) => crate::text::date_display(d),
        Y::DateTime(dt, off) => crate::text::datetime_display(dt, *off),
        Y::Str(s) | Y::Other(s) => s.clone(),
        Y::Seq(_) | Y::Map(_) => String::new(),
    }
}

/// YAML dump, block style (16 §2.3) of a top-level mapping.
pub fn dump_block(pairs: &[(Y, Y)]) -> String {
    let mut em = Em::new(false);
    if pairs.is_empty() {
        em.write("{}");
    } else {
        em.block_map(pairs, 0);
    }
    em.line_break();
    em.out
}

/// YAML dump, flow-leaf style (16 §2.3) of a top-level mapping.
pub fn dump_flow_leaf(pairs: &[(Y, Y)]) -> String {
    let mut em = Em::new(true);
    let root = Y::Map(pairs.to_vec());
    if is_leaf(&root) {
        em.flow(&root, 2);
    } else {
        em.block_map(pairs, 0);
    }
    em.line_break();
    em.out
}

#[cfg(test)]
mod tests {
    use super::*;

    trait Get {
        fn get(&self, key: &str) -> Option<&Y>;
    }
    impl Get for Y {
        fn get(&self, key: &str) -> Option<&Y> {
            match self {
                Y::Map(pairs) => crate::vault::get(pairs, key),
                _ => None,
            }
        }
    }

    #[test]
    fn deep_nesting_is_invalid_not_a_crash() {
        let doc = format!("a:\n{}x\n", "- ".repeat(5000));
        let r = std::thread::Builder::new().stack_size(2 << 20).spawn(move || parse_doc(&doc).unwrap()).unwrap().join().unwrap();
        assert_eq!(r, None);
        let ok = format!("a: {}x{}\n", "[".repeat(200), "]".repeat(200));
        assert!(parse_doc(&ok).unwrap().is_some());
        let mid = format!("a:\n{}x\n", "- ".repeat(600));
        let r = std::thread::Builder::new().stack_size(2 << 20).spawn(move || parse_doc(&mid).unwrap().is_some()).unwrap().join().unwrap();
        assert!(r, "600 levels are legal and parse off-stack");
    }

    #[test]
    fn alias_bomb_is_invalid() {
        let mut doc = String::from("l0: &l0 [x, x, x, x, x, x, x, x]\n");
        for i in 1..9 {
            doc.push_str(&format!("l{i}: &l{i} [{}]\n", vec![format!("*l{}", i - 1); 8].join(", ")));
        }
        let start = std::time::Instant::now();
        assert_eq!(parse_doc(&doc).unwrap(), None);
        assert!(start.elapsed().as_secs() < 5);
    }

    fn s(v: &str) -> Y {
        Y::Str(v.into())
    }

    fn line(v: &str) -> String {
        dump_block(&[(s("k"), s(v))])
    }

    #[test]
    fn emission_examples() {
        for (v, want) in [
            ("lighthouse", "k: lighthouse\n"),
            ("", "k: ''\n"),
            ("2026-07-05 14:30", "k: 2026-07-05 14:30\n"),
            ("2026-07-05", "k: '2026-07-05'\n"),
            ("yes", "k: 'yes'\n"),
            ("123", "k: '123'\n"),
            ("1.5", "k: '1.5'\n"),
            ("null", "k: 'null'\n"),
            ("grid: outage [pipeline]", "k: 'grid: outage [pipeline]'\n"),
            ("b:c", "k: b:c\n"),
            ("odd\"tag", "k: odd\"tag\n"),
            ("it's", "k: it's\n"),
            ("#x", "k: '#x'\n"),
            ("- x", "k: '- x'\n"),
            (" lead", "k: ' lead'\n"),
            ("don't: stop", "k: 'don''t: stop'\n"),
            ("café", "k: \"caf\\xE9\"\n"),
            ("a—b", "k: \"a\\u2014b\"\n"),
            ("a\tb", "k: \"a\\tb\"\n"),
            ("a\nb", "k: 'a\n\n  b'\n"),
        ] {
            assert_eq!(line(v), want, "{v:?}");
        }
        assert_eq!(dump_block(&[(s("tags"), Y::Seq(vec![s("grid")]))]), "tags:\n- grid\n");
        assert_eq!(dump_block(&[(s("tags"), Y::Seq(vec![]))]), "tags: []\n");
    }

    #[test]
    fn flow_leaf_style() {
        let pairs = vec![
            (s("created"), s("2026-07-05 14:30")),
            (s("aliases"), Y::Seq(vec![s("ORBF"), s("Orbital Inc"), s("b:c")])),
            (s("tags"), Y::Seq(vec![])),
            (s("m"), Y::Map(vec![(s("k"), Y::Int(1))])),
        ];
        assert_eq!(dump_flow_leaf(&pairs), "created: 2026-07-05 14:30
aliases: [ORBF, Orbital Inc, 'b:c']
tags: []
m: {k: 1}
");
        assert_eq!(dump_flow_leaf(&[(s("a"), s("x")), (s("merged_into"), s("y"))]), "{a: x, merged_into: y}
");
        let long: Vec<Y> = (0..20).map(|i| s(&format!("alias{i}"))).collect();
        let out = dump_flow_leaf(&[(s("aliases"), Y::Seq(long.clone()))]);
        assert!(out.lines().nth(1).unwrap().starts_with("  alias"), "{out}");
        assert_eq!(parse_doc(&out).unwrap().unwrap().get("aliases"), Some(&Y::Seq(long)));
    }

    #[test]
    fn long_plain_wraps_at_column_two() {
        let words = vec!["word"; 30].join(" ");
        let out = line(&words);
        assert!(out.lines().nth(1).unwrap().starts_with("  word"));
        assert_eq!(parse_doc(&out).unwrap().unwrap().get("k"), Some(&s(&words)));
    }

    #[test]
    fn implicit_types() {
        let doc = parse_doc("a: yes\nb: 010\nc: 1:30\nd: 09\ne: 1e5\nf: 1.0e+5\ng: 2026-07-05\nh: ~\ni: .inf\nj: 0x1f\nk: 1_000\n").unwrap().unwrap();
        assert_eq!(doc.get("a"), Some(&Y::Bool(true)));
        assert_eq!(doc.get("b"), Some(&Y::Int(8)));
        assert_eq!(doc.get("c"), Some(&Y::Int(90)));
        assert_eq!(doc.get("d"), Some(&s("09")));
        assert_eq!(doc.get("e"), Some(&s("1e5")));
        assert_eq!(doc.get("f"), Some(&Y::Float(100000.0)));
        assert_eq!(doc.get("g"), Some(&Y::Date(NaiveDate::from_ymd_opt(2026, 7, 5).unwrap())));
        assert_eq!(doc.get("h"), Some(&Y::Null));
        assert_eq!(doc.get("i"), Some(&Y::Float(f64::INFINITY)));
        assert_eq!(doc.get("j"), Some(&Y::Int(31)));
        assert_eq!(doc.get("k"), Some(&Y::Int(1000)));
    }

    #[test]
    fn calendar_errors_and_yaml_errors() {
        assert_eq!(parse_doc("d: 2026-02-30\n").unwrap_err().msg, "that day does not exist in that month");
        assert_eq!(parse_doc("d: 2026-13-01\n").unwrap_err().msg, "the month must be between 1 and 12");
        assert_eq!(parse_doc("a: [unclosed\n").unwrap(), None);
        assert_eq!(parse_doc("a: =\n").unwrap(), None);
        assert_eq!(parse_doc("? [a]\n: b\n").unwrap(), None);
    }

    #[test]
    fn merge_and_duplicates() {
        let doc = parse_doc("base: &b {x: 1, y: 2}\nm:\n  <<: *b\n  y: 3\nz: 1\nz: 2\n").unwrap().unwrap();
        assert_eq!(doc.get("m"), Some(&Y::Map(vec![(s("x"), Y::Int(1)), (s("y"), Y::Int(3))])));
        assert_eq!(doc.get("z"), Some(&Y::Int(2)));
        let dt = parse_doc("t: 2026-07-05 10:00:00.5Z\n").unwrap().unwrap();
        assert_eq!(crate::text::display(dt.get("t").unwrap()), "2026-07-05 10:00:00.500000+00:00");
    }
}
