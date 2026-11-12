use clap::{CommandFactory, Parser, Subcommand};
use enigma_mcp::error::{Error, Kind, Result};
use enigma_mcp::vault::Vault;
use enigma_mcp::{
    cleanup, config, consolidate, conventions, embed, evals, extractor, graph, hooks, indexer, json, librarian, lock, mcp_sync, nightly, organizer,
    provision, recall, schedule, server, sheets, text,
};
use std::path::Path;
use rmcp::ServiceExt;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

/// Enigma MCP: memory server for an Obsidian vault. Without a subcommand it
/// runs the stdio MCP server.
#[derive(Parser)]
#[command(name = "enigma-mcp", version, disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Index new and changed notes into the semantic graph
    Index {
        vault: PathBuf,
        /// codex or claude; checked when the extractor is created (exit 1)
        #[arg(long)]
        backend: Option<String>,
        #[arg(long)]
        batch: Option<usize>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        retry_failures: bool,
        #[arg(long)]
        rebuild_only: bool,
    },
    /// Merge duplicate graph entities (dry run by default)
    Consolidate {
        vault: PathBuf,
        /// codex or claude; checked when the extractor is created (exit 1)
        #[arg(long)]
        backend: Option<String>,
        #[arg(long)]
        apply: bool,
    },
    /// Neighbour-vote filing sweep
    Organize {
        vault: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    /// The scheduled caretaker sweep
    Librarian {
        vault: PathBuf,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, allow_negative_numbers = true)]
        max_moves: Option<i64>,
        #[arg(long)]
        auto_cleanup: bool,
        #[arg(long)]
        confirm_first_run: bool,
    },
    /// Deleted-note retraction, relation repair, retirements
    Cleanup {
        vault: PathBuf,
        #[arg(long)]
        apply_repairs: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        apply_retirements: bool,
        #[arg(long, num_args = 0..)]
        paths: Option<Vec<String>>,
    },
    /// Check typed sheets
    Sheets {
        vault: PathBuf,
        #[arg(long, required = true)]
        check: bool,
    },
    /// Recall context block
    Recall {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        context: bool,
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value_t = 2000, allow_negative_numbers = true)]
        budget: i64,
    },
    /// Retrieval evaluation harness
    Evals {
        #[arg(long)]
        vault: Option<PathBuf>,
        #[arg(long)]
        golden: Option<PathBuf>,
        #[arg(long)]
        live: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        no_history: bool,
        #[arg(long)]
        init_live: bool,
    },
    /// Nightly index runner
    IndexNightly { vault: Option<PathBuf>, state_dir: Option<PathBuf> },
    /// Install the constitution and vault guides
    InstallConventions { vault: Option<PathBuf> },
    /// Register MCP servers with agent clients
    McpSync {
        #[arg(long)]
        check: bool,
        #[arg(long)]
        vault: Option<PathBuf>,
        #[arg(long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Install bundled skills
    SkillSync {
        #[arg(long)]
        check: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dest: Option<PathBuf>,
    },
    /// Register Claude Code hooks
    HookSync {
        #[arg(long)]
        check: bool,
        #[arg(long)]
        install: bool,
    },
    /// Claude Code hook entry points
    Hook {
        #[arg(value_parser = ["session-start", "stop-nudge"])]
        name: String,
    },
    /// Install, remove or print the nightly launchd job
    Schedule {
        #[arg(value_parser = ["install", "uninstall", "print"])]
        action: String,
        #[arg(long, required = true, value_parser = ["index", "librarian"])]
        job: String,
        vault: Option<PathBuf>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Provision Obsidian plugins
    Provision {
        vault: PathBuf,
        #[arg(long)]
        check: bool,
    },
}

const SUBCOMMANDS: [&str; 16] = [
    "index", "consolidate", "organize", "librarian", "cleanup", "sheets", "recall", "evals", "index-nightly",
    "install-conventions", "mcp-sync", "skill-sync", "hook-sync", "hook", "schedule", "provision",
];

fn usage_error(e: clap::Error) -> ExitCode {
    use clap::error::ErrorKind::*;
    if matches!(e.kind(), DisplayHelp | DisplayVersion | DisplayHelpOnMissingArgumentOrSubcommand) {
        let _ = e.print();
        return ExitCode::SUCCESS;
    }
    let sub = std::env::args().nth(1).filter(|a| SUBCOMMANDS.contains(&a.as_str()));
    let prog = sub.map_or("enigma-mcp".to_string(), |s| format!("enigma-mcp {s}"));
    let rendered = e.render().to_string();
    let msg = rendered.lines().next().unwrap_or_default().trim_start_matches("error: ");
    eprintln!("{}\n{prog}: error: {msg}", Cli::command().render_usage());
    ExitCode::from(2)
}

/// A usage error raised by a subcommand's own rule (14 §1).
fn usage_fail(sub: &str, msg: &str) -> ExitCode {
    eprintln!("{}\nenigma-mcp {sub}: error: {msg}", Cli::command().render_usage());
    ExitCode::from(2)
}

const FIRST_RUN: &str = "no Librarian sweep has run on this vault yet. Run it with --dry-run, review the report, then run once by hand with --confirm-first-run before scheduling it.";

/// enigma-mcp librarian (14 §2).
fn librarian(vault: &Path, dry_run: bool, max_moves: Option<i64>, auto_cleanup: bool, confirm_first_run: bool) -> ExitCode {
    if max_moves.is_some_and(|m| m < 0) {
        return usage_fail("librarian", "--max-moves must be >= 0");
    }
    let run = || -> Result<Option<serde_json::Value>> {
        let v = Vault::open(vault)?;
        let st = config::state_dir(Some(&v.root))?;
        if !dry_run && !confirm_first_run && !librarian::state_path(&st).exists() {
            return Ok(None);
        }
        let _lock = if dry_run { None } else { Some(lock::acquire(&st)?) };
        let emb = load_model()?;
        let deps = librarian::Deps { extractor: None, consolidator: None, emb: &emb };
        let auto = auto_cleanup || config::truthy("ENIGMA_AUTO_CLEANUP");
        librarian::run_sweep(&v, &st, &deps, !dry_run, max_moves, &text::now(), auto).map(Some)
    };
    match run() {
        Ok(None) => usage_fail("librarian", FIRST_RUN),
        Ok(Some(result)) => {
            if dry_run {
                // the section's last line ends with LF; one more LF leaves a blank line before the JSON
                println!("{}", librarian::report_section(&result, &text::now()));
            }
            println!("{}", json::indented(&result));
            if result["errors"].as_object().is_some_and(|e| !e.is_empty()) { ExitCode::from(1) } else { ExitCode::SUCCESS }
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

/// enigma-mcp evals (14 §2).
fn evals(vault: Option<PathBuf>, golden: Option<PathBuf>, live: bool, as_json: bool, no_history: bool, init_live: bool) -> ExitCode {
    let fail = |e: Error| {
        if e.is(Kind::EvalConfigError) || e.is(Kind::VaultError) {
            eprintln!("error: {}", e.msg);
            ExitCode::from(2)
        } else {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    };
    let (vault, golden) = if live || init_live {
        let Some(root) = config::var("ENIGMA_VAULT_PATH").map(PathBuf::from) else {
            return fail(Error::new(Kind::EvalConfigError, "--live/--init-live require ENIGMA_VAULT_PATH"));
        };
        let golden = root.join(evals::LIVE_NOTE);
        (root, golden)
    } else {
        (vault.unwrap_or_else(evals::fixture_vault), golden.unwrap_or_else(evals::fixture_golden))
    };
    if init_live {
        return match evals::init_live(&vault) {
            Ok(created) => {
                println!("{}: {}", if created { "created" } else { "already exists" }, golden.display());
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        };
    }
    let run = || -> Result<(evals::Report, PathBuf)> {
        let queries = evals::load_golden(&golden)?;
        let v = Vault::open(&vault)?;
        let st = config::state_dir(Some(&v.root))?;
        let emb = load_model()?;
        Ok((evals::run(&v, &st, &emb, &queries, live)?, st))
    };
    match run() {
        Ok((r, st)) => {
            let result = evals::to_json(&r);
            println!("{}", if as_json { json::indented(&result) } else { evals::table(&r) });
            if !no_history && let Err(e) = evals::append_history(&st, &vault, &golden, &result, &text::now()) {
                eprintln!("warning: could not write eval history: {e}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(e),
    }
}

fn serve() -> ExitCode {
    let profile = config::profile();
    let emb = match embed::Bge::load() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("enigma-mcp: cannot load embedding model {}: {}", embed::MODEL, e.lines().next().unwrap_or_default());
            return ExitCode::from(1);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: IoError: {e}");
            return ExitCode::from(1);
        }
    };
    let srv = server::Server::new(profile, Arc::new(emb));
    let res = rt.block_on(async {
        let running = srv.serve(rmcp::transport::stdio()).await?;
        running.waiting().await?;
        Ok::<(), Box<dyn std::error::Error>>(())
    });
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: Internal: {e}");
            ExitCode::from(1)
        }
    }
}

fn load_model() -> Result<embed::Bge> {
    embed::Bge::load().map_err(|e| Error::new(Kind::Internal, format!("cannot load embedding model {}: {e}", embed::MODEL)))
}

/// enigma-mcp index (14 §2).
fn index(vault: &Path, backend: Option<&str>, opts: indexer::Options, rebuild_only: bool) -> Result<String> {
    let v = Vault::open(vault)?;
    let st = config::state_dir(Some(&v.root))?;
    let _lock = lock::acquire(&st)?;
    if rebuild_only {
        let db = st.join("graph.db");
        graph::rebuild(&v, &db)?;
        return Ok(json::compact(&serde_json::json!({"rebuilt": true, "db": db.to_string_lossy()})));
    }
    let ex = extractor::Cli::extraction(backend)?;
    let emb = load_model()?;
    let counts = indexer::run(&v, &st, &ex, Some(&emb), &opts, &text::now())?;
    Ok(json::indented(&counts))
}

/// enigma-mcp organize (14 §2).
fn organize(vault: &Path, dry_run: bool) -> Result<String> {
    let v = Vault::open(vault)?;
    let st = config::state_dir(Some(&v.root))?;
    let _lock = if dry_run { None } else { Some(lock::acquire(&st)?) };
    let emb = load_model()?;
    Ok(json::indented(&organizer::run(&v, &st, &emb, !dry_run, None, &text::now())?))
}

/// enigma-mcp cleanup (12 §9).
fn cleanup(vault: &Path, apply_repairs: bool, force: bool, apply_retirements: bool, paths: Option<Vec<String>>) -> Result<String> {
    let v = Vault::open(vault)?;
    let st = config::state_dir(Some(&v.root))?;
    if !apply_repairs && !apply_retirements {
        return Ok(json::indented(&cleanup::report(&v, &st)?));
    }
    let _lock = lock::acquire(&st)?;
    let r = if apply_repairs {
        cleanup::apply_repairs(&v, &st, &text::now(), force)?
    } else {
        cleanup::apply_retirements(&v, &st, paths.as_deref(), &text::now())?
    };
    Ok(json::indented(&r))
}

/// enigma-mcp consolidate (14 §2).
fn consolidate(vault: &Path, backend: Option<&str>, apply: bool) -> Result<String> {
    let v = Vault::open(vault)?;
    let st = config::state_dir(Some(&v.root))?;
    let ex = extractor::Cli::consolidation(backend)?;
    let _lock = if apply { Some(lock::acquire(&st)?) } else { None };
    let emb = load_model()?;
    Ok(json::indented(&consolidate::run(&v, &st, &ex, apply, &emb, &text::now())?))
}

fn report(r: Result<String>) -> ExitCode {
    match r {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => return usage_error(e),
    };
    match cli.cmd {
        None => serve(),
        Some(Cmd::Index { vault, backend, batch, force, retry_failures, rebuild_only }) => {
            let opts = indexer::Options { batch: batch.unwrap_or(indexer::DEFAULT_BATCH), force, retry_failures };
            report(index(&vault, backend.as_deref(), opts, rebuild_only))
        }
        Some(Cmd::Consolidate { vault, backend, apply }) => report(consolidate(&vault, backend.as_deref(), apply)),
        Some(Cmd::Organize { vault, dry_run }) => report(organize(&vault, dry_run)),
        Some(Cmd::Librarian { vault, dry_run, max_moves, auto_cleanup, confirm_first_run }) => {
            librarian(&vault, dry_run, max_moves, auto_cleanup, confirm_first_run)
        }
        Some(Cmd::Recall { vault, context, project, budget }) => {
            // every runtime failure is swallowed: the session-start hook relies on it (11 §5)
            if context
                && let Ok(v) = Vault::open(&vault)
                && let Ok(st) = config::state_dir(Some(&v.root))
            {
                print!("{}", recall::context_block(&v, &st, project.as_deref().unwrap_or(""), budget));
            }
            ExitCode::SUCCESS
        }
        Some(Cmd::Cleanup { vault, apply_repairs, force, apply_retirements, paths }) => {
            report(cleanup(&vault, apply_repairs, force, apply_retirements, paths))
        }
        Some(Cmd::Evals { vault, golden, live, json, no_history, init_live }) => evals(vault, golden, live, json, no_history, init_live),
        Some(Cmd::Sheets { vault, .. }) => match Vault::open(&vault).and_then(|v| sheets::check_vault(&v)) {
            Ok(r) => {
                println!("{}", json::indented_literal(&r));
                if r["clean"] == serde_json::json!(true) { ExitCode::SUCCESS } else { ExitCode::from(1) }
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        },
        // a missing VAULT exits 1, not as a usage error (14 §2)
        Some(Cmd::IndexNightly { vault: None, .. }) => no_vault("usage: enigma-mcp index-nightly <vault> [state-dir]"),
        Some(Cmd::IndexNightly { vault: Some(vault), state_dir }) => ExitCode::from(nightly::run(&vault, state_dir.as_deref(), &hooks::exe())),
        Some(Cmd::InstallConventions { vault: None }) => no_vault("Usage: enigma-mcp install-conventions <vault-path>"),
        Some(Cmd::InstallConventions { vault: Some(vault) }) => install_conventions(&vault),
        Some(Cmd::McpSync { check, vault, manifest, config }) => mcp_sync(check, vault, manifest, config),
        Some(Cmd::SkillSync { check, force, dest }) => {
            let dest = dest.unwrap_or_else(|| config::home().join(".claude/skills"));
            match conventions::skill_sync(&dest, check, force) {
                Ok(r) => {
                    println!("{}", json::indented(&r));
                    let pending = ["installed", "drift"].iter().any(|k| r[k].as_array().is_some_and(|a| !a.is_empty()));
                    ExitCode::from(u8::from(check && pending))
                }
                Err(e) => fail(e, 1),
            }
        }
        Some(Cmd::HookSync { check, install }) => {
            if !check && !install {
                return usage_fail("hook-sync", "specify --check or --install");
            }
            match hooks::hook_sync(&hooks::settings_path(), &hooks::exe(), install && !check) {
                Ok(r) => {
                    println!("{}", json::indented(&r));
                    let pending = ["missing", "drift"].iter().any(|k| r[k].as_array().is_some_and(|a| !a.is_empty()));
                    ExitCode::from(u8::from(check && pending))
                }
                Err(e) => fail(e, 2),
            }
        }
        Some(Cmd::Hook { name }) => {
            let mut bytes = Vec::new();
            let _ = std::io::Read::read_to_end(&mut std::io::stdin(), &mut bytes);
            // undecodable stdin is not JSON: silence, like any other bad payload
            let stdin = String::from_utf8(bytes).unwrap_or_else(|_| "\u{0}".into());
            if name == "session-start" {
                let (out, err) = hooks::session_start(&stdin);
                print!("{out}");
                eprint!("{err}");
            } else if let Some(nudge) = hooks::stop_nudge(&stdin) {
                println!("{nudge}");
            }
            ExitCode::SUCCESS
        }
        Some(Cmd::Schedule { action, job, vault, force, dry_run }) => {
            let exe = hooks::exe();
            let j = schedule::Job { name: &job, force, dry_run: dry_run || action == "print", exe: &exe };
            match (action.as_str(), vault) {
                ("uninstall", _) => ExitCode::from(schedule::uninstall(&j)),
                (_, Some(vault)) => ExitCode::from(schedule::install(&j, &vault)),
                (_, None) => usage_fail("schedule", "the following required arguments were not provided: <VAULT>"),
            }
        }
        Some(Cmd::Provision { vault, check }) => {
            let lock = provision::bundled_lock();
            let r = if check { provision::check(&vault, &lock) } else { provision::provision(&vault, &lock, &provision::Http) };
            match r {
                Ok(report) => {
                    println!("{}", json::indented(&report));
                    if !check {
                        print!("{}", provision::epilogue(&report));
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e, 2),
            }
        }
    }
}

/// `error: {message}` for the command's own error kind (exit code), `error: {Kind}: {message}` (exit 1) otherwise.
fn fail(e: Error, code: u8) -> ExitCode {
    if code != 1 && matches!(e.kind, Kind::SyncError | Kind::ProvisionError) {
        eprintln!("error: {}", e.msg);
        return ExitCode::from(code);
    }
    eprintln!("error: {e}");
    ExitCode::from(1)
}

fn no_vault(usage: &str) -> ExitCode {
    eprintln!("{usage}");
    ExitCode::from(1)
}

/// enigma-mcp install-conventions (15 §2).
fn install_conventions(vault: &Path) -> ExitCode {
    if !vault.is_dir() {
        eprintln!("Vault not found: {}", vault.display());
        return ExitCode::from(1);
    }
    match conventions::install(vault, None) {
        Ok(c) if c.is_empty() => {
            println!("Created: nothing (already installed)");
            ExitCode::SUCCESS
        }
        Ok(c) => {
            println!("Created: {}", c.join(", "));
            ExitCode::SUCCESS
        }
        Err(e) => fail(e, 1),
    }
}

/// enigma-mcp mcp-sync (15 §6).
fn mcp_sync(check: bool, vault: Option<PathBuf>, manifest: Option<PathBuf>, config: Option<PathBuf>) -> ExitCode {
    if let Some(m) = manifest.as_ref().filter(|m| !m.is_file()) {
        let msg = format!("manifest not found at {} \u{2014} pass --manifest pointing at your checkout's mcp-servers.json", m.display());
        return usage_fail("mcp-sync", &msg);
    }
    let o = mcp_sync::Options {
        check,
        vault: vault.map(|v| v.display().to_string()).or_else(|| config::var("ENIGMA_VAULT_PATH")),
        manifest,
        config: config.unwrap_or_else(|| config::home().join(".claude.json")),
        exe: hooks::exe(),
    };
    ExitCode::from(mcp_sync::run(&o, &mut std::io::stdout()) as u8)
}
