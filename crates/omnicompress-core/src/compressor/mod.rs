pub mod passthrough;
pub mod json_crusher;
pub mod log_text;
pub mod code;
pub mod prose;

pub use passthrough::PassThrough;

/// Result of compressing a single block.
/// `original` is `Some` only when the original was stored in the CCR;
/// `None` means the content was left untouched (no CCR write needed).
pub struct Outcome {
    pub compressed: String,
    pub original: Option<String>,
    pub detail: String,
}

impl Outcome {
    /// Convenience: content was not modified, no CCR entry.
    pub fn untouched(s: &str) -> Self {
        Outcome {
            compressed: s.to_string(),
            original: None,
            detail: "untouched".into(),
        }
    }
}

/// A content compressor. Implementations must be fail-open:
/// when in doubt or on error, return `Outcome::untouched`.
pub trait Compressor: Send + Sync {
    fn compress(&self, content: &str) -> Outcome;
    fn name(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_returns_input_unchanged_and_no_ccr() {
        let out = PassThrough.compress("hello world");
        assert_eq!(out.compressed, "hello world");
        assert!(out.original.is_none());
    }
}
