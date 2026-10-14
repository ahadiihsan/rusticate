use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use op_context::{ActorId, ActorKind, CorrelationId, OpCtx, TenantId, mint};
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AuthConfig;
use crate::config::PublicKeyAlgorithm;

/// Minimum HS256 secret length (RFC 7518 §3.2: key size ≥ hash output).
const MIN_HS256_SECRET_BYTES: usize = 32;

/// Kind of principal named by `sub`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenActor {
    #[default]
    User,
    ServiceAccount,
}

/// `aud` is a string or an array of strings (RFC 7519 §4.1.3). Identity
/// providers such as Auth0 and Keycloak emit arrays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Audience {
    One(String),
    Many(Vec<String>),
}

/// Access-token claims understood by the platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Actor id.
    pub sub: Uuid,
    /// Tenant id. A token is valid for exactly one tenant.
    pub tid: Uuid,
    /// Granted permissions, already expanded from roles by the issuer.
    #[serde(default)]
    pub perms: Vec<String>,
    #[serde(default)]
    pub act: TokenActor,
    pub iss: String,
    pub aud: Audience,
    pub exp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iat: Option<u64>,
    /// Omitted (never `null`) when absent: validators reject a null `nbf`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nbf: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("auth misconfigured: {0}")]
    Misconfigured(String),
    #[error("token rejected: {0}")]
    InvalidToken(#[from] jsonwebtoken::errors::Error),
}

/// Verifies access tokens. Build once at startup; share behind an `Arc`.
pub struct TokenVerifier {
    key: DecodingKey,
    validation: Validation,
}

impl std::fmt::Debug for TokenVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenVerifier")
            .field("algorithms", &self.validation.algorithms)
            .finish_non_exhaustive()
    }
}

impl TokenVerifier {
    pub fn new(cfg: &AuthConfig) -> Result<Self, AuthError> {
        let (key, algorithm) = match (&cfg.public_key_pem, &cfg.hs256_secret) {
            (Some(pem), None) => match cfg.public_key_algorithm {
                PublicKeyAlgorithm::RS256 => {
                    (DecodingKey::from_rsa_pem(pem.as_bytes())?, Algorithm::RS256)
                }
                PublicKeyAlgorithm::ES256 => {
                    (DecodingKey::from_ec_pem(pem.as_bytes())?, Algorithm::ES256)
                }
            },
            (None, Some(secret)) => {
                let secret = secret.expose_secret().as_bytes();
                if secret.len() < MIN_HS256_SECRET_BYTES {
                    return Err(AuthError::Misconfigured(format!(
                        "hs256_secret must be at least {MIN_HS256_SECRET_BYTES} bytes"
                    )));
                }
                (DecodingKey::from_secret(secret), Algorithm::HS256)
            }
            _ => {
                return Err(AuthError::Misconfigured(
                    "set exactly one of public_key_pem or hs256_secret".to_owned(),
                ));
            }
        };

        // Validation::new pins the algorithm: tokens signed with any other
        // algorithm (including `none`) are rejected.
        let mut validation = Validation::new(algorithm);
        validation.set_issuer(&[&cfg.issuer]);
        validation.set_audience(&[&cfg.audience]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.validate_nbf = true;
        validation.leeway = cfg.leeway_secs;
        Ok(Self { key, validation })
    }

    /// Verifies `token` and mints the operation context it grants.
    pub fn verify(&self, token: &str, correlation_id: CorrelationId) -> Result<OpCtx, AuthError> {
        let claims = decode::<Claims>(token, &self.key, &self.validation)?.claims;
        let actor_kind = match claims.act {
            TokenActor::User => ActorKind::User,
            TokenActor::ServiceAccount => ActorKind::ServiceAccount,
        };
        Ok(mint::authenticated(
            TenantId::from_uuid(claims.tid),
            ActorId::from_uuid(claims.sub),
            actor_kind,
            correlation_id,
            claims.perms.into_iter().map(String::into_boxed_str),
        ))
    }
}

#[cfg(test)]
mod tests {
    use op_context::Permission;
    use secrecy::SecretString;

