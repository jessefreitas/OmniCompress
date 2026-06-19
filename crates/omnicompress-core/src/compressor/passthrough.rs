use super::{Compressor, Outcome};

pub struct PassThrough;

impl Compressor for PassThrough {
    fn compress(&self, content: &str) -> Outcome {
        Outcome::untouched(content)
    }

    fn name(&self) -> &'static str {
        "passthrough"
    }
}
