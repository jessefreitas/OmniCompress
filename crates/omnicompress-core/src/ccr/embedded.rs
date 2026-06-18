//! Durable, on-disk CCRStore implementation backed by redb.
//!
//! Uses a single redb database file. The table stores blake3 hex hashes as
//! keys and raw byte payloads as values. All writes are committed atomically.

use std::io;
use std::path::Path;
use std::sync::Mutex;

use redb::{Database, TableDefinition};

use crate::types::Hash;

use super::{hash_of, CCRStore};

/// Table definition: key = blake3 hex string, value = raw bytes.
const BLOCKS: TableDefinition<&str, &[u8]> = TableDefinition::new("blocks");

/// A content-addressed store backed by a redb database file.
///
/// The database is opened (or created) at construction time with
/// [`EmbeddedStore::open`]. A `Mutex` serialises writes; reads use
/// redb's built-in MVCC and are fully concurrent in production use,
/// but are serialised here through the same mutex for simplicity.
///
/// # Durability
///
/// Every `put` call opens a write transaction and commits it before
/// returning, so data survives process crashes.
pub struct EmbeddedStore {
    db: Mutex<Database>,
}

impl EmbeddedStore {
    /// Open (or create) a redb database at `path`.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let db = Database::create(path.as_ref())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        Ok(EmbeddedStore {
            db: Mutex::new(db),
        })
    }
}

impl CCRStore for EmbeddedStore {
    fn put(&self, bytes: &[u8]) -> io::Result<Hash> {
        let hash = hash_of(bytes);
        let db = self
            .db
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "lock poisoned"))?;

        let write_txn = db
            .begin_write()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        {
            let mut table = write_txn
                .open_table(BLOCKS)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
            table
                .insert(hash.as_str(), bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        }
        write_txn
            .commit()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        Ok(hash)
    }

    fn get(&self, hash: &str) -> io::Result<Option<Vec<u8>>> {
        let db = self
            .db
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "lock poisoned"))?;

        let read_txn = db
            .begin_read()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        // The table may not exist yet (freshly created db with no puts).
        let table = match read_txn.open_table(BLOCKS) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(io::Error::new(io::ErrorKind::Other, e.to_string())),
        };

        let result = table
            .get(hash)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        Ok(result.map(|ag| ag.value().to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        let mut p = temp_dir();
        p.push(name);
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn embedded_put_get_roundtrip() {
        let path = tmp_path("omnicompress_embedded_test_unit.redb");
        let store = EmbeddedStore::open(&path).unwrap();
        let data = b"durable bytes";
        let hash = store.put(data).unwrap();
        let got = store.get(&hash).unwrap().unwrap();
        assert_eq!(got, data);
    }

    #[test]
    fn embedded_missing_key_returns_none() {
        let path = tmp_path("omnicompress_embedded_missing.redb");
        let store = EmbeddedStore::open(&path).unwrap();
        let result = store.get("0000000000000000000000000000000000000000000000000000000000000000").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn embedded_put_is_idempotent() {
        let path = tmp_path("omnicompress_embedded_idem.redb");
        let store = EmbeddedStore::open(&path).unwrap();
        let data = b"same bytes twice";
        let h1 = store.put(data).unwrap();
        let h2 = store.put(data).unwrap();
        assert_eq!(h1, h2);
    }
}
