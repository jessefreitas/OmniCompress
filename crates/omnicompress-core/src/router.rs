use crate::types::ContentKind;

#[derive(Default)]
pub struct ContentRouter;

impl ContentRouter {
    /// Classifies content by its structure and textual patterns.
    /// Deterministic and allocation-light. Never inspects tool names.
    pub fn route(&self, content: &str) -> ContentKind {
        let t = content.trim_start();

        // JSON: must start with `{` or `[` and parse cleanly (a single value).
        if (t.starts_with('{') || t.starts_with('['))
            && serde_json::from_str::<serde_json::Value>(content).is_ok()
        {
            return ContentKind::Json;
        }

        // Tabular: NDJSON/JSONL (one JSON object per line) or CSV/TSV. Checked
        // before code/log so a table of records isn't misread by keyword/prefix
        // heuristics.
        if crate::compressor::tabular::parse_ndjson(content).is_some()
            || crate::compressor::tabular::parse_delimited(content).is_some()
        {
            return ContentKind::Tabular;
        }

        // Diff: common unified-diff header patterns.
        if t.starts_with("diff ") || t.starts_with("--- ") || t.starts_with("@@ ") {
            return ContentKind::Diff;
        }

        if Self::looks_like_code(content) {
            return ContentKind::Code;
        }

        if Self::looks_like_log(content) {
            return ContentKind::Log;
        }

        if content.trim().is_empty() {
            return ContentKind::Unknown;
        }

        ContentKind::Prose
    }

    fn looks_like_code(s: &str) -> bool {
        const KW: [&str; 8] = [
            "def ",
            "fn ",
            "class ",
            "import ",
            "function ",
            "func ",
            "public ",
            "const ",
        ];
        let hits = KW.iter().filter(|k| s.contains(*k)).count();
        let braces = s.matches(['{', '}', ';']).count();
        // Need at least one keyword AND (structural braces/semicolons OR Python-style colon syntax).
        hits >= 1 && (braces >= 2 || s.contains("():") || s.contains("->"))
    }

    fn looks_like_log(s: &str) -> bool {
        let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() < 3 {
            return false;
        }
        // Many lines beginning with a timestamp-like prefix or a log level keyword => log.
        let with_prefix = lines.iter().filter(|l| {
            let head: String = l.chars().take(4).collect();
            head.chars().all(|c| c.is_ascii_digit())
                || l.contains("INFO")
                || l.contains("ERROR")
                || l.contains("WARN")
                || l.contains("DEBUG")
        }).count();
        // More than half the non-empty lines look like log lines.
        with_prefix * 2 >= lines.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ContentKind;

    #[test]
    fn detects_json_array() {
        assert_eq!(
            ContentRouter::default().route(r#"[{"a":1},{"a":2}]"#),
            ContentKind::Json
        );
    }

    #[test]
    fn detects_code_by_keywords() {
        assert_eq!(
            ContentRouter::default().route("def foo():\n    return 1\n"),
            ContentKind::Code
        );
    }

    #[test]
    fn detects_log_by_repetition() {
        let log = "2026-06-18 INFO x\n2026-06-18 INFO y\n2026-06-18 INFO z\n";
        assert_eq!(ContentRouter::default().route(log), ContentKind::Log);
    }

    #[test]
    fn prose_is_default_textual() {
        assert_eq!(
            ContentRouter::default().route("isto é apenas um texto comum em português"),
            ContentKind::Prose
        );
    }
}
