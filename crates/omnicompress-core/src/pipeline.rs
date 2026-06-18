use std::sync::Arc;
use std::panic::AssertUnwindSafe;

use crate::ccr::CCRStore;
use crate::compressor::code::CodeCompressor;
use crate::compressor::json_crusher::JsonCrusher;
use crate::compressor::log_text::LogTextCompressor;
use crate::compressor::passthrough::PassThrough;
use crate::compressor::{Compressor, Outcome};
use crate::protection::{CompressConfig, ProtectionPolicy};
use crate::router::ContentRouter;
use crate::tokenizer::{HeuristicTokenizer, Tokenizer};
use crate::types::{Block, CcrRef, CompressResult, ContentKind, Transform};

pub struct CompressionPipeline {
    router: ContentRouter,
    tok: HeuristicTokenizer,
    ccr: Arc<dyn CCRStore>,
}

impl CompressionPipeline {
    /// Primary constructor: share ownership of the CCR store via `Arc`.
    pub fn new_arc(ccr: Arc<dyn CCRStore>) -> Self {
        Self {
            router: ContentRouter,
            tok: HeuristicTokenizer,
            ccr,
        }
    }

    /// Convenience wrapper that wraps an owned `Arc<dyn CCRStore>`.
    /// Identical to `new_arc`.
    pub fn new(ccr: Arc<dyn CCRStore>) -> Self {
        Self::new_arc(ccr)
    }

    /// Select the appropriate compressor for a classified content kind.
    fn pick(&self, kind: ContentKind) -> Box<dyn Compressor> {
        match kind {
            ContentKind::Json => Box::new(JsonCrusher),
            ContentKind::Code => Box::new(CodeCompressor),
            ContentKind::Log | ContentKind::Prose | ContentKind::Diff => {
                Box::new(LogTextCompressor)
            }
            ContentKind::Unknown => Box::new(PassThrough),
        }
    }

    /// Compress a sequence of blocks according to `cfg`.
    ///
    /// Per-block behaviour:
    /// - Protected blocks pass through unchanged.
    /// - For other blocks: classify → pick compressor → run (fail-open via catch_unwind).
    /// - If the compressor produced an `original` payload: store in CCR (fail-open on error),
    ///   replace block content with compressed+marker, record `CcrRef` + `Transform`.
    /// - If CCR write fails: keep original content (no data loss).
    pub fn compress(&self, messages: Vec<Block>, cfg: &CompressConfig) -> CompressResult {
        self.compress_with(messages, cfg, |kind| self.pick(kind))
    }

