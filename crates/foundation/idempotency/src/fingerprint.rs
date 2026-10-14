use serde::Serialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// SHA-256 of the operation name and the canonical JSON of the parsed
/// command. Hashing the *command* (not raw bytes) makes the fingerprint
/// insensitive to whitespace and field order in the client's JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    pub fn of<T: Serialize>(operation: &str, command: &T) -> Result<Self, serde_json::Error> {
        let value = serde_json::to_value(command)?;
        let mut canonical = Vec::with_capacity(256);
        write_canonical(&value, &mut canonical)?;

        let mut hasher = Sha256::new();
        hasher.update(operation.as_bytes());
        hasher.update([0_u8]);
        hasher.update(&canonical);
        Ok(Self(hasher.finalize().into()))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// JSON with object keys sorted recursively, independent of `serde_json`'s
/// `preserve_order` feature and of `HashMap` iteration order.
fn write_canonical(value: &Value, out: &mut Vec<u8>) -> Result<(), serde_json::Error> {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_unstable_by_key(|(key, _)| *key);
            out.push(b'{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key)?;
                out.push(b':');
                write_canonical(item, out)?;
            }
            out.push(b'}');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_canonical(item, out)?;
            }
            out.push(b']');
        }
        scalar => serde_json::to_writer(&mut *out, scalar)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use super::*;

    #[test]
    fn key_order_does_not_change_the_fingerprint() {
        let forward: HashMap<&str, i32> = [("a", 1), ("b", 2), ("c", 3)].into_iter().collect();
        let sorted: BTreeMap<&str, i32> = forward.iter().map(|(k, v)| (*k, *v)).collect();

        assert_eq!(
            Fingerprint::of("op", &forward).unwrap(),
            Fingerprint::of("op", &sorted).unwrap()
        );
    }

    #[test]
    fn operation_and_payload_both_matter() {
        let a = Fingerprint::of("billing.issue_invoice", &serde_json::json!({"n": 1})).unwrap();
        let b = Fingerprint::of("billing.void_invoice", &serde_json::json!({"n": 1})).unwrap();
        let c = Fingerprint::of("billing.issue_invoice", &serde_json::json!({"n": 2})).unwrap();

        assert_ne!(a, b);
        assert_ne!(a, c);
    }
}
