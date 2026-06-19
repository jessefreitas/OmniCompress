//! OmniCompress proxy surface.
//!
//! Wire-level request compression: parse a chat-completion request body,
//! compress its `messages` via the SP1 pipeline, and re-serialise — preserving
//! every other field. Fail-open: any parse/serialise error returns the body
//! unchanged. The HTTP server (Axum) wraps these pure functions.

pub mod server;
pub mod shaper;
pub mod wire;
