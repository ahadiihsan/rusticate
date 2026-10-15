//! Local database and broker lifecycle (docker compose) and migrations.
//!
//! Migrations run through `sqlx-cli` from each schema-owning crate's
//! directory (so its `sqlx.toml` applies), never through the application
//! binary: a migration must be appliable before code that depends on it
//! compiles.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result};
use proc_macro2::{TokenStream, TokenTree};
use sha2::{Digest as _, Sha256};
use walkdir::WalkDir;

use crate::workspace::{self, Workspace};
use crate::{dev_env, sh};

pub fn up() -> Result<()> {
    sh::run(sh::cmd("docker", ["compose", "up", "--detach", "--wait"]))?;
    println!("✓ postgres on :55432");
    // rusticate:module(messaging):start
    println!("✓ nats on :4222");
    // rusticate:module(messaging):end
    Ok(())
}

pub fn down() -> Result<()> {
    sh::run(sh::cmd("docker", ["compose", "down"]))
}

pub fn reset() -> Result<()> {
    sh::run(sh::cmd(
        "docker",
        [
            "compose",
            "exec",
            "-T",
            "postgres",
            "psql",
            "-U",
            "postgres",
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            "DROP DATABASE IF EXISTS app WITH (FORCE)",
            "-c",
            "CREATE DATABASE app OWNER app_owner",
        ],
    ))?;
    migrate()
}

/// Applies every crate's migrations: the platform schema first, then each
/// context (contexts never reference each other's schemas, DB-013).
pub fn migrate() -> Result<()> {
    let workspace = Workspace::load()?;
    for owner in workspace.schema_owners() {
        let mut cmd = sh::cmd("sqlx", ["migrate", "run"]);
        cmd.current_dir(&owner.abs_dir);
        dev_env::apply(&mut cmd);
        sh::run(cmd)?;
    }
    Ok(())
}

/// Regenerates (or verifies) the committed query metadata.
///
/// Each crate keeps the metadata of its own queries in its own `.sqlx/`
/// (sqlx reads it first), so the metadata leaves a project with its crate.
/// sqlx-cli can only generate it for the whole workspace at once, so this
/// generates it there and hands every file to the crate whose source holds
/// that query.
pub fn prepare(check: bool) -> Result<()> {
    let generated = generate_workspace_metadata()?;
    let workspace = Workspace::load()?;
    let mut expected = Vec::new();
    let mut claimed = BTreeSet::new();
    for member in workspace.members.iter().filter(|m| !m.exempt) {
        let mut files = BTreeMap::new();
        for hash in query_hashes(&member.abs_dir)? {
            let name = format!("query-{hash}.json");
            let content = generated.get(&name).with_context(|| {
                format!(
                    "{}: sqlx produced no metadata for one of its queries ({name})",
                    member.dir
                )
            })?;
            files.insert(name.clone(), content.clone());
            claimed.insert(name);
        }
        expected.push((member, files));
    }
    let unclaimed: Vec<&String> = generated
        .keys()
        .filter(|name| !claimed.contains(*name))
        .collect();
    anyhow::ensure!(
        unclaimed.is_empty(),
        "query metadata no crate claims (a query macro xtask cannot read?): {unclaimed:?}"
    );

    if check {
        let stale: Vec<&str> = expected
            .iter()
            .filter(|(member, files)| {
                read_metadata(&member.abs_dir.join(".sqlx")).ok().as_ref() != Some(files)
            })
            .map(|(member, _)| member.dir.as_str())
            .collect();
        anyhow::ensure!(
            stale.is_empty(),
            "query metadata is stale in {}: run `cargo rusticate db prepare` (needs `db up`)",
            stale.join(", ")
        );
        println!("✓ query metadata is up to date");
        return Ok(());
    }
    for (member, files) in &expected {
        let dir = member.abs_dir.join(".sqlx");
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        if files.is_empty() {
            continue;
        }
        std::fs::create_dir_all(&dir)?;
        for (name, content) in files {
            std::fs::write(dir.join(name), content)?;
        }
    }
    if !check {
        // rust-analyzer expands `query!` once per source edit and does not
        // watch `.sqlx/`, so an expansion from before this run stays stale.
        println!(
            "✓ .sqlx updated. If your editor still shows \"no cached data for this query\", \
             restart rust-analyzer."
        );
    }
    Ok(())
}

/// Runs sqlx-cli for the whole workspace into a scratch `.sqlx/` at the root
/// and returns its files (name → content).
fn generate_workspace_metadata() -> Result<BTreeMap<String, String>> {
    let scratch = workspace::root().join(".sqlx");
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch)?;
    }
    let mut cmd = sh::cargo([
        "sqlx",
        "prepare",
        "--workspace",
        "--",
        "--all-targets",
        "--all-features",
    ]);
    cmd.env("SQLX_OFFLINE", "false");
    dev_env::apply(&mut cmd);
    let outcome = sh::run(cmd).and_then(|()| read_metadata(&scratch));
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch)?;
    }
    outcome
}

/// The files of a `.sqlx/` directory (name → content); empty if it is absent.
fn read_metadata(dir: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    if !dir.exists() {
        return Ok(files);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        files.insert(name, std::fs::read_to_string(entry.path())?);
    }
    Ok(files)
}

/// The sqlx hash (SHA-256 of the SQL text, hex) of every checked query macro
/// (`sqlx::query!`, `query_as!`, `query_scalar!`, ...) in the crate's
/// sources and tests. Tokens are scanned, so queries nested in other macros
/// count too.
fn query_hashes(crate_dir: &Path) -> Result<BTreeSet<String>> {
    let mut hashes = BTreeSet::new();
    for dir in ["src", "tests"] {
        for entry in WalkDir::new(crate_dir.join(dir))
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let tokens: TokenStream = std::fs::read_to_string(path)?
                .parse()
                .map_err(|err| anyhow::anyhow!("{}: {err}", path.display()))?;
            collect_queries(tokens, &mut hashes)?;
        }
    }
    Ok(hashes)
}

fn collect_queries(tokens: TokenStream, hashes: &mut BTreeSet<String>) -> Result<()> {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    for (at, token) in tokens.iter().enumerate() {
        let TokenTree::Group(group) = token else {
            continue;
        };
        let name = at.checked_sub(2).and_then(|i| tokens.get(i));
        let bang = at.checked_sub(1).and_then(|i| tokens.get(i));
        let sqlx = at.checked_sub(5).and_then(|i| tokens.get(i));
        let is_query_macro = matches!(bang, Some(TokenTree::Punct(p)) if p.as_char() == '!')
            && matches!(name, Some(TokenTree::Ident(i)) if i.to_string().starts_with("query"))
            && matches!(sqlx, Some(TokenTree::Ident(i)) if i == "sqlx");
        if is_query_macro {
            let sql = group.stream().into_iter().find_map(|t| match t {
                TokenTree::Literal(literal) => {
                    syn::parse_str::<syn::LitStr>(&literal.to_string()).ok()
                }
                _ => None,
            });
            if let Some(sql) = sql {
                let digest = Sha256::digest(sql.value().as_bytes());
                hashes.insert(hex(&digest));
            }
        }
        collect_queries(group.stream(), hashes)?;
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}
