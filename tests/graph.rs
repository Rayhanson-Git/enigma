//! acceptance/graph.yaml
mod common;
use common::*;
use enigma_mcp::graph::*;
use enigma_mcp::search::{backlinks, list_recent, query_notes};
use enigma_mcp::vault::{get, parse, str_list};
use enigma_mcp::yaml::Y;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

fn ent(name: &str, ty: &str, aliases: &[&str], summary: &str) -> Entity {
    Entity { name: name.into(), ty: ty.into(), aliases: aliases.iter().map(|a| a.to_string()).collect(), summary: summary.into() }
}
fn orbital() -> Entity {
    ent("Orbital Freight", "organization", &["ORBF"], "A company.")
}
fn routes() -> Entity {
    ent("Trade Routes", "domain", &[], "Logistics.")
}
fn rel_(from: &str, ft: &str, rel: &str, to: &str, tt: &str) -> Relation {
    Relation { from: from.into(), from_type: ft.into(), rel: rel.into(), to: to.into(), to_type: tt.into(), evidence: String::new() }
}
fn orb_rel() -> Relation {
    rel_("Orbital Freight", "organization", "operates_in", "Trade Routes", "domain")
}
fn x(entities: Vec<Entity>, relations: Vec<Relation>) -> Extraction {
    Extraction { entities, relations }
}
const ORB: &str = "Claude/Graph/Organizations/Orbital Freight.md";

fn paths(rows: &[Value]) -> Vec<String> {
    rows.iter().map(|r| r["path"].as_str().unwrap().to_string()).collect()
}

// ---------------------------------------------------------- vault queries

#[test]
fn graph_gq_01() {
    let d = mini_vault();
    write_file(d.path(), "Claude/Sessions/2026-07-05 Q.md", "---\nproject: enigma\ntags: [infra]\n---\n\nBody\n");
    let rows = query_notes(&open(&d), Some("enigma"), None, None, 50).unwrap();
    assert_eq!(paths(&rows), ["Claude/Sessions/2026-07-05 Q.md"]);
    assert_eq!(rows[0]["frontmatter"]["project"], "enigma");
}

#[test]
fn graph_gq_02() {
    let d = mini_vault();
    let tags = vec!["GRID".to_string()];
    assert_eq!(paths(&query_notes(&open(&d), None, Some(&tags), None, 50).unwrap()), ["Projects/Lighthouse Grid.md"]);
}

#[test]
fn graph_gq_03() {
    let d = mini_vault();
    let all = paths(&query_notes(&open(&d), None, None, None, 50).unwrap());
    assert!(all.contains(&"Projects/Lighthouse Grid.md".into()) && !all.contains(&"Daily.md".into()));
    assert_eq!(query_notes(&open(&d), None, None, None, 1).unwrap().len(), 1);
}

#[test]
fn graph_gq_04() {
    let d = mini_vault();
    write_file(d.path(), "A.md", "see [[postgres]]\n");
    write_file(d.path(), "B.md", "see [[Claude/Concepts/Postgres|the db]]\n");
    let b = backlinks(&open(&d), "Claude/Concepts/Postgres.md").unwrap();
    assert!(b.contains(&"A.md".into()) && b.contains(&"B.md".into()));
}

#[test]
fn graph_gq_05() {
    let d = mini_vault();
    write_file(d.path(), "Self.md", "I link to [[Self]].\n");
    assert!(backlinks(&open(&d), "Daily.md").unwrap().is_empty());
    assert!(backlinks(&open(&d), "Self.md").unwrap().is_empty());
}

#[test]
fn graph_gq_06() {
    let d = mini_vault();
    write_file(d.path(), "Typed.md", "---\ncreated: 2026-07-05\ntags: [a, b]\ncount: 3\ndone: true\n---\n");
    let tags = vec!["a".to_string()];
    let rows = query_notes(&open(&d), None, Some(&tags), None, 50).unwrap();
    let fm = &rows[0]["frontmatter"];
    assert_eq!(fm["tags"], json!(["a", "b"]));
    assert_eq!(fm["count"], json!(3));
    assert_eq!(fm["done"], json!(true));
    assert!(fm["created"].as_str().unwrap().starts_with("2026-07-05"));
    assert!(serde_json::to_string(&rows).is_ok());
}

#[test]
fn graph_gq_07() {
    let d = mini_vault();
    for (i, rel) in ["Daily.md", "Projects/Lighthouse Grid.md"].iter().enumerate() {
        let f = fs::File::options().write(true).open(d.path().join(rel)).unwrap();
        f.set_modified(UNIX_EPOCH + Duration::from_secs(4_000_000_000 + i as u64 * 100)).unwrap();
    }
    let rows = list_recent(&open(&d), 2).unwrap();
    assert_eq!(paths(&rows), ["Projects/Lighthouse Grid.md", "Daily.md"]);
    assert!(rows.iter().all(|r| r["modified"].is_string()));
}

