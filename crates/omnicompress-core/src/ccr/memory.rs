//! In-memory CCRStore implementation.
//!
//! Suitable for unit tests and single-session pipelines. Not durable —
//! all data is lost when the store is dropped.

use std::collections::HashMap;
use std::io;
use std::sync::RwLock;

use crate::types::Hash;

use super::{hash_of, CCRStore};

/// An ephemeral, in-process content-addressed store backed by a `HashMap`.
///
/// All operations are protected by an `RwLock` so the store can be shared
/// across threads (e.g., with `Arc<MemoryStore>`).
#[derive(Default)]
pub struct MemoryStore {
    inner: RwLock<HashMap<String, Vec<u8>>>,
}

impl CCRStore for MemoryStore {
    fn put(&self, bytes: &[u8]) -> io::Result<Hash> {
        let hash = hash_of(bytes);
        let mut map = self
            .inner
            .write()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "lock poisoned"))?;
        map.entry(hash.clone()).or_insert_with(|| bytes.to_vec());
        Ok(hash)
    }

    fn get(&self, hash: &str) -> io::Result<Option<Vec<u8>>> {
        let map = self
            .inner
            .read()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "lock poisoned"))?;
        Ok(map.get(hash).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_get_roundtrip() {
        let store = MemoryStore::default();
        let data = b"hello from memory store";
        let hash = store.put(data).unwrap();
        let got = store.get(&hash).unwrap().unwrap();
        assert_eq!(got, data);
    }

    #[test]
    fn missing_key_returns_none() {
        let store = MemoryStore::default();
        let result = store.get("deadbeef").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn put_is_idempotent() {
        let store = MemoryStore::default();
        let data = b"idempotent data";
        let h1 = store.put(data).unwrap();
        let h2 = store.put(data).unwrap();
        assert_eq!(h1, h2);
    }
}
