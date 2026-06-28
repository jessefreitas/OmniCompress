use super::{Compressor, Outcome};
use serde_json::Value;

#[derive(Default)]
pub struct JsonCrusher {
    /// When `true`, compress losslessly (columnar table, all rows kept, no CCR).
    /// When `false`, aggressive schema+sample with the original stored in the CCR.
    pub lossless: bool,
}

/// Recursively elide large values inside a JSON structure.
///
/// Any value whose serialized form exceeds `limit` bytes is replaced with a
/// compact marker `{"_omnicompress_elided": "<kind>", "bytes": N}` where
/// `kind` ∈ {"object","array","string"} and `N` is the original byte size.
/// Scalars and small structures are kept intact; keys are never removed.
fn elide(v: &Value, limit: usize) -> Value {
    // Serialize once to measure. If the value is small enough, keep it whole.
    let bytes = v.to_string();
    if bytes.len() <= limit {
        return v.clone();
    }

    match v {
        Value::Object(o) => {
            // Rebuild preserving every key, recursing into the children.
            let mut rebuilt = serde_json::Map::with_capacity(o.len());
            for (k, child) in o {
                rebuilt.insert(k.clone(), elide(child, limit));
            }
            // The rebuilt object may still be large, but its children are
            // individually bounded; we keep it as a structured skeleton
            // rather than collapsing the whole node, so the LLM sees the shape.
            Value::Object(rebuilt)
        }
        Value::Array(a) => {
            let rebuilt: Vec<Value> = a.iter().map(|c| elide(c, limit)).collect();
            Value::Array(rebuilt)
        }
        // Scalars that are too big to keep (only strings can realistically be).
        Value::String(_) => serde_json::json!({
            "_omnicompress_elided": "string",
            "bytes": bytes.len(),
        }),
        // Numbers/bools/null that serialize large is essentially impossible;
        // keep them as-is as a safe fallback.
        other => other.clone(),
    }
}

/// Lossless columnar form of an array of objects: a single shared schema (the
/// ordered union of all keys) plus one row per item. Every value is preserved
/// and the encoding is fully reversible via [`restore_table`] — reconstructable,
/// no CCR needed. Returns `None` if any item is not an object (columnar does not
/// apply).
///
/// ## Absent vs. `null`
/// A key that is *missing* from an object is distinguished from a key whose value
/// is explicitly `null`, so `{}` and `{"a":null}` never collapse together:
/// - **Dense** tables (every row has every schema key) emit `rows` as plain
///   value-arrays aligned to `schema` — the compact, common case.
/// - **Sparse** tables additionally emit a `present` array: for each row, the
///   list of schema indices actually present. Each row's values are stored in
///   `present` order, so absent keys are simply not listed (a present key with a
///   `null` value still appears, with value `null`).
fn lossless_table(arr: &[Value]) -> Option<String> {
    encode_columnar(arr, "json_table")
}

/// Shared columnar encoder used by both the JSON-array crusher (`json_table`) and
/// the NDJSON tabular compressor (`ndjson_table`). See [`lossless_table`] for the
/// absent-vs-`null` semantics. `marker` is written as the `_omnicompress` tag and
/// must be passed back to [`decode_columnar`] to decode.
pub(crate) fn encode_columnar(arr: &[Value], marker: &str) -> Option<String> {
    use std::collections::HashSet;

    let mut schema: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for item in arr {
        let obj = item.as_object()?;
        for key in obj.keys() {
            if seen.insert(key.clone()) {
                schema.push(key.clone());
            }
        }
    }

    // Dense when every object contains every schema key.
    let dense = arr
        .iter()
        .all(|item| item.as_object().unwrap().len() == schema.len());

    let mut table = serde_json::Map::new();
    table.insert("_omnicompress".into(), Value::from(marker));
    table.insert("count".into(), Value::from(arr.len()));
    table.insert("schema".into(), Value::from(schema.clone()));

    if dense {
        let rows: Vec<Value> = arr
            .iter()
            .map(|item| {
                let obj = item.as_object().unwrap();
                let row: Vec<Value> = schema
                    .iter()
                    .map(|key| obj.get(key).cloned().expect("dense: key present"))
                    .collect();
                Value::from(row)
            })
            .collect();
        table.insert("rows".into(), Value::from(rows));
    } else {
        let mut rows: Vec<Value> = Vec::with_capacity(arr.len());
        let mut present: Vec<Value> = Vec::with_capacity(arr.len());
        for item in arr {
            let obj = item.as_object().unwrap();
            let mut row_vals: Vec<Value> = Vec::new();
            let mut row_idx: Vec<Value> = Vec::new();
            for (i, key) in schema.iter().enumerate() {
                if let Some(v) = obj.get(key) {
                    row_idx.push(Value::from(i));
                    row_vals.push(v.clone());
                }
            }
            rows.push(Value::from(row_vals));
            present.push(Value::from(row_idx));
        }
        table.insert("rows".into(), Value::from(rows));
        table.insert("present".into(), Value::from(present));
    }

    Some(Value::Object(table).to_string())
}