    use super::*;
    use crate::dev;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";
    const READ: Permission = Permission::new("billing.invoice.read");

    fn config(secret: &str) -> AuthConfig {
        AuthConfig {
            issuer: "https://issuer.test".to_owned(),
            audience: "api".to_owned(),
            public_key_pem: None,
            public_key_algorithm: PublicKeyAlgorithm::default(),
            hs256_secret: Some(SecretString::from(secret.to_owned())),
            leeway_secs: 0,
        }
    }

    fn token(issuer: &str, audience: &str, secret: &str, ttl_secs: i64) -> String {
        dev::issue_hs256(&dev::DevToken {
            secret,
            issuer,
            audience,
            tenant: TenantId::from_uuid(Uuid::from_u128(1)),
            actor: ActorId::from_uuid(Uuid::from_u128(2)),
            permissions: &["billing.invoice.read"],
            ttl_secs,
        })
        .unwrap()
    }

    #[test]
    fn valid_token_yields_context_with_its_permissions() {
        let verifier = TokenVerifier::new(&config(SECRET)).unwrap();

        let ctx = verifier
            .verify(
                &token("https://issuer.test", "api", SECRET, 60),
                CorrelationId::new_v7(),
            )
            .unwrap();

        assert_eq!(ctx.tenant_id(), TenantId::from_uuid(Uuid::from_u128(1)));
        assert!(ctx.has(READ));
    }

    #[test]
    fn rejects_expired_wrong_audience_wrong_issuer_and_wrong_key() {
        let verifier = TokenVerifier::new(&config(SECRET)).unwrap();
        let other = "ffffffffffffffffffffffffffffffff";

        for bad in [
            token("https://issuer.test", "api", SECRET, -120),
            token("https://issuer.test", "other-api", SECRET, 60),
            token("https://evil.test", "api", SECRET, 60),
            token("https://issuer.test", "api", other, 60),
            "not.a.jwt".to_owned(),
        ] {
            assert!(
                verifier.verify(&bad, CorrelationId::new_v7()).is_err(),
                "accepted {bad}"
            );
        }
    }

    #[test]
    fn rejects_short_secrets_and_ambiguous_key_config() {
        let mut both = config(SECRET);
        both.public_key_pem = Some("-----BEGIN PUBLIC KEY-----".to_owned());

        assert!(matches!(
            TokenVerifier::new(&config("short")),
            Err(AuthError::Misconfigured(_))
        ));
        assert!(matches!(
            TokenVerifier::new(&both),
            Err(AuthError::Misconfigured(_))
        ));
    }

    #[test]
    fn accepts_audience_arrays_that_contain_ours() {
        let verifier = TokenVerifier::new(&config(SECRET)).unwrap();
        let token_for = |audiences: &[&str]| {
            let claims = Claims {
                sub: Uuid::from_u128(2),
                tid: Uuid::from_u128(1),
                perms: Vec::new(),
                act: TokenActor::User,
                iss: "https://issuer.test".to_owned(),
                aud: Audience::Many(audiences.iter().map(|a| (*a).to_owned()).collect()),
                exp: u64::MAX / 2,
                iat: None,
                nbf: None,
            };
            jsonwebtoken::encode(
                &jsonwebtoken::Header::default(),
                &claims,
                &jsonwebtoken::EncodingKey::from_secret(SECRET.as_bytes()),
            )
            .unwrap()
        };

        assert!(
            verifier
                .verify(&token_for(&["billing-ui", "api"]), CorrelationId::new_v7())
                .is_ok()
        );
        assert!(
            verifier
                .verify(&token_for(&["billing-ui"]), CorrelationId::new_v7())
                .is_err()
        );
    }
}
