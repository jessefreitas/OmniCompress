// Stub — real implementation comes in Task 7.
use super::{Compressor, Outcome};

#[derive(Default)]
pub struct LogTextCompressor;

impl Compressor for LogTextCompressor {
    fn compress(&self, content: &str) -> Outcome {
        Outcome::untouched(content)
    }
    fn name(&self) -> &'static str {
        "log_text"
    }
}
