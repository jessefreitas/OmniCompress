//! Benchmark harness for OmniCompress: measures compression efficacy
//! per content kind over a corpus of payloads.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::ccr::{CCRStore, MemoryStore};
use crate::pipeline::CompressionPipeline;
use crate::protection::CompressConfig;
use crate::router::ContentRouter;
use crate::types::{Block, CompressResult, ContentKind, Role};

/// Aggregated statistics for a single content kind (or overall).
#[derive(Debug, Clone, Default)]
pub struct KindStats {
    pub count: usize,
    pub tokens_before: usize,
    pub tokens_after: usize,
}

impl KindStats {
    pub fn ratio(&self) -> f64 {
        if self.tokens_before == 0 {
            0.0
        } else {
            self.tokens_after as f64 / self.tokens_before as f64
        }
    }
}

/// Full report produced by [`bench`].
#[derive(Debug, Clone, Default)]
pub struct BenchReport {
    pub overall: KindStats,
    pub by_kind: BTreeMap<String, KindStats>,
    pub files: usize,
    pub roundtrip_ok: bool,
}

impl BenchReport {
    pub fn overall_ratio(&self) -> f64 {
        self.overall.ratio()
    }
}

/// Run the benchmark over the supplied `(name, content)` payloads.
///
/// Each payload is classified by [`ContentRouter`], compressed through a
/// single shared pipeline backed by an in-memory CCR store, and its
/// token accounting is bucketed per kind as well as overall. A round-trip
/// check is performed at the end to ensure every generated `ccr_ref` can be
/// resolved back from the store.
pub fn bench(payloads: Vec<(String, String)>) -> BenchReport {
    let router = ContentRouter;
    let store: Arc<dyn CCRStore> = Arc::new(MemoryStore::default());
    let pipeline = CompressionPipeline::new_arc(store.clone());

    let cfg = CompressConfig {
        protect_recent: 0,
        min_chars_to_compress: 1,
        excluded_tools: vec![],
        cache_stable: false,
    };

    let mut report = BenchReport {
        overall: KindStats::default(),
        by_kind: BTreeMap::new(),
        files: payloads.len(),
        roundtrip_ok: true,
    };

    let mut all_refs: Vec<crate::types::CcrRef> = Vec::new();

    for (_name, content) in payloads {
        let kind: ContentKind = router.route(&content);
        let key = format!("{:?}", kind);

        let blocks = vec![Block {
            role: Role::User,
            content: content.clone(),
            tool_name: None,
        }];

        let result: CompressResult = pipeline.compress(blocks, &cfg);

        // Accumulate overall.
        report.overall.count += 1;
        report.overall.tokens_before += result.tokens_before;
        report.overall.tokens_after += result.tokens_after;

        // Accumulate per-kind bucket.
        let bucket = report.by_kind.entry(key).or_default();
        bucket.count += 1;
        bucket.tokens_before += result.tokens_before;
        bucket.tokens_after += result.tokens_after;

        all_refs.extend(result.ccr_refs);
    }

    // Round-trip verification: every emitted ccr_ref must resolve via the
    // shared store.
    let mut roundtrip_ok = true;
    for r in &all_refs {
        if !matches!(store.get(&r.hash), Ok(Some(_))) {
            roundtrip_ok = false;
        }
    }
    report.roundtrip_ok = roundtrip_ok;

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_json_payload() -> String {
        // Array with >= 30 objects with repetitive structure -> should
        // compress well below 0.5 ratio.
        let mut s = String::from("[");
        for i in 0..40 {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(
                "{{\"id\":{},\"name\":\"item_{i}\",\"kind\":\"widget\",\"active\":true,\"score\":{},\"tags\":[\"a\",\"b\",\"c\"],\"owner\":\"team_alpha\",\"path\":\"/var/data/items/item_{i}.bin\",\"note\":\"this is a fairly verbose note that repeats across many entries so that the compressor has plenty of redundancy to exploit\"}}",
                i, i * 3
            ));
        }
        s.push(']');
        s
    }

    #[test]
    fn bench_corpus_mixed_kinds() {
        let payloads = vec![
            ("json_big.json".to_string(), make_json_payload()),
            ("short_prose.txt".to_string(),
             "Hello world, this is a short single-line prose payload with no special structure.".to_string()),
        ];

        let report = bench(payloads);

        assert_eq!(report.files, 2);
        assert!(report.roundtrip_ok, "round-trip must succeed for all ccr_refs");

        let json_stats = report
            .by_kind
            .get("Json")
            .expect("expected a Json bucket in by_kind");
        assert!(
            json_stats.ratio() < 0.5,
            "JSON payload should compress below 0.5, got {}",
            json_stats.ratio()
        );

        // Overall must be a sensible weighted average between the two.
        let overall = report.overall_ratio();
        assert!(overall > 0.0 && overall < 1.0, "overall ratio out of range: {}", overall);

        // The prose bucket may or may not be present depending on router
        // classification; if present it should be near-intact.
        if let Some(prose) = report.by_kind.get("Prose") {
            assert!(
                prose.ratio() > 0.9,
                "short prose should stay roughly intact, got {}",
                prose.ratio()
            );
        }
    }
}
