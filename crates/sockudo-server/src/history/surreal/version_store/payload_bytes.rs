use serde::{Deserialize, Serialize};
use std::ops::Deref;
use surrealdb::types::{Bytes, Kind, SurrealValue, Value};

/// Full rows retain the numeric-array representation understood by old nodes.
/// Format-2 compact rows use native bytes to avoid one database number per byte.
/// Keep the original variant when reading: materialization compares that exact
/// value before replacing it with a legacy full row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum StoredPayloadBytes {
    Array(Vec<u8>),
    Native(Bytes),
}

impl Default for StoredPayloadBytes {
    fn default() -> Self {
        Self::Array(Vec::new())
    }
}

impl From<Vec<u8>> for StoredPayloadBytes {
    fn from(bytes: Vec<u8>) -> Self {
        if bytes.starts_with(br#"{"sockudo_append_storage":2,"#) {
            Self::Native(bytes.into())
        } else {
            Self::Array(bytes)
        }
    }
}

impl AsRef<[u8]> for StoredPayloadBytes {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Array(bytes) => bytes,
            Self::Native(bytes) => bytes.as_ref(),
        }
    }
}

impl Deref for StoredPayloadBytes {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl SurrealValue for StoredPayloadBytes {
    fn kind_of() -> Kind {
        Kind::Either(vec![Vec::<u8>::kind_of(), Bytes::kind_of()])
    }

    fn into_value(self) -> Value {
        match self {
            Self::Array(bytes) => bytes.into_value(),
            Self::Native(bytes) => bytes.into_value(),
        }
    }

    fn from_value(value: Value) -> Result<Self, surrealdb::types::Error> {
        match value {
            Value::Bytes(bytes) => Ok(Self::Native(bytes)),
            value => Vec::<u8>::from_value(value).map(Self::Array),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_and_format_one_payloads_keep_legacy_array_shape() {
        for bytes in [
            br#"{"app_id":"app"}"#.as_slice(),
            br#"{"sockudo_append_storage":1,"#.as_slice(),
        ] {
            let encoded = StoredPayloadBytes::from(bytes.to_vec()).into_value();
            assert!(matches!(encoded, Value::Array(_)));
            assert_eq!(Vec::<u8>::from_value(encoded.clone()).unwrap(), bytes);
            let decoded = StoredPayloadBytes::from_value(encoded.clone()).unwrap();
            assert_eq!(decoded.as_ref(), bytes);
            assert_eq!(decoded.into_value(), encoded);
        }
    }

    #[test]
    fn format_two_payloads_use_native_bytes_and_legacy_decoder_rejects_them() {
        let bytes = br#"{"sockudo_append_storage":2,"#;
        let encoded = StoredPayloadBytes::from(bytes.to_vec()).into_value();
        assert!(matches!(encoded, Value::Bytes(_)));
        assert!(Vec::<u8>::from_value(encoded.clone()).is_err());
        let decoded = StoredPayloadBytes::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded.as_ref(), bytes);
        assert_eq!(decoded.into_value(), encoded);
        // An existing array is never silently retyped in a compare condition.
        let old_array = bytes.to_vec().into_value();
        let decoded = StoredPayloadBytes::from_value(old_array.clone()).unwrap();
        assert_eq!(decoded.into_value(), old_array);
    }
}
