//! Search (04): note walk, candidates, BM25L, vectors, Smart Connections,
//! hybrid fusion, plus the scan tools query_notes, get_backlinks, list_recent.
use crate::embed::Embedder;
use crate::error::{Error, Result};
use crate::text::{casefold, display, minute_stamp, split_lines, strip};
use crate::vault::{Vault, body, get, map_safe, parse, read_lenient, str_list};
use crate::yaml::Y;
use indexmap::IndexMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

const SKIPPED: [&str; 3] = [".obsidian", ".trash", ".git"];
const LANE_DEPTH: usize = 50;
const MODEL_KEY: &str = "TaylorAI/bge-micro-v2";

/// walk(folder) (04 §2): (absolute, rel) pairs in path order.
pub fn walk(v: &Vault, folder: Option<&str>) -> Result<Vec<(PathBuf, String)>> {
    let base = match folder {
        Some(f) if !f.is_empty() => v.resolve(f)?,
        _ => v.root.clone(),
    };
    if !base.is_dir() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    let entries = WalkDir::new(&base)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !(e.file_type().is_dir() && e.file_name().to_str().is_some_and(|n| SKIPPED.contains(&n))));
    for e in entries.flatten() {
        if !e.file_name().to_string_lossy().ends_with(".md") {
            continue;
        }
        let rel = v.rel_of(e.path());
        if rel.split('/').any(|c| SKIPPED.contains(&c)) {
            continue;
        }
        // 00 §7: a symlink whose real path leaves the vault is never walked
        if e.path_is_symlink() && !crate::vault::resolve_path(e.path()).starts_with(&v.root) {
            continue;
        }
        out.push((e.into_path(), rel));
    }
    Ok(out)
}

/// The casefolded tag subset test of 04 §3.
fn tags_match(meta: &[(Y, Y)], tags: &[String]) -> bool {
    let have: HashSet<String> = str_list(get(meta, "tags")).iter().map(|t| casefold(t)).collect();
    tags.iter().all(|t| have.contains(&casefold(t)))
}

/// candidates(tags, folder) (04 §3): (rel, text) in walk order.
pub fn candidates(v: &Vault, tags: Option<&[String]>, folder: Option<&str>) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (abs, rel) in walk(v, folder)? {
        let text = read_lenient(&abs)?;
        if let Some(t) = tags.filter(|t| !t.is_empty())
            && !tags_match(&parse(&text)?, t)
        {
            continue;
        }
        out.push((rel, text));
    }
    Ok(out)
}

// ------------------------------------------------------------------ BM25L

pub fn tokenize(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    lower.split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit())).filter(|t| !t.is_empty()).map(String::from).collect()
}

const K1: f64 = 1.5;
const B: f64 = 0.75;
const DELTA: f64 = 0.5;
/// A note's title words count this many extra times: the title says what the note is about.
const TITLE_WEIGHT: usize = 3;

