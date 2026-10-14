//! HTTP runtime shared by every binary (SOP OPS-001..OPS-006).
//!
//! ```ignore
//! let shutdown = server_kit::shutdown_signal();
//! let readiness = Readiness::new(vec![Arc::new(db.clone())]);
//! let app = server_kit::with_middleware(api_router.merge(readiness.router()), &cfg.http);
//! server_kit::serve(app, &cfg.http, readiness, shutdown).await?;
//! ```

mod config;
mod docs;
mod health;
mod middleware;
mod shutdown;

pub use config::HttpConfig;
pub use docs::docs_router;
pub use health::{Readiness, ReadinessCheck};
pub use middleware::{REQUEST_ID_HEADER, with_middleware};
pub use shutdown::{serve, shutdown_signal};
