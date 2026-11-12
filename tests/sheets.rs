//! acceptance/sheets.yaml
mod common;
use common::*;
use enigma_mcp::error::Kind;
use enigma_mcp::sheets::*;
use enigma_mcp::yaml::Y;
use serde_json::{Map, Value, json};
use std::process::Command;

const ROLES: &str = "---\nsheet: roles\nfilename: \"{org} - {title}\"\nkey: [org, title]\nidentity: [posting_id, posting_url]\ncolumns:\n  org:        {type: string, required: true, max_length: 120}\n  title:      {type: string, required: true, max_length: 160}\n  posting_id: {type: string, max_length: 80}\n  stage:      {type: enum, required: true, values: [Found, Sent, Screen, Onsite, Offer, Declined, Silent, Pulled]}\n  sent_on:    {type: date}\n  visa:       {type: bool}\n  posting_url: {type: url, max_length: 500}\n  chase_on:   {type: date}\n---\nOne row per posting. Rows are never deleted.\n";

fn jobs() -> tempfile::TempDir {
    let d = mini_vault();
    write_file(d.path(), "Hiring/Roles/_schema.md", ROLES);
    d
}

fn s(v: &str) -> Y {
    Y::Str(v.into())
}

fn f(pairs: &[(&str, Y)]) -> Vec<(Y, Y)> {
    pairs.iter().map(|(k, v)| (s(k), v.clone())).collect()
}

fn valid() -> Vec<(Y, Y)> {
    f(&[("org", s("Lumen")), ("title", s("Data Engineer")), ("stage", s("Found"))])
}

fn roles(d: &tempfile::TempDir) -> Schema {
    get_schema(&open(d), "roles").unwrap()
}

fn upsert_(d: &tempfile::TempDir, fields: &[(Y, Y)]) -> Value {
    upsert(&open(d), "roles", fields, None, "cowork", &now()).unwrap()
}

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn sheets_sh_01() {
    let d = jobs();
    let sc = load_schema(&open(&d), "Hiring/Roles").unwrap();
    assert_eq!(sc.name, "roles");
    assert_eq!(sc.key, ["org", "title"]);
    assert_eq!(sc.identity, ["posting_id", "posting_url"]);
    let stage = sc.columns.iter().find(|c| c.name == "stage").unwrap();
    assert_eq!(stage.ty, "enum");
    assert!(stage.values.as_ref().unwrap().contains(&"Silent".to_string()));
    let org = sc.columns.iter().find(|c| c.name == "org").unwrap();
    assert!(org.required);
    assert_eq!(org.max_length, Some(Y::Int(120)));
}

#[test]
fn sheets_sh_02() {
    let d = jobs();
    let reg = discover(&open(&d)).unwrap();
    assert_eq!(reg.sheets, [("roles".to_string(), "Hiring/Roles".to_string())]);
    assert_eq!(roles(&d).folder, "Hiring/Roles");
    let e = get_schema(&open(&d), "nope").unwrap_err();
    assert_eq!(e.kind, Kind::SheetError);
    assert_eq!(e.msg, "Unknown sheet 'nope'. Registered: ['roles'].");
}

#[test]
fn sheets_sh_03() {
    let d = jobs();
    assert!(is_sheet_folder(&open(&d), "Hiring/Roles"));
    assert!(!is_sheet_folder(&open(&d), "Projects"));
}

#[test]
fn sheets_sh_04() {
    let d = mini_vault();
    write_file(d.path(), "S/_schema.md", "---\nsheet: s\nfilename: \"{a}\"\nkey: [a]\ncolumns:\n  a: {type: martian}\n---\n");
    let e = load_schema(&open(&d), "S").unwrap_err();
    assert_eq!(e.msg, "S/_schema.md: column 'a' has invalid type 'martian' (allowed: ['bool', 'date', 'enum', 'number', 'string', 'url']).");
}