/// rank(corpus, query, limit) (04 §4).
pub fn rank(corpus: &[(String, String)], query: &str, limit: usize) -> Vec<(String, f64)> {
    if corpus.is_empty() {
        return vec![];
    }
    let docs: Vec<HashMap<String, usize>> = corpus
        .iter()
        .map(|(rel, t)| {
            let mut tf = HashMap::new();
            for tok in tokenize(t) {
                *tf.entry(tok).or_insert(0) += 1;
            }
            for tok in tokenize(stem_of(rel)) {
                *tf.entry(tok).or_insert(0) += TITLE_WEIGHT;
            }
            tf
        })
        .collect();
    let lens: Vec<f64> = docs.iter().map(|d| d.values().sum::<usize>() as f64).collect();
    let n = corpus.len() as f64;
    let avgdl = lens.iter().sum::<f64>() / n;
    let mut df: HashMap<&str, usize> = HashMap::new();
    for d in &docs {
        for t in d.keys() {
            *df.entry(t.as_str()).or_insert(0) += 1;
        }
    }
    let q = tokenize(query);
    let mut scored: Vec<(String, f64)> = corpus
        .iter()
        .zip(docs.iter().zip(&lens))
        .map(|((rel, _), (d, dl))| {
            let mut score = 0.0;
            for t in &q {
                let tf = d.get(t).copied().unwrap_or(0) as f64;
                let Some(&dft) = df.get(t.as_str()) else { continue };
                if tf == 0.0 {
                    continue;
                }
                let idf = (n + 1.0).ln() - (dft as f64 + 0.5).ln();
                let c = tf / (1.0 - B + B * dl / avgdl);
                score += idf * (K1 + 1.0) * (c + DELTA) / (K1 + c + DELTA);
            }
            (rel.clone(), score)
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().filter(|(_, s)| *s > 0.0).take(limit).collect()
}

// ---------------------------------------------------------------- vectors

pub fn cosine(a: &[f64], b: &[f64]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
}

pub fn content_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub hash: String,
    pub vec: Vec<f64>,
}

pub type Cache = IndexMap<String, Entry>;

type Stamp = (u128, u64);

fn stamp(path: &Path) -> Option<Stamp> {
    let m = fs::metadata(path).ok()?;
    let ns = m.modified().ok()?.duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    Some((ns, m.len()))
}

static CACHE_MEMO: Memo<Cache> = LazyLock::new(Default::default);

/// load(path) of a vector cache file (04 §5), memoised by (mtime ns, size).
pub fn load_cache(path: &Path) -> Cache {
    let Some(st) = stamp(path) else { return Cache::new() };
    let mut memo = CACHE_MEMO.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((s, m)) = memo.get(path)
        && *s == st
    {
        return m.clone();
    }
    match fs::read(path).ok().and_then(|b| serde_json::from_slice::<Cache>(&b).ok()) {
        Some(m) => {
            memo.insert(path.to_path_buf(), (st, m.clone()));
            m
        }
        None => Cache::new(),
    }
}

/// save(path, map): temp file in the same folder, then atomic rename.
pub fn save_cache(path: &Path, map: &Cache) -> Result<()> {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_";
    let rand: String = (0..8).map(|_| CHARS[fastrand::usize(..CHARS.len())] as char).collect();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.{rand}.tmp"));
    let res = fs::write(&tmp, crate::json::compact(map))
        .and_then(|_| crate::config::owner_only(&tmp, 0o600))
        .and_then(|_| crate::faults::os("cache_rename"))
        .and_then(|_| fs::rename(&tmp, path));
    if let Err(e) = res {
        let _ = fs::remove_file(&tmp);
        return Err(Error::io(&e, path));
    }
    if let Some(st) = stamp(path) {
        CACHE_MEMO.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf(), (st, map.clone()));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScVec {
    pub vec: Vec<f64>,
    pub fresh: bool,
}

type Memo<T> = LazyLock<Mutex<HashMap<PathBuf, (Stamp, T)>>>;

/// One usable smart_sources entry: (rel path, vector, last_embed.at).
type ScEntry = (String, Vec<f64>, f64);

static SC_MEMO: Memo<Arc<Vec<ScEntry>>> = LazyLock::new(Default::default);

fn truthy_json(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// The usable smart_sources entries of one .ajson file, in key order; memoised
/// by (mtime, size), failures included, and shared rather than copied.
fn parse_ajson(path: &Path) -> Arc<Vec<ScEntry>> {
    let Some(st) = stamp(path) else { return Arc::default() };
    let mut memo = SC_MEMO.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((s, m)) = memo.get(path)
        && *s == st
    {
        return m.clone();
    }
    let text = read_lenient(path).unwrap_or_default();
    let lines: Vec<&str> =
        split_lines(&text).into_iter().map(strip).filter(|l| !l.is_empty()).map(|l| l.trim_end_matches(',')).collect();
    let obj = serde_json::from_str::<Map<String, Value>>(&format!("{{{}}}", lines.join(","))).unwrap_or_default();
    let mut entries = Vec::new();
    for (key, entry) in obj {
        if !key.starts_with("smart_sources:") {
            continue;
        }
        let Some(rel) = entry.get("path").filter(|p| truthy_json(p)).and_then(Value::as_str) else { continue };
        let Some(model) = entry.get("embeddings").and_then(|e| e.get(MODEL_KEY)).filter(|m| truthy_json(m)) else { continue };
        let Some(vec) = model.get("vec").and_then(Value::as_array) else { continue };
        let Some(vec) = vec.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>() else { continue };
        let at = entry.get("last_embed").and_then(|l| l.get("at")).and_then(Value::as_f64).unwrap_or(0.0);
        entries.push((rel.to_string(), vec, at));
    }
    let shared = Arc::new(entries);
    memo.insert(path.to_path_buf(), (st, shared.clone()));
    shared
}

/// Smart Connections vectors (04 §6).
pub fn smart_vectors(root: &Path) -> IndexMap<String, ScVec> {
    let dir = root.join(".smart-env").join("multi");
    let mut out = IndexMap::new();
    let Ok(rd) = fs::read_dir(&dir) else { return out };
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "ajson")).collect();
    files.sort();
    for f in files {
        for (rel, vec, at) in parse_ajson(&f).iter() {
            let Ok(meta) = fs::metadata(root.join(rel)) else { continue };
            if !meta.is_file() {
                continue;
            }
            let mtime = meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_secs_f64());
            out.insert(rel.clone(), ScVec { vec: vec.clone(), fresh: *at >= mtime * 1000.0 });
        }
    }
    out
}

