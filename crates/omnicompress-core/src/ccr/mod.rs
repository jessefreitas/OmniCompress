//! Content-addressed chunk repository (CCR).
//!
//! Provides a `CCRStore` trait and two concrete implementations:
//!  - `MemoryStore` — in-process, ephemeral, useful for unit tests and pipelines
//!    that do not need durability.
//!  - `EmbeddedStore` — on-disk, durable store backed by a single-file embedded
//!    database (redb).
//!
//! All stores are content-addressed: the storage key is derived deterministically
//! from the content bytes via `hash_of`, so identical content is deduplicated.

pub mod memory;
pub mod embedded;

pub use memory::MemoryStore;
pub use embedded::EmbeddedStore;

use crate::types::Hash;

/// Compute the canonical content-address hash for `bytes`.
///
/// The hash is a lower-case hex-encoded BLAKE3 digest (64 characters).
pub fn hash_of(bytes: &[u8]) -> Hash {
    let digest = blake3::hash(bytes);
    digest.to_hex().to_string()
}

/// A content-addressed store for raw byte payloads.
///
/// # Contract
///
/// Every implementation must satisfy all invariants verified by the
/// `roundtrip` function in `tests/contract_ccrstore.rs`:
///
/// 1. `put` is idempotent and returns `hash_of(bytes)`.
/// 2. After a successful `put(bytes)`, `get(hash_of(bytes))` returns the
///    byte-identical payload.
/// 3. `get` on a key that was never stored returns `Ok(None)`.
/// 4. Both operations are thread-safe (implementations use interior mutability).
pub trait CCRStore: Send + Sync {
    /// Store `bytes` and return the content-address hash.
    ///
    /// Calling `put` with the same bytes more than once must be idempotent and
    /// must return the same hash every time.
    fn put(&self, bytes: &[u8]) -> std::io::Result<Hash>;

    /// Retrieve the bytes stored under `hash`, or `None` if absent.
    fn get(&self, hash: &str) -> std::io::Result<Option<Vec<u8>>>;
}