#[test]
fn sheets_sh_05() {
    let d = mini_vault();
    write_file(d.path(), "S/_schema.md", "---\nsheet: s\nfilename: \"{org}\"\nkey: org\ncolumns:\n  org: {type: string}\n---\n");
    assert_eq!(load_schema(&open(&d), "S").unwrap_err().msg, "S/_schema.md: field 'key' expected list, got 'org'.");
}

#[test]
fn sheets_sh_06() {
    assert_eq!(norm_str(&s("  Lumen   Labs ")), norm_str(&s("lumen labs")));
    assert_eq!(normalize_link("HTTPS://Careers.Example.org/r/77/?utm_medium=x&q=2#top"), normalize_link("https://careers.example.org/r/77?q=2"));
    assert_eq!(normalize_link("HTTPS://Careers.Example.org/r/77/?utm_medium=x&q=2#top"), "https://careers.example.org/r/77?q=2");
}

#[test]
fn sheets_sh_07() {
    let d = jobs();
    validate_fields(&roles(&d), &valid(), true).unwrap();
}

#[test]
fn sheets_sh_08() {
    let d = jobs();
    let sc = roles(&d);
    let with = |extra: &[(&str, Y)]| {
        let mut v = valid();
        for (k, val) in extra {
            match v.iter_mut().find(|(kk, _)| *kk == s(k)) {
                Some(slot) => slot.1 = val.clone(),
                None => v.push((s(k), val.clone())),
            }
        }
        v
    };
    let cases: Vec<(Vec<(Y, Y)>, &str)> = vec![
        (with(&[("recruiter", s("Kim"))]), "recruiter"),
        (with(&[("stage", s("found"))]), "found"),
        (with(&[("sent_on", s("11/07/2026"))]), "YYYY-MM-DD"),
        (with(&[("visa", s("yes"))]), "bool"),
        (with(&[("org", s(&"x".repeat(121)))]), "max_length"),
        (f(&[("org", s("Lumen")), ("stage", s("Found"))]), "title"),
        (with(&[("posting_url", s("Apply on the site, ask for Dana"))]), "URL"),
        (f(&[("org", Y::Null), ("title", s("T")), ("stage", s("Found"))]), "org"),
    ];
    for (fields, fragment) in cases {
        let e = validate_fields(&sc, &fields, true).unwrap_err();
        assert!(e.msg.contains(fragment), "{fragment}: {}", e.msg);
    }
    assert_eq!(
        validate_fields(&sc, &with(&[("visa", s("yes"))]), true).unwrap_err().msg,
        "Field 'visa': expected bool, got ''yes''."
    );
}

#[test]
fn sheets_sh_09() {
    let d = jobs();
    validate_fields(&roles(&d), &f(&[("stage", s("Sent"))]), false).unwrap();
}

#[test]
fn sheets_sh_10() {
    let d = jobs();
    let mut v = valid();
    v.push((s("tags"), Y::Seq(vec![s("job")])));
    validate_fields(&roles(&d), &v, true).unwrap();
}

#[test]
fn sheets_sh_11() {
    let d = jobs();
    write_file(d.path(), "Hiring/Roles/Lumen - DE.md", "---\norg: Lumen\n---\n");
    write_file(d.path(), "Hiring/Roles/Archive/Old.md", "---\norg: Old\n---\n");
    let paths: Vec<String> = rows(&open(&d), &roles(&d)).unwrap().into_iter().map(|(p, _)| p).collect();
    assert_eq!(paths, ["Hiring/Roles/Lumen - DE.md"]);
}

#[test]
fn sheets_sh_12() {
    let d = jobs();
    let sc = roles(&d);
    assert_eq!(render_filename(&sc, &f(&[("org", s("X/Y: Ltd?")), ("title", s("Ops|Lead"))])).unwrap(), "X-Y- Ltd- - Ops-Lead");
    assert!(render_filename(&sc, &f(&[("org", s(&"C".repeat(200))), ("title", s("R"))])).unwrap().chars().count() <= 120);
}

fn put(d: &tempfile::TempDir, name: &str, fm: &str) {
    write_file(d.path(), &format!("Hiring/Roles/{name}.md"), &format!("---\n{fm}---\n\n"));
}

