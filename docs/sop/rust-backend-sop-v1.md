# Enterprise Rust Backend Architecture & Engineering SOP
**Classification:** Principal Engineering Standard / Production Boilerplate & Agent Skill Specification  
**Architecture Paradigm:** Rust-Native ("School B": Crate DAG + Sans-I/O Engines + Capability-Guarded DataStore)  
**Core Stack:** Rust (2024 Edition) • Tokio • Axum • Tower • SQLx (PostgreSQL RLS) • NATS JetStream / Apalis • OpenTelemetry • Utoipa  

---

## 1. Architectural Philosophy & Non-Negotiable Tenets

This specification defines the baseline architecture for all multi-tenant backend services across the organization. Rather than importing Object-Oriented Java/C# patterns (which rely on ubiquitous runtime interface indirection, `Arc<dyn Repository>` heap allocations, and complex mocking frameworks), we enforce **Rust-Native Architecture**:

1. **Compile-Time Boundaries Over Runtime Conventions:** Architectural isolation is enforced by the Cargo module and crate dependency graph (Directed Acyclic Graph). If a layer must not perform database I/O or network calls, those crates (`sqlx`, `reqwest`, `tokio`) are physically absent from its `Cargo.toml`.
2. **Sans-I/O Functional Core, Imperative Orchestrator Shell:** Complex business rules, state machines, mathematical calculations, and AST evaluations live in **100% pure, synchronous, deterministic Sans-I/O Engine Crates** (`*-engine`). They contain zero `async/await`, zero database connections, and zero mocking requirements.
3. **Capability-Based Multi-Tenancy (`OpCtx` & `TenantTx`):** No database query or workflow can execute without an unforgeable `OpCtx` (Operation Context) token and a tenant-scoped database transaction (`TenantTx`) that enforces PostgreSQL Row-Level Security (`RLS`) at the connection level.
4. **3-Hop Traceability:** Every feature maps 1-to-1 with Domain-Driven Design (DDD) Ubiquitous Language (`<verb>_<noun>`) and traces from HTTP entry $\rightarrow$ Workflow Orchestrator $\rightarrow$ Pure Engine / Database Query in a maximum of three direct function hops—with zero unnecessary trait indirection.
5. **Parse, Don't Validate:** Raw primitives (`String`, `i64`, `f64`) are rejected at the HTTP boundary and parsed into strict Domain Value Objects (`TenantId`, `MinorMoney`, `ValidEmail`, `IdempotencyKey`) via `TryFrom`. Floating-point types (`f32`, `f64`) are strictly forbidden in financial or inventory arithmetic.

---

## 2. Pillar 0: Workspace Topology, Crate DAG & Compiler Guardrails

### 2.1 The Architectural Rule (Domain-Agnostic Standard)
All backend products reside in a Cargo Workspace organized into three distinct tiers. Dependencies must flow strictly downward (`apps` $\rightarrow$ `crates/domains` $\rightarrow$ `crates/foundation`). Circular dependencies are impossible by design.

```text
workspace-root/
├── Cargo.toml                          # Centralized dependency versions & strict workspace lints
├── clippy.toml                         # Disallowed methods/types & test exemptions
├── deny.toml                           # License, advisory, and duplicate crate checks (cargo-deny)
├── AGENTS.md                           # AI Coding Agent & Engineer SOP Playbook
│
├── apps/                               # TIER 1: DEPLOYABLE BINARIES (Thin Runtime Wiring Only)
│   ├── product-alpha-api/              # HTTP/gRPC server binary mounting selected domain crates
│   └── product-alpha-worker/           # Background job / Outbox stream processor binary
│
├── crates/
│   ├── foundation/                     # TIER 2: PLATFORM CHASSIS (Zero Business/Product Logic)
│   │   ├── op-context/                 # Unforgeable OpCtx, TenantId, ActorId, Permission RBAC
│   │   ├── datastore-core/             # DataStore, TenantTx (RLS), OutboxWriter, Cursor Streamer
│   │   ├── api-error/                  # RFC 9457 ProblemDetails + Axum IntoResponse
│   │   ├── idempotency/                # Tower middleware & storage guard for idempotent mutations
│   │   └── telemetry/                  # OpenTelemetry (OTLP), structured JSON logs, healthchecks
│   │
│   └── domains/                        # TIER 3: BOUNDED CONTEXTS (Reusable Domain Modules)
│       ├── shared-iam/                 # Identity, JWT/API-Key verification, RBAC policy store
│       ├── shared-tenant/              # Tenant lifecycle, quotas, tier configuration
│       ├── domain-alpha-engine/        # PURE SANS-I/O ENGINE (No tokio, no sqlx, pure DDD math/rules)
│       └── domain-alpha/               # Feature Orchestration (HTTP + Workflows + DB Queries)
```

#### Strict Dependency Matrix

| Crate Tier | Allowed Internal Dependencies | Allowed External Crates | Strictly Forbidden Crates |
| :--- | :--- | :--- | :--- |
| **`crates/foundation/*`** | Other `foundation` crates | `tokio`, `axum`, `sqlx`, `tracing`, `serde`, `uuid` | Any `crates/domains/*` crate |
| **`crates/domains/*-engine`** *(Sans-I/O)* | None (or pure primitive shared types) | `serde`, `uuid`, `chrono`, `rust_decimal`, `thiserror`, `proptest` | `tokio`, `sqlx`, `axum`, `reqwest`, `redis`, `std::fs`, `std::net` |
| **`crates/domains/*`** *(Feature Crate)* | `foundation/*`, its companion `*-engine`, public API of `shared-*` | `tokio`, `axum`, `sqlx`, `utoipa`, `tracing`, `validator` | Direct access to another domain crate's internal `db` module |
| **`apps/*`** *(Deployable Binary)* | `foundation/*`, `domains/*` | `tokio`, `axum`, `clap`, `dotenvy` | Direct business logic or raw SQL queries |

---

### 2.2 Canonical Boilerplate: Root `Cargo.toml` & `clippy.toml`

Every workspace MUST enforce these exact compiler and Clippy lints at the root level. Notice how `unwrap_used`, `expect_used`, `panic`, `todo`, `unsafe_code`, and `float_arithmetic` are denied at compile time.

```toml
# File: Cargo.toml (Workspace Root)
[workspace]
resolver = "2"
members = [
    "apps/*",
    "crates/foundation/*",
    "crates/domains/*",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
publish = false

[workspace.dependencies]
# Async Runtime & Web Framework
tokio = { version = "1.43", features = ["full"] }
axum = { version = "0.8", features = ["macros"] }
tower = { version = "0.5", features = ["util", "timeout", "limit"] }
tower-http = { version = "0.6", features = ["trace", "cors", "compression-gzip", "request-id", "catch-panic"] }
futures = "0.3"

# Database & Serialization
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "postgres", "uuid", "chrono", "rust_decimal", "json"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Domain Primitives & Security
uuid = { version = "1.11", features = ["v7", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
rust_decimal = { version = "1.36", features = ["serde-with-str", "maths"] }
secrecy = { version = "0.10", features = ["serde"] }
thiserror = "2.0"
anyhow = "1.0"

# OpenAPI & Observability
utoipa = { version = "5.3", features = ["axum_extras", "uuid", "chrono", "decimal"] }
utoipa-axum = "0.2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# Testing
proptest = "1.6"

# ============================================================================
# COMPILER GUARDRAILS: Enforced across all crates via `lints.workspace = true`
# ============================================================================
[workspace.lints.rust]
unsafe_code = "forbid"
unused_must_use = "deny"
unreachable_pub = "warn"

[workspace.lints.clippy]
all = { level = "deny", priority = -1 }
pedantic = { level = "warn", priority = -1 }

# Zero-Panic Production Mandate
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
indexing_slicing = "deny"

# Financial & Resource Safety
float_arithmetic = "deny"
large_futures = "warn"
dbg_macro = "deny"
print_stdout = "deny"
print_stderr = "deny"

# Ergonomic overrides for pedantic rules
module_name_repetitions = "allow"
missing_errors_doc = "allow"
must_use_candidate = "allow"
```

