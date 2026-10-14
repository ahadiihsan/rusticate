use datastore::{DataStore, SystemStore};
use sqlx::PgPool;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// A migrated, throwaway database with role-accurate handles.
///
/// ```ignore
/// #[sqlx::test(migrations = false)]
/// async fn my_test(pool: PgPoolOptions, conn: PgConnectOptions) {
///     let db = TestDb::new(pool, conn, &[&billing::MIGRATOR]).await;
///     // db.app runs as app_rw (RLS enforced), db.system as app_system.
/// }
/// ```
#[derive(Debug)]
pub struct TestDb {
    owner: PgPool,
    /// RLS-bound application store (`SET ROLE app_rw`).
    pub app: DataStore,
    /// Cross-tenant system store (`SET ROLE app_system`).
    pub system: SystemStore,
}

#[derive(Debug, Clone, Copy)]
enum Role {
    App,
    System,
}

impl Role {
    const fn set_role(self) -> &'static str {
        match self {
            Self::App => "SET ROLE app_rw",
            Self::System => "SET ROLE app_system",
        }
    }
}

impl TestDb {
    /// Runs the platform migrator followed by `domain_migrators`, in order.
    pub async fn new(
        pool_options: PgPoolOptions,
        connect_options: PgConnectOptions,
        domain_migrators: &[&Migrator],
    ) -> Self {
        let owner = pool_options
            .connect_with(connect_options.clone())
            .await
            .expect("connect to the test database as owner");

        for migrator in
            std::iter::once(&datastore::MIGRATOR).chain(domain_migrators.iter().copied())
        {
            migrator.run(&owner).await.expect("run migrations");
        }

        let app = DataStore::from_pool(role_pool(&connect_options, Role::App).await)
            .await
            .expect("app_rw must have an RLS-bound posture");
        let system = SystemStore::from_pool(role_pool(&connect_options, Role::System).await)
            .await
            .expect("app_system must have the system posture");

        Self { owner, app, system }
    }

    /// Owner connection, for catalog inspection and fixtures that must bypass
    /// the application (never for exercising application code).
    pub fn owner_pool(&self) -> &PgPool {
        &self.owner
    }
}

async fn role_pool(connect_options: &PgConnectOptions, role: Role) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |conn, _meta| {
            Box::pin(async move {
                sqlx::query(role.set_role()).execute(&mut *conn).await?;
                Ok(())
            })
        })
        .connect_with(connect_options.clone())
        .await
        .expect("connect role pool")
}
