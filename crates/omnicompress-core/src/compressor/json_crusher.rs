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
/// ordered union of all keys) plus one value-row per item (`null` for absent
/// keys). Every value is preserved — reconstructable, no CCR needed. Returns
/// `None` if any item is not an object (columnar does not apply).
fn lossless_table(arr: &[Value]) -> Option<String> {
    use std::collections::HashSet;

    let mut schema: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut rows: Vec<Vec<Value>> = Vec::with_capacity(arr.len());

    for item in arr {
        let obj = item.as_object()?;
        for key in obj.keys() {
            if seen.insert(key.clone()) {
                schema.push(key.clone());
            }
        }
    }

    for item in arr {
        let obj = item.as_object().unwrap();
        let row = schema
            .iter()
            .map(|key| obj.get(key).cloned().unwrap_or(Value::Null))
            .collect();
        rows.push(row);
    }

    let table = serde_json::json!({
        "_omnicompress": "json_table",
        "count": arr.len(),
        "schema": schema,
        "rows": rows
    });

    Some(table.to_string())
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
    use super::lossless_table;
    use serde_json::{json, Value};

    #[test]
    fn table_is_lossless_union_schema() {
        let arr = vec![
            json!({"a": 1, "b": 2}),
            json!({"a": 1, "b": 2, "owner": "me"}),
            json!({"a": 1, "b": 2}),
        ];
        let res = lossless_table(&arr);
        assert!(res.is_some());
        
        let parsed: Value = serde_json::from_str(&res.unwrap()).unwrap();
        let schema = parsed["schema"].as_array().unwrap();
        assert!(schema.iter().any(|v| v == "owner"));
        assert_eq!(parsed["count"], 3);
        
        let rows = parsed["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        
        let owner_idx = schema.iter().position(|v| v == "owner").unwrap();
        assert_eq!(rows[0][owner_idx], Value::Null);
        assert_eq!(rows[1][owner_idx], json!("me"));
        assert_eq!(rows[2][owner_idx], Value::Null);
    }

    #[test]
    fn non_objects_return_none() {
        let arr = vec![
            json!({"a": 1}),
            json!(5),
            json!({"a": 2}),
        ];
        let res = lossless_table(&arr);
        assert!(res.is_none());
    }

    #[test]
    fn all_values_preserved() {
        let mut arr = Vec::new();
        for i in 0..5 {
            arr.push(json!({"id": i, "v": format!("val-{}", i)}));
        }
        let res = lossless_table(&arr);
        assert!(res.is_some());
        
        let parsed: Value = serde_json::from_str(&res.unwrap()).unwrap();
        let rows = parsed["rows"].as_array().unwrap();
        let schema = parsed["schema"].as_array().unwrap();
        
        let id_idx = schema.iter().position(|v| v == "id").unwrap();
        let v_idx = schema.iter().position(|v| v == "v").unwrap();

        for i in 0..5 {
            assert_eq!(rows[i][id_idx], i);
            assert_eq!(rows[i][v_idx], json!(format!("val-{}", i)));
        }
    }
}
