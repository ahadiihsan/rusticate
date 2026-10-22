//! The project's commands: the single entry point for engineers, agents and CI.
//!
//! Every quality gate in the SOP is a subcommand here, so "done" means the
//! same thing on a laptop, in an agent sandbox and in CI. People run them as
//! `cargo rusticate <command>` (cargo-rusticate forwards to this binary); CI,
//! git hooks and machines without cargo-rusticate run `cargo xtask <command>`.

mod arch;
// rusticate:module(postgres):start
mod db;
// rusticate:module(postgres):end
mod dev_env;
// rusticate:template-only:start
mod generator;
// rusticate:template-only:end
mod metadata;
mod scaffold;
mod sh;
mod workspace;

use anyhow::Result;
use clap::{Parser, Subcommand};
use dev_env::App;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "cargo rusticate",
    bin_name = "cargo rusticate",
    about = "Project automation and architecture enforcement",
    after_help = "`cargo rusticate <command>` and `cargo xtask <command>` are the same command \
                  (install the first with `cargo install cargo-rusticate --locked`).\n\
                  To start another project, run `cargo rusticate new <name>` outside this one."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// One-time setup: git hooks and required tools.
    Setup,
    /// The definition of done. `--fast` skips everything that needs a database.
    Verify {
        #[arg(long)]
        fast: bool,
    },
    /// Everything CI runs, in CI order (needs `db up`).
    Ci,
    /// Architecture conformance: dependency graph, capabilities, layout, migrations.
    Arch,
    /// Format all code (`--check` only verifies).
    Fmt {
        #[arg(long)]
        check: bool,
    },
    /// Clippy on every crate and target, warnings are errors.
    Lint,
    /// Run the test suite with nextest (needs `db up`). Extra args go to nextest.
    Test {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Line coverage of the product crates (tooling excluded); fails under the floor.
    Coverage {
        /// Minimum line coverage in percent (TEST-006).
        #[arg(long, default_value_t = 80)]
        fail_under: u8,
    },
    // rusticate:module(postgres):start
    /// Local services (Docker): Postgres, and NATS with messaging.
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
    // rusticate:module(postgres):end
    /// Regenerate the committed OpenAPI contract (`--check` only verifies).
    Openapi {
        #[arg(long)]
        check: bool,
    },
    /// Scaffold a bounded context, a feature slice or a migration.
    New {
        #[command(subcommand)]
        what: NewWhat,
    },
    /// Mint a local development access token.
    Token {
        /// Tenant id (random if omitted).
        #[arg(long)]
        tenant: Option<Uuid>,
        /// Permission to grant; repeat for several.
        #[arg(long = "perm", required = true)]
        permissions: Vec<String>,
        /// Lifetime in seconds.
        #[arg(long, default_value_t = 3600)]
        ttl: i64,
    },
    /// Run a binary with local development configuration.
    Run {
        app: App,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

// rusticate:module(postgres):start
#[derive(Debug, Subcommand)]
enum DbAction {
    /// Start Postgres and NATS and wait until healthy.
    Up,
    /// Stop them (data is kept).
    Down,
    /// Drop and recreate the dev database, then migrate.
    Reset,
    /// Apply every crate's migrations (platform first).
    Migrate,
    /// Regenerate `.sqlx` query metadata (`--check` only verifies).
    Prepare {
        #[arg(long)]
        check: bool,
    },
}
// rusticate:module(postgres):end

#[derive(Debug, Subcommand)]
enum NewWhat {
    /// A bounded context: `<name>` and `<name>-engine` crates, schema, router, wiring.
    Domain { name: String },
    /// A feature slice `<verb>_<noun>` inside a context.
    Feature {
        domain: String,
        name: String,
        #[arg(long, value_enum, default_value_t = scaffold::FeatureKind::Command)]
        kind: scaffold::FeatureKind,
    },
    // rusticate:module(postgres):start
    /// A timestamped migration in a context's schema.
    Migration { domain: String, description: String },
    // rusticate:module(postgres):end
}

fn main() {
    if let Err(err) = run(Cli::parse()) {
        eprintln!("\nerror: {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Setup => setup(),
        Command::Verify { fast } => verify(fast),
        Command::Ci => ci(),
        Command::Arch => arch::run(),
        Command::Fmt { check } => fmt(check),
        Command::Lint => lint(),
        Command::Test { args } => test(&args, false),
        Command::Coverage { fail_under } => coverage(fail_under),
        // rusticate:module(postgres):start
        Command::Db { action } => match action {
            DbAction::Up => db::up(),
            DbAction::Down => db::down(),
            DbAction::Reset => db::reset(),
            DbAction::Migrate => db::migrate(),
            DbAction::Prepare { check } => db::prepare(check),
        },
        // rusticate:module(postgres):end
        Command::Openapi { check } => openapi(check),
        Command::New { what } => match what {
            NewWhat::Domain { name } => scaffold::domain(&name),
            NewWhat::Feature { domain, name, kind } => scaffold::feature(&domain, &name, kind),
            // rusticate:module(postgres):start
            NewWhat::Migration {
                domain,
                description,
            } => scaffold::migration(&domain, &description).map(|_| ()),
            // rusticate:module(postgres):end
        },
        Command::Token {
            tenant,
            permissions,
            ttl,
        } => token(tenant, &permissions, ttl),
        Command::Run { app, args } => run_app(app, &args),
    }
}

fn setup() -> Result<()> {
    sh::run(sh::cmd("git", ["config", "core.hooksPath", ".githooks"]))?;
    let mut missing = Vec::new();
    for (tool, install) in [
        ("cargo-nextest", "cargo install --locked cargo-nextest"),
        ("cargo-deny", "cargo install --locked cargo-deny"),
        ("cargo-llvm-cov", "cargo install --locked cargo-llvm-cov"),
        // rusticate:module(postgres):start
        (
            "sqlx",
            "cargo install sqlx-cli --no-default-features --features postgres,rustls,sqlx-toml",
        ),
        ("docker", "https://docs.docker.com/get-docker/"),
        // rusticate:module(postgres):end
    ] {
        if !sh::available(tool) {
            missing.push(format!("  {tool:<16} {install}"));
        }
    }
    anyhow::ensure!(missing.is_empty(), "missing tools:\n{}", missing.join("\n"));
    if !sh::available("cargo-rusticate") {
        println!(
            "tip: `cargo install --locked cargo-rusticate` lets you run every command as \
             `cargo rusticate <command>` (`cargo xtask <command>` works without it)"
        );
    }
    // A project generated with fewer modules starts with the template's
    // formatting and contract; make both match this project.
    fmt(false)?;
    if openapi(true).is_err() {
        openapi(false)?;
    }
    println!(
        "✓ setup complete: git hooks installed, all tools present, code formatted, API contract current"
    );
    Ok(())
}

fn fmt(check: bool) -> Result<()> {
    let mut args = vec!["fmt", "--all"];
    if check {
        args.extend(["--", "--check"]);
    }
    sh::run(sh::cargo(args))?;
    // rusticate:template-only:start
    if generator::present() {
        generator::fmt(check)?;
    }
    // rusticate:template-only:end
    Ok(())
}

fn lint() -> Result<()> {
    sh::run(sh::cargo([
        "clippy",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--locked",
        "--",
        "-D",
        "warnings",
    ]))
}

fn test(extra: &[String], ci: bool) -> Result<()> {
    let mut cmd = sh::cargo(["nextest", "run", "--all-features", "--locked"]);
    let scoped = extra
        .iter()
        .any(|arg| arg == "-p" || arg == "--package" || arg.starts_with("--package="));
    if !scoped {
        cmd.arg("--workspace");
    }
    if ci {
        cmd.args(["--profile", "ci"]);
    }
    cmd.args(extra);
    dev_env::apply(&mut cmd);
    sh::run(cmd)
}

fn coverage(fail_under: u8) -> Result<()> {
    let floor = fail_under.to_string();
    let mut cmd = sh::cargo([
        "llvm-cov",
        "nextest",
        "--workspace",
        "--exclude",
        "xtask",
        "--all-features",
        "--locked",
        "--lcov",
        "--output-path",
        "lcov.info",
        "--fail-under-lines",
        floor.as_str(),
        // Binary entry points are wiring exercised in deployment; everything
        // they call is library code that is measured.
        "--ignore-filename-regex",
        r"apps/[^/]+/src/main\.rs$",
    ]);
    dev_env::apply(&mut cmd);
    sh::run(cmd)?;
    sh::run(sh::cargo(["llvm-cov", "report", "--summary-only"]))
}

/// The usage examples in the API docs compile and run (nextest skips them).
fn doc_tests() -> Result<()> {
    let mut cmd = sh::cargo(["test", "--doc", "--workspace", "--all-features", "--locked"]);
    dev_env::apply(&mut cmd);
    sh::run(cmd)
}

fn docs() -> Result<()> {
    let mut cmd = sh::cargo([
        "doc",
        "--workspace",
        "--no-deps",
        "--all-features",
        "--locked",
    ]);
    cmd.env("RUSTDOCFLAGS", "-D warnings");
    sh::run(cmd)
}

fn verify(fast: bool) -> Result<()> {
    sh::step("format", || fmt(true))?;
    sh::step("architecture", arch::run)?;
    sh::step("lint", lint)?;
    // rusticate:template-only:start
    if generator::present() {
        sh::step("generator (cargo-rusticate)", generator::check)?;
    }
    // rusticate:template-only:end
    if fast {
        println!(
            "\n✓ verify --fast passed (tests skipped: run `cargo rusticate verify` before finishing)"
        );
        return Ok(());
    }
    // rusticate:module(postgres):start
    // Query metadata compiles every `query!` against the live schema, so a
    // database fresh from `db up` must be migrated first.
    sh::step("migrate", db::migrate)?;
    sh::step("query metadata", || db::prepare(true))?;
    // rusticate:module(postgres):end
    sh::step("tests", || test(&[], false))?;
    sh::step("doc examples", doc_tests)?;
    println!("\n✓ verify passed");
    Ok(())
}

fn ci() -> Result<()> {
    sh::step("format", || fmt(true))?;
    sh::step("architecture", arch::run)?;
    sh::step("lint", lint)?;
    sh::step("docs", docs)?;
    sh::step("dependencies (cargo-deny)", || {
        sh::run(sh::cargo(["deny", "--all-features", "check"]))
    })?;
    sh::step("openapi contract", || openapi(true))?;
    // rusticate:module(postgres):start
    sh::step("migrate", db::migrate)?;
    sh::step("query metadata", || db::prepare(true))?;
    // rusticate:module(postgres):end
    sh::step("tests", || test(&[], true))?;
    sh::step("doc examples", doc_tests)?;
    // rusticate:template-only:start
    if generator::present() {
        sh::step("generator (cargo-rusticate)", generator::check)?;
        sh::step("generator dependencies (cargo-deny)", generator::deny)?;
        sh::step("generator minimum Rust version", generator::msrv)?;
        sh::step(
            "generator smoke test: generate, then verify",
            generator::smoke,
        )?;
    }
    // rusticate:template-only:end
    println!("\n✓ ci passed");
    Ok(())
}

fn openapi(check: bool) -> Result<()> {
    let path = workspace::root().join("apps/api/openapi.json");
    let generated = sh::output(sh::cargo([
        "run", "--quiet", "--locked", "-p", "api", "--", "openapi",
    ]))?;
    if check {
        let committed = std::fs::read_to_string(&path)?;
        anyhow::ensure!(
            committed == generated,
            "apps/api/openapi.json is stale: run `cargo rusticate openapi` and review the contract diff \
             (in a newly generated project, `cargo rusticate setup` does it)"
        );
        println!("✓ OpenAPI contract is up to date");
    } else {
        std::fs::write(&path, generated)?;
        println!("✓ wrote {}", path.display());
    }
    Ok(())
}

fn token(tenant: Option<Uuid>, permissions: &[String], ttl: i64) -> Result<()> {
    let tenant = op_context::TenantId::from_uuid(tenant.unwrap_or_else(Uuid::now_v7));
    let perms: Vec<&str> = permissions.iter().map(String::as_str).collect();
    let token = iam::dev::issue_hs256(&iam::dev::DevToken {
        secret: dev_env::HS256_SECRET,
        issuer: dev_env::ISSUER,
        audience: dev_env::AUDIENCE,
        tenant,
        actor: op_context::ActorId::from_uuid(Uuid::now_v7()),
        permissions: &perms,
        ttl_secs: ttl,
    })?;
    eprintln!("tenant: {tenant}");
    println!("{token}");
    Ok(())
}

fn run_app(app: App, args: &[String]) -> Result<()> {
    let mut cmd = sh::cargo(["run", "--locked", "-p", app.package(), "--"]);
    if args.is_empty() && matches!(app, App::Api) {
        cmd.arg("serve");
    }
    cmd.args(args);
    dev_env::apply_for(&mut cmd, app);
    sh::run(cmd)
}
