//! Local database and broker lifecycle (docker compose) and migrations.
//!
//! Migrations run through `sqlx-cli` from each schema-owning crate's
//! directory (so its `sqlx.toml` applies), never through the application
//! binary: a migration must be appliable before code that depends on it
//! compiles.

use anyhow::Result;

use crate::workspace::Workspace;
use crate::{dev_env, sh};

pub fn up() -> Result<()> {
    sh::run(sh::cmd("docker", ["compose", "up", "--detach", "--wait"]))?;
    println!("✓ postgres on :55432, nats on :4222");
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
            "DROP DATABASE IF EXISTS rusticate WITH (FORCE)",
            "-c",
            "CREATE DATABASE rusticate OWNER app_owner",
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

/// Regenerates (or verifies) the committed `.sqlx` query metadata.
pub fn prepare(check: bool) -> Result<()> {
    let mut args = vec!["sqlx", "prepare", "--workspace"];
    if check {
        args.push("--check");
    }
    args.extend(["--", "--all-targets", "--all-features"]);
    let mut cmd = sh::cargo(args);
    cmd.env("SQLX_OFFLINE", "false");
    dev_env::apply(&mut cmd);
    sh::run(cmd)?;
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