// ------------------------------------------------------------------ cache

fn table_names(db: &Path) -> Vec<String> {
    let c = rusqlite::Connection::open(db).unwrap();
    let mut st = c.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").unwrap();
    st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
}

#[test]
fn graph_gc_01() {
    let d = mini_vault();
    let state = tempfile::tempdir().unwrap();
    let db = state.path().join("graph.db");
    let v = open(&d);
    apply(&v, "Projects/Lighthouse Grid.md", &x(vec![orbital(), routes()], vec![orb_rel()]), &now()).unwrap();
    write_file(d.path(), "Claude/Inbox/interview.md", "Interview with Orbital.\n");
    apply(&v, "Claude/Inbox/interview.md", &x(vec![orbital()], vec![]), &now()).unwrap();
    rebuild(&v, &db).unwrap();
    assert_eq!(table_names(&db), ["edges", "entities", "mentions"]);
    let g = Graph::load(&db).unwrap();
    let orb = &g.find("orbital", None)[0];
    assert_eq!((orb["name"].as_str(), orb["type"].as_str(), orb["mention_count"].as_i64()), (Some("Orbital Freight"), Some("organization"), Some(2)));
    assert!(orb["relations"].as_array().unwrap().contains(&json!({"rel": "operates_in", "to": "Trade Routes"})));
    assert!(!g.find("ORBF", None).is_empty());
    assert!(!g.find("trade", Some("domain")).is_empty());
    assert!(g.find("trade", Some("person")).is_empty());
    let r1 = g.related("Claude/Inbox/interview.md", 1, 20).unwrap();
    let lh = r1.iter().find(|r| r["path"] == "Projects/Lighthouse Grid.md").unwrap();
    assert!(lh["via"].as_str().unwrap().contains("Orbital Freight"));
    for p in ["Claude/Inbox/interview.md", "Claude/Inbox/interview"] {
        let r2 = paths(&g.related(p, 2, 20).unwrap());
        assert!(!r2.contains(&"Claude/Inbox/interview.md".into()) && r2.iter().all(|x| !x.starts_with("Claude/Graph/")));
        assert!(r2.contains(&"Projects/Lighthouse Grid.md".into()));
    }
    assert_eq!(g.stats(), json!({"entities": {"domain": 1, "organization": 1}, "edges": 1, "mentions": 3}));
    rebuild(&v, &db).unwrap();
    let g = Graph::load(&db).unwrap();
    assert_eq!(g.note_entity_paths("Claude/Inbox/interview.md"), ["Claude/Graph/Organizations/Orbital Freight"]);
    assert!(g.note_entity_paths("Nope.md").is_empty());
}

#[test]
fn graph_gc_02() {
    let d = mini_vault();
    let state = tempfile::tempdir().unwrap();
    let db = state.path().join("graph.db");
    let v = open(&d);
    write_file(d.path(), "A.md", "a");
    write_file(d.path(), "B.md", "b");
    let ops = |ty: &str| ent("Ops", ty, &[], "");
    apply(&v, "A.md", &x(vec![ops("domain")], vec![rel_("Globex", "organization", "operates_in", "Ops", "domain")]), &now()).unwrap();
    apply(&v, "B.md", &x(vec![ops("topic")], vec![]), &now()).unwrap();
    rebuild(&v, &db).unwrap();
    let found = Graph::load(&db).unwrap().find("ops", None);
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|e| e["mention_count"] == 1));
    let topic = found.iter().find(|e| e["type"] == "topic").unwrap();
    assert_eq!(topic["relations"], json!([]));
}

