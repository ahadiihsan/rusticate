use secrecy::SecretString;
use serde::Deserialize;

/// Signature algorithm for asymmetric (production) keys.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
pub enum PublicKeyAlgorithm {
    #[default]
    RS256,
    ES256,
}

/// Token verification settings.
///
/// Exactly one key source must be set: `public_key_pem` in every deployed
/// environment, `hs256_secret` only for local development and tests.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    /// Expected `iss` claim.
    pub issuer: String,
    /// Expected `aud` claim.
    pub audience: String,
    #[serde(default)]
    pub public_key_pem: Option<String>,
    #[serde(default)]
    pub public_key_algorithm: PublicKeyAlgorithm,
    #[serde(default)]
    pub hs256_secret: Option<SecretString>,
    /// Clock-skew tolerance for `exp` / `nbf`.
    #[serde(default = "default_leeway_secs")]
    pub leeway_secs: u64,
}

const fn default_leeway_secs() -> u64 {
    30
}
