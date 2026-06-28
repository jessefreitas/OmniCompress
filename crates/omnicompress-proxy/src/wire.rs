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

/// Collect every compressible text location in `messages`, in a deterministic
/// order, as `(role, text)`. Handles both the OpenAI shape (`content` is a string)
/// and the Anthropic shape (`content` is an array of blocks): `text` blocks and
/// `tool_result` blocks (string content, or a nested array of `text` blocks — where
/// the bulk of Claude's tool output lives). MUST mirror `apply_texts` exactly so the
/// compressed results map back to the same slots.
fn collect_texts(messages: &[Value]) -> Vec<(Role, String)> {
    let mut out = Vec::new();
    for msg in messages {
        let role = role_from(msg.get("role").and_then(Value::as_str).unwrap_or(""));
        let content = match msg.get("content") {
            Some(c) => c,
            None => continue,
        };
        if let Some(s) = content.as_str() {
            out.push((role, s.to_string()));
        } else if let Some(arr) = content.as_array() {
            for block in arr {
                let btype = block.get("type").and_then(Value::as_str).unwrap_or("");
                if btype == "text" {
                    if let Some(t) = block.get("text").and_then(Value::as_str) {
                        out.push((role, t.to_string()));
                    }
                } else if btype == "tool_result" {
                    match block.get("content") {
                        Some(tc) if tc.is_string() => {
                            out.push((role, tc.as_str().unwrap_or("").to_string()))
                        }
                        Some(tc) => {
                            if let Some(inner) = tc.as_array() {
                                for ib in inner {
                                    if ib.get("type").and_then(Value::as_str) == Some("text") {
                                        if let Some(itx) = ib.get("text").and_then(Value::as_str) {
                                            out.push((role, itx.to_string()));
                                        }
                                    }
                                }
                            }
                        }
                        None => {}
                    }
                }
            }
        }
    }
    out
}

/// Write `compressed[i]` back into each compressible slot, walking `messages` in the
/// SAME order as `collect_texts` (the alignment invariant — pinned by
/// `collect_and_apply_texts_stay_aligned`). serde_json has no `as_str_mut`, so a string
/// slot is replaced wholesale with `Value::String`. The walk is decomposed into helpers
/// that take the running index `i` by value and return the advanced index, keeping each
/// piece flat instead of one deeply-nested loop. Every helper only ever indexes
/// `compressed[i]` after an `i < compressed.len()` guard, so it never panics.
fn apply_texts(messages: &mut [Value], compressed: &[String]) {
    let mut i = 0;
    for msg in messages.iter_mut() {
        if i >= compressed.len() {
            break;
        }
        let content = match msg.get_mut("content") {
            Some(c) => c,
            None => continue,
        };
        i = apply_to_content(content, compressed, i);
    }
}

/// One message's `content`: a bare string (OpenAI) or an array of blocks (Anthropic).
fn apply_to_content(content: &mut Value, compressed: &[String], i: usize) -> usize {
    if content.is_string() {
        *content = Value::String(compressed[i].clone());
        i + 1
    } else if let Some(arr) = content.as_array_mut() {
        apply_to_blocks(arr, compressed, i)
    } else {
        i
    }
}

/// A block array — re-checks the budget before each block (mirrors the original loop).
fn apply_to_blocks(arr: &mut [Value], compressed: &[String], mut i: usize) -> usize {
    for block in arr.iter_mut() {
        if i >= compressed.len() {
            break;
        }
        i = apply_to_block(block, compressed, i);
    }
    i
}

/// Dispatch one block by `type`: `text` writes its `text` field; `tool_result` recurses;
/// any other type is left untouched (and does not consume a slot).
fn apply_to_block(block: &mut Value, compressed: &[String], i: usize) -> usize {
    match block.get("type").and_then(Value::as_str).unwrap_or("") {
        "text" => apply_text_field(block.get_mut("text"), compressed, i),
        "tool_result" => apply_to_tool_result(block.get_mut("content"), compressed, i),
        _ => i,
    }
}

