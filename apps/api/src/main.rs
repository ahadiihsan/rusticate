use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use api::app::{self, AppState};
use api::config::Config;
// rusticate:module(postgres):start
use api::migrate;
// rusticate:module(postgres):end
use clap::{Parser, Subcommand};
// rusticate:module(postgres):start
use datastore::DataStore;
// rusticate:module(postgres):end
use iam::TokenVerifier;
use server_kit::Readiness;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const SERVICE_NAME: &str = "api";

#[derive(Debug, Parser)]
#[command(about = "Product HTTP API")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve HTTP until SIGTERM.
    Serve,
    // rusticate:module(postgres):start
    /// Apply all pending migrations (platform first, then each context), then exit.
    Migrate,
    // rusticate:module(postgres):end
    /// Write the OpenAPI document (stdout, or --out).
    Openapi {
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Serve => serve().await,
        // rusticate:module(postgres):start
        Command::Migrate => {
            let config = Config::load()?;
            let _telemetry = telemetry::init(SERVICE_NAME, &config.telemetry)?;
            let url = config
                .migration_database_url
                .context("APP_MIGRATION_DATABASE_URL is required for `migrate`")?;
            migrate::run(&url).await
        }
        // rusticate:module(postgres):end
        Command::Openapi { out } => write_openapi(out),
    }
}

async fn serve() -> anyhow::Result<()> {
    let config = Config::load()?;
    let _telemetry = telemetry::init(SERVICE_NAME, &config.telemetry)?;
    tracing::info!(environment = ?config.environment, "starting");

    // rusticate:module(postgres):start
    let db = DataStore::connect(&config.database, SERVICE_NAME)
        .await
        .context("connecting to the database")?;
    // rusticate:module(postgres):end
    let verifier =
        Arc::new(TokenVerifier::new(&config.auth).context("building the token verifier")?);
    let readiness = Readiness::new(vec![
        // rusticate:module(postgres):start
        Arc::new(db.clone()),
        // rusticate:module(postgres):end
    ]);
    let state = AppState {
        // rusticate:module(postgres):start
        db: db.clone(),
        // rusticate:module(postgres):end
    };
    let router = app::build(state, verifier, &readiness, &config.http);

    let shutdown = server_kit::shutdown_signal();
    server_kit::serve(router, &config.http, readiness, shutdown).await?;

    // rusticate:module(postgres):start
    db.close().await;
    // rusticate:module(postgres):end
    tracing::info!("stopped");
    Ok(())
}

fn write_openapi(out: Option<PathBuf>) -> anyhow::Result<()> {
    let document = app::openapi_json()?;
    match out {
        Some(path) => std::fs::write(&path, document)
            .with_context(|| format!("writing {}", path.display()))?,
        None => std::io::stdout().write_all(document.as_bytes())?,
    }
    Ok(())
}
