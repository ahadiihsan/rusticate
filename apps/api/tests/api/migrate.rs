//! DB-012: the `migrate` command applies every migrator, idempotently.

use secrecy::SecretString;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::schema;

#[sqlx::test(migrations = false)]
async fn migrate_applies_every_migrator_idempotently(pool: PgPoolOptions, conn: PgConnectOptions) {
    // Local role contract: the owner's password is its name (docker-compose).
    let url = SecretString::from(format!(
        "postgres://app_owner:app_owner@{}:{}/{}",
        conn.get_host(),
        conn.get_port(),
        conn.get_database().unwrap_or("postgres"),
    ));

    api::migrate::run(&url).await.unwrap();
    api::migrate::run(&url).await.unwrap();

    let owner = pool.connect_with(conn).await.unwrap();
    schema::assert_schema_invariants(&owner).await;
}