/// A `tool_result` block's `content`: a string, or a nested array of `text` blocks.
fn apply_to_tool_result(content: Option<&mut Value>, compressed: &[String], i: usize) -> usize {
    match content {
        Some(tc) if tc.is_string() => {
            *tc = Value::String(compressed[i].clone());
            i + 1
        }
        Some(tc) => match tc.as_array_mut() {
            Some(inner) => apply_to_inner_blocks(inner, compressed, i),
            None => i,
        },
        None => i,
    }
}

/// The nested array inside a `tool_result` — only `text` blocks consume a slot.
fn apply_to_inner_blocks(inner: &mut [Value], compressed: &[String], mut i: usize) -> usize {
    for ib in inner.iter_mut() {
        if i >= compressed.len() {
            break;
        }
        if ib.get("type").and_then(Value::as_str) == Some("text") {
            i = apply_text_field(ib.get_mut("text"), compressed, i);
        }
    }
    i
}

/// Replace a string `text` slot in place. A missing or non-string slot consumes nothing
/// (returns `i` unchanged) — matching `collect_texts`, which only emits string slots.
fn apply_text_field(slot: Option<&mut Value>, compressed: &[String], i: usize) -> usize {
    match slot {
        Some(t) if t.is_string() => {
            *t = Value::String(compressed[i].clone());
            i + 1
        }
        _ => i,
    }
}

