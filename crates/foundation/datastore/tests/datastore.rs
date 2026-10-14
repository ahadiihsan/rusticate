//! Security properties of the data layer, verified as the real runtime roles.
#![cfg(test)]

use datastore::{DataStore, DataStoreConfig, DataStoreError, DomainEvent, SystemStore};
use op_context::Permission;
use serde::Serialize;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use test_kit::{TestDb, ctx, schema};
use uuid::Uuid;

const NOOP: Permission = Permission::new("test.noop.run");

async fn pool_as(connect: &PgConnectOptions, set_role: &'static str, max: u32) -> PgPool {
    PgPoolOptions::new()
        .max_connections(max)
        .after_connect(move |conn, _| {
            Box::pin(async move {
                sqlx::query(set_role).execute(&mut *conn).await?;
                Ok(())
            })
        })
        .connect_with(connect.clone())
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn stores_refuse_the_wrong_database_role(pool: PgPoolOptions, connect: PgConnectOptions) {
    TestDb::new(pool, connect.clone(), &[]).await;

    let bypassing = DataStore::from_pool(pool_as(&connect, "SET ROLE app_system", 1).await).await;
    let rls_bound = SystemStore::from_pool(pool_as(&connect, "SET ROLE app_rw", 1).await).await;
    let owner =
        DataStore::from_pool(PgPoolOptions::new().connect_with(connect).await.unwrap()).await;

    assert!(matches!(
        bypassing,
        Err(DataStoreError::RoleMisconfigured(_))
    ));
    assert!(matches!(
        rls_bound,
        Err(DataStoreError::RoleMisconfigured(_))
    ));
    assert!(matches!(owner, Err(DataStoreError::RoleMisconfigured(_))));
}

#[sqlx::test(migrations = false)]
async fn a_tenant_never_sees_another_tenants_rows(pool: PgPoolOptions, connect: PgConnectOptions) {
    let db = TestDb::new(pool, connect, &[]).await;
    let (a, b) = (
        ctx::user(ctx::tenant(), &[NOOP]),
        ctx::user(ctx::tenant(), &[NOOP]),
    );
    let mut tx = db.app.begin(&a).await.unwrap();
    assert!(tx.record_inbox("test", Uuid::now_v7()).await.unwrap());
    tx.commit().await.unwrap();

    let count = |ctx| {
        let app = db.app.clone();
        async move {
            let mut tx = app.begin_read_only(&ctx).await.unwrap();
            // Deliberately no WHERE tenant_id: RLS alone must filter.
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM platform.inbox_events")
                .fetch_one(tx.conn())
                .await
                .unwrap()
        }
    };

    assert_eq!(count(a).await, 1);
    assert_eq!(count(b).await, 0);
}

#[sqlx::test(migrations = false)]
async fn tenant_setting_never_leaks_to_the_next_borrower(
    pool: PgPoolOptions,
    connect: PgConnectOptions,
) {
    TestDb::new(pool, connect.clone(), &[]).await;
    // One connection, so the second statement provably reuses it.
    let raw = pool_as(&connect, "SET ROLE app_rw", 1).await;
    let store = DataStore::from_pool(raw.clone()).await.unwrap();
    let tx = store.begin(&ctx::user(ctx::tenant(), &[])).await.unwrap();
    tx.commit().await.unwrap();

    let leaked: Option<Uuid> = sqlx::query_scalar("SELECT platform.current_tenant_id()")
        .fetch_one(&raw)
        .await
        .unwrap();

    assert_eq!(
        leaked, None,
        "tenant context must not survive the transaction"
    );
}

#[sqlx::test(migrations = false)]
async fn writes_without_tenant_context_are_rejected(
    pool: PgPoolOptions,
    connect: PgConnectOptions,
) {
    TestDb::new(pool, connect.clone(), &[]).await;
    let raw = pool_as(&connect, "SET ROLE app_rw", 1).await;

    let result =
        sqlx::query("INSERT INTO platform.inbox_events (consumer, event_id) VALUES ('x', $1)")
            .bind(Uuid::now_v7())
            .execute(&raw)
            .await;

    assert!(
        result.is_err(),
        "tenant_id default must not resolve without a tenant"
    );
}

#[sqlx::test(migrations = false)]
async fn read_only_transactions_reject_writes(pool: PgPoolOptions, connect: PgConnectOptions) {
    let db = TestDb::new(pool, connect, &[]).await;
    let mut tx = db
        .app
        .begin_read_only(&ctx::user(ctx::tenant(), &[]))
        .await
        .unwrap();

    let result = tx.record_inbox("test", Uuid::now_v7()).await;

    assert!(matches!(result, Err(DataStoreError::Database(_))));
}

#[derive(Serialize)]
struct ThingHappened {
    thing_id: Uuid,
}

impl DomainEvent for ThingHappened {
    const EVENT_TYPE: &'static str = "test.thing.happened.v1";
    const AGGREGATE_TYPE: &'static str = "test.thing";

    fn aggregate_id(&self) -> Uuid {
        self.thing_id
    }
}

#[sqlx::test(migrations = false)]
async fn emit_records_event_with_tenant_and_correlation(
    pool: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let db = TestDb::new(pool, connect, &[]).await;
    let user = ctx::user(ctx::tenant(), &[]);
    let thing_id = Uuid::now_v7();
    let mut tx = db.app.begin(&user).await.unwrap();
    let event_id = tx.emit(&ThingHappened { thing_id }).await.unwrap();
    tx.commit().await.unwrap();

    let mut sys = db.system.begin().await.unwrap();
    let (tenant, event_type, aggregate_id, correlation, payload): (
        Uuid,
        String,
        Uuid,
        Uuid,
        serde_json::Value,
    ) = sqlx::query_as(
        "SELECT tenant_id, event_type, aggregate_id, correlation_id, payload \
             FROM platform.outbox_events WHERE id = $1",
    )
    .bind(event_id)
    .fetch_one(sys.conn())
    .await
    .unwrap();

    assert_eq!(tenant, user.tenant_id().as_uuid());
    assert_eq!(event_type, "test.thing.happened.v1");
    assert_eq!(aggregate_id, thing_id);
    assert_eq!(correlation, user.correlation_id().as_uuid());
    assert_eq!(payload["thing_id"], thing_id.to_string());
}

#[sqlx::test(migrations = false)]
async fn sqlstates_are_classified(pool: PgPoolOptions, connect: PgConnectOptions) {
    let db = TestDb::new(pool, connect, &[]).await;
    let owner = db.owner_pool();
    sqlx::query("CREATE TABLE public.t (id int PRIMARY KEY, n int CHECK (n > 0))")
        .execute(owner)
        .await
        .unwrap();
    sqlx::query("INSERT INTO public.t VALUES (1, 1)")
        .execute(owner)
        .await
        .unwrap();

    let unique = sqlx::query("INSERT INTO public.t VALUES (1, 1)")
        .execute(owner)
        .await
        .unwrap_err();
    let check = sqlx::query("INSERT INTO public.t VALUES (2, 0)")
        .execute(owner)
        .await
        .unwrap_err();

    let mut holder = owner.begin().await.unwrap();
    sqlx::query("SELECT * FROM public.t WHERE id = 1 FOR UPDATE")
        .execute(&mut *holder)
        .await
        .unwrap();
    let mut waiter = owner.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout = '50ms'")
        .execute(&mut *waiter)
        .await
        .unwrap();
    let lock = sqlx::query("SELECT * FROM public.t WHERE id = 1 FOR UPDATE")
        .execute(&mut *waiter)
        .await
        .unwrap_err();

    assert!(
        matches!(DataStoreError::from(unique), DataStoreError::UniqueViolation { constraint } if constraint == "t_pkey")
    );
    assert!(matches!(
        DataStoreError::from(check),
        DataStoreError::CheckViolation { .. }
    ));
    let lock = DataStoreError::from(lock);
    assert!(matches!(lock, DataStoreError::LockTimeout));
    assert!(lock.is_transient());
}

#[sqlx::test(migrations = false)]
async fn platform_schema_satisfies_invariants(pool: PgPoolOptions, connect: PgConnectOptions) {
    let db = TestDb::new(pool, connect, &[]).await;

    schema::assert_schema_invariants(db.owner_pool()).await;
}

#[sqlx::test(migrations = false)]
async fn schema_guard_detects_isolation_mistakes(pool: PgPoolOptions, connect: PgConnectOptions) {
    let db = TestDb::new(pool, connect, &[]).await;
    sqlx::raw_sql(
        r"
        CREATE SCHEMA demo;
        -- no RLS at all
        CREATE TABLE demo.no_rls (tenant_id uuid NOT NULL, id uuid, PRIMARY KEY (tenant_id, id));
        -- RLS, but a permissive policy that opens everything
        CREATE TABLE demo.open (tenant_id uuid NOT NULL, id uuid, PRIMARY KEY (tenant_id, id));
        ALTER TABLE demo.open ENABLE ROW LEVEL SECURITY;
        ALTER TABLE demo.open FORCE ROW LEVEL SECURITY;
        CREATE POLICY tenant_isolation ON demo.open
            USING (tenant_id = (SELECT platform.current_tenant_id()))
            WITH CHECK (tenant_id = (SELECT platform.current_tenant_id()));
        CREATE POLICY oops ON demo.open USING (true);
        -- a global table nobody declared global, and a cross-tenant FK
        CREATE TABLE demo.lookup (id uuid PRIMARY KEY);
        CREATE TABLE demo.child (tenant_id uuid NOT NULL, id uuid, parent_id uuid,
            PRIMARY KEY (tenant_id, id),
            CONSTRAINT child_parent_fkey FOREIGN KEY (parent_id, tenant_id) REFERENCES demo.no_rls (tenant_id, id));
        CREATE UNIQUE INDEX child_parent_key ON demo.child (parent_id);
        GRANT TRUNCATE ON demo.child TO app_rw;
        -- the cross-tenant system role reading a context's table
        GRANT SELECT ON demo.child TO app_system;
        -- an object outside any context's schema
        CREATE TABLE public.stray (id uuid PRIMARY KEY);
        -- hijacking the tenant source every policy depends on
        CREATE OR REPLACE FUNCTION platform.current_tenant_id() RETURNS uuid
            LANGUAGE sql STABLE AS $$ SELECT NULL::uuid $$;
        ",
    )
    .execute(db.owner_pool())
    .await
    .unwrap();

    let violations = schema::schema_violations(db.owner_pool()).await.join("\n");

    for expected in [
        "DB-004a tenant table must ENABLE and FORCE row level security: demo.no_rls",
        "DB-004c",
        "demo.open policy oops",
        "DB-004e",
        "demo.lookup",
        "DB-005a",
        "child_parent_fkey",
        "DB-005b",
        "child_parent_key",
        "DB-002a",
        "DB-002c",
        "DB-011b",
        "public.stray",
        "DB-003a",
    ] {
        assert!(
            violations.contains(expected),
            "missing `{expected}` in:\n{violations}"
        );
    }
}

/// URL for a local role (the docker-compose role contract uses the role name
/// as password) pointing at the test database.
fn role_url(connect: &PgConnectOptions, role: &str) -> secrecy::SecretString {
    secrecy::SecretString::from(format!(
        "postgres://{role}:{role}@{}:{}/{}",
        connect.get_host(),
        connect.get_port(),
        connect.get_database().unwrap_or("postgres"),
    ))
}

#[sqlx::test(migrations = false)]
async fn connect_verifies_the_role_posture(pool: PgPoolOptions, connect: PgConnectOptions) {
    TestDb::new(pool, connect.clone(), &[]).await;

    let app = DataStore::connect(&DataStoreConfig::new(role_url(&connect, "app_rw")), "test").await;
    let system = SystemStore::connect(
        &DataStoreConfig::new(role_url(&connect, "app_system")),
        "test",
    )
    .await;
    let owner_as_app = DataStore::connect(
        &DataStoreConfig::new(role_url(&connect, "app_owner")),
        "test",
    )
    .await;

    let app = app.unwrap();
    app.ping().await.unwrap();
    system.unwrap().ping().await.unwrap();
    assert!(matches!(
        owner_as_app,
        Err(DataStoreError::RoleMisconfigured(_))
    ));
    app.close().await;
}

#[derive(Serialize)]
struct Bloated {
    blob: String,
}

impl DomainEvent for Bloated {
    const EVENT_TYPE: &'static str = "test.thing.bloated.v1";
    const AGGREGATE_TYPE: &'static str = "test.thing";

    fn aggregate_id(&self) -> Uuid {
        Uuid::nil()
    }
}

#[sqlx::test(migrations = false)]
async fn oversized_events_are_rejected_at_write_time(
    pool: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let db = TestDb::new(pool, connect, &[]).await;
    let mut tx = db.app.begin(&ctx::user(ctx::tenant(), &[])).await.unwrap();

    let outcome = tx
        .emit(&Bloated {
            blob: "x".repeat(datastore::MAX_EVENT_BYTES),
        })
        .await;

    assert!(matches!(outcome, Err(DataStoreError::EventTooLarge { .. })));
}
