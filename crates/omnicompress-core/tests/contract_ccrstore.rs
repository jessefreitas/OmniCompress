/// Contract test suite for `CCRStore`.
///
/// Every store implementation must pass `roundtrip`. Add one test function
/// per concrete store that calls `roundtrip` with an instance of that store.

use omnicompress_core::ccr::{hash_of, CCRStore, EmbeddedStore, MemoryStore};

// ── contract ───────────────────────────────────────────────────────────────

/// Reusable contract: any CCRStore must satisfy these three invariants.
fn roundtrip<S: CCRStore>(store: S) {
    let payload: &[u8] = b"the quick brown fox jumps over the lazy dog";

    // 1. put succeeds and returns the blake3 hex hash
    let hash = store.put(payload).expect("put must succeed");

    // 2. get with the returned hash retrieves byte-identical content
    let retrieved = store
        .get(&hash)
        .expect("get must not error")
        .expect("get must return Some for a key that was put");
    assert_eq!(retrieved, payload, "retrieved bytes must be identical to stored bytes");

    // 3. hash is deterministic — same bytes → same hash every time
    let hash2 = store.put(payload).expect("second put must succeed");
    assert_eq!(hash, hash2, "hash must be deterministic for the same content");

    // 4. hash_of is consistent with the hash returned by put
    let direct = hash_of(payload);
    assert_eq!(hash, direct, "hash_of and put must agree on the hash");

    // 5. a key that was never stored returns None (not an error)
    let missing = store
        .get("0000000000000000000000000000000000000000000000000000000000000000")
        .expect("get for unknown key must not error");
    assert!(missing.is_none(), "get for unknown key must return None");
}

// ── concrete store tests ───────────────────────────────────────────────────

#[test]
fn memory_store_satisfies_contract() {
    roundtrip(MemoryStore::default());
}

#[test]
fn embedded_store_satisfies_contract() {
    let mut path = std::env::temp_dir();
    path.push("omnicompress_contract_embedded.redb");
    // Remove any leftover from a previous run so each test starts clean.
    let _ = std::fs::remove_file(&path);
    let store = EmbeddedStore::open(&path).expect("EmbeddedStore::open must succeed");
    roundtrip(store);
}