pub fn fallback_path(state_dir: &Path) -> PathBuf {
    state_dir.join("fallback_embeddings.json")
}

fn whole_vault(v: &Vault) -> Result<Vec<(String, String)>> {
    walk(v, None)?.into_iter().map(|(abs, rel)| Ok((rel, read_lenient(&abs)?))).collect()
}

/// partition(): the known vectors in order plus the stale (rel, text) list.
type Vectors = Vec<(String, Vec<f64>)>;

fn partition(v: &Vault, state_dir: &Path, texts: &[(String, String)]) -> (Vectors, Vec<(String, String)>, Cache) {
    let sc = smart_vectors(&v.root);
    let cache = load_cache(&fallback_path(state_dir));
    let (mut known, mut stale) = (Vec::new(), Vec::new());
    for (rel, text) in texts {
        if let Some(s) = sc.get(rel).filter(|s| s.fresh) {
            known.push((rel.clone(), s.vec.clone()));
        } else if let Some(e) = cache.get(rel).filter(|e| e.hash == content_hash(text)) {
            known.push((rel.clone(), e.vec.clone()));
        } else {
            stale.push((rel.clone(), text.clone()));
        }
    }
    (known, stale, cache)
}

/// note_vectors(note_texts, persist) (04 §5); None means the whole vault.
pub fn note_vectors(
    v: &Vault,
    state_dir: &Path,
    emb: &dyn Embedder,
    texts: Option<&[(String, String)]>,
    persist: bool,
) -> Result<Vectors> {
    let owned;
    let texts = match texts {
        Some(t) => t,
        None => {
            owned = whole_vault(v)?;
            &owned
        }
    };
    // one embedder at a time per process, so concurrent searches neither embed
    // the same notes twice nor overwrite each other's new cache entries
    static VECTORS: Mutex<()> = Mutex::new(());
    let _guard = VECTORS.lock().unwrap_or_else(|e| e.into_inner());
    let (mut known, stale, mut cache) = partition(v, state_dir, texts);
    if !stale.is_empty() {
        let refs: Vec<&str> = stale.iter().map(|(_, t)| t.as_str()).collect();
        let vecs = emb.embed(&refs)?;
        for ((rel, text), vec) in stale.into_iter().zip(vecs) {
            cache.insert(rel.clone(), Entry { hash: content_hash(&text), vec: vec.clone() });
            known.push((rel, vec));
        }
        if persist {
            save_cache(&fallback_path(state_dir), &cache)?;
        }
    }
    Ok(known)
}

