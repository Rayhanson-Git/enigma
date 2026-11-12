//! acceptance/blocking.yaml (fake embedder [len(text), 1.0])
mod common;
use common::*;
use enigma_mcp::consolidate::*;
use enigma_mcp::embed::Embedder;
use enigma_mcp::error::Result;
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

#[derive(Default)]
struct Rec(Mutex<Vec<String>>);
impl Embedder for Rec {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f64>>> {
        self.0.lock().unwrap().extend(texts.iter().map(|t| t.to_string()));
        Ok(texts.iter().map(|t| vec![t.chars().count() as f64, 1.0]).collect())
    }
}

fn e(path: &str, ty: &str) -> Ent {
    Ent { name: path.to_string(), ty: ty.into(), path: path.to_string(), aliases: vec![], summary: String::new() }
}

fn vecs(items: &[(&str, Vec<f64>)]) -> HashMap<String, Vec<f64>> {
    items.iter().map(|(p, v)| (p.to_string(), v.clone())).collect()
}

fn pairs(v: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

#[test]
fn blocking_bl_01() {
    let c = Ent { name: "Cedar Cloud".into(), ty: "organization".into(), path: "p".into(), aliases: vec!["CC".into()], summary: "Cloud VM.".into() };
    assert_eq!(identity_text(&c), "Cedar Cloud\nCC\nCloud VM.");
}

#[test]
fn blocking_bl_02() {
    let a = Ent { name: "Acme".into(), ty: "organization".into(), path: "x".into(), aliases: vec![], summary: "One.".into() };
    let mut b = a.clone();
    b.path = "elsewhere".into();
    assert_eq!(identity_hash(&a), identity_hash(&b));
    b.summary = "Two.".into();
    assert_ne!(identity_hash(&a), identity_hash(&b));
    assert_eq!(identity_hash(&a).len(), 64);
}

#[test]
fn blocking_bl_03() {
    let st = tempfile::tempdir().unwrap();
    let mut es = vec![e("Acme", "organization"), e("Beta", "organization")];
    let r = Rec::default();
    let v = compute_entity_vectors(st.path(), &es, &r, true).unwrap();
    assert!(v.contains_key("Acme") && v.contains_key("Beta"));
    r.0.lock().unwrap().clear();
    compute_entity_vectors(st.path(), &es, &r, true).unwrap();
    assert!(r.0.lock().unwrap().is_empty());
    es[0].summary = "DIFFERENT".into();
    compute_entity_vectors(st.path(), &es, &r, true).unwrap();
    assert_eq!(*r.0.lock().unwrap(), ["Acme\n\nDIFFERENT"]);
}

#[test]
fn blocking_bl_04() {
    let all = vec![e("p1", "person"), e("p2", "person"), e("o1", "organization")];
    let v = vecs(&[("p1", vec![1.0, 0.0]), ("p2", vec![1.0, 0.01]), ("o1", vec![1.0, 0.0])]);
    assert_eq!(candidate_pairs(&all, &all, &v, K_NEIGHBORS), pairs(&[("p1", "p2")]));
}

#[test]
fn blocking_bl_05() {
    let all = vec![e("a", "topic"), e("b", "topic")];
    let v = vecs(&[("a", vec![1.0, 0.0]), ("b", vec![0.0, 1.0])]);
    assert!(candidate_pairs(&all, &all, &v, K_NEIGHBORS).is_empty());
    let six: Vec<Ent> = (0..6).map(|i| e(&format!("n{i}"), "topic")).collect();
    let v: HashMap<String, Vec<f64>> = six.iter().enumerate().map(|(i, x)| (x.path.clone(), vec![1.0, 0.001 * i as f64])).collect();
    assert_eq!(candidate_pairs(&six[..1], &six, &v, 2).len(), 2);
}

#[test]
fn blocking_bl_06() {
    assert_eq!(clusters(&pairs(&[("a", "b"), ("b", "c")])), [vec!["a", "b", "c"]]);
    let star: Vec<(String, String)> = (1..=10).map(|i| ("a00".to_string(), format!("a{i:02}"))).collect();
    let star: BTreeSet<(String, String)> = star.into_iter().collect();
    let cs = clusters(&star);
    let mut sizes: Vec<usize> = cs.iter().map(Vec::len).collect();
    assert_eq!(sizes, [6, 5]);
    sizes.sort();
    assert_eq!(sizes, [5, 6]);
    let members: BTreeSet<String> = cs.into_iter().flatten().collect();
    assert_eq!(members.len(), 11);
}

#[test]
fn blocking_bl_07() {
    let all = vec![e("a", "topic"), e("b", "topic"), e("lonely", "person")];
    let v = vecs(&[("a", vec![1.0, 0.0]), ("b", vec![1.0, 0.0]), ("lonely", vec![1.0, 0.0])]);
    let cs = candidate_clusters(&all, &all, &v);
    assert_eq!(cs, [vec![all[0].clone(), all[1].clone()]]);
}

#[test]
fn blocking_bl_08() {
    let c = |n: usize, tag: &str| (0..n).map(|i| e(&format!("{tag}{i}"), "topic")).collect::<Vec<_>>();
    let sizes = |b: Vec<Vec<Ent>>| b.iter().map(Vec::len).collect::<Vec<_>>();
    assert_eq!(sizes(batch_clusters(vec![c(3, "a"), c(4, "b"), c(3, "c")], 8)), [7, 3]);
    assert_eq!(sizes(batch_clusters(vec![c(6, "a"), c(6, "b")], 8)), [6, 6]);
}

fn named(paths: &[&str]) -> Vec<Ent> {
    paths.iter().map(|p| e(p, "topic")).collect()
}

fn all_checked(es: &[Ent]) -> HashMap<String, String> {
    es.iter().map(|x| (x.path.clone(), identity_hash(x))).collect()
}

fn paths(s: &[Ent]) -> Vec<&str> {
    s.iter().map(|x| x.path.as_str()).collect()
}

#[test]
fn blocking_bl_09() {
    let es: Vec<Ent> = (0..500).map(|i| e(&format!("e{i:03}"), "topic")).collect();
    let (s, _, used) = select_slice(&es, &HashMap::new(), None, 200, true);
    assert_eq!((s.len(), used), (200, false));
}

#[test]
fn blocking_bl_10() {
    let es = named(&["a", "b", "c", "d"]);
    let mut checked = all_checked(&es);
    checked.remove("c");
    let (s, _, used) = select_slice(&es, &checked, None, 3, true);
    assert_eq!(paths(&s)[0], "c");
    assert!(used);
}

#[test]
fn blocking_bl_11() {
    let es = named(&["a", "b", "c", "d"]);
    let checked = all_checked(&es);
    let (s, cur, used) = select_slice(&es, &checked, Some("b"), 2, true);
    assert_eq!((paths(&s), cur.as_deref(), used), (vec!["c", "d"], Some("d"), true));
    let (s, cur, _) = select_slice(&es, &checked, Some("d"), 2, true);
    assert_eq!((paths(&s), cur.as_deref()), (vec!["a", "b"], Some("b")));
}

#[test]
fn blocking_bl_12() {
    let es = named(&["a", "b"]);
    let (s, cur, used) = select_slice(&es, &all_checked(&es), Some("a"), 2, false);
    assert!(s.is_empty());
    assert_eq!((cur.as_deref(), used), (Some("a"), false));
}

#[test]
fn blocking_bl_13() {
    let es = named(&["a", "c", "d", "b"]);
    let (s, cur, _) = select_slice(&es, &all_checked(&es), Some("a"), 1, true);
    assert_eq!((paths(&s), cur.as_deref()), (vec!["b"], Some("b")));
}

#[test]
fn blocking_bl_14() {
    let st = tempfile::tempdir().unwrap();
    std::fs::write(vectors_path(st.path()), "{corrupt").unwrap();
    let r = Rec::default();
    let es = named(&["a"]);
    compute_entity_vectors(st.path(), &es, &r, true).unwrap();
    assert_eq!(r.0.lock().unwrap().len(), 1);
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(vectors_path(st.path())).unwrap()).unwrap();
    assert!(saved["a"]["vec"].is_array());
}

#[test]
fn blocking_bl_15() {
    let st = tempfile::tempdir().unwrap();
    compute_entity_vectors(st.path(), &named(&["a", "b"]), &Rec::default(), true).unwrap();
    assert_eq!(prune_entity_vectors(st.path(), &["a".into()]).unwrap(), 1);
    assert_eq!(prune_entity_vectors(st.path(), &["a".into()]).unwrap(), 0);
}

#[test]
fn blocking_bl_16() {
    let st = tempfile::tempdir().unwrap();
    let v = compute_entity_vectors(st.path(), &named(&["a"]), &LenEmb::default(), false).unwrap();
    assert!(v.contains_key("a"));
    assert!(!vectors_path(st.path()).exists());
}