#[test]
fn sheets_sh_13() {
    let d = jobs();
    put(&d, "R1", "org: Lumen\ntitle: DE\nstage: Found\nposting_id: R1\n");
    put(&d, "R2", "org: Lumen\ntitle: DE\nstage: Found\nposting_id: R2\n");
    let (p, back) = match_row(&open(&d), &roles(&d), &f(&[("org", s("lumen")), ("title", s("DE")), ("posting_id", s("R2"))])).unwrap();
    assert_eq!(p.as_deref(), Some("Hiring/Roles/R2.md"));
    assert!(back.is_empty());
}

#[test]
fn sheets_sh_14() {
    let d = jobs();
    put(&d, "K", "org: Kite\ntitle: Analyst\nstage: Found\n");
    let (p, back) = match_row(&open(&d), &roles(&d), &f(&[("org", s("Kite")), ("title", s("Analyst")), ("posting_id", s("R9"))])).unwrap();
    assert_eq!(p.as_deref(), Some("Hiring/Roles/K.md"));
    assert_eq!(back, f(&[("posting_id", s("R9"))]));
}

#[test]
fn sheets_sh_15() {
    let d = jobs();
    put(&d, "K", "org: Kite\ntitle: Analyst\nstage: Found\nposting_id: R1\n");
    let (p, _) = match_row(&open(&d), &roles(&d), &f(&[("org", s("Kite")), ("title", s("Analyst")), ("posting_id", s("R2"))])).unwrap();
    assert!(p.is_none());
}

#[test]
fn sheets_sh_16() {
    let d = jobs();
    put(&d, "K1", "org: Kite\ntitle: Analyst\nstage: Found\nposting_id: R1\n");
    put(&d, "K2", "org: Kite\ntitle: Analyst\nstage: Found\nposting_id: R2\n");
    let e = match_row(&open(&d), &roles(&d), &f(&[("org", s("Kite")), ("title", s("Analyst"))])).unwrap_err();
    assert!(e.msg.contains("K1") && e.msg.contains("K2") && !e.msg.ends_with('.'), "{}", e.msg);
}

#[test]
fn sheets_sh_17() {
    let d = jobs();
    put(&d, "K", "org: Kite\ntitle: Analyst\nstage: Found\nposting_url: https://jobs.kite.io/x?utm_source=a\n");
    let (p, _) =
        match_row(&open(&d), &roles(&d), &f(&[("org", s("Kite")), ("title", s("Analyst")), ("posting_url", s("HTTPS://JOBS.KITE.IO/x/"))])).unwrap();
    assert_eq!(p.as_deref(), Some("Hiring/Roles/K.md"));
}

#[test]
fn sheets_sh_18() {
    let d = jobs();
    let r = upsert_(&d, &f(&[("org", s("Nova")), ("title", s("MLE")), ("stage", s("Found"))]));
    assert_eq!(r["result"], "created");
    let t = read(&d, r["path"].as_str().unwrap());
    for frag in ["org: Nova", "## Log", "stage: (new) \u{2192} Found (agent: cowork)", "agent: cowork", "created:"] {
        assert!(t.contains(frag), "{frag}\n{t}");
    }
}

#[test]
fn sheets_sh_19() {
    let d = jobs();
    let path = "Hiring/Roles/Lumen - DE.md";
    let body = "Story para.\n\n## Log\n- 2026-07-10 stage: (new) \u{2192} Found (agent: claude)\n";
    write_file(d.path(), path, &format!("---\norg: Lumen\ntitle: DE\nstage: Found\nchannel: Referral   # via a friend\n---\n\n{body}"));
    let r = upsert_(&d, &f(&[("org", s("Lumen")), ("title", s("DE")), ("stage", s("Sent")), ("sent_on", s("2026-07-11"))]));
    assert_eq!(r["result"], "updated");
    assert_eq!(r["changed"]["stage"], json!({"from": "Found", "to": "Sent"}));
    let t = read(&d, path);
    assert!(t.contains("\nchannel: Referral   # via a friend\n"));
    assert!(t.contains("\nsent_on: 2026-07-11\n"));
    assert!(t.contains("Story para.\n\n## Log\n- 2026-07-10 stage: (new)"));
    assert_eq!(t.matches("## Log").count(), 1);
    assert!(t.contains("stage: Found \u{2192} Sent"));
}