```toml
# File: clippy.toml (Workspace Root)
# Allow unwrap/expect/panic ONLY inside #[cfg(test)] blocks and test functions
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests = true
allow-dbg-in-tests = true
max-fn-params-bools = 1
```

---

### 2.3 Inside a Domain Crate: The 3-Hop Feature Anatomy

Within any feature crate (`crates/domains/<bounded-context>/`), code is organized by **Use Case / Domain Verb** (Ubiquitous Language), never by horizontal technical folders (`controllers/`, `services/`, `repositories/`). We use the modern Rust 2018+ module layout (avoiding `mod.rs` files so IDE tabs always display descriptive filenames).

```text
crates/domains/billing-ledger/
├── Cargo.toml
└── src/
    ├── lib.rs                          # Public Facade: ONLY exports `router()`, public workflows, and `types`
    ├── types.rs                        # Bounded Context Value Objects & Domain Events
    ├── error.rs                        # Domain-specific error enum -> converts to foundation `ApiError`
    │
    ├── issue_invoice.rs                # HOP 2: Workflow Orchestrator (`pub async fn issue_invoice(...)`)
    └── issue_invoice/
        ├── http.rs                     # HOP 1: Axum Handler + HTTP Request/Response DTOs + OpenAPI
        └── db.rs                       # HOP 3: Private SQLx queries (`mod db;` - strictly private to feature!)
```

#### The 3-Hop Traceability Contract
1. **Hop 1 (`<feature>/http.rs`):** Extracts `OpCtx`, `State<AppState>`, and `Json<RequestDto>`. Converts `RequestDto` into validated Domain Command/Value Objects via `TryFrom`. Calls Hop 2. Formats output into `Json<ResponseDto>`. Zero business logic; zero SQL.
2. **Hop 2 (`<feature>.rs`):** Decorated with `#[tracing::instrument]`. Verifies `opctx.require_permission(...)`. Opens a tenant-isolated transaction (`db.begin_tenant_tx(&opctx)`). Calls Hop 3 (`db::*`) to load state, passes pure structs to the **Sans-I/O Engine** (`*-engine`) for business decisions/calculations, persists changes and outbox events via Hop 3 (`db::*`), and commits the transaction.
3. **Hop 3 (`<feature>/db.rs` & `*-engine`):**
   * `db.rs` executes compile-time checked `sqlx` queries against `&mut TenantTx` and maps rows to domain structs. Contains zero `if/else` domain branching.
   * `*-engine` executes pure synchronous business rules.

---

### 2.4 Real-World Context Examples

> **Context A — Multi-Product Modularity (Shared IAM & Tenant Management):**  
> Because `crates/domains/shared-iam` and `crates/domains/shared-tenant` expose a standard `pub fn router() -> OpenApiRouter<AppState>` and public orchestrator functions, the **CDP**, **HRIS**, and **D2C Commerce** binaries (`apps/cdp-api`, `apps/hris-api`, `apps/d2c-api`) simply mount those crates in `main.rs`. A bug fix in JWT token rotation or tenant quota enforcement immediately propagates across all three products with zero code duplication.
>
> **Context B — HRIS Payroll Engine (`hris-payroll-engine` vs `hris-payroll`):**  
> Progressive tax brackets, prorated attendance deductions, social security contributions, and overtime multipliers live inside `crates/domains/hris-payroll-engine`. Because `sqlx` and `tokio` are excluded from its `Cargo.toml` and `clippy::float_arithmetic = "deny"` is enforced, it is physically impossible for an engineer to introduce a floating-point rounding bug or an N+1 query inside the payroll calculation loop.

---

## 3. Pillar 1: Capability Context (`OpCtx`), Data Layer, RLS & Storage Topology

### 3.1 The Architectural Rule (Domain-Agnostic Standard)
1. **Capability-Based Security (`OpCtx`):** Every workflow orchestrator and database transaction constructor MUST take `&OpCtx` as its first parameter. `OpCtx` fields are private to `crates/foundation/op-context` and can only be constructed by authenticated middleware (HTTP JWT/API Key extractor or signed background worker envelope) or explicit `OpCtx::system_for_tenant(...)` constructors audited for background jobs.
2. **Defense-in-Depth Multi-Tenancy (`TenantTx` + PostgreSQL RLS):**
   * Raw `&PgPool` access is forbidden inside domain crates.
   * Domain crates interact with PostgreSQL exclusively through `TenantTx<'a>`, which wraps a `sqlx::Transaction<'a, Postgres>`.
   * Upon transaction creation, `begin_tenant_tx(&self, opctx: &OpCtx)` automatically executes `SELECT set_config('app.current_tenant', $1, true)` (transaction-local configuration).
   * Every multi-tenant table in PostgreSQL MUST have:
     1. `tenant_id UUID NOT NULL DEFAULT current_setting('app.current_tenant')::uuid`
     2. `ENABLE ROW LEVEL SECURITY` and `FORCE ROW LEVEL SECURITY`
     3. A strict RLS policy: `USING (tenant_id = current_setting('app.current_tenant', true)::uuid)`
3. **UUIDv7 Primary Keys:** All primary keys and entity IDs across all tables MUST use **UUIDv7** (`uuid::Uuid::now_v7()`), never UUIDv4 or `BIGSERIAL`. UUIDv7 is time-ordered, preventing B-Tree index fragmentation in PostgreSQL and ClickHouse under heavy insert loads while remaining globally unique across distributed nodes.
4. **Workload-Aware Storage Routing (OLTP vs. OLAP):**
   * **OLTP (PostgreSQL via `sqlx`):** Used for transactional state, entity CRUD, ACID workflows, row-level locking, and the Transactional Outbox.
   * **OLAP (ClickHouse or Partitioned Analytical Store):** Used when a feature requires scanning, aggregating, or filtering $>100,000$ historical event rows or evaluating dynamic boolean set algebra (e.g., user behavior timelines, telemetry, analytical segmentation). Analytical stores must enforce `WHERE tenant_id = ?` via a mandatory `AnalyticalTenantClient` wrapper that requires `&OpCtx`.

---

### 3.2 Canonical Boilerplate: `op-context` & `datastore-core`

#### A. `crates/foundation/op-context/src/lib.rs`
```rust
// File: crates/foundation/op-context/src/lib.rs
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// Strongly-typed Tenant Identifier (UUIDv7)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TenantId(Uuid);

impl TenantId {
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Strongly-typed Actor Identifier (User, ServiceAccount, or System Worker)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActorId(Uuid);

impl ActorId {
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

/// Fine-grained capability permissions enforced at the top of every workflow.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    ResourceRead,
    ResourceWrite,
    BatchExecute,
    TenantAdmin,
    Custom(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AuthzError {
    #[error("Actor {actor_id:?} lacks required permission '{required:?}' in tenant {tenant_id}")]
    Forbidden {
        tenant_id: TenantId,
        actor_id: ActorId,
        required: Permission,
    },
    #[error("Invalid or missing authentication context")]
    Unauthenticated,
}

/// Unforgeable Operation Context.
/// Fields are private so domain code cannot fabricate or mutate tenant/actor identity.
#[derive(Debug, Clone)]
pub struct OpCtx {
    tenant_id: TenantId,
    actor_id: ActorId,
    permissions: HashSet<Permission>,
    correlation_id: Uuid,
}

impl OpCtx {
    /// Restricted constructor: Only called by verified auth middleware or signed worker envelopes.
    pub fn new_authenticated(
        tenant_id: TenantId,
        actor_id: ActorId,
        permissions: impl IntoIterator<Item = Permission>,
        correlation_id: Uuid,
    ) -> Self {
        Self {
            tenant_id,
            actor_id,
            permissions: permissions.into_iter().collect(),
            correlation_id,
        }
    }

    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    pub const fn actor_id(&self) -> ActorId {
        self.actor_id
    }

    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Capability guard: Call at the entry of every workflow orchestrator.
    pub fn require_permission(&self, required: Permission) -> Result<(), AuthzError> {
        if self.permissions.contains(&Permission::TenantAdmin)
            || self.permissions.contains(&required)
        {
            Ok(())
        } else {
            Err(AuthzError::Forbidden {
                tenant_id: self.tenant_id,
                actor_id: self.actor_id,
                required,
            })
        }
    }
}
```

