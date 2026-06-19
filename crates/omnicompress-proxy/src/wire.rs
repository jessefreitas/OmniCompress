//! Wire-format request compression (pure, no I/O).
//!
//! Compresses the `messages` array of a chat-completion request body using the
//! SP1 `CompressionPipeline`, then re-serialises. Every non-`messages` field
//! (model, system, temperature, …) is preserved verbatim. Fail-open: any
//! JSON parse or serialise failure returns the original bytes unchanged.

use std::sync::Arc;

use serde_json::Value;

use omnicompress_core::ccr::MemoryStore;
use omnicompress_core::pipeline::CompressionPipeline;
use omnicompress_core::protection::CompressConfig;
use omnicompress_core::types::{Block, Role};

fn role_from(s: &str) -> Role {
    match s {
        "system" => Role::System,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

/// Compress the `messages` array in `body` in place, preserving all other fields.
/// Fail-open: returns `body.to_vec()` unchanged on any parse/serialise failure or
/// when there is no `messages` array to compress.
fn compress_messages_in_place(body: &[u8]) -> Vec<u8> {
    let mut root: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return body.to_vec(),
    };

    let messages = match root.get_mut("messages") {
        Some(Value::Array(arr)) => arr,
        _ => return body.to_vec(),
    };

    // Build blocks only for messages whose `content` is a plain string; remember
    // their indices so we can map the compressed content back afterwards.
    let mut blocks: Vec<Block> = Vec::new();
    let mut src_indices: Vec<usize> = Vec::new();
    for (idx, item) in messages.iter().enumerate() {
        let role = role_from(item.get("role").and_then(Value::as_str).unwrap_or(""));
        if let Some(content) = item.get("content").and_then(Value::as_str) {
            blocks.push(Block {
                role,
                content: content.to_string(),
                tool_name: None,
            });
            src_indices.push(idx);
        }
    }

    let pipeline = CompressionPipeline::new_arc(Arc::new(MemoryStore::default()));
    // cache_stable: a block's compressed form is position-independent, so the prefix
    // stays byte-stable as the conversation grows and the provider's prompt cache keeps
    // hitting. Without it, an old message flips bytes as it leaves the recent window,
    // busting the cache → the user pays MORE, the opposite of the product's promise.
    let cfg = CompressConfig {
        cache_stable: true,
        ..CompressConfig::default()
    };
    let result = pipeline.compress(blocks, &cfg);

    for (k, &src_idx) in src_indices.iter().enumerate() {
        if let Some(Value::Object(obj)) = messages.get_mut(src_idx) {
            if let Some(field) = obj.get_mut("content") {
                if field.is_string() {
                    *field = Value::String(result.messages[k].content.clone());
                }
            }
        }
    }

    serde_json::to_vec(&root).unwrap_or_else(|_| body.to_vec())
}

/// Compress an OpenAI-format chat-completion request body (wire compatibility).
pub fn compress_openai_request(body: &[u8]) -> Vec<u8> {
    compress_messages_in_place(body)
}

/// Compress an Anthropic-format messages request body (wire compatibility).
/// The top-level `system` field is not part of `messages` and is left intact.
pub fn compress_anthropic_request(body: &[u8]) -> Vec<u8> {
    compress_messages_in_place(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JSON-array string with `n` records — the structured shape the
    /// deterministic JsonCrusher actually compresses (n >= 20).
    fn compressible_json(n: usize) -> String {
        let items: Vec<String> = (0..n)
            .map(|i| format!(r#"{{"id":{i},"name":"item{i}","score":0.5}}"#))
            .collect();
        format!("[{}]", items.join(","))
    }

    #[test]
    fn openai_compresses_old_message_and_preserves_other_fields() {
        // Big compressible message at index 0 (outside the recent-protect window),
        // followed by short recent messages.
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": compressible_json(60)
        })];
        for i in 0..5 {
            messages.push(serde_json::json!({
                "role": if i % 2 == 0 { "assistant" } else { "user" },
                "content": format!("short message {i}")
            }));
        }
        let req = serde_json::json!({
            "model": "wire-test-model",
            "temperature": 0.2,
            "messages": messages
        });
        let body = serde_json::to_vec(&req).unwrap();
        let out = compress_openai_request(&body);

        let parsed: Value = serde_json::from_slice(&out).expect("output must be valid JSON");
        assert_eq!(parsed["model"], "wire-test-model");
        assert_eq!(parsed["temperature"], serde_json::json!(0.2));
        assert!(parsed["messages"].is_array());
        assert!(out.len() < body.len(), "output should be smaller: {} vs {}", out.len(), body.len());
    }

    #[test]
    fn cache_stable_compresses_even_a_lone_recent_message() {
        // A single big-array message would be protected by the recent-window in the
        // default config; the proxy uses cache_stable, which ignores recency, so it
        // still compresses — keeping the prefix byte-stable across turns.
        let req = serde_json::json!({
            "model": "x",
            "messages": [{"role": "user", "content": compressible_json(60)}]
        });
        let body = serde_json::to_vec(&req).unwrap();
        let out = compress_openai_request(&body);
        assert!(
            out.len() < body.len(),
            "cache_stable should compress the lone message: {} vs {}",
            out.len(),
            body.len()
        );
    }

    #[test]
    fn fail_open_on_malformed_body() {
        let body = b"not json at all";
        assert_eq!(compress_openai_request(body), body.to_vec());
    }

    #[test]
    fn no_messages_field_returns_input_unchanged() {
        let body = serde_json::to_vec(&serde_json::json!({"model": "x"})).unwrap();
        assert_eq!(compress_openai_request(&body), body);
    }

    #[test]
    fn anthropic_preserves_top_level_system() {
        let req = serde_json::json!({
            "model": "wire-test",
            "system": "you are helpful",
            "messages": [{"role": "user", "content": compressible_json(60)}]
        });
        let body = serde_json::to_vec(&req).unwrap();
        let out = compress_anthropic_request(&body);
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["system"], "you are helpful");
    }
}