#[test]
fn graph_gc_03() {
    let d = mini_vault();
    let state = tempfile::tempdir().unwrap();
    let db = state.path().join("graph.db");
    let v = open(&d);
    let hub = ent("Everything", "domain", &[], "");
    write_file(d.path(), "Projects/Seed.md", "seed");
    apply(&v, "Projects/Seed.md", &x(vec![orbital()], vec![rel_("Orbital Freight", "organization", "operates_in", "Everything", "domain")]), &now()).unwrap();
    write_file(d.path(), "Projects/Direct.md", "direct");
    apply(&v, "Projects/Direct.md", &x(vec![orbital()], vec![]), &now()).unwrap();
    for i in 0..6 {
        let p = format!("Projects/Hubbed {i}.md");
        write_file(d.path(), &p, "hub");
        apply(&v, &p, &x(vec![hub.clone()], vec![]), &now()).unwrap();
    }
    rebuild(&v, &db).unwrap();
    let g = Graph::load(&db).unwrap();
    let r = g.related("Projects/Seed.md", 2, 20).unwrap();
    assert_eq!(r[0]["path"], "Projects/Direct.md");
    assert!(r[0]["via"].as_str().unwrap().contains("Orbital Freight"));
    let three = g.related("Projects/Seed.md", 2, 3).unwrap();
    assert_eq!(three.len(), 3);
    assert_eq!(three[0]["path"], "Projects/Direct.md");
    let e = g.related("Projects/Seed.md", 2, -1).unwrap_err();
    assert!(e.msg.contains("limit"));
}

// ------------------------------------------------------------ graph store

#[test]
fn graph_gs_01() {
    assert_eq!(entity_rel("organization", "Orbital Freight").unwrap(), ORB);
    assert_eq!(entity_rel("person", "Bad:Name?").unwrap(), "Claude/Graph/People/BadName.md");
}

#[test]
fn graph_gs_02() {
    let d = mini_vault();
    let (rel, created) = upsert_entity(&open(&d), &orbital(), &now()).unwrap();
    assert!(created);
    let t = read(&d, &rel);
    for s in ["entity: organization", "# Orbital Freight", "A company.", "## Mentions", "## Relations"] {
        assert!(t.contains(s), "{s}");
    }
    assert_eq!(str_list(get(&parse(&t).unwrap(), "aliases")), ["ORBF"]);
    assert_eq!(t, "---\ncreated: 2026-07-05 14:30\nagent: claude\nentity: organization\naliases:\n- ORBF\ntags:\n- graph/organization\n---\n\n# Orbital Freight\n\nA company.\n\n## Mentions\n\n## Relations\n");
}

#[test]
fn graph_gs_03() {
    let d = mini_vault();
    let v = open(&d);
    upsert_entity(&v, &orbital(), &now()).unwrap();
    let (rel, created) = upsert_entity(&v, &ent("Orbital Freight", "organization", &["Orbital Inc"], "Different."), &now()).unwrap();
    assert_eq!((rel.as_str(), created), (ORB, false));
    let t = read(&d, ORB);
    assert_eq!(str_list(get(&parse(&t).unwrap(), "aliases")), ["ORBF", "Orbital Inc"]);
    assert!(t.contains("A company.") && !t.contains("Different."));
    assert!(t.starts_with("---\ncreated: 2026-07-05 14:30\nagent: claude\nentity: organization\naliases: [ORBF, Orbital Inc]\n"));
}

#[test]
fn graph_gs_04() {
    let d = mini_vault();
    let text = "---\naliases: [ORBF]\n# Orbital Freight\n\n## Mentions\n";
    write_file(d.path(), ORB, text);
    upsert_entity(&open(&d), &ent("Orbital Freight", "organization", &["Orbital Inc"], ""), &now()).unwrap();
    assert_eq!(read(&d, ORB), text);
}

#[test]
fn graph_gs_05() {
    let d = mini_vault();
    let v = open(&d);
    upsert_entity(&v, &orbital(), &now()).unwrap();
    let (rel, created) = upsert_entity(&v, &ent("Orbital: Freight", "organization", &[], "Other."), &now()).unwrap();
    assert_eq!((rel.as_str(), created), (ORB, false));
    let t = read(&d, ORB);
    assert!(str_list(get(&parse(&t).unwrap(), "aliases")).contains(&"Orbital: Freight".to_string()));
    assert!(t.contains("A company.") && !t.contains("Other."));
}

#[test]
fn graph_gs_06() {
    let d = mini_vault();
    let v = open(&d);
    upsert_entity(&v, &orbital(), &now()).unwrap();
    assert_eq!(find_entity_note(&v, "organization", "orbf").unwrap().as_deref(), Some(ORB));
    assert_eq!(find_entity_note(&v, "organization", "ORBITAL FREIGHT").unwrap().as_deref(), Some(ORB));
    assert_eq!(find_entity_note(&v, "organization", "Nobody").unwrap(), None);
}