#### B. `crates/foundation/datastore-core/src/lib.rs`
```rust
// File: crates/foundation/datastore-core/src/lib.rs
use op_context::{OpCtx, TenantId};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use std::ops::{Deref, DerefMut};

#[derive(Debug, thiserror::Error)]
pub enum DataStoreError {
    #[error("Database operation failed: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("Serialization failure in outbox/datastore: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Concrete DataStore handle shared across the application via Axum State.
/// Does NOT expose raw PgPool to domain crates to prevent unscoped queries.
#[derive(Clone)]
pub struct DataStore {
    pool: PgPool,
}

impl DataStore {
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Opens an ACID transaction and immediately binds PostgreSQL Row-Level Security (RLS)
    /// to the caller's verified `TenantId` using transaction-local `set_config`.
    pub async fn begin_tenant_tx<'a>(
        &'a self,
        opctx: &OpCtx,
    ) -> Result<TenantTx<'a>, DataStoreError> {
        let mut tx = self.pool.begin().await?;

        // Set `app.current_tenant` locally for the lifetime of this transaction only (`is_local = true`).
        // When the transaction commits or rolls back, the connection returns to the pool clean.
        sqlx::query("SELECT set_config('app.current_tenant', $1, true), set_config('app.current_actor', $2, true)")
            .bind(opctx.tenant_id().as_uuid().to_string())
            .bind(opctx.actor_id().as_uuid().to_string())
            .execute(&mut *tx)
            .await?;

        Ok(TenantTx {
            tx,
            tenant_id: opctx.tenant_id(),
        })
    }

    /// Healthcheck probe for readiness endpoints (`/health/ready`).
    pub async fn ping(&self) -> Result<(), DataStoreError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }
}

/// A Tenant-Isolated PostgreSQL Transaction.
/// Guarantees that PostgreSQL RLS is active for `self.tenant_id`.
pub struct TenantTx<'a> {
    tx: Transaction<'a, Postgres>,
    tenant_id: TenantId,
}

impl<'a> TenantTx<'a> {
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Provides mutable access to the underlying `PgConnection` for `sqlx::query!` execution.
    pub fn conn(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    pub async fn commit(self) -> Result<(), DataStoreError> {
        self.tx.commit().await?;
        Ok(())
    }

    pub async fn rollback(self) -> Result<(), DataStoreError> {
        self.tx.rollback().await?;
        Ok(())
    }
}

impl<'a> Deref for TenantTx<'a> {
    type Target = PgConnection;
    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl<'a> DerefMut for TenantTx<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tx
    }
}
```

#### C. Standard PostgreSQL Migration Template (RLS + UUIDv7)
Every database table migration in `migrations/` MUST follow this template:

```sql
-- Migration: Standard Multi-Tenant Table with Defense-in-Depth RLS
CREATE TABLE IF NOT EXISTS ledger_entries (
    id              UUID PRIMARY KEY,
    tenant_id       UUID NOT NULL DEFAULT (current_setting('app.current_tenant'))::uuid,
    account_id      UUID NOT NULL,
    amount_minor    BIGINT NOT NULL,
    currency        VARCHAR(3) NOT NULL,
    version         INT NOT NULL DEFAULT 1,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    -- Composite indexes ALWAYS lead with `tenant_id` for selective B-Tree scans
    CONSTRAINT chk_currency_len CHECK (char_length(currency) = 3)
);

CREATE INDEX idx_ledger_entries_tenant_account ON ledger_entries (tenant_id, account_id);

-- Enforce Row-Level Security even for table owners
ALTER TABLE ledger_entries ENABLE ROW LEVEL SECURITY;
ALTER TABLE ledger_entries FORCE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_policy ON ledger_entries
    AS RESTRICTIVE
    FOR ALL
    USING (tenant_id = (current_setting('app.current_tenant', true))::uuid)
    WITH CHECK (tenant_id = (current_setting('app.current_tenant', true))::uuid);
```

---

### 3.3 Real-World Context Examples

> **Context A — High-Scale Analytical Segmentation (CDP):**  
> In the **CDP**, evaluating a dynamic segment like *"Customers who purchased $\ge 3$ times in the last 30 days AND opened an email in the last 7 days"* across 20 million profiles and 500 million behavioral events would stall an OLTP PostgreSQL instance if run on primary tables.  
> Following Pillar 1:
> * **PostgreSQL (`TenantTx`)** stores the *Segment Definition AST*, *Campaign Metadata*, and *Tenant Configuration*.
> * **Sans-I/O `cdp-segment-engine`** compiles the Segment AST into a parameterized analytical query (ClickHouse SQL or partitioned RoaringBitmap expression) while injecting the mandatory `tenant_id` predicate.
> * **ClickHouse (`AnalyticalTenantClient`)** executes the vectorized columnar scan in $<100\text{ ms}$ and streams the resulting `CustomerId` chunks to the campaign dispatcher.
>
> **Context B — Zero-Leak Compliance in HRIS Payroll:**  
> In **HRIS**, a single database holds sensitive salary structures and tax filings for hundreds of enterprise client companies. Because every query runs inside `TenantTx` with `FORCE ROW LEVEL SECURITY`, even if an engineer writes a complex raw SQL join in `db.rs` and forgets `WHERE tenant_id = $1`, PostgreSQL's RLS engine automatically filters every joined table to `app.current_tenant`, making cross-tenant data leakage structurally impossible.

---

## 4. Pillar 2: Async Pipelines, Transactional Outbox & Million-Row Streaming

### 4.1 The Architectural Rule (Domain-Agnostic Standard)
1. **Strict Ban on Dual-Writes:** Workflows MUST NEVER publish directly to NATS, Kafka, Redis, or external webhooks inside a database transaction or immediately after `tx.commit()` without an outbox guarantee. If the process crashes between `tx.commit()` and `broker.publish()`, the system enters an inconsistent state.
2. **The Transactional Outbox Pattern:** All domain events and asynchronous job triggers are inserted into the `outbox_events` PostgreSQL table **inside the exact same `TenantTx`** that mutates the domain state.
3. **Bounded-Memory Cursor Streaming (`O(1)` RAM for `O(N)` Rows):**
   * Calling `.fetch_all()` on an unbounded query or batch job is strictly forbidden.
   * Any operation processing $>1,000$ records MUST use `sqlx::query_as(...).fetch(conn)` (which opens a server-side/streamed cursor) combined with `futures::StreamExt::try_chunks(BATCH_SIZE)`.
   * Each chunk of 500–2,000 items is processed or published to NATS JetStream with backpressure (`await`ing broker `Ack`) before the next chunk is pulled from the database.

---

### 4.2 Canonical Boilerplate: Transactional Outbox & Bounded Streamer