/// Compress the `messages` array in `body` in place, preserving all other fields.
/// Handles OpenAI (string content) and Anthropic (content-block arrays, incl. nested
/// `tool_result`). Fail-open: returns `body.to_vec()` unchanged on any parse/serialise
/// failure or when there is nothing compressible.
fn compress_messages_in_place(body: &[u8]) -> Vec<u8> {
    let mut root: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return body.to_vec(),
    };

    let collected = match root.get("messages").and_then(Value::as_array) {
        Some(arr) => collect_texts(arr),
        None => return body.to_vec(),
    };
    if collected.is_empty() {
        return body.to_vec();
    }

    let blocks: Vec<Block> = collected
        .into_iter()
        .map(|(role, content)| Block {
            role,
            content,
            tool_name: None,
        })
        .collect();

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
    let compressed: Vec<String> = result.messages.iter().map(|b| b.content.clone()).collect();

    if let Some(arr) = root.get_mut("messages").and_then(Value::as_array_mut) {
        apply_texts(arr, &compressed);
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

    #[test]
    fn anthropic_content_blocks_are_compressed() {
        // Anthropic shape: content is an ARRAY of blocks. The big tool_result is
        // where Claude's bloat lives — a string-only parser would skip it entirely.
        let req = serde_json::json!({
            "model": "claude-3",
            "messages": [
                { "role": "user", "content": [
                    { "type": "text", "text": "here are the search results" },
                    { "type": "tool_result", "content": compressible_json(60) }
                ]},
                { "role": "assistant", "content": "ok" }
            ]
        });
        let body = serde_json::to_vec(&req).unwrap();
        let out = compress_anthropic_request(&body);
        assert!(out.len() < body.len(), "block content must compress: {} vs {}", out.len(), body.len());

        // Structure preserved: still 2 blocks, tool_result still present.
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        let blocks = parsed["messages"][0]["content"].as_array().expect("content array");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1]["type"], "tool_result");
    }

    #[test]
    fn anthropic_nested_tool_result_array_is_compressed() {
        // tool_result.content can itself be an array of text blocks (common in Claude).
        let req = serde_json::json!({
            "model": "claude-3",
            "messages": [{ "role": "user", "content": [
                { "type": "tool_result", "content": [
                    { "type": "text", "text": compressible_json(60) }
                ]}
            ]}]
        });
        let body = serde_json::to_vec(&req).unwrap();
        let out = compress_anthropic_request(&body);
        assert!(out.len() < body.len(), "nested tool_result text must compress");
    }

    #[test]
    fn anthropic_cache_control_is_preserved() {
        // A block can carry `cache_control` (Anthropic's explicit prompt-cache
        // breakpoint). We only ever rewrite the `text`/`content` value, so sibling
        // fields survive — and under cache_stable the compressed bytes are stable
        // across turns, so the breakpoint still caches (on the smaller payload).
        let req = serde_json::json!({
            "model": "claude-3",
            "messages": [{ "role": "user", "content": [
                { "type": "text", "text": compressible_json(60),
                  "cache_control": { "type": "ephemeral" } }
            ]}]
        });
        let body = serde_json::to_vec(&req).unwrap();
        let out = compress_anthropic_request(&body);
        assert!(out.len() < body.len(), "marked block text should still compress");

        let parsed: Value = serde_json::from_slice(&out).unwrap();
        let block = &parsed["messages"][0]["content"][0];
        assert_eq!(block["type"], "text");
        assert_eq!(
            block["cache_control"]["type"], "ephemeral",
            "cache_control must survive compression"
        );
    }

    /// THE alignment invariant: `apply_texts` must write back into EXACTLY the slots
    /// `collect_texts` reads, in the same order — same count, no slot skipped or
    /// double-written. The two functions traverse the message tree independently; if
    /// they ever drift (e.g. a new block type handled by one but not the other), the
    /// proxy would silently write a compressed block into the wrong slot. Fail-open
    /// cannot catch that — it's a silent lossless violation. We pin it: stamp each
    /// collected slot with a unique sentinel via `apply_texts`, then re-collect and
    /// assert we read the sentinels back, one per slot, same order, same role.
    #[test]
    fn collect_and_apply_texts_stay_aligned() {
        // Diverse shapes exercising every branch shared by collect/apply:
        // OpenAI strings, a missing-content message (skipped), Anthropic text blocks
        // with a non-text sibling, tool_result string content, nested tool_result
        // text-block arrays with a non-text sibling, and a mixed message.
        let cases: Vec<serde_json::Value> = vec![
            serde_json::json!([
                {"role": "user", "content": "OpenAI first"},
                {"role": "assistant", "content": "OpenAI second"},
                {"role": "system", "content": "OpenAI third"}
            ]),
            serde_json::json!([
                {"role": "user", "content": "before"},
                {"role": "assistant"},
                {"role": "user", "content": "after"}
            ]),
            serde_json::json!([
                {"role": "user", "content": [
                    {"type": "text", "text": "array text first"},
                    {"type": "image", "source": {}},
                    {"type": "text", "text": "array text second"}
                ]}
            ]),
            serde_json::json!([
                {"role": "tool", "content": [
                    {"type": "tool_result", "content": "tool plain content"}
                ]}
            ]),
            serde_json::json!([
                {"role": "tool", "content": [
                    {"type": "tool_result", "content": [
                        {"type": "text", "text": "nested text first"},
                        {"type": "image", "source": {}},
                        {"type": "text", "text": "nested text second"}
                    ]}
                ]}
            ]),
            serde_json::json!([
                {"role": "user", "content": "outer first"},
                {"role": "assistant", "content": [
                    {"type": "text", "text": "mixed text"},
                    {"type": "tool_result", "content": "mixed tool plain"},
                    {"type": "tool_result", "content": [
                        {"type": "text", "text": "mixed nested first"},
                        {"type": "text", "text": "mixed nested second"}
                    ]}
                ]},
                {"role": "user", "content": "outer last"}
            ]),
        ];

        for (c, case) in cases.iter().enumerate() {
            let mut messages = case.as_array().unwrap().clone();
            let before = collect_texts(&messages);
            assert!(
                !before.is_empty(),
                "case {c}: collect_texts returned no slots for a non-empty case"
            );

            let sentinels: Vec<String> = (0..before.len())
                .map(|i| format!("__SLOT_{i}__"))
                .collect();

            apply_texts(&mut messages, &sentinels);

            let after = collect_texts(&messages);
            assert_eq!(
                after.len(),
                before.len(),
                "case {c}: collect/apply misaligned: slot count changed from {} to {}",
                before.len(),
                after.len()
            );

            for (i, ((before_role, _), (after_role, after_text))) in
                before.iter().zip(after.iter()).enumerate()
            {
                assert_eq!(
                    *after_text, sentinels[i],
                    "case {c}: collect/apply misaligned at slot {i}: expected sentinel `{}`, got `{}`",
                    sentinels[i], after_text
                );
                assert_eq!(
                    *after_role, *before_role,
                    "case {c}: collect/apply misaligned at slot {i}: role changed"
                );
            }
        }
    }
}