#[test]
fn graph_gs_07() {
    let d = mini_vault();
    let v = open(&d);
    upsert_entity(&v, &orbital(), &now()).unwrap();
    assert!(add_mention(&v, ORB, "Projects/Lighthouse Grid.md", "ships cargo").unwrap());
    assert!(!add_mention(&v, ORB, "Projects/Lighthouse Grid.md", "ships cargo").unwrap());
    let t = read(&d, ORB);
    assert_eq!(t.matches("[[Projects/Lighthouse Grid|Lighthouse Grid]]").count(), 1);
    assert!(t.contains("- [[Projects/Lighthouse Grid|Lighthouse Grid]] \u{2014} ships cargo\n"));
    assert!(add_mention(&v, ORB, "Archive/Lighthouse Grid.md", "").unwrap());
    assert_eq!(read(&d, ORB).matches("|Lighthouse Grid]]").count(), 2);
}

#[test]
fn graph_gs_08() {
    let d = mini_vault();
    let v = open(&d);
    upsert_entity(&v, &orbital(), &now()).unwrap();
    let (dst, _) = upsert_entity(&v, &routes(), &now()).unwrap();
    assert!(add_relation(&v, ORB, "operates_in", &dst).unwrap());
    assert!(!add_relation(&v, ORB, "operates_in", &dst).unwrap());
    let t = read(&d, ORB);
    assert!(t.contains("- operates_in [[Claude/Graph/Domains/Trade Routes|Trade Routes]]\n"));
    assert!(t.ends_with("## Relations\n- operates_in [[Claude/Graph/Domains/Trade Routes|Trade Routes]]\n"));
}

#[test]
fn graph_gs_09() {
    let d = mini_vault();
    let v = open(&d);
    let full = x(vec![orbital(), routes()], vec![orb_rel()]);
    let c = apply(&v, "Projects/Lighthouse Grid.md", &full, &now()).unwrap();
    assert_eq!(c, ApplyCounts { entities_created: 2, entities_merged: 0, mentions_added: 2, relations_added: 1 });
    let c = apply(&v, "Projects/Lighthouse Grid.md", &full, &now()).unwrap();
    assert_eq!((c.entities_created, c.mentions_added), (0, 0));
}

#[test]
fn graph_gs_10() {
    let d = mini_vault();
    let v = open(&d);
    let c = apply(&v, "Daily.md", &x(vec![orbital()], vec![rel_("Orbital Freight", "organization", "part_of", "Galactic Holdings", "organization")]), &now()).unwrap();
    assert_eq!(c.entities_created, 2);
    assert!(d.path().join("Claude/Graph/Organizations/Galactic Holdings.md").is_file());
}

#[test]
fn graph_gs_11() {
    let d = mini_vault();
    let v = open(&d);
    upsert_entity(&v, &orbital(), &now()).unwrap();
    add_mention(&v, ORB, "Daily.md", "").unwrap();
    add_mention(&v, ORB, "Projects/Lighthouse Grid.md", "").unwrap();
    assert!(remove_mention(&v, ORB, "Daily.md").unwrap());
    let t = read(&d, ORB);
    assert!(!t.contains("[[Daily|") && t.contains("[[Projects/Lighthouse Grid|"));
    assert!(!remove_mention(&v, ORB, "Daily.md").unwrap());
}

#[test]
fn graph_gs_12() {
    let d = mini_vault();
    let v = open(&d);
    upsert_entity(&v, &orbital(), &now()).unwrap();
    write_file(d.path(), "Claude/Graph/Organizations/Orbital Old.md", "---\nentity: organization\nmerged_into: Claude/Graph/Organizations/Orbital Freight\n---\n\n# Orbital Old\n");
    assert_eq!(find_entity_note(&v, "organization", "Orbital Old").unwrap().as_deref(), Some(ORB));
    let dead = "Claude/Graph/Organizations/Dead End.md";
    write_file(d.path(), dead, "---\nentity: organization\nmerged_into: Claude/Graph/Organizations/Gone\n---\n\n# Dead End\n");
    assert_eq!(find_entity_note(&v, "organization", "Dead End").unwrap().as_deref(), Some(dead));
}

#[test]
fn graph_gs_13() {
    let d = mini_vault();
    write_file(d.path(), ORB, "---\nentity: organization\nretired: true\naliases: [ORB Inc]\n---\n\n# Orbital Freight\n\nOld.\n");
    let (rel, created) = upsert_entity(&open(&d), &ent("Orbital Freight", "organization", &["Orbital Co"], "Back again."), &now()).unwrap();
    assert_eq!((rel.as_str(), created), (ORB, true));
    let t = read(&d, ORB);
    let meta = parse(&t).unwrap();
    assert!(get(&meta, "retired").is_none());
    assert_eq!(str_list(get(&meta, "aliases")), ["Orbital Co", "ORB Inc"]);
    assert!(t.contains("Back again.") && t.contains("## Mentions"));
    assert!(matches!(get(&meta, "entity"), Some(Y::Str(s)) if s == "organization"));
}
