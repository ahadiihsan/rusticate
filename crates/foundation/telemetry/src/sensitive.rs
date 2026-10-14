use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A value that must never appear in logs or traces (SEC-006): personal data
/// (emails, phone numbers, national ids, bank accounts) and secrets.
///
/// * `Debug` prints `[REDACTED]`, so `#[instrument]` fields, `?value` log
///   fields and error messages cannot leak it.
/// * No `Display`: it cannot be interpolated into a message by accident.
/// * Reading it requires the explicit, greppable [`Sensitive::expose`].
/// * Serializes transparently: it is still stored and returned where the
///   contract requires it (API responses, database columns).
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct Sensitive<T>(T);

impl<T> Sensitive<T> {
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Deliberate access to the protected value.
    pub const fn expose(&self) -> &T {
        &self.0
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Sensitive<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl<T: Serialize> Serialize for Sensitive<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Sensitive<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    #[expect(dead_code, reason = "only Debug output is exercised")]
    struct Customer {
        email: Sensitive<String>,
    }

    #[test]
    fn debug_never_reveals_the_value() {
        let customer = Customer {
            email: Sensitive::new("ana@example.com".to_owned()),
        };

        assert_eq!(format!("{customer:?}"), "Customer { email: [REDACTED] }");
    }
}
