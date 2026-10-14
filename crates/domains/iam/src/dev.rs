//! Development token issuing (feature `dev-tokens`). Never enabled in a
//! deployable binary: production tokens come from the identity provider.

use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use op_context::{ActorId, TenantId};

use crate::AuthError;
use crate::verifier::{Audience, Claims, TokenActor};

/// Parameters for a development token.
#[derive(Debug, Clone, Copy)]
pub struct DevToken<'a> {
    pub secret: &'a str,
    pub issuer: &'a str,
    pub audience: &'a str,
    pub tenant: TenantId,
    pub actor: ActorId,
    pub permissions: &'a [&'a str],
    /// Lifetime in seconds; negative values produce already-expired tokens (tests).
    pub ttl_secs: i64,
}

/// Issues an HS256 access token.
pub fn issue_hs256(token: &DevToken<'_>) -> Result<String, AuthError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| AuthError::Misconfigured(err.to_string()))?
        .as_secs();
    let exp = now.saturating_add_signed(token.ttl_secs);
    let claims = Claims {
        sub: token.actor.as_uuid(),
        tid: token.tenant.as_uuid(),
        perms: token.permissions.iter().map(|p| (*p).to_owned()).collect(),
        act: TokenActor::User,
        iss: token.issuer.to_owned(),
        aud: Audience::One(token.audience.to_owned()),
        exp,
        iat: Some(now),
        nbf: None,
    };
    Ok(encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(token.secret.as_bytes()),
    )?)
}
