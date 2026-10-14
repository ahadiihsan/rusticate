//! TEST-004: the product's complete schema (every migrator it ships) passes
//! the catalog-level tenant-isolation guard.

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::{TestDb, schema};

#[sqlx::test(migrations = false)]
async fn product_schema_satisfies_tenant_invariants(pool: PgPoolOptions, conn: PgConnectOptions) {
    let domain_migrators: Vec<_> = api::migrate::MIGRATORS
        .iter()
        .filter(|(name, _)| *name != "platform")
        .map(|(_, migrator)| *migrator)
        .collect();
    let db = TestDb::new(pool, conn, &domain_migrators).await;

    schema::assert_schema_invariants(db.owner_pool()).await;
}