#[test]
fn sheets_sh_20() {
    let d = jobs();
    let r = upsert_(&d, &valid());
    let path = d.path().join(r["path"].as_str().unwrap());
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    let again = upsert_(&d, &valid());
    assert_eq!((again["result"].clone(), again["changed"].clone()), (json!("updated"), json!({})));
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
}

#[test]
fn sheets_sh_21() {
    let d = jobs();
    let mut v = valid();
    v.push((s("recruiter"), s("Kim")));
    assert!(upsert(&open(&d), "roles", &v, None, "c", &now()).unwrap_err().msg.contains("recruiter"));
    assert!(upsert(&open(&d), "subscriptions", &valid(), None, "c", &now()).unwrap_err().msg.contains("Unknown sheet"));
}

#[test]
fn sheets_sh_22() {
    let d = jobs();
    assert!(open(&d).write("Hiring/Roles/Sneak.md", "x", false, false).unwrap_err().msg.contains("outside Claude/"));
}

#[test]
fn sheets_sh_23() {
    let d = jobs();
    put(&d, "Nova - MLE", "org: Nova\ntitle: MLE\nstage: Found\nposting_id: R1\n");
    let r = upsert_(&d, &f(&[("org", s("Nova")), ("title", s("MLE")), ("stage", s("Found")), ("posting_id", s("R2"))]));
    assert_eq!(r["result"], "created");
    assert!(r["path"].as_str().unwrap().ends_with("Nova - MLE 2.md"));
}

fn q(d: &tempfile::TempDir, filters: Value, sort: Option<Value>, limit: i64) -> Vec<Value> {
    let fm = filters.as_object().cloned();
    let sm = sort.and_then(|s| s.as_object().cloned());
    query(&open(d), "roles", fm.as_ref(), sm.as_ref(), limit).unwrap()
}

fn names(rows: &[Value]) -> Vec<String> {
    rows.iter().map(|r| r["org"].as_str().unwrap().to_string()).collect()
}

#[test]
fn sheets_sh_24() {
    let d = jobs();
    put(&d, "A", "org: A\ntitle: T\nstage: Found\n");
    put(&d, "B", "org: B\ntitle: T\nstage: Sent\nchase_on: 2026-07-01\n");
    put(&d, "C", "org: C\ntitle: T\nstage: Declined\nchase_on: 2026-07-05\n");
    assert_eq!(names(&q(&d, json!({"chase_on": {"lte": "2026-07-11"}, "stage": {"nin": ["Declined", "Silent", "Pulled"]}}), None, 50)), ["B"]);
    assert_eq!(q(&d, json!({"stage": {"eq": "Found"}}), None, 50).len(), 1);
    assert_eq!(q(&d, json!({"chase_on": {"missing": true}}), None, 50).len(), 1);
    let sorted = names(&q(&d, json!({}), Some(json!({"by": "chase_on", "dir": "desc"})), 50));
    assert_eq!((sorted[0].as_str(), sorted[2].as_str()), ("C", "A"));
    let two = q(&d, json!({}), None, 2);
    assert_eq!(two.len(), 2);
    assert!(two.iter().all(|r| !r["path"].as_str().unwrap().ends_with("_schema.md")));
    assert_eq!(q(&d, json!({"stage": {"eq": "Sent"}}), None, 50)[0]["chase_on"], "2026-07-01");
}

#[test]
fn sheets_sh_25() {
    let d = jobs();
    let bad = |filters: Value| query(&open(&d), "roles", filters.as_object(), None, 50).unwrap_err().msg;
    assert!(bad(json!({"stage": {"like": "x"}})).contains("Unknown operator"));
    assert!(bad(json!({"org": {"lt": "M"}})).contains("ordering"));
}

