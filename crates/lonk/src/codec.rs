//! A bounded compact codec, private so it cannot construct an unchecked Program.
use crate::artifact::{ArtifactError, ArtifactKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::cell::Cell;

const MAGIC: &[u8; 8] = b"LONKAR01";
const HEADER: usize = 72;
const DEPTH_LIMIT: usize = 128;
thread_local! { static DEPTH: Cell<usize> = const { Cell::new(0) }; }

/// Every recursive payload edge enters this guard *before* deserialization.
/// It is thread-local solely for the lifetime of a decode, never a data cache.
pub(crate) fn nested<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            DEPTH.with(|d| d.set(d.get() - 1));
        }
    }
    DEPTH.with(|d| {
        if d.get() >= DEPTH_LIMIT {
            return Err(serde::de::Error::custom("artifact nesting limit exceeded"));
        }
        d.set(d.get() + 1);
        let _guard = Guard;
        T::deserialize(deserializer)
    })
}

struct Limited {
    bytes: Vec<u8>,
    limit: usize,
}

pub(crate) fn fingerprint<T: Serialize>(value: &T) -> Result<[u8; 32], ArtifactError> {
    struct Hash(blake3::Hasher);
    impl postcard::ser_flavors::Flavor for Hash {
        type Output = [u8; 32];
        fn try_push(&mut self, byte: u8) -> postcard::Result<()> {
            self.0.update(&[byte]);
            Ok(())
        }
        fn try_extend(&mut self, bytes: &[u8]) -> postcard::Result<()> {
            self.0.update(bytes);
            Ok(())
        }
        fn finalize(self) -> postcard::Result<Self::Output> {
            Ok(*self.0.finalize().as_bytes())
        }
    }
    postcard::serialize_with_flavor(value, Hash(blake3::Hasher::new()))
        .map_err(|e| ArtifactError(e.to_string()))
}
impl postcard::ser_flavors::Flavor for Limited {
    type Output = Vec<u8>;
    fn try_push(&mut self, byte: u8) -> postcard::Result<()> {
        self.try_extend(&[byte])
    }
    fn try_extend(&mut self, bytes: &[u8]) -> postcard::Result<()> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(postcard::Error::SerializeBufferFull);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn finalize(self) -> postcard::Result<Self::Output> {
        Ok(self.bytes)
    }
}

pub(crate) fn encode<T: Serialize>(
    key: ArtifactKey,
    value: &T,
    limit: usize,
) -> Result<Option<Vec<u8>>, ArtifactError> {
    if limit < HEADER {
        return Ok(None);
    }
    let storage = Limited {
        bytes: vec![0; HEADER],
        limit,
    };
    let mut bytes = match postcard::serialize_with_flavor(value, storage) {
        Ok(b) => b,
        Err(postcard::Error::SerializeBufferFull) => return Ok(None),
        Err(e) => return Err(ArtifactError(format!("artifact encoding: {e}"))),
    };
    let checksum = blake3::hash(&bytes[HEADER..]);
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..40].copy_from_slice(&key.0);
    bytes[40..HEADER].copy_from_slice(checksum.as_bytes());
    Ok(Some(bytes))
}

pub(crate) fn decode<T: DeserializeOwned>(
    key: ArtifactKey,
    bytes: &[u8],
    limit: usize,
) -> Result<T, ArtifactError> {
    if bytes.len() > limit || bytes.len() < HEADER || &bytes[..8] != MAGIC || bytes[8..40] != key.0
    {
        return Err(ArtifactError(
            "invalid artifact header, identity or size".into(),
        ));
    }
    if blake3::hash(&bytes[HEADER..]).as_bytes() != &bytes[40..HEADER] {
        return Err(ArtifactError("artifact checksum mismatch".into()));
    }
    let (value, rest) = postcard::take_from_bytes(&bytes[HEADER..])
        .map_err(|e| ArtifactError(format!("artifact decoding: {e}")))?;
    if !rest.is_empty() {
        return Err(ArtifactError("trailing artifact data".into()));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Type, Value};

    #[test]
    fn damage_identity_trailing_data_and_limits_are_rejected() {
        let key = ArtifactKey([1; 32]);
        let value = Value::Array(vec![Value::Int(42), Value::Str("hello".into())]);
        let bytes = encode(key, &value, 1024).unwrap().unwrap();
        assert_eq!(decode::<Value>(key, &bytes, 1024).unwrap(), value);
        assert!(decode::<Value>(ArtifactKey([2; 32]), &bytes, 1024).is_err());
        for at in 0..bytes.len() {
            let mut damaged = bytes.clone();
            damaged[at] ^= 1;
            assert!(decode::<Value>(key, &damaged, 1024).is_err());
        }
        assert!(decode::<Value>(key, &bytes, bytes.len() - 1).is_err());
        assert!(encode(key, &value, bytes.len() - 1).unwrap().is_none());
        // Even a correctly checksummed but malformed producer payload fails.
        let pair = encode(key, &(value, 1u8), 1024).unwrap().unwrap();
        assert!(decode::<Value>(key, &pair, 1024).is_err());
    }

    #[test]
    fn recursive_decode_is_bounded_and_guard_resets_after_failure() {
        let key = ArtifactKey([3; 32]);
        let mut ty = Type::Int;
        for _ in 0..140 {
            ty = Type::Array(Box::new(ty));
        }
        let bytes = encode(key, &ty, 1024).unwrap().unwrap();
        assert!(decode::<Type>(key, &bytes, 1024).is_err());
        let simple = encode(key, &Type::Int, 1024).unwrap().unwrap();
        assert_eq!(decode::<Type>(key, &simple, 1024).unwrap(), Type::Int);
    }
}
