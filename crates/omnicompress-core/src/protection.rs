use crate::types::Block;

/// Configuration for compression decisions.
#[derive(Clone)]
pub struct CompressConfig {
    /// Number of trailing blocks (most recent) to always keep unmodified.
    pub protect_recent: usize,
    /// Minimum character count a block must have before compression is attempted.
    pub min_chars_to_compress: usize,
    /// Tool names whose output must never be compressed (case-insensitive).
    pub excluded_tools: Vec<String>,
    /// Cache-stable mode: when `true`, the recent-N window rule is ignored so a
    /// block's compressed form depends ONLY on its content, never on its position.
    ///
    /// Why: providers cache by exact prefix. With recency protection, a message
    /// flips from uncompressed→compressed as the window slides, changing its bytes
    /// and busting the provider's prefix cache for the tail. In cache-stable mode
    /// every block renders identically regardless of position, so the whole prefix
    /// stays byte-stable across turns and the cache holds. Trade-off: even the most
    /// recent large output is compressed (recoverable via CCR).
    pub cache_stable: bool,
    /// Lossless mode: when `true`, compressors NEVER drop information — they only
    /// apply reversible-in-place transforms (e.g. a JSON array becomes a compact
    /// columnar table keeping every row). Nothing is sent to the CCR, so the model
    /// can answer from the compressed view alone — no retrieve loop required.
    /// When `false`, aggressive lossy compression (sampling, elision) is used and
    /// the original is stored in the CCR for on-demand retrieval. Default `true`:
    /// a consumer without a retrieve path (e.g. a passthrough proxy) must never
    /// silently lose data.
    pub lossless: bool,
}

impl Default for CompressConfig {
    fn default() -> Self {
        CompressConfig {
            protect_recent: 4,
            min_chars_to_compress: 600,
            excluded_tools: ["Read", "Edit", "Glob", "Grep", "Write"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            cache_stable: false,
            lossless: true,
        }
    }
}

/// Decides whether a block is exempt from compression.
pub struct ProtectionPolicy<'a> {
    cfg: &'a CompressConfig,
}

impl<'a> ProtectionPolicy<'a> {
    pub fn new(cfg: &'a CompressConfig) -> Self {
        Self { cfg }
    }

    /// Returns `true` when the block must NOT be compressed.
    ///
    /// A block is protected when ANY of the following holds:
    /// 1. It falls within the `protect_recent` trailing window.
    /// 2. Its `tool_name` (if set) matches any entry in `excluded_tools` (case-insensitive).
    /// 3. Its content is shorter than `min_chars_to_compress`.
    pub fn is_protected(&self, idx: usize, total: usize, block: &Block) -> bool {
        // Rule 1 — recent-N window: block is in the last `protect_recent` positions.
        // Skipped in cache-stable mode so protection is position-independent
        // (keeps the provider's prefix cache byte-stable across turns).
        if !self.cfg.cache_stable && idx + self.cfg.protect_recent >= total {
            return true;
        }

        // Rule 2 — edit-critical tool name.
        if let Some(tool) = &block.tool_name {
            if self.cfg
                .excluded_tools
                .iter()
                .any(|e| e.eq_ignore_ascii_case(tool))
            {
                return true;
            }
        }

        // Rule 3 — content too short to be worth compressing.
        if block.content.len() < self.cfg.min_chars_to_compress {
            return true;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Block, Role};

    #[test]
    fn protects_recent_n() {
        let cfg = CompressConfig::default(); // protect_recent = 4
        let p = ProtectionPolicy::new(&cfg);
        // Build content large enough to bypass the min_chars rule
        let big = "y".repeat(700);
        // index 0 of 10 is old — not protected by recency
        assert!(!p.is_protected(0, 10, &Block::tool(Role::User, &big, "Bash")));
        // index 9 of 10 is recent — protected (recency window covers 6..=9)
        assert!(p.is_protected(9, 10, &Block::tool(Role::User, &big, "Bash")));
    }

    #[test]
    fn protects_edit_critical_tools() {
        let cfg = CompressConfig::default();
        let p = ProtectionPolicy::new(&cfg);
        // "Read" is in excluded_tools — always protected regardless of position
        assert!(p.is_protected(0, 10, &Block::tool(Role::User, "x", "Read")));
    }

    #[test]
    fn protects_short_content() {
        let cfg = CompressConfig::default(); // min_chars = 600
        let p = ProtectionPolicy::new(&cfg);
        let short = Block::tool(Role::User, "tiny", "Bash");
        // position 0 of 10 (not recent), tool not excluded, but content < 600 chars
        assert!(p.is_protected(0, 10, &short));
    }

    #[test]
    fn does_not_protect_old_large_noncritical_block() {
        let cfg = CompressConfig::default();
        let p = ProtectionPolicy::new(&cfg);
        let big_content = "x".repeat(700);
        let block = Block::tool(Role::User, &big_content, "Bash");
        // old position, not excluded tool, large content — should compress
        assert!(!p.is_protected(0, 10, &block));
    }

    #[test]
    fn case_insensitive_tool_match() {
        let cfg = CompressConfig::default();
        let p = ProtectionPolicy::new(&cfg);
        // "edit" lowercase should still match "Edit"
        assert!(p.is_protected(0, 10, &Block::tool(Role::User, "x", "edit")));
        assert!(p.is_protected(0, 10, &Block::tool(Role::User, "x", "WRITE")));
    }

    #[test]
    fn cache_stable_ignores_recency_but_keeps_other_rules() {
        let cfg = CompressConfig {
            cache_stable: true,
            ..CompressConfig::default()
        };
        let p = ProtectionPolicy::new(&cfg);
        let big = "z".repeat(700);
        // Most-recent position: normally protected by recency; cache-stable mode
        // ignores recency (decision is position-independent → stable prefix).
        assert!(!p.is_protected(9, 10, &Block::tool(Role::User, &big, "Bash")));
        // Edit-critical tool and short content are still protected.
        assert!(p.is_protected(9, 10, &Block::tool(Role::User, &big, "Read")));
        assert!(p.is_protected(9, 10, &Block::tool(Role::User, "tiny", "Bash")));
    }
}
