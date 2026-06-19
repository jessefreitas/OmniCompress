//! Compressor for line-oriented tabular tool-output: **NDJSON/JSONL** and
//! **CSV/TSV** — formats that JSON's single-value parser rejects (so they used to
//! fall through to `Prose` and pass intact).
//!
//! - **NDJSON/JSONL** (one JSON object per line): the same redundancy as a JSON
//!   array — every line repeats the keys. In lossless mode it folds into a compact
//!   columnar table (shared schema + one row per line, every value preserved,
//!   reversible via [`restore_ndjson`]); in aggressive mode it is reduced to a
//!   schema + a small sample with the original kept in the CCR.
//! - **CSV/TSV** is already columnar, so there is no lossless token win — lossless
//!   mode leaves it intact (honest zero). In aggressive mode it is reduced to its
//!   header + a sample row count, original in the CCR.

use super::json_crusher::{decode_columnar, encode_columnar};
use super::{Compressor, Outcome};
use serde_json::Value;

const NDJSON_MARKER: &str = "ndjson_table";
const MIN_ROWS: usize = 5;

#[derive(Default)]
pub struct TabularCompressor {
    pub lossless: bool,
}

impl Compressor for TabularCompressor {
    fn name(&self) -> &'static str {
        "tabular"
    }

    fn compress(&self, content: &str) -> Outcome {
        if let Some(objects) = parse_ndjson(content) {
            return self.compress_ndjson(content, &objects);
        }
        if let Some((delim, rows)) = parse_delimited(content) {
            return self.compress_delimited(content, delim, &rows);
        }
        Outcome::untouched(content)
    }
}

impl TabularCompressor {
    fn compress_ndjson(&self, content: &str, objects: &[Value]) -> Outcome {
        if self.lossless {
            // Columnar table keeping every row — self-contained, no CCR write.
            let Some(table) = encode_columnar(objects, NDJSON_MARKER) else {
                return Outcome::untouched(content);
            };
            if table.len() < content.len() {
                return Outcome {
                    compressed: table,
                    original: None,
                    detail: format!("ndjson_table_lossless:{}", objects.len()),
                };
            }
            return Outcome::untouched(content);
        }

        // Aggressive: schema (union of keys) + a two-row sample; original in CCR.
        let mut schema: Vec<String> = Vec::new();
        for o in objects {
            if let Some(map) = o.as_object() {
                for k in map.keys() {
                    if !schema.iter().any(|s| s == k) {
                        schema.push(k.clone());
                    }
                }
            }
        }
        let sample: Vec<&Value> = objects.iter().take(2).collect();
        let summary = serde_json::json!({
            "_omnicompress": "ndjson",
            "count": objects.len(),
            "schema": schema,
            "sample": sample,
        });
        let compressed = summary.to_string();
        if compressed.len() < content.len() {
            return Outcome {
                compressed,
                original: Some(content.to_string()),
                detail: format!("ndjson:{}", objects.len()),
            };
        }
        Outcome::untouched(content)
    }

    fn compress_delimited(&self, content: &str, delim: char, rows: &[Vec<String>]) -> Outcome {
        // CSV/TSV is already columnar — no lossless token win to be had. Leave it
        // intact in lossless mode (honest zero).
        if self.lossless {
            return Outcome::untouched(content);
        }

        let fmt = if delim == '\t' { "tsv" } else { "csv" };
        let header = rows.first().cloned().unwrap_or_default();
        let sample: Vec<&Vec<String>> = rows.iter().take(2).collect();
        let summary = serde_json::json!({
            "_omnicompress": fmt,
            "delimiter": delim.to_string(),
            "rows": rows.len(),
            "header": header,
            "sample": sample,
        });
        let compressed = summary.to_string();
        if compressed.len() < content.len() {
            return Outcome {
                compressed,
                original: Some(content.to_string()),
                detail: format!("{fmt}:{}", rows.len()),
            };
        }
        Outcome::untouched(content)
    }
}

/// Parse NDJSON/JSONL: every non-empty line must independently parse as a JSON
/// **object**, and there must be at least `MIN_ROWS` of them. Returns `None`
/// otherwise (so a single multi-line JSON value is left to the JSON crusher).
pub fn parse_ndjson(content: &str) -> Option<Vec<Value>> {
    let mut objects: Vec<Value> = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line).ok()?;
        if !v.is_object() {
            return None;
        }
        objects.push(v);
    }
    (objects.len() >= MIN_ROWS).then_some(objects)
}

/// Reverse of the NDJSON columnar form: reconstruct one compact JSON object per
/// line. Data-lossless (object key order/whitespace are canonicalised).
pub fn restore_ndjson(table_json: &str) -> Option<String> {
    let objects = decode_columnar(table_json, NDJSON_MARKER)?;
    let mut out = String::new();
    for (i, o) in objects.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&o.to_string());
    }
    Some(out)
}

