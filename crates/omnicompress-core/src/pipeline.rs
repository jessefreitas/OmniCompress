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
    fn pick(&self, kind: ContentKind, cfg: &CompressConfig) -> Box<dyn Compressor> {
        match kind {
            ContentKind::Json => Box::new(JsonCrusher { lossless: cfg.lossless }),
            ContentKind::Code => Box::new(CodeCompressor { lossless: cfg.lossless }),
            ContentKind::Prose => {
                Box::new(crate::compressor::prose::ProseCompressor { lossless: cfg.lossless })
            }
            ContentKind::Log | ContentKind::Diff => Box::new(LogTextCompressor),
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
        self.compress_with(messages, cfg, |kind| self.pick(kind, cfg))
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
        // Token cost of the ORIGINAL input (before any transform) so tokens_saved
        // reflects both dedup and compression honestly.
        let tokens_before: usize = messages.iter().map(|b| self.tok.count(&b.content)).sum();

        // Pre-pass: replace earlier copies of identical repeated content with a
        // marker (reversible — the content remains in the kept later copy).
        // Deterministic + position-stable, so it does not bust the prefix cache.
        let messages = crate::dedup::dedup_repeated(messages, 200);

        let total = messages.len();
        let policy = ProtectionPolicy::new(cfg);

        let mut tokens_after: usize = 0;
        let mut transforms: Vec<Transform> = Vec::new();
        let mut ccr_refs: Vec<CcrRef> = Vec::new();
        let mut out_msgs: Vec<Block> = Vec::with_capacity(total);

        for (idx, block) in messages.into_iter().enumerate() {
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
                            // Append a retrieval hint with the FULL hash so the
                            // reader can actually retrieve the original from the CCR.
                            let marker = format!(
                                "{}\n[omnicompress: original in CCR hash={} (~{} tok). retrieve to expand.]",
                                outcome.compressed, hash, original_tokens
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
                    // Untouched, or a lossless transform (compressed without a CCR write).
                    let changed = outcome.compressed != block.content;
                    let mut nb = block;
                    nb.content = outcome.compressed;
                    tokens_after += self.tok.count(&nb.content);
                    if changed {
                        // Record the transform for observability; no CcrRef since
                        // nothing was elided (the compressed form is self-contained).
                        transforms.push(Transform {
                            unit: comp_name.to_string(),
                            detail: outcome.detail,
                        });
                    }
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

        // Default is lossless: the old JSON array compresses to a columnar table
        // (all rows kept) with NO CCR write — the data stays in the visible content.
        let r = pipe.compress(msgs, &CompressConfig::default());
        assert!(
            r.tokens_after < r.tokens_before,
            "should compress: {} -> {}",
            r.tokens_before,
            r.tokens_after
        );
        assert_eq!(r.ccr_refs.len(), 0, "lossless mode stores nothing in the CCR");
        assert_eq!(
            r.transforms.len(),
            1,
            "should record exactly 1 lossless Transform entry"
        );
    }

    #[test]
    fn aggressive_mode_stores_original_in_ccr() {
        let store = Arc::new(MemoryStore::default());
        let pipe = CompressionPipeline::new_arc(store.clone());
        let mut msgs = vec![Block::tool(Role::User, &big_json(), "search")];
        for i in 0..6 {
            msgs.push(Block::from_text(Role::Assistant, &format!("ok {i}")));
        }
        let cfg = CompressConfig {
            lossless: false,
            ..CompressConfig::default()
        };
        let r = pipe.compress(msgs, &cfg);
        assert!(r.tokens_after < r.tokens_before);
        assert_eq!(r.ccr_refs.len(), 1, "aggressive mode stores the original in CCR");
        assert_eq!(r.transforms.len(), 1);
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

    #[test]
    fn old_block_renders_identically_regardless_of_tail_length() {
        // Cache-friendliness: a given old block must render byte-identically no
        // matter how many newer messages follow it — otherwise the provider's
        // prefix cache is busted as the conversation grows.
        let big = big_json();
        let render = |extra: usize| -> String {
            let store = Arc::new(MemoryStore::default());
            let pipe = CompressionPipeline::new_arc(store);
            let mut msgs = vec![Block::tool(Role::User, &big, "search")];
            for i in 0..extra {
                msgs.push(Block::from_text(Role::Assistant, &format!("turn {i}")));
            }
            pipe.compress(msgs, &CompressConfig::default()).messages[0]
                .content
                .clone()
        };
        // Block 0 sits well outside the recent window in both cases.
        assert_eq!(
            render(6),
            render(10),
            "old block must be byte-identical so the prefix cache holds"
        );
    }
}