```rust
// File: crates/foundation/datastore-core/src/outbox.rs
use crate::{DataStoreError, TenantTx};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Envelope for all asynchronous Domain Events persisted in the Transactional Outbox.
#[derive(Debug, Clone, Serialize)]
pub struct OutboxEvent<P: Serialize> {
    pub id: Uuid,
    pub event_type: &'static str,
    pub aggregate_type: &'static str,
    pub aggregate_id: Uuid,
    pub payload: P,
    pub occurred_at: DateTime<Utc>,
}

impl<'a> TenantTx<'a> {
    /// Persists a Domain Event inside the active tenant transaction.
    /// Guaranteed to commit or roll back atomically with business state changes.
    pub async fn emit_outbox_event<P: Serialize>(
        &mut self,
        event_type: &'static str,
        aggregate_type: &'static str,
        aggregate_id: Uuid,
        payload: &P,
    ) -> Result<Uuid, DataStoreError> {
        let event_id = Uuid::now_v7();
        let tenant_id = self.tenant_id().as_uuid();
        let payload_json = serde_json::to_value(payload)?;

        sqlx::query(
            r#"
            INSERT INTO outbox_events (
                id, tenant_id, event_type, aggregate_type, aggregate_id, payload, status, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, 'PENDING', NOW())
            "#,
        )
        .bind(event_id)
        .bind(tenant_id)
        .bind(event_type)
        .bind(aggregate_type)
        .bind(aggregate_id)
        .bind(payload_json)
        .execute(self.conn())
        .await?;

        Ok(event_id)
    }
}
```

#### Memory-Bounded Million-Row Streamer Pattern
Use this canonical pattern whenever a background worker needs to stream millions of rows without spiking pod memory:

```rust
// Canonical Pattern: Streaming Million-Row Datasets in Constant O(1) Memory
use futures::{StreamExt, TryStreamExt};
use op_context::OpCtx;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub struct TargetRecipientRow {
    pub recipient_id: Uuid,
    pub destination_address: String,
}

/// Streams potentially millions of rows in fixed chunks of 1,000.
/// Peak memory footprint remains flat (~15 MB) whether streaming 10,000 or 10,000,000 rows.
pub async fn stream_recipient_batches<F, Fut>(
    pool: &PgPool,
    opctx: &OpCtx,
    segment_id: Uuid,
    chunk_size: usize,
    mut publish_batch_fn: F,
) -> Result<u64, anyhow::Error>
where
    F: FnMut(Vec<TargetRecipientRow>) -> Fut,
    Fut: std::future::Future<Output = Result<(), anyhow::Error>>,
{
    let mut conn = pool.acquire().await?;

    // Bind tenant context on the dedicated streaming connection
    sqlx::query("SELECT set_config('app.current_tenant', $1, false)")
        .bind(opctx.tenant_id().as_uuid().to_string())
        .execute(&mut *conn)
        .await?;

    let row_stream = sqlx::query_as::<_, TargetRecipientRow>(
        r#"
        SELECT recipient_id, destination_address
        FROM segment_memberships
        WHERE tenant_id = $1 AND segment_id = $2
        ORDER BY recipient_id ASC
        "#,
    )
    .bind(opctx.tenant_id().as_uuid())
    .bind(segment_id)
    .fetch(&mut *conn); // <-- Returns an async Stream, NOT a Vec!

    let mut chunked_stream = row_stream.try_chunks(chunk_size);
    let mut total_dispatched: u64 = 0;

    while let Some(batch) = chunked_stream.try_next().await.map_err(|e| e.1)? {
        let batch_len = batch.len() as u64;
        // Backpressure-aware dispatch: awaits broker ACK before pulling next 1,000 rows from DB
        publish_batch_fn(batch).await?;
        total_dispatched += batch_len;
    }

    Ok(total_dispatched)
}
```

---

### 4.3 Real-World Context Examples

> **Context A — CDP Million-Customer Campaign Dispatch:**  
> When a marketer triggers a campaign targeting a dynamic segment of **2,500,000 customers**:
> 1. The HTTP endpoint `POST /v1/campaigns/{id}/launch` transitions the campaign state to `LAUNCHING`, writes a `CampaignLaunchRequested` event into `outbox_events` inside `TenantTx`, and returns `202 Accepted` in $8\text{ ms}$.
> 2. The Outbox relay publishes `CampaignLaunchRequested` to **NATS JetStream**.
> 3. The campaign worker consumes the event, invokes `stream_recipient_batches(..., 1_000, ...)` to stream 1,000 customers at a time from the segment store, and fans out micro-batches (`CampaignDispatchChunk` of 1,000 recipients) into the delivery queue with deterministic deduplication keys (`campaign_id:recipient_id`).
>
> **Context B — HRIS Asynchronous Payroll Batch Disbursement:**  
> Calculating and generating PDF payslips + bank disbursement files for 15,000 employees cannot block an HTTP socket. The orchestrator commits the batch state and emits `PayrollBatchCalculationRequested` via the Transactional Outbox. Workers process employee chunks idempotently and update progress counters cleanly.

---

## 5. Pillar 3: Idempotency, Concurrency Control & State Machines

### 5.1 The Architectural Rule (Domain-Agnostic Standard)
1. **Mandatory `Idempotency-Key` on State-Mutating Endpoints:** All `POST` and `PATCH` endpoints that trigger financial transactions, inventory reservations, batch jobs, or external dispatches MUST require an `Idempotency-Key` header (UUID or client-generated token up to 64 chars).
2. **Atomic Idempotency Lifecycle:**
   * The idempotency layer computes a SHA-256 hash of the request payload and inserts `(tenant_id, actor_id, idempotency_key, request_hash, status = 'IN_PROGRESS')` using `INSERT ... ON CONFLICT DO NOTHING`.
   * If a conflict occurs:
     * If `request_hash` differs $\rightarrow$ return `422 Unprocessable Entity` (Idempotency Key Reused With Different Payload).
     * If `status == 'IN_PROGRESS'` $\rightarrow$ return `409 Conflict` (Concurrent Request In Flight).
     * If `status == 'COMPLETED'` $\rightarrow$ return the stored `response_status` and `response_body` immediately without re-executing the workflow.
3. **Database Concurrency Control:**
   * **Pessimistic Locking (`SELECT ... FOR UPDATE`):** Required when mutating shared counters, wallet balances, or scarce inventory inside `TenantTx`. Always lock rows in a deterministic sort order (`ORDER BY id ASC`) to prevent deadlocks when locking multiple items.
   * **Optimistic Concurrency (`version` column):** Required on collaborative aggregates (e.g., configuration documents, employee records, draft orders). Updates must execute `UPDATE table SET ..., version = version + 1 WHERE id = $1 AND version = $2`. If `rows_affected() == 0`, return `ApiError::Conflict("Stale object version")`.
4. **Typestate Pattern for Domain Lifecycles:** Multi-stage entities in Sans-I/O engines should encode their lifecycle state (`Draft`, `Approved`, `Committed`) at the type level or via exhaustive Rust `enum` transitions so invalid state transitions are rejected at compile time or in pure synchronous unit tests.

---

### 5.2 Canonical Boilerplate: Concurrency-Safe SQL & Typestate Aggregate

```rust
// Canonical Pattern: Deterministic Pessimistic Locking in `db.rs` (Zero Deadlocks)
use datastore_core::{DataStoreError, TenantTx};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct LockedQuotaRow {
    pub resource_id: Uuid,
    pub available_units: i64,
    pub version: i32,
}

/// Locks multiple quota/inventory records in deterministic ascending UUID order
/// to guarantee deadlock-free acquisition under high concurrency.
pub async fn lock_resources_for_allocation(
    tx: &mut TenantTx<'_>,
    mut resource_ids: Vec<Uuid>,
) -> Result<Vec<LockedQuotaRow>, DataStoreError> {
    // CRITICAL: Sort and deduplicate IDs so concurrent transactions always acquire
    // row locks in the exact same order, eliminating circular wait deadlocks.
    resource_ids.sort_unstable();
    resource_ids.dedup();

    let rows = sqlx::query_as::<_, LockedQuotaRow>(
        r#"
        SELECT resource_id, available_units, version
        FROM resource_quotas
        WHERE tenant_id = $1 AND resource_id = ANY($2)
        ORDER BY resource_id ASC
        FOR UPDATE
        "#,
    )
    .bind(tx.tenant_id().as_uuid())
    .bind(&resource_ids)
    .fetch_all(tx.conn())
    .await?;

    Ok(rows)
}
```

