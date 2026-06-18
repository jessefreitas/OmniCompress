// Stub — real implementation comes in Task 5.
use super::{Compressor, Outcome};

#[derive(Default)]
pub struct JsonCrusher;

impl Compressor for JsonCrusher {
    fn compress(&self, content: &str) -> Outcome {
        Outcome::untouched(content)
    }
    fn name(&self) -> &'static str {
        "json_crusher"
    }
}
