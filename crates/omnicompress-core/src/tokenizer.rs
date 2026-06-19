/// Abstraction over token-counting strategies.
pub trait Tokenizer: Send + Sync {
    fn count(&self, text: &str) -> usize;

    /// Honesty label for the metric: "exact" | "~estimated"
    fn fidelity(&self) -> &'static str {
        "~estimated"
    }
}

/// Calibrated heuristic: ~4 chars/token (rounds up).
///
/// When an exact tokenizer (e.g. tiktoken/HF) is available in a future SP,
/// create `ExactTokenizer` with `fidelity() == "exact"`. SP1 uses this heuristic.
#[derive(Default)]
pub struct HeuristicTokenizer;

impl Tokenizer for HeuristicTokenizer {
    fn count(&self, text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }
        // ~4 chars/token — rounds up
        text.chars().count().div_ceil(4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_roughly_chars_over_four() {
        let t = HeuristicTokenizer::default();
        // ~4 chars/token is the heuristic; 400 chars ~= 100 tokens
        let n = t.count(&"x".repeat(400));
        assert!((90..=110).contains(&n), "got {n}");
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(HeuristicTokenizer::default().count(""), 0);
    }
}