/// Reverse of [`lossless_table`]: reconstruct the original JSON array (as a
/// canonical JSON string) from a `json_table` document. Returns `None` if the
/// input is not a well-formed `json_table` produced by this module.
///
/// Reconstruction is **data-lossless**: every value (including the absent-vs-`null`
/// distinction) is recovered. Object key order and whitespace are canonicalised by
/// `serde_json` — the recovered *data* is identical, not necessarily the original
/// byte layout.
pub fn restore_table(table_json: &str) -> Option<String> {
    let arr = decode_columnar(table_json, "json_table")?;
    Some(Value::from(arr).to_string())
}

/// Shared columnar decoder: reverse of [`encode_columnar`]. Reconstructs the
/// original objects (as `Value`s), recovering the absent-vs-`null` distinction.
/// Returns `None` unless the document is a well-formed table whose `_omnicompress`
/// tag equals `marker`.
/// Extract the column-name schema, failing (`None`) if any entry is not a string.
fn decode_schema(obj: &serde_json::Map<String, Value>) -> Option<Vec<&str>> {
    obj.get("schema")?
        .as_array()?
        .iter()
        .map(|k| k.as_str())
        .collect::<Option<Vec<_>>>()
}

/// Rebuild one object from its row. `idxs = Some(..)` is the sparse encoding (each cell
/// carries its schema index); `None` is the dense encoding (cells align 1:1 with the
/// schema). Insertion order matches the original — cell order when sparse, schema order
/// when dense — and every original `?`/length-mismatch failure is preserved.
fn decode_row(
    schema: &[&str],
    cells: &[Value],
    idxs: Option<&[Value]>,
) -> Option<serde_json::Map<String, Value>> {
    let mut rebuilt = serde_json::Map::new();
    match idxs {
        Some(idxs) => {
            if idxs.len() != cells.len() {
                return None;
            }
            for (cell, idx) in cells.iter().zip(idxs) {
                let i = idx.as_u64()? as usize;
                let key = schema.get(i)?;
                rebuilt.insert((*key).to_string(), cell.clone());
            }
        }
        None => {
            if cells.len() != schema.len() {
                return None;
            }
            for (key, cell) in schema.iter().zip(cells) {
                rebuilt.insert((*key).to_string(), cell.clone());
            }
        }
    }
    Some(rebuilt)
}

pub(crate) fn decode_columnar(table_json: &str, marker: &str) -> Option<Vec<Value>> {
    let v: Value = serde_json::from_str(table_json).ok()?;
    let obj = v.as_object()?;
    if obj.get("_omnicompress")?.as_str()? != marker {
        return None;
    }

    let schema = decode_schema(obj)?;
    let rows = obj.get("rows")?.as_array()?;
    let present = obj.get("present").and_then(Value::as_array);

    let mut out: Vec<Value> = Vec::with_capacity(rows.len());
    for (r, row) in rows.iter().enumerate() {
        let cells = row.as_array()?;
        // Sparse: `present[r]` holds the schema index of each cell; dense: 1:1.
        let idxs = match present {
            Some(p) => Some(p.get(r)?.as_array()?.as_slice()),
            None => None,
        };
        out.push(Value::Object(decode_row(&schema, cells, idxs)?));
    }

    Some(out)
}

