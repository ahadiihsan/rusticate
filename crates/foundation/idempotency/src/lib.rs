//! Idempotent mutations (SOP API-005).
//!
//! Every state-changing endpoint with an external effect (money, inventory,
//! messages, batch jobs) requires an `Idempotency-Key` header. The key is
//! **claimed and completed inside the same transaction as the business
//! change**:
//!
//! * a crash or error rolls the claim back with the change, so a key is never
//!   left "in progress";
//! * a concurrent duplicate blocks on the key row until the first request
//!   commits, then replays its stored response (or gets 409 if the wait exceeds
//!   `lock_timeout`);
//! * a reused key with a different payload is rejected with 422;
//! * a replay returns the response **as it was when first completed** (IETF
//!   Idempotency-Key semantics), not the resource's current state: clients
//!   that need current state GET the resource (its `Location`).
//!
//! ```ignore
//! let fingerprint = Fingerprint::of(OPERATION, &cmd)?;
//! let mut tx = db.begin(ctx).await?;
//! let pending = match idempotency::claim(&mut tx, OPERATION, &key, &fingerprint).await? {
//!     Claim::Replay(previous) => return Ok(previous),
//!     Claim::Fresh(pending) => pending,
//! };
//! let result = /* ... the workflow ... */;
//! pending.complete(&mut tx, &result).await?;
//! tx.commit().await?;
//! ```

mod fingerprint;
mod key;
mod store;

pub use fingerprint::Fingerprint;
pub use key::{HEADER, IdempotencyKey};
pub use store::{Claim, IdempotencyError, Pending, claim};