#[test]
fn sheets_sh_26() {
    let d = jobs();
    for n in ["A", "B", "C"] {
        put(&d, n, &format!("org: {n}\ntitle: T\nstage: Found\n"));
    }
    let listing = schema_info(&open(&d), None).unwrap();
    assert_eq!(listing["roles"]["rows"], 3);
    let detail = schema_info(&open(&d), Some("roles")).unwrap();
    assert_eq!(detail["columns"]["stage"]["values"][0], "Found");
    assert!(detail["instructions"].as_str().unwrap().contains("never deleted"));
    assert_eq!(obj(detail).keys().collect::<Vec<_>>(), ["sheet", "folder", "filename", "key", "identity", "columns", "instructions"]);
}

fn check_cli(d: &tempfile::TempDir) -> (i32, String) {
    let out = Command::new(bin()).args(["sheets", d.path().to_str().unwrap(), "--check"]).output().unwrap();
    (out.status.code().unwrap(), String::from_utf8(out.stdout).unwrap())
}

#[test]
fn sheets_sh_27() {
    let d = jobs();
    put(&d, "Ok", "org: Ok\ntitle: Row\nstage: Found\n");
    put(&d, "Bad", "org: Bad\ntitle: Row\nstage: Found\nphase: x\n");
    put(&d, "Dup1", "org: Dup\ntitle: A\nstage: Found\n");
    put(&d, "Dup2", "org: Dup\ntitle: A\nstage: Found\n");
    let (code, out) = check_cli(&d);
    assert_eq!(code, 1);
    assert!(out.contains("phase") && out.contains("Dup") && out.contains("\"clean\": false"), "{out}");
}

#[test]
fn sheets_sh_28() {
    let d = jobs();
    put(&d, "Ok", "org: Ok\ntitle: Row\nstage: Sent\nsent_on: 2026-04-20\nchase_on:\n");
    let (code, out) = check_cli(&d);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("\"clean\": true"));
}

#[test]
fn sheets_sh_29() {
    let d = jobs();
    let path = "Hiring/Roles/Nova - MLE.md";
    write_file(d.path(), path, "---\norg: Nova\ntitle: MLE\nstage: Found\ntags:\n  - old\n  - stale\n---\n\n");
    let fields = f(&[("org", s("Nova")), ("title", s("MLE")), ("tags", Y::Seq(vec![s("new"), s("shiny")]))]);
    upsert_(&d, &fields);
    let t = read(&d, path);
    assert!(!t.contains("- old") && !t.contains("- stale"));
    let meta = enigma_mcp::vault::parse(&t).unwrap();
    assert_eq!(enigma_mcp::vault::str_list(enigma_mcp::vault::get(&meta, "tags")), ["new", "shiny"]);
    assert_eq!(enigma_mcp::vault::get(&meta, "stage"), Some(&s("Found")));
}

#[test]
fn sheets_sh_30() {
    let d = mini_vault();
    write_file(d.path(), "Applications/_schema.md", ROLES);
    write_file(d.path(), "Claude/Fake/_schema.md", ROLES);
    let reg = discover(&open(&d)).unwrap();
    assert_eq!(reg.sheets, [("roles".to_string(), "Applications".to_string())]);
    assert_eq!(get_schema(&open(&d), "roles").unwrap().folder, "Applications");
}

#[test]
fn sheets_sh_31() {
    let d = jobs();
    write_file(d.path(), "Other/_schema.md", ROLES);
    let e = discover(&open(&d)).err().unwrap();
    assert!(e.msg.contains("Duplicate sheet name"), "{}", e.msg);
}

#[test]
fn sheets_sh_32() {
    let d = jobs();
    write_file(d.path(), "Hiring/Bad/_schema.md", "---\nsheet: bad\nfilename: x\nkey: [a]\ncolumns:\n  a: {type: martian}\n---\n");
    let reg = discover(&open(&d)).unwrap();
    assert_eq!(reg.sheets.len(), 1);
    assert_eq!(upsert_(&d, &valid())["result"], "created");
    assert_eq!(schema_info(&open(&d), None).unwrap()["invalid"][0]["folder"], "Hiring/Bad");
}

