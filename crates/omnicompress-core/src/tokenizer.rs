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

/// Contador BPE exato cl100k_base (tokenizer do GPT-4), proxy fiel vs o heurístico
/// chars/4; ranks embutidos no tiktoken-rs (sem rede); construir 1× e reusar (não no
/// hot path).
pub struct ExactTokenizer {
    bpe: tiktoken_rs::CoreBPE,
}

impl ExactTokenizer {
    pub fn new() -> Self {
        let bpe = tiktoken_rs::cl100k_base()
            .expect("Falha ao inicializar o tokenizer cl100k_base do tiktoken-rs");
        Self { bpe }
    }
}

impl Default for ExactTokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Tokenizer for ExactTokenizer {
    fn count(&self, text: &str) -> usize {
        self.bpe.encode_ordinary(text).len()
    }

    fn fidelity(&self) -> &'static str {
        "exact"
    }
}

#[cfg(test)]
mod exact_tests {
    use super::*;

    #[test]
    fn count_empty_string() {
        assert_eq!(ExactTokenizer::new().count(""), 0);
    }

    #[test]
    fn count_hello_world() {
        // "hello world" = ["hello", " world"] = 2 tokens in cl100k_base.
        assert_eq!(ExactTokenizer::new().count("hello world"), 2);
    }

    #[test]
    fn check_fidelity() {
        assert_eq!(ExactTokenizer::new().fidelity(), "exact");
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
