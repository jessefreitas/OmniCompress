use crate::ccr::CCRStore;
use crate::pipeline::CompressionPipeline;
use crate::protection::CompressConfig;
use crate::types::Block;

/// Summary statistics produced by a single evaluation run.
pub struct EvalReport {
    /// Total tokens across all input blocks before compression.
    pub tokens_before: usize,
    /// Total tokens across all output blocks after compression.
    pub tokens_after: usize,
    /// `tokens_after / tokens_before` (0.0 when `tokens_before == 0`).
    /// A value below 1.0 means net compression was achieved.
    pub ratio: f64,
    /// `true` when every `CcrRef` produced by the pipeline resolves back to
    /// `Some(_)` via `ccr.get()` — i.e., full round-trip fidelity.
    pub roundtrip_ok: bool,
}

/// Stateless harness that runs the pipeline once and measures the outcome.
pub struct EvalHarness;

impl EvalHarness {
    /// Run one compression pass and return `EvalReport`.
    ///
    /// * `pipe`  — the `CompressionPipeline` to evaluate.
    /// * `ccr`   — the backing store (must be the same instance the pipeline writes to).
    /// * `msgs`  — input message sequence.
    /// * `cfg`   — compression configuration.
    pub fn run_one(
        pipe: &CompressionPipeline,
        ccr: &dyn CCRStore,
        msgs: Vec<Block>,
        cfg: &CompressConfig,
    ) -> EvalReport {
        let result = pipe.compress(msgs, cfg);

        let ratio = if result.tokens_before == 0 {
            0.0
        } else {
            result.tokens_after as f64 / result.tokens_before as f64
        };

        // Verify every stored original can be retrieved.
        let roundtrip_ok = result
            .ccr_refs
            .iter()
            .all(|cr| matches!(ccr.get(&cr.hash), Ok(Some(_))));

        EvalReport {
            tokens_before: result.tokens_before,
            tokens_after: result.tokens_after,
            ratio,
            roundtrip_ok,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccr::MemoryStore;
    use crate::pipeline::CompressionPipeline;
    use crate::protection::CompressConfig;
    use crate::types::{Block, Role};
    use std::sync::Arc;

    /// JSON array large enough for both min_chars (>600) and JsonCrusher (>=20 items).
    fn big_json() -> String {
        "[".to_string()
            + &(0..60)
                .map(|i| format!(r#"{{"id":{i},"name":"item-{i}","score":{i}.0}}"#))
                .collect::<Vec<_>>()
                .join(",")
            + "]"
    }

    #[test]
    fn report_has_ratio_and_roundtrip_ok() {
        let big = big_json();
        // Old JSON block (position 0) + 6 recent prose blocks to push it outside protect window
        let mut msgs = vec![Block::tool(Role::User, &big, "search")];
        for i in 0..6 {
            msgs.push(Block::from_text(Role::Assistant, &format!("ok {i}")));
        }

        let store = Arc::new(MemoryStore::default());
        let pipe = CompressionPipeline::new_arc(store.clone());
        let rep = EvalHarness::run_one(&pipe, &*store, msgs, &CompressConfig::default());

        // The JSON block should have been compressed → ratio < 1.0
        assert!(
            rep.ratio > 0.0 && rep.ratio < 1.0,
            "expected 0 < ratio < 1, got {}",
            rep.ratio
        );
        assert!(
            rep.roundtrip_ok,
            "every CCR original must resolve via ccr.get()"
        );
    }

    #[test]
    fn ratio_is_one_when_nothing_compresses() {
        // All blocks are short (protected by min_chars) — nothing stored in CCR.
        let store = Arc::new(MemoryStore::default());
        let pipe = CompressionPipeline::new_arc(store.clone());
        let msgs = vec![
            Block::from_text(Role::User, "hello"),
            Block::from_text(Role::Assistant, "world"),
        ];
        let rep = EvalHarness::run_one(&pipe, &*store, msgs, &CompressConfig::default());
        // Nothing compressed, so ratio == 1.0 (approximately — heuristic tokenizer may round)
        assert!(
            (rep.ratio - 1.0).abs() < 0.05,
            "expected ratio ~1.0, got {}",
            rep.ratio
        );
        assert!(rep.roundtrip_ok, "no CCR refs means vacuously roundtrip_ok");
    }

    #[test]
    fn tokens_before_and_after_match_result() {
        let store = Arc::new(MemoryStore::default());
        let pipe = CompressionPipeline::new_arc(store.clone());
        let big = big_json();
        let mut msgs = vec![Block::tool(Role::User, &big, "search")];
        for i in 0..6 {
            msgs.push(Block::from_text(Role::Assistant, &format!("ok {i}")));
        }
        let rep = EvalHarness::run_one(&pipe, &*store, msgs, &CompressConfig::default());
        assert!(rep.tokens_before > 0);
        assert!(rep.tokens_after > 0);
    }
}