/// stale_notes(): read-only, never embeds or writes.
pub fn stale_notes(v: &Vault, state_dir: &Path) -> Result<Vec<String>> {
    let texts = whole_vault(v)?;
    Ok(partition(v, state_dir, &texts).1.into_iter().map(|(rel, _)| rel).collect())
}

// ----------------------------------------------------------------- hybrid

pub fn rrf(lanes: &[Vec<String>]) -> Vec<String> {
    rrf_weighted(lanes, |_| 1.0)
}

/// Entity notes are generated from other notes, so a hit on one counts half:
/// the note someone wrote should come before the entity that was extracted from it.
const DERIVED_WEIGHT: f64 = 0.5;

fn hit_weight(rel: &str) -> f64 {
    if rel.starts_with("Claude/Graph/") { DERIVED_WEIGHT } else { 1.0 }
}

/// RRF with each item's fused score multiplied by weight(item) before sorting.
pub fn rrf_weighted(lanes: &[Vec<String>], weight: impl Fn(&str) -> f64) -> Vec<String> {
    let mut scores: IndexMap<&str, f64> = IndexMap::new();
    for lane in lanes {
        for (i, item) in lane.iter().enumerate() {
            *scores.entry(item.as_str()).or_insert(0.0) += 1.0 / (60.0 + (i + 1) as f64);
        }
    }
    let mut items: Vec<(&str, f64)> = scores.into_iter().map(|(k, s)| (k, s * weight(k))).collect();
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    items.into_iter().map(|(s, _)| s.to_string()).collect()
}

fn stem_of(rel: &str) -> &str {
    let last = rel.rsplit('/').next().unwrap_or(rel);
    let cut = last.char_indices().rev().nth(2).map_or(0, |(i, _)| i);
    &last[..cut]
}

/// Slice-style limit (edge case Q7).
pub fn slice<T>(mut v: Vec<T>, limit: i64) -> Vec<T> {
    let keep = if limit >= 0 { (limit as usize).min(v.len()) } else { v.len().saturating_sub(limit.unsigned_abs() as usize) };
    v.truncate(keep);
    v
}

pub fn excerpt(text: &str, rel: &str, query: &str) -> String {
    let ql = query.to_lowercase();
    if stem_of(rel).to_lowercase().contains(&ql) {
        return "(title match)".into();
    }
    let lines = split_lines(body(text));
    if let Some(l) = lines.iter().find(|l| l.to_lowercase().contains(&ql)) {
        return strip(l).to_string();
    }
    match lines.iter().map(|l| strip(l)).find(|l| !l.is_empty()) {
        Some(l) => l.chars().take(120).collect(),
        None => String::new(),
    }
}

fn substring_lane(corpus: &[(String, String)], query: &str) -> Vec<String> {
    let ql = query.to_lowercase();
    let mut sorted: Vec<&(String, String)> = corpus.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted
        .into_iter()
        .filter(|(rel, text)| stem_of(rel).to_lowercase().contains(&ql) || split_lines(text).iter().any(|l| l.to_lowercase().contains(&ql)))
        .take(LANE_DEPTH)
        .map(|(rel, _)| rel.clone())
        .collect()
}

