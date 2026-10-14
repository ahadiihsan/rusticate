//! Shared IAM bounded context (SOP SEC-001..SEC-005).
//!
//! The *only* HTTP authentication edge: verifies a bearer token and inserts
//! an [`OpCtx`](op_context::OpCtx) into the request. Mounted by every product
//! binary, so a fix here (key rotation, claim validation) reaches all of them.
//!
//! Tokens are issued by the identity provider; this crate verifies them. Roles
//! are expanded into explicit permissions at issuance (`perms` claim).

mod config;
#[cfg(feature = "dev-tokens")]
pub mod dev;
mod middleware;
mod verifier;

pub use config::{AuthConfig, PublicKeyAlgorithm};
pub use middleware::authenticate;
pub use verifier::{Audience, AuthError, Claims, TokenVerifier};