impl Compressor for JsonCrusher {
    fn name(&self) -> &'static str {
        "json_crusher"
    }

    fn compress(&self, content: &str) -> Outcome {
        let Ok(v) = serde_json::from_str::<Value>(content) else {
            return Outcome::untouched(content);
        };

        // Resolve the array: either the root value is an array, or a top-level
        // object that contains exactly one array-valued field (common envelope).
        let arr = match &v {
            Value::Array(a) => a.clone(),
            Value::Object(o) => o
                .values()
                .find_map(|x| {
                    if let Value::Array(a) = x {
                        Some(a.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_default(),
            _ => {
                // Non-array, non-object root — nothing to do.
                return Outcome::untouched(content);
            }
        };

        // ---- Case 1: array crushing (preserved) ----
        if arr.len() >= 20 {
            // Lossless: keep every row in a compact columnar table (no CCR write),
            // so a consumer without a retrieve loop still has all the data.
            if self.lossless {
                if let Some(table) = lossless_table(&arr) {
                    if table.len() < content.len() {
                        return Outcome {
                            compressed: table,
                            original: None,
                            detail: format!("json_table_lossless:{}", arr.len()),
                        };
                    }
                    return Outcome::untouched(content);
                }
                return self.compress_large_object(&v, content);
            }

            // Aggressive: schema = keys of the first object; bail if items aren't objects.
            let keys: Vec<String> = match arr.first() {
                Some(Value::Object(o)) => o.keys().cloned().collect(),
                _ => {
                    // Not a crushable array; fall through to case 2 below.
                    return self.compress_large_object(&v, content);
                }
            };

            // Two-item sample so the reader can still inspect representative rows.
            let sample: Vec<&Value> = arr.iter().take(2).collect();

            let summary = serde_json::json!({
                "_omnicompress": "json_array",
                "count": arr.len(),
                "schema": keys,
                "sample": sample,
            });

            let compressed = summary.to_string();

            if compressed.len() < content.len() {
                return Outcome {
                    compressed,
                    original: Some(content.to_string()),
                    detail: format!("json_array:{}", arr.len()),
                };
            }
            return Outcome::untouched(content);
        }

        // ---- Case 2: large nested object skeleton ----
        self.compress_large_object(&v, content)
    }
}

impl JsonCrusher {
    /// Build a structural skeleton of a large root object, eliding bulky values
    /// while preserving all keys. Falls back to `untouched` if the root is not
    /// an object, is too small, or if the skeleton doesn't shrink the payload.
    fn compress_large_object(&self, v: &Value, content: &str) -> Outcome {
        // Lossless mode never elides values — leave the nested object intact.
        if self.lossless {
            return Outcome::untouched(content);
        }
        // Only applies when the array case did NOT and the root is a big object.
        if !matches!(v, Value::Object(_)) {
            return Outcome::untouched(content);
        }
        if content.len() <= 800 {
            return Outcome::untouched(content);
        }

        let skeleton = elide(v, 120);
        let compressed = skeleton.to_string();

        if compressed.len() >= content.len() {
            return Outcome::untouched(content);
        }

        Outcome {
            compressed,
            original: Some(content.to_string()),
            detail: "json_object_skeleton".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressor::Compressor;

    #[test]
    fn crushes_array_of_records_to_schema_plus_sample() {
        let arr: String = "[".to_string()
            + &(0..50)
                .map(|i| format!(r#"{{"id":{i},"name":"n{i}","score":0.5}}"#))
                .collect::<Vec<_>>()
                .join(",")
            + "]";
        let out = JsonCrusher::default().compress(&arr);
        assert!(out.original.is_some(), "deve guardar original no CCR");
        assert!(
            out.compressed.len() * 3 < arr.len(),
            "deve cortar >2/3: {} vs {}",
            out.compressed.len(),
            arr.len()
        );
        assert!(
            out.compressed.contains("id") && out.compressed.contains("50"),
            "schema+contagem: {}",
            out.compressed
        );
    }

    #[test]
    fn tiny_array_untouched() {
        let out = JsonCrusher::default().compress(r#"[{"a":1}]"#);
        assert!(out.original.is_none(), "array pequeno não vale a pena");
    }

    #[test]
    fn crushes_large_nested_object() {
        let huge_string = "x".repeat(2000);
        let big_nested = serde_json::json!({
            "matrix": (0..40).map(|i| {
                serde_json::json!({"row": i, "payload": "y".repeat(80)})
            }).collect::<Vec<_>>(),
            "config": {
                "rules": (0..30).map(|i| format!("rule-{i}-with-some-padding-text")).collect::<Vec<_>>(),
                "version": 7,
            }
        });
        let root = serde_json::json!({
            "id": "doc-1",
            "kind": "report",
            "active": true,
            "huge_text": huge_string,
            "body": big_nested,
        });
        let content = root.to_string();
        assert!(content.len() > 800, "fixture deve ser grande");

        let out = JsonCrusher::default().compress(&content);
        assert!(
            out.original.is_some(),
            "objeto grande deve guardar original no CCR"
        );
        assert!(
            out.compressed.len() < content.len(),
            "esqueleto deve ser menor: {} vs {}",
            out.compressed.len(),
            content.len()
        );
        assert!(
            out.compressed.contains("_omnicompress_elided"),
            "deve conter marcador de elisão: {}",
            out.compressed
        );
        for key in ["id", "kind", "active", "huge_text", "body"] {
            assert!(
                out.compressed.contains(&format!("\"{key}\"")),
                "esqueleto deve preservar chave '{}': {}",
                key,
                out.compressed
            );
        }
        assert_eq!(out.detail, "json_object_skeleton");
    }

    #[test]
    fn small_object_untouched() {
        let small = r#"{"a":1,"b":"hi","c":[1,2,3]}"#;
        assert!(small.len() < 800);
        let out = JsonCrusher::default().compress(small);
        assert!(
            out.original.is_none(),
            "objeto pequeno não deve ser comprimido"
        );
    }
}

#[cfg(test)]
mod lossless_tests {
    use super::{lossless_table, restore_table};
    use serde_json::{json, Value};

    /// Helper: assert that table → restore round-trips to the same data.
    fn assert_roundtrip(arr: &[Value]) {
        let table = lossless_table(arr).expect("table should build");
        let restored = restore_table(&table).expect("table should restore");
        let restored: Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(restored, Value::from(arr.to_vec()), "roundtrip must be lossless");
    }

    #[test]
    fn table_carries_union_schema_and_count() {
        let arr = vec![
            json!({"a": 1, "b": 2}),
            json!({"a": 1, "b": 2, "owner": "me"}),
            json!({"a": 1, "b": 2}),
        ];
        let parsed: Value = serde_json::from_str(&lossless_table(&arr).unwrap()).unwrap();
        let schema = parsed["schema"].as_array().unwrap();
        assert!(schema.iter().any(|v| v == "owner"));
        assert_eq!(parsed["count"], 3);
        // Heterogeneous rows → sparse encoding carries a `present` map.
        assert!(parsed.get("present").is_some(), "sparse table must record presence");
    }

    #[test]
    fn non_objects_return_none() {
        let arr = vec![json!({"a": 1}), json!(5), json!({"a": 2})];
        assert!(lossless_table(&arr).is_none());
    }

    #[test]
    fn dense_roundtrips() {
        let arr: Vec<Value> = (0..5).map(|i| json!({"id": i, "v": format!("val-{i}")})).collect();
        let parsed: Value = serde_json::from_str(&lossless_table(&arr).unwrap()).unwrap();
        assert!(parsed.get("present").is_none(), "uniform rows must stay dense");
        assert_roundtrip(&arr);
    }

    #[test]
    fn sparse_roundtrips() {
        let arr = vec![
            json!({"a": 1, "b": 2}),
            json!({"a": 1, "b": 2, "owner": "me"}),
            json!({"b": 9}),
        ];
        assert_roundtrip(&arr);
    }

    /// The bug this fix targets: an absent key must NOT collapse into an explicit
    /// `null`. `{}` and `{"a":null}` must round-trip distinctly.
    #[test]
    fn absent_key_is_distinct_from_explicit_null() {
        let arr = vec![json!({"a": null}), json!({})];
        let table = lossless_table(&arr).unwrap();
        let restored: Value =
            serde_json::from_str(&restore_table(&table).unwrap()).unwrap();
        let items = restored.as_array().unwrap();
        // Row 0 keeps an explicit null; row 1 has no key at all.
        assert!(items[0].as_object().unwrap().contains_key("a"));
        assert_eq!(items[0]["a"], Value::Null);
        assert!(items[1].as_object().unwrap().is_empty());
        // And the whole thing is byte-for-byte the same data going back out.
        assert_eq!(restored, json!([{"a": null}, {}]));
    }

    #[test]
    fn nested_values_roundtrip() {
        let arr = vec![
            json!({"id": 1, "tags": ["x", "y"], "meta": {"k": 1}}),
            json!({"id": 2, "tags": [], "meta": {"k": 2, "z": null}}),
        ];
        assert_roundtrip(&arr);
    }

    #[test]
    fn restore_rejects_non_table() {
        assert!(restore_table(r#"{"hello":"world"}"#).is_none());
        assert!(restore_table("not json").is_none());
    }
}