/// hybrid(query, tags, folder, limit) (04 §7): [{path, excerpt}].
pub fn hybrid(
    v: &Vault,
    state_dir: &Path,
    emb: &dyn Embedder,
    query: &str,
    tags: Option<&[String]>,
    folder: Option<&str>,
    limit: i64,
) -> Result<Vec<Value>> {
    let corpus = candidates(v, tags, folder)?;
    if corpus.is_empty() {
        return Ok(vec![]);
    }
    let keyword: Vec<String> = rank(&corpus, query, LANE_DEPTH).into_iter().map(|(r, _)| r).collect();
    let unfiltered = tags.is_none_or(|t| t.is_empty()) && folder.is_none_or(|f| f.is_empty());
    let vectors = note_vectors(v, state_dir, emb, unfiltered.then_some(corpus.as_slice()), true)?;
    let q = emb.embed(&[query])?.into_iter().next().unwrap_or_default();
    let in_corpus: HashSet<&str> = corpus.iter().map(|(r, _)| r.as_str()).collect();
    let mut sem: Vec<(String, f64)> =
        vectors.into_iter().filter(|(r, _)| in_corpus.contains(r.as_str())).map(|(r, vec)| (r, cosine(&vec, &q))).collect();
    sem.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let sem: Vec<String> = sem.into_iter().filter(|(_, s)| *s > 0.0).take(LANE_DEPTH).map(|(r, _)| r).collect();
    let mut lanes = vec![keyword, sem];
    if lanes[0].is_empty() {
        lanes.push(substring_lane(&corpus, query));
    }
    let texts: HashMap<&str, &str> = corpus.iter().map(|(r, t)| (r.as_str(), t.as_str())).collect();
    Ok(slice(rrf_weighted(&lanes, hit_weight), limit)
        .into_iter()
        .map(|rel| {
            let ex = excerpt(texts.get(rel.as_str()).copied().unwrap_or(""), &rel, query);
            json!({"path": rel, "excerpt": ex})
        })
        .collect())
}

// ------------------------------------------------------------- scan tools

/// query_notes (02 §5.17).
pub fn query_notes(v: &Vault, project: Option<&str>, tags: Option<&[String]>, folder: Option<&str>, limit: i64) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    let tags = tags.filter(|t| !t.is_empty());
    for (abs, rel) in walk(v, folder)? {
        let meta = parse(&read_lenient(&abs)?)?;
        if let Some(p) = project {
            let have = get(&meta, "project").map(display).unwrap_or_default();
            if casefold(&have) != casefold(p) {
                continue;
            }
        }
        if let Some(t) = tags
            && !tags_match(&meta, t)
        {
            continue;
        }
        if project.is_none() && tags.is_none() && meta.is_empty() {
            continue;
        }
        out.push(json!({"path": rel, "frontmatter": map_safe(&meta)}));
        if out.len() as i64 >= limit {
            break;
        }
    }
    Ok(out)
}

static WIKILINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\[([^\]|#]+)").expect("static regex"));

/// get_backlinks (02 §5.18).
pub fn backlinks(v: &Vault, path: &str) -> Result<Vec<String>> {
    let target = v.resolve(path)?;
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name.as_str(),
    };
    let want = casefold(stem);
    let mut out = Vec::new();
    for (abs, rel) in walk(v, None)? {
        if abs == target {
            continue;
        }
        let text = read_lenient(&abs)?;
        let hit = WIKILINK.captures_iter(&text).any(|c| casefold(strip(&c[1]).rsplit('/').next().unwrap_or("")) == want);
        if hit {
            out.push(rel);
        }
    }
    Ok(out)
}

/// list_recent (02 §5.19).
pub fn list_recent(v: &Vault, n: i64) -> Result<Vec<Value>> {
    let mut items = Vec::new();
    for (abs, rel) in walk(v, None)? {
        let m = fs::metadata(&abs).and_then(|m| m.modified()).map_err(|e| Error::io(&e, &abs))?;
        let secs = match m.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs_f64(),
            Err(e) => -e.duration().as_secs_f64(),
        };
        items.push((secs, rel));
    }
    items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then_with(|| b.1.cmp(&a.1)));
    Ok(slice(items, n)
        .into_iter()
        .map(|(secs, rel)| {
            let t = chrono::DateTime::from_timestamp(secs.floor() as i64, 0)
                .map(|d| d.with_timezone(&chrono::Local).naive_local())
                .unwrap_or_default();
            json!({"path": rel, "modified": minute_stamp(&t)})
        })
        .collect())
}
