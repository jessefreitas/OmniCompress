use super::{Compressor, Outcome};
use serde_json::Value;

#[derive(Default)]
pub struct JsonCrusher;

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
            _ => return Outcome::untouched(content),
        };

        // Only worth crushing when there are at least 20 items.
        if arr.len() < 20 {
            return Outcome::untouched(content);
        }

        // Schema = keys of the first object; bail if items aren't objects.
        let keys: Vec<String> = match arr.first() {
            Some(Value::Object(o)) => o.keys().cloned().collect(),
            _ => return Outcome::untouched(content),
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

        // Only replace when it actually shrinks the payload.
        if compressed.len() >= content.len() {
            return Outcome::untouched(content);
        }

        Outcome {
            compressed,
            original: Some(content.to_string()),
            detail: format!("json_array:{}", arr.len()),
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
}