/// Detect a CSV/TSV table: pick the delimiter (`,` then `\t`) under which every
/// non-empty line splits into the **same** number of fields (≥ 2), with at least
/// `MIN_ROWS` lines. Returns the delimiter and the rows of string cells.
pub fn parse_delimited(content: &str) -> Option<(char, Vec<Vec<String>>)> {
    for delim in [',', '\t'] {
        let rows: Vec<Vec<String>> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.split(delim).map(|c| c.to_string()).collect::<Vec<_>>())
            .collect();
        if rows.len() < MIN_ROWS {
            continue;
        }
        let cols = rows[0].len();
        if cols >= 2 && rows.iter().all(|r| r.len() == cols) {
            return Some((delim, rows));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ndjson(n: usize) -> String {
        (0..n)
            .map(|i| format!(r#"{{"id":{i},"name":"svc-{i}","ok":true}}"#))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn ndjson_lossless_columnar_roundtrips() {
        let content = ndjson(12);
        let out = TabularCompressor { lossless: true }.compress(&content);
        assert!(out.original.is_none(), "lossless: nada no CCR");
        assert!(out.compressed.contains("ndjson_table"));
        assert!(out.compressed.len() < content.len(), "deve encolher");

        // Roundtrip is data-lossless: each restored line parses to the original object.
        let restored = restore_ndjson(&out.compressed).unwrap();
        let got: Vec<Value> = restored
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let want: Vec<Value> = content
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn ndjson_aggressive_samples_and_stores_ccr() {
        let content = ndjson(40);
        let out = TabularCompressor { lossless: false }.compress(&content);
        assert!(out.original.is_some(), "agressivo guarda original no CCR");
        assert!(out.compressed.contains("\"count\":40"));
        assert!(out.compressed.contains("sample"));
        assert!(out.compressed.len() < content.len());
    }

    #[test]
    fn heterogeneous_ndjson_roundtrips() {
        let content = [
            r#"{"a":1,"b":2}"#,
            r#"{"a":3}"#,
            r#"{"a":4,"b":5,"c":6}"#,
            r#"{"b":7}"#,
            r#"{"a":8,"b":9}"#,
        ]
        .join("\n");
        let out = TabularCompressor { lossless: true }.compress(&content);
        // Might not shrink (small), but if it does it must round-trip exactly.
        if out.original.is_none() && out.compressed.contains("ndjson_table") {
            let restored = restore_ndjson(&out.compressed).unwrap();
            let got: Vec<Value> = restored.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
            let want: Vec<Value> = content.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
            assert_eq!(got, want);
        }
    }

    #[test]
    fn non_object_lines_are_not_ndjson() {
        let content = "1\n2\n3\n4\n5\n"; // numbers, not objects
        assert!(parse_ndjson(content).is_none());
    }

    #[test]
    fn single_json_value_is_not_ndjson() {
        // A pretty-printed JSON array spans many lines but is a single value —
        // it must NOT be treated as NDJSON (the JSON crusher owns it).
        let arr = json!([{"a":1},{"a":2}]);
        let pretty = serde_json::to_string_pretty(&arr).unwrap();
        assert!(parse_ndjson(&pretty).is_none());
    }

    #[test]
    fn csv_detected_and_aggressively_compressed() {
        let mut content = String::from("id,name,region\n");
        for i in 0..30 {
            content.push_str(&format!("{i},svc-{i},us-east-1\n"));
        }
        let (delim, rows) = parse_delimited(&content).expect("deve detectar CSV");
        assert_eq!(delim, ',');
        assert_eq!(rows[0], vec!["id", "name", "region"]);

        let out = TabularCompressor { lossless: false }.compress(&content);
        assert!(out.original.is_some(), "CSV agressivo guarda original");
        assert!(out.compressed.contains("\"csv\""));
        assert!(out.compressed.len() < content.len());
    }

    #[test]
    fn csv_lossless_is_untouched() {
        let mut content = String::from("a,b\n");
        for i in 0..30 {
            content.push_str(&format!("{i},{}\n", i * 2));
        }
        let out = TabularCompressor { lossless: true }.compress(&content);
        assert!(out.original.is_none());
        assert_eq!(out.compressed, content, "CSV não tem ganho lossless");
    }

    #[test]
    fn tsv_uses_tab_delimiter() {
        let mut content = String::from("id\tname\n");
        for i in 0..20 {
            content.push_str(&format!("{i}\tn{i}\n"));
        }
        let (delim, _) = parse_delimited(&content).expect("deve detectar TSV");
        assert_eq!(delim, '\t');
    }
}