#[test]
fn sheets_sh_33() {
    let d = jobs();
    let path = "Hiring/Roles/Lumen - DE.md";
    put(&d, "Lumen - DE", "org: Lumen\ntitle: DE\nstage: Sent\nsent_on: 2026-07-11\n");
    let before = std::fs::metadata(d.path().join(path)).unwrap().modified().unwrap();
    let r = upsert_(&d, &f(&[("org", s("Lumen")), ("title", s("DE")), ("sent_on", s("2026-07-11"))]));
    assert_eq!(r["changed"], json!({}));
    assert_eq!(std::fs::metadata(d.path().join(path)).unwrap().modified().unwrap(), before);
    assert_eq!(q(&d, json!({"sent_on": {"eq": "2026-07-11"}}), None, 50).len(), 1);
}

#[test]
fn sheets_sh_34() {
    let d = jobs();
    let created = upsert_(&d, &valid());
    let path = created["path"].as_str().unwrap().to_string();
    let stamp = enigma_mcp::vault::get(&enigma_mcp::vault::parse(&read(&d, &path)).unwrap(), "created").cloned();
    let mut v = valid();
    v.extend(f(&[("created", s("2099-01-01 00:00")), ("agent", s("sneaky")), ("stage", s("Sent"))]));
    v.retain(|(k, val)| !(*k == s("stage") && *val == s("Found")));
    let r = upsert(&open(&d), "roles", &v, None, "cowork", &now()).unwrap();
    let changed = obj(r["changed"].clone());
    assert!(!changed.contains_key("created") && !changed.contains_key("agent"));
    let t = read(&d, &path);
    assert!(!t.contains("sneaky") && !t.contains("2099"));
    let meta = enigma_mcp::vault::parse(&t).unwrap();
    assert_eq!(enigma_mcp::vault::get(&meta, "created").cloned(), stamp);
    assert_eq!(enigma_mcp::vault::get(&meta, "agent"), Some(&s("cowork")));
}

#[test]
fn sheets_sh_35() {
    let d = jobs();
    upsert_(&d, &valid());
    let e = upsert(&open(&d), "roles", &valid(), Some("new body"), "cowork", &now()).unwrap_err();
    assert!(e.msg.contains("body"));
}

#[test]
fn sheets_sh_36() {
    let line = "- 2026-07-05 stage: Found \u{2192} Sent (agent: c)";
    let out = append_log("Intro.\n\n## Log\n- old\n\n## Notes\nN.\n", line);
    assert_eq!(out, format!("Intro.\n\n## Log\n- old\n{line}\n\n## Notes\nN.\n"));
    let out = append_log("## Logistics\nTrucks.\n", line);
    assert_eq!(out, format!("## Logistics\nTrucks.\n\n## Log\n{line}\n"));
}

#[test]
fn sheets_sh_37() {
    let d = jobs();
    assert!(open(&d).write("Projects/_schema.md", "x", false, false).unwrap_err().msg.contains("outside Claude/"));
    let r = upsert_(&d, &f(&[("org", s("../../evil")), ("title", s("T")), ("stage", s("Found"))]));
    let path = r["path"].as_str().unwrap();
    assert!(path.starts_with("Hiring/Roles/") && !path["Hiring/Roles/".len()..].contains('/'), "{path}");
    assert!(d.path().join(path).is_file());
    assert!(upsert(&open(&d), "../../Claude/Evil", &valid(), None, "c", &now()).unwrap_err().msg.contains("Unknown sheet"));
}

#[test]
fn sheets_sh_38() {
    let d = mini_vault();
    write_file(d.path(), "Records/_schema.md", "---\nsheet: records\nfilename: \"{name}\"\nkey: [name]\ncolumns:\n  name: {type: string, required: true}\n---\n");
    write_file(d.path(), "Records/Bad.md", "---\nnote: x\n---\n");
    assert_eq!(invalid_rows(&open(&d)).unwrap(), 1);
    assert_eq!(invalid_rows(&open(&mini_vault())).unwrap(), 0);
}