    /// Internal workhorse that accepts an injectable `picker` closure.
    ///
    /// `compress` delegates here using the real `pick` implementation.
    /// Tests may call this directly with a custom `picker` to inject
    /// compressors that panic, fail, or behave in controlled ways.
    pub(crate) fn compress_with(
        &self,
        messages: Vec<Block>,
        cfg: &CompressConfig,
        picker: impl Fn(ContentKind) -> Box<dyn Compressor>,
    ) -> CompressResult {
        let total = messages.len();
        let policy = ProtectionPolicy::new(cfg);

        let mut tokens_before: usize = 0;
        let mut tokens_after: usize = 0;
        let mut transforms: Vec<Transform> = Vec::new();
        let mut ccr_refs: Vec<CcrRef> = Vec::new();
        let mut out_msgs: Vec<Block> = Vec::with_capacity(total);

        for (idx, block) in messages.into_iter().enumerate() {
            tokens_before += self.tok.count(&block.content);

            if policy.is_protected(idx, total, &block) {
                tokens_after += self.tok.count(&block.content);
                out_msgs.push(block);
                continue;
            }

            let kind = self.router.route(&block.content);
            let comp = picker(kind);
            let comp_name = comp.name();

            // Fail-open: if the compressor panics, treat as untouched.
            let outcome: Outcome =
                std::panic::catch_unwind(AssertUnwindSafe(|| comp.compress(&block.content)))
                    .unwrap_or_else(|_| Outcome::untouched(&block.content));

            match outcome.original {
                Some(original) => {
                    let original_tokens = self.tok.count(&original);
                    match self.ccr.put(original.as_bytes()) {
                        Ok(hash) => {
                            // Append a retrieval hint so readers know the CCR exists.
                            let short_hash = &hash[..hash.len().min(12)];
                            let marker = format!(
                                "{}\n[omnicompress: original in CCR hash={} (~{} tok)]",
                                outcome.compressed, short_hash, original_tokens
                            );
                            let mut nb = block;
                            nb.content = marker;
                            tokens_after += self.tok.count(&nb.content);
                            transforms.push(Transform {
                                unit: comp_name.to_string(),
                                detail: outcome.detail,
                            });
                            ccr_refs.push(CcrRef {
                                hash,
                                original_tokens,
                            });
                            out_msgs.push(nb);
                        }
                        Err(_) => {
                            // CCR write failed — fail-open: keep original content.
                            tokens_after += self.tok.count(&block.content);
                            out_msgs.push(block);
                        }
                    }
                }
                None => {
                    // Compressor left content untouched or returned compressed without CCR.
                    let mut nb = block;
                    nb.content = outcome.compressed;
                    tokens_after += self.tok.count(&nb.content);
                    out_msgs.push(nb);
                }
            }
        }

        CompressResult {
            messages: out_msgs,
            tokens_before,
            tokens_after,
            transforms,
            ccr_refs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccr::MemoryStore;
    use crate::protection::CompressConfig;
    use crate::types::{Block, Role};

    /// Build a JSON array large enough that JsonCrusher will compress it.
    fn big_json() -> String {
        "[".to_string()
            + &(0..60)
                .map(|i| format!(r#"{{"id":{i},"v":"{i}"}}"#))
                .collect::<Vec<_>>()
                .join(",")
            + "]"
    }

    #[test]
    fn compresses_old_tool_output_protects_recent() {
        let store = Arc::new(MemoryStore::default());
        let pipe = CompressionPipeline::new_arc(store.clone());

        // One old JSON block + 6 recent prose blocks
        let mut msgs = vec![Block::tool(Role::User, &big_json(), "search")];
        for i in 0..6 {
            msgs.push(Block::from_text(Role::Assistant, &format!("ok {i}")));
        }

        let r = pipe.compress(msgs, &CompressConfig::default());
        assert!(
            r.tokens_after < r.tokens_before,
            "should compress: {} -> {}",
            r.tokens_before,
            r.tokens_after
        );
        assert_eq!(r.ccr_refs.len(), 1, "should store exactly 1 original in CCR");
        assert_eq!(
            r.transforms.len(),
            1,
            "should record exactly 1 Transform entry"
        );
    }

    #[test]
    fn fail_open_keeps_content_on_min_size_protection() {
        // A short block is protected by the min_chars rule and must pass through intact.
        let store = Arc::new(MemoryStore::default());
        let pipe = CompressionPipeline::new_arc(store);
        let msgs = vec![Block::from_text(Role::User, "curto")];
        let r = pipe.compress(msgs, &CompressConfig::default());
        assert_eq!(r.messages[0].content, "curto");
        assert_eq!(r.ccr_refs.len(), 0);
    }

    #[test]
    fn recent_blocks_are_never_compressed() {
        let store = Arc::new(MemoryStore::default());
        let pipe = CompressionPipeline::new_arc(store);
        // All blocks are "recent" (total=3, protect_recent=4 covers all)
        let msgs: Vec<Block> = (0..3)
            .map(|i| Block::tool(Role::User, &big_json(), &format!("tool{i}")))
            .collect();
        let r = pipe.compress(msgs, &CompressConfig::default());
        // Nothing should have been put into CCR
        assert_eq!(r.ccr_refs.len(), 0, "recent blocks must not compress");
    }

    /// A compressor that always panics — used to verify the fail-open path.
    #[cfg(test)]
    struct PanickingCompressor;

    #[cfg(test)]
    impl Compressor for PanickingCompressor {
        fn name(&self) -> &'static str { "panicking" }
        fn compress(&self, _content: &str) -> Outcome {
            panic!("intentional panic in PanickingCompressor");
        }
    }

    /// Verifies that a panicking compressor results in the block being returned
    /// unchanged (fail-open) and no CCR refs being recorded.
    #[test]
    fn panicking_compressor_is_fail_open() {
        let store = Arc::new(MemoryStore::default());
        let pipe = CompressionPipeline::new_arc(store.clone());

        // Build content large enough to escape the min_chars protection threshold
        // and old enough (position 0 with 6 trailing blocks) to exit the recent window.
        let content = "x".repeat(700);
        let mut msgs = vec![Block::from_text(Role::User, &content)];
        for i in 0..6 {
            msgs.push(Block::from_text(Role::Assistant, &format!("ok {i}")));
        }
        let original_content = msgs[0].content.clone();

        let r = pipe.compress_with(msgs, &CompressConfig::default(), |_kind| {
            Box::new(PanickingCompressor)
        });

        // The first block must be returned unchanged.
        assert_eq!(
            r.messages[0].content, original_content,
            "panicking compressor must not modify block content"
        );
        // No CCR refs must have been recorded (fail-open = no store write).
        assert_eq!(
            r.ccr_refs.len(),
            0,
            "panicking compressor must not produce CCR refs"
        );
    }

    #[test]
    fn tokens_before_and_after_are_consistent() {
        let store = Arc::new(MemoryStore::default());
        let pipe = CompressionPipeline::new_arc(store);
        let content = big_json();
        // Large block at position 0, with enough trailing blocks to push it outside recent window
        let mut msgs = vec![Block::tool(Role::User, &content, "search")];
        for i in 0..6 {
            msgs.push(Block::from_text(
                Role::Assistant,
                &format!("response {i}"),
            ));
        }
        let r = pipe.compress(msgs, &CompressConfig::default());
        // tokens_before must be positive and after must be non-zero
        assert!(r.tokens_before > 0);
        assert!(r.tokens_after > 0);
        // tokens_saved() must use saturating_sub and be non-negative
        let _ = r.tokens_saved();
    }
}
