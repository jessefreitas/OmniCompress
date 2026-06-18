// Stub — real implementation (tree-sitter AST) comes in Task 9.
use super::{Compressor, Outcome};

#[derive(Default)]
pub struct CodeCompressor;

impl Compressor for CodeCompressor {
    fn compress(&self, content: &str) -> Outcome {
        Outcome::untouched(content)
    }
    fn name(&self) -> &'static str {
        "code"
    }
}