---

### 5.3 Real-World Context Examples

> **Context A — D2C Commerce Flash-Sale Checkout:**  
> During a flash sale, 5,000 shoppers attempt to buy the last 50 units of a SKU simultaneously, and mobile clients on flaky networks retry `POST /v1/checkout` multiple times:
> * The `Idempotency-Key` guard guarantees that a shopper tapping "Pay Now" three times never creates duplicate orders or double charges.
> * Inside `checkout/db.rs`, `lock_resources_for_allocation` sorts the cart's SKU UUIDs ascending and executes `SELECT ... FOR UPDATE`. Two shoppers purchasing `[SKU_A, SKU_B]` and `[SKU_B, SKU_A]` will never deadlock PostgreSQL, and inventory can never drop below zero.
>
> **Context B — HRIS Payroll Approval & Bank Disbursement:**  
> If two HR directors click "Approve & Disburse Payroll" simultaneously, optimistic locking (`WHERE batch_id = $1 AND status = 'APPROVED' AND version = $2`) plus the `Idempotency-Key` header ensures only one disbursement instruction is ever emitted to the banking gateway.

---

## 6. Pillar 4: Financial Precision, Type-Driven Validation & PII Security

### 6.1 The Architectural Rule (Domain-Agnostic Standard)
1. **Zero Floating-Point Money:** `f32` and `f64` are banned across all domain crates via `clippy::float_arithmetic = "deny"`.
   * All monetary values in the database are stored either as **`BIGINT` in minor currency units** (e.g., cents/sen/satang) or `NUMERIC(20, 4)` paired with an explicit ISO-4217 `Currency` code.
   * All monetary calculations in Sans-I/O engines use `rust_decimal::Decimal` (with explicit **`MidpointNearestEven` / Bankers' Rounding** or domain-mandated tax rounding rules) or a strongly-typed `Money` struct that checks currency equality before addition/subtraction.
2. **Secret & PII Redaction (`secrecy::SecretBox`):** Any struct field containing passwords, API keys, JWTs, tax identification numbers (NPWP/SSN), bank account numbers, or raw phone/email PII MUST be wrapped in `SecretString` or a custom `Redacted<T>` wrapper whose `std::fmt::Debug` and `std::fmt::Display` implementations output `"[REDACTED]"`. This guarantees `#[tracing::instrument]` never leaks sensitive PII into logs or traces.
3. **Strict Serde Deserialization:** Request DTOs must include `#[serde(deny_unknown_fields)]` on security-critical endpoints and parse raw inputs into validated Domain Value Objects via `TryFrom` before reaching the workflow orchestrator.

---

### 6.2 Canonical Boilerplate: Currency-Safe `Money` & Redacted PII Value Objects

```rust
// File: crates/domains/domain-alpha-engine/src/money.rs
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Currency {
    IDR,
    USD,
    SGD,
}

impl Currency {
    /// Number of decimal places for minor units (e.g., IDR = 0 or 2 depending on ledger policy, USD = 2)
    pub const fn scale(self) -> u32 {
        match self {
            Self::IDR => 0,
            Self::USD | Self::SGD => 2,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MoneyError {
    #[error("Currency mismatch: cannot combine {left:?} and {right:?}")]
    CurrencyMismatch { left: Currency, right: Currency },
    #[error("Arithmetic overflow during monetary calculation")]
    Overflow,
    #[error("Negative amount not permitted for this domain value: {0}")]
    NegativeNotAllowed(Decimal),
}

/// Immutable, currency-safe Monetary Value Object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    amount: Decimal,
    currency: Currency,
}

impl Money {
    pub fn new(amount: Decimal, currency: Currency) -> Self {
        let rounded = amount.round_dp_with_strategy(
            currency.scale(),
            RoundingStrategy::MidpointNearestEven,
        );
        Self {
            amount: rounded,
            currency,
        }
    }

    pub fn non_negative(amount: Decimal, currency: Currency) -> Result<Self, MoneyError> {
        if amount.is_sign_negative() {
            return Err(MoneyError::NegativeNotAllowed(amount));
        }
        Ok(Self::new(amount, currency))
    }

    pub const fn amount(&self) -> Decimal {
        self.amount
    }

    pub const fn currency(&self) -> Currency {
        self.currency
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, MoneyError> {
        if self.currency != rhs.currency {
            return Err(MoneyError::CurrencyMismatch {
                left: self.currency,
                right: rhs.currency,
            });
        }
        let sum = self.amount.checked_add(rhs.amount).ok_or(MoneyError::Overflow)?;
        Ok(Self::new(sum, self.currency))
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, MoneyError> {
        if self.currency != rhs.currency {
            return Err(MoneyError::CurrencyMismatch {
                left: self.currency,
                right: rhs.currency,
            });
        }
        let diff = self.amount.checked_sub(rhs.amount).ok_or(MoneyError::Overflow)?;
        Ok(Self::new(diff, self.currency))
    }

    /// Applies a basis-points rate (e.g., 500 bps = 5.00%) using Bankers' Rounding.
    pub fn apply_bps(self, bps: u32) -> Result<Self, MoneyError> {
        let rate = Decimal::from(bps) / Decimal::from(10_000_u32);
        let product = self.amount.checked_mul(rate).ok_or(MoneyError::Overflow)?;
        Ok(Self::new(product, self.currency))
    }
}
```

```rust
// Canonical PII Redaction Wrapper (Safe for #[tracing::instrument])
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SensitivePii<T>(T);

impl<T> SensitivePii<T> {
    pub const fn new(val: T) -> Self {
        Self(val)
    }
    /// Explicit accessor required so engineers consciously unwrap PII only when necessary
    pub const fn expose_pii(&self) -> &T {
        &self.0
    }
}

impl<T> std::fmt::Debug for SensitivePii<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("\"[REDACTED_PII]\"")
    }
}
```

---

## 7. Pillar 5: Contract-First API (`utoipa`) & RFC 9457 Problem Details

### 7.1 The Architectural Rule (Domain-Agnostic Standard)
1. **RFC 9457 Problem Details Everywhere:** Every HTTP error response across all services MUST return `Content-Type: application/problem+json` conforming to **RFC 9457** (`type`, `title`, `status`, `detail`, `instance`, `trace_id`, `error_code`, and optional `validation_errors`).
2. **No Internal Error Leakage:** 500-level database or infrastructure errors (`sqlx::Error`, network timeouts) are logged internally via `tracing::error!` with full context, while the client receives a sanitized `500 Internal Server Error` containing the `trace_id` for support correlation.
3. **Code-First OpenAPI 3.1 (`utoipa`):** Every HTTP DTO (`*Request`, `*Response`) MUST derive `utoipa::ToSchema`, and every handler MUST carry `#[utoipa::path(...)]` and be mounted via `utoipa_axum::router::OpenApiRouter`. An automated test in CI verifies that the generated `openapi.json` is always valid and up to date.

---

### 7.2 Canonical Boilerplate: `crates/foundation/api-error/src/lib.rs`

```rust
// File: crates/foundation/api-error/src/lib.rs
use axum::{
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    Json,
};
use op_context::AuthzError;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

/// RFC 9457 Problem Details standard error envelope.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProblemDetails {
    /// URI reference identifying the problem type
    #[serde(rename = "type")]
    pub problem_type: &'static str,
    /// Short, human-readable summary of the problem type
    pub title: &'static str,
    /// HTTP status code
    pub status: u16,
    /// Human-readable explanation specific to this occurrence
    pub detail: String,
    /// Stable machine-readable application error code (e.g., "QUOTA_EXCEEDED")
    pub error_code: &'static str,
    /// Correlation ID for tracing in logs/OpenTelemetry
    pub trace_id: Uuid,
    /// Optional field-level validation errors
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<FieldViolation>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FieldViolation {
    pub field: String,
    pub reason: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Validation failed: {detail}")]
    Validation {
        detail: String,
        violations: Vec<FieldViolation>,
    },

    #[error("Authorization error: {0}")]
    Authz(#[from] AuthzError),

    #[error("Resource not found: {resource} ({id})")]
    NotFound { resource: &'static str, id: String },

    #[error("Conflict: {detail}")]
    Conflict {
        error_code: &'static str,
        detail: String,
    },

    #[error("Domain rule violation [{error_code}]: {detail}")]
    UnprocessableDomainRule {
        error_code: &'static str,
        detail: String,
    },

    #[error("Internal infrastructure failure")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let trace_id = Uuid::now_v7();

        let (status, problem_type, title, error_code, detail, errors) = match self {
            Self::Validation { detail, violations } => (
                StatusCode::BAD_REQUEST,
                "https://errors.platform.internal/validation-error",
                "Request Validation Failed",
                "VALIDATION_FAILED",
                detail,
                violations,
            ),
            Self::Authz(AuthzError::Unauthenticated) => (
                StatusCode::UNAUTHORIZED,
                "https://errors.platform.internal/unauthenticated",
                "Authentication Required",
                "UNAUTHENTICATED",
                "Missing or invalid authentication credentials.".to_string(),
                vec![],
            ),
            Self::Authz(err @ AuthzError::Forbidden { .. }) => (
                StatusCode::FORBIDDEN,
                "https://errors.platform.internal/forbidden",
                "Insufficient Permissions",
                "FORBIDDEN",
                err.to_string(),
                vec![],
            ),
            Self::NotFound { resource, id } => (
                StatusCode::NOT_FOUND,
                "https://errors.platform.internal/not-found",
                "Resource Not Found",
                "RESOURCE_NOT_FOUND",
                format!("{resource} with identifier '{id}' was not found."),
                vec![],
            ),
            Self::Conflict { error_code, detail } => (
                StatusCode::CONFLICT,
                "https://errors.platform.internal/conflict",
                "Resource State Conflict",
                error_code,
                detail,
                vec![],
            ),
            Self::UnprocessableDomainRule { error_code, detail } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "https://errors.platform.internal/domain-rule-violation",
                "Domain Invariant Violated",
                error_code,
                detail,
                vec![],
            ),
            Self::Internal(err) => {
                // Log full error chain internally; never leak SQL or stack details to HTTP caller
                tracing::error!(%trace_id, error = ?err, "Unhandled internal server error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "https://errors.platform.internal/internal-error",
                    "Internal Server Error",
                    "INTERNAL_ERROR",
                    format!("An unexpected error occurred. Reference trace_id: {trace_id}"),
                    vec![],
                )
            }
        };

        let body = ProblemDetails {
            problem_type,
            title,
            status: status.as_u16(),
            detail,
            error_code,
            trace_id,
            errors,
        };

        let mut response = (status, Json(body)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}
```

---

## 8. Pillar 6: Complete Reference Implementation (End-to-End Feature Slice)

Below is the **Canonical Reference Template** showing how a Sans-I/O Engine Crate (`domain-alpha-engine`) and a 3-Hop Domain Crate (`domain-alpha`) work together. **Every engineer and AI coding agent MUST follow this exact structural pattern when implementing any PRD/TRD.**

### 8.1 Part A: The Pure Sans-I/O Engine Crate (`crates/domains/domain-alpha-engine`)

Notice: **Zero `async`, zero `sqlx`, zero `axum`**, and tested with **`proptest`**.

```rust
// File: crates/domains/domain-alpha-engine/src/lib.rs
use crate::money::{Currency, Money, MoneyError};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod money;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationItemInput {
    pub item_id: Uuid,
    pub unit_price: Money,
    pub quantity: u32,
    pub discount_bps: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationLineComputed {
    pub item_id: Uuid,
    pub gross_amount: Money,
    pub discount_amount: Money,
    pub net_amount: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationComputation {
    pub lines: Vec<AllocationLineComputed>,
    pub total_gross: Money,
    pub total_discount: Money,
    pub total_tax: Money,
    pub final_payable: Money,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EngineError {
    #[error("Cannot compute allocation for an empty item list")]
    EmptyItems,
    #[error("Item {item_id} has invalid zero quantity")]
    ZeroQuantity { item_id: Uuid },
    #[error("Discount basis points {bps} exceeds maximum allowed 10,000 bps (100%)")]
    InvalidDiscountBps { bps: u32 },
    #[error("Monetary calculation error: {0}")]
    Money(#[from] MoneyError),
}

/// PURE SANS-I/O ENGINE FUNCTION:
/// Deterministic, synchronous, zero I/O, zero database dependencies.
pub fn compute_allocation(
    currency: Currency,
    items: &[AllocationItemInput],
    tax_rate_bps: u32,
) -> Result<AllocationComputation, EngineError> {
    if items.is_empty() {
        return Err(EngineError::EmptyItems);
    }

    let zero = Money::new(Decimal::ZERO, currency);
    let mut total_gross = zero;
    let mut total_discount = zero;
    let mut lines = Vec::with_capacity(items.len());

    for item in items {
        if item.quantity == 0 {
            return Err(EngineError::ZeroQuantity {
                item_id: item.item_id,
            });
        }
        if item.discount_bps > 10_000 {
            return Err(EngineError::InvalidDiscountBps {
                bps: item.discount_bps,
            });
        }

        let gross_dec = item
            .unit_price
            .amount()
            .checked_mul(Decimal::from(item.quantity))
            .ok_or(MoneyError::Overflow)?;
        let gross = Money::new(gross_dec, item.unit_price.currency());
        let discount = gross.apply_bps(item.discount_bps)?;
        let net = gross.checked_sub(discount)?;

        total_gross = total_gross.checked_add(gross)?;
        total_discount = total_discount.checked_add(discount)?;

        lines.push(AllocationLineComputed {
            item_id: item.item_id,
            gross_amount: gross,
            discount_amount: discount,
            net_amount: net,
        });
    }

    let net_subtotal = total_gross.checked_sub(total_discount)?;
    let total_tax = net_subtotal.apply_bps(tax_rate_bps)?;
    let final_payable = net_subtotal.checked_add(total_tax)?;

    Ok(AllocationComputation {
        lines,
        total_gross,
        total_discount,
        total_tax,
        final_payable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // Property-Based Test: Generates thousands of randomized inputs to prove mathematical invariants
    proptest! {
        #[test]
        fn prop_allocation_invariants_hold(
            unit_amount in 1_i64..100_000_000_i64,
            qty in 1_u32..1_000_u32,
            discount_bps in 0_u32..=10_000_u32,
            tax_bps in 0_u32..=2_500_u32,
        ) {
            let item = AllocationItemInput {
                item_id: Uuid::now_v7(),
                unit_price: Money::new(Decimal::from(unit_amount), Currency::IDR),
                quantity: qty,
                discount_bps,
            };

            let result = compute_allocation(Currency::IDR, &[item], tax_bps).expect("valid inputs must succeed");

            // Invariant 1: Discount can never exceed gross
            prop_assert!(result.total_discount.amount() <= result.total_gross.amount());
            // Invariant 2: Final payable is never negative
            prop_assert!(!result.final_payable.amount().is_sign_negative());
            // Invariant 3: Exact conservation of money: Gross - Discount + Tax == Final Payable
            let expected_final = result.total_gross
                .checked_sub(result.total_discount).unwrap()
                .checked_add(result.total_tax).unwrap();
            prop_assert_eq!(result.final_payable, expected_final);
        }
    }
}
```

---

### 8.2 Part B: The 3-Hop Feature Slice (`crates/domains/domain-alpha`)

#### Hop 1: HTTP Boundary (`src/commit_allocation/http.rs`)
```rust
// File: crates/domains/domain-alpha/src/commit_allocation/http.rs
use crate::commit_allocation::{CommitAllocationCmd, commit_allocation};
use api_error::{ApiError, FieldViolation};
use axum::{Json, extract::State, http::StatusCode};
use datastore_core::DataStore;
use domain_alpha_engine::{
    AllocationItemInput,
    money::{Currency, Money},
};
use op_context::OpCtx;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CommitAllocationRequest {
    pub currency: Currency,
    pub tax_rate_bps: u32,
    pub items: Vec<AllocationItemDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AllocationItemDto {
    pub item_id: Uuid,
    pub unit_price: Decimal,
    pub quantity: u32,
    pub discount_bps: u32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CommitAllocationResponse {
    pub allocation_id: Uuid,
    pub total_gross: Decimal,
    pub total_discount: Decimal,
    pub total_tax: Decimal,
    pub final_payable: Decimal,
}

/// Parse, Don't Validate: Convert HTTP DTO -> Domain Command via TryFrom
impl TryFrom<CommitAllocationRequest> for CommitAllocationCmd {
    type Error = ApiError;

    fn try_from(req: CommitAllocationRequest) -> Result<Self, Self::Error> {
        if req.items.is_empty() {
            return Err(ApiError::Validation {
                detail: "At least one allocation item is required.".to_string(),
                violations: vec![FieldViolation {
                    field: "items".to_string(),
                    reason: "must not be empty".to_string(),
                }],
            });
        }

        let mut domain_items = Vec::with_capacity(req.items.len());
        for (idx, item) in req.items.into_iter().enumerate() {
            let price = Money::non_negative(item.unit_price, req.currency).map_err(|e| {
                ApiError::Validation {
                    detail: format!("Invalid unit_price at index {idx}: {e}"),
                    violations: vec![FieldViolation {
                        field: format!("items[{idx}].unit_price"),
                        reason: e.to_string(),
                    }],
                }
            })?;

            domain_items.push(AllocationItemInput {
                item_id: item.item_id,
                unit_price: price,
                quantity: item.quantity,
                discount_bps: item.discount_bps,
            });
        }

        Ok(Self {
            currency: req.currency,
            tax_rate_bps: req.tax_rate_bps,
            items: domain_items,
        })
    }
}

/// HOP 1: Thin HTTP Handler. Extracts context, parses DTO, calls Hop 2, returns Response.
#[utoipa::path(
    post,
    path = "/v1/allocations",
    request_body = CommitAllocationRequest,
    responses(
        (status = 201, description = "Allocation committed", body = CommitAllocationResponse),
        (status = 400, description = "Validation error", body = api_error::ProblemDetails),
        (status = 422, description = "Domain invariant error", body = api_error::ProblemDetails)
    ),
    tag = "Allocations"
)]
pub async fn handle_commit_allocation(
    opctx: OpCtx,
    State(db): State<DataStore>,
    Json(req): Json<CommitAllocationRequest>,
) -> Result<(StatusCode, Json<CommitAllocationResponse>), ApiError> {
    let cmd = CommitAllocationCmd::try_from(req)?;
    let record = commit_allocation(&opctx, &db, cmd).await?;

    Ok((
        StatusCode::CREATED,
        Json(CommitAllocationResponse {
            allocation_id: record.allocation_id,
            total_gross: record.computation.total_gross.amount(),
            total_discount: record.computation.total_discount.amount(),
            total_tax: record.computation.total_tax.amount(),
            final_payable: record.computation.final_payable.amount(),
        }),
    ))
}
```

#### Hop 2: Workflow Orchestrator (`src/commit_allocation.rs`)
```rust
// File: crates/domains/domain-alpha/src/commit_allocation.rs
pub mod http;
mod db; // <-- Strictly private! Cannot be called outside `commit_allocation`

use api_error::ApiError;
use datastore_core::DataStore;
use domain_alpha_engine::{
    AllocationComputation, AllocationItemInput, compute_allocation, money::Currency,
};
use op_context::{OpCtx, Permission};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug)]
pub struct CommitAllocationCmd {
    pub currency: Currency,
    pub tax_rate_bps: u32,
    pub items: Vec<AllocationItemInput>,
}

#[derive(Debug, Serialize)]
pub struct AllocationCommittedEvent {
    pub allocation_id: Uuid,
    pub final_payable: rust_decimal::Decimal,
    pub currency: Currency,
}

pub struct CommittedAllocationRecord {
    pub allocation_id: Uuid,
    pub computation: AllocationComputation,
}

/// HOP 2: Workflow Orchestrator.
/// Coordinates AuthZ -> TenantTx -> DB Load -> Pure Sans-I/O Engine -> DB Save + Outbox -> Commit.
#[tracing::instrument(
    name = "domain_alpha.commit_allocation",
    skip(db, cmd),
    fields(
        tenant_id = %opctx.tenant_id(),
        actor_id = %opctx.actor_id(),
        correlation_id = %opctx.correlation_id(),
        item_count = cmd.items.len()
    )
)]
pub async fn commit_allocation(
    opctx: &OpCtx,
    db: &DataStore,
    cmd: CommitAllocationCmd,
) -> Result<CommittedAllocationRecord, ApiError> {
    // 1. Enforce Capability Permission
    opctx.require_permission(Permission::ResourceWrite)?;

    // 2. Execute Pure Synchronous Domain Engine (Zero DB locks held during CPU math!)
    let computation = compute_allocation(cmd.currency, &cmd.items, cmd.tax_rate_bps).map_err(
        |engine_err| ApiError::UnprocessableDomainRule {
            error_code: "ALLOCATION_CALCULATION_FAILED",
            detail: engine_err.to_string(),
        },
    )?;

    // 3. Open Tenant-Scoped Transaction (PostgreSQL RLS automatically bound)
    let mut tx = db
        .begin_tenant_tx(opctx)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    // 4. Persist Domain Aggregate via Hop 3 (`db.rs`)
    let allocation_id = Uuid::now_v7();
    db::insert_allocation(&mut tx, allocation_id, cmd.currency, &computation)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    // 5. Emit Domain Event into Transactional Outbox (Same ACID Transaction)
    let event = AllocationCommittedEvent {
        allocation_id,
        final_payable: computation.final_payable.amount(),
        currency: cmd.currency,
    };
    tx.emit_outbox_event(
        "domain_alpha.allocation.committed.v1",
        "Allocation",
        allocation_id,
        &event,
    )
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    // 6. Commit Transaction
    tx.commit().await.map_err(|e| ApiError::Internal(e.into()))?;

    tracing::info!(%allocation_id, "Committed allocation and queued outbox event");

    Ok(CommittedAllocationRecord {
        allocation_id,
        computation,
    })
}
```

#### Hop 3: Private Database Queries (`src/commit_allocation/db.rs`)
```rust
// File: crates/domains/domain-alpha/src/commit_allocation/db.rs
use datastore_core::{DataStoreError, TenantTx};
use domain_alpha_engine::{AllocationComputation, money::Currency};
use uuid::Uuid;

/// HOP 3: Pure Data Access. Zero business branching.
pub(super) async fn insert_allocation(
    tx: &mut TenantTx<'_>,
    allocation_id: Uuid,
    currency: Currency,
    computation: &AllocationComputation,
) -> Result<(), DataStoreError> {
    let currency_code = match currency {
        Currency::IDR => "IDR",
        Currency::USD => "USD",
        Currency::SGD => "SGD",
    };

    sqlx::query(
        r#"
        INSERT INTO allocations (
            id, tenant_id, currency, total_gross, total_discount, total_tax, final_payable, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        "#,
    )
    .bind(allocation_id)
    .bind(tx.tenant_id().as_uuid())
    .bind(currency_code)
    .bind(computation.total_gross.amount())
    .bind(computation.total_discount.amount())
    .bind(computation.total_tax.amount())
    .bind(computation.final_payable.amount())
    .execute(tx.conn())
    .await?;

    Ok(())
}
```

---

## 9. Pillar 7: Observability, Graceful Lifecycle & Verification Matrix

### 9.1 The Architectural Rule (Domain-Agnostic Standard)
1. **Structured Telemetry:** Every orchestrator function MUST be annotated with `#[tracing::instrument]` recording `tenant_id`, `actor_id`, and `correlation_id`. `println!` and `dbg!` are blocked at compile time by Clippy.
2. **Graceful Drain on `SIGTERM`/`SIGINT`:** Kubernetes sends `SIGTERM` before terminating pods. Every binary in `apps/*` MUST install a `tokio::signal` shutdown coordinator that:
   * Immediately marks `/health/ready` as `503 Service Unavailable` so load balancers stop routing new traffic.
   * Allows in-flight HTTP requests and active background job chunks up to `30 seconds` to complete and commit their `TenantTx` before closing the `PgPool`.
3. **The 3-Tier Testing Mandate:**
   * **Tier 1 (Unit & Property Tests in `*-engine`):** Every Sans-I/O function must have deterministic unit tests AND at least one `proptest!` property suite verifying invariants across randomized inputs. Runs in $<50\text{ ms}$ with zero external dependencies.
   * **Tier 2 (Isolated DB Integration Tests via `#[sqlx::test]`):** Every orchestrator (`Hop 2`) and `db.rs` (`Hop 3`) must have integration tests using `#[sqlx::test]`, including an explicit **Cross-Tenant Isolation Test** verifying that Tenant B cannot read or mutate Tenant A's data.
   * **Tier 3 (Contract Verification):** Snapshot test verifying the OpenAPI 3.1 spec generated by `utoipa`.

---

### 9.2 Canonical Boilerplate: Multi-Tenant Isolation Test (`#[sqlx::test]`)

Every feature slice MUST include this cross-tenant verification test pattern:

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;
    use op_context::{ActorId, OpCtx, Permission, TenantId};
    use sqlx::PgPool;
    use uuid::Uuid;

    #[sqlx::test(migrations = "../../migrations")]
    async fn test_strict_rls_tenant_isolation(pool: PgPool) {
        let db = DataStore::new(pool);

        let tenant_a = TenantId::from_uuid(Uuid::now_v7());
        let tenant_b = TenantId::from_uuid(Uuid::now_v7());

        let ctx_a = OpCtx::new_authenticated(
            tenant_a,
            ActorId::from_uuid(Uuid::now_v7()),
            [Permission::ResourceWrite, Permission::ResourceRead],
            Uuid::now_v7(),
        );
        let ctx_b = OpCtx::new_authenticated(
            tenant_b,
            ActorId::from_uuid(Uuid::now_v7()),
            [Permission::ResourceWrite, Permission::ResourceRead],
            Uuid::now_v7(),
        );

        // 1. Tenant A commits an allocation
        let cmd = CommitAllocationCmd {
            currency: Currency::IDR,
            tax_rate_bps: 1100,
            items: vec![AllocationItemInput {
                item_id: Uuid::now_v7(),
                unit_price: Money::new(rust_decimal::Decimal::from(100_000), Currency::IDR),
                quantity: 2,
                discount_bps: 0,
            }],
        };
        let created = commit_allocation(&ctx_a, &db, cmd).await.expect("Tenant A commit succeeds");

        // 2. Tenant B attempts to query Tenant A's record inside Tenant B's RLS transaction
        let mut tx_b = db.begin_tenant_tx(&ctx_b).await.expect("Tenant B tx opens");
        let leaked_row: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM allocations WHERE id = $1")
            .bind(created.allocation_id)
            .fetch_optional(tx_b.conn())
            .await
            .expect("Query executes cleanly");

        // PostgreSQL Row-Level Security MUST filter out Tenant A's row even without `WHERE tenant_id = ...`
        assert!(leaked_row.is_none(), "CRITICAL SECURITY FAILURE: Cross-tenant row leaked!");
    }
}
```

---

## 10. Standard Operating Procedure (SOP): The TRD-to-Code Execution Playbook

> **Instructions for Engineers & AI Coding Agents (`AGENTS.md` / Skill Prompt):**  
> When given a Product Requirements Document (PRD) or Technical Requirements Document (TRD), execute the implementation in this exact 6-step sequence. **Never skip a step or combine layers.**

### Step 1: Classify the Bounded Context & Workload
1. Identify which `crates/domains/<context>` owns the feature.
2. Determine if the feature involves complex state transitions, pricing/tax math, or rule evaluation:
   * **Yes:** Target `crates/domains/<context>-engine` first.
   * **No (Pure CRUD):** Target `crates/domains/<context>` directly.
3. Determine data volume:
   * **Single aggregate or $<1,000$ rows:** Standard `TenantTx` query.
   * **$>1,000$ rows / batch:** Mandatory `sqlx::query_as(...).fetch()` cursor stream + Outbox/NATS worker.

### Step 2: Database Migration & RLS Verification
1. Create migration in `migrations/` using **UUIDv7** for primary keys and `BIGINT` / `NUMERIC` for money.
2. Add `tenant_id UUID NOT NULL DEFAULT (current_setting('app.current_tenant'))::uuid`.
3. Add composite indexes starting with `(tenant_id, ...)`.
4. Add `ENABLE ROW LEVEL SECURITY`, `FORCE ROW LEVEL SECURITY`, and the `tenant_isolation_policy`.

### Step 3: Implement Pure Sans-I/O Engine (`crates/domains/<context>-engine`)
1. Define Value Objects, Domain Input/Output structs, and `EngineError` (`thiserror`).
2. Write the pure synchronous function. Zero `async`, zero `f64`, zero `.unwrap()`.
3. Write deterministic unit tests AND a `proptest!` property test proving domain invariants.
4. Run `cargo test -p <context>-engine` and ensure it compiles and passes in $<1\text{ second}$.

### Step 4: Implement the 3-Hop Feature Slice (`crates/domains/<context>/src/<verb>_<noun>`)
1. **Hop 3 (`<verb>_<noun>/db.rs`):** Keep module private (`mod db;`). Write `sqlx` functions accepting `&mut TenantTx<'_>`. Use `ORDER BY id ASC FOR UPDATE` if locking multiple rows.
2. **Hop 2 (`<verb>_<noun>.rs`):** Add `#[tracing::instrument]`. Check `opctx.require_permission(...)`. Call Sans-I/O engine, open `TenantTx`, call `db::*`, emit `tx.emit_outbox_event(...)`, and call `tx.commit().await?`.
3. **Hop 1 (`<verb>_<noun>/http.rs`):** Define `*Request` (`#[serde(deny_unknown_fields)]`) and `*Response` with `utoipa::ToSchema`. Implement `TryFrom<*Request> for *Cmd`. Write thin Axum handler and register route in `src/lib.rs`.

### Step 5: Write Integration & Multi-Tenant Isolation Tests
1. Add `#[sqlx::test]` in `<verb>_<noun>.rs` testing the happy path, domain error path, outbox event persistence, and **Cross-Tenant RLS Isolation** (Tenant B cannot read/modify Tenant A's resource).

### Step 6: Pre-PR Compiler & Quality Gate Checklist
Before marking any task complete, every item below MUST pass with zero warnings:
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes (`unwrap_used`, `expect_used`, `float_arithmetic`, `unsafe_code` all zero).
- [ ] `cargo nextest run --workspace` passes all unit, `proptest`, and `#[sqlx::test]` suites.
- [ ] `cargo sqlx prepare --workspace -- --all-targets` succeeds so offline CI builds remain reproducible.
- [ ] No raw `&PgPool` is passed into any domain function (only `&DataStore` or `&mut TenantTx`).
- [ ] All PII fields use `SensitivePii<T>` or `secrecy::SecretBox`.