use serde_json::{Value};

pub const TERSE_INSTRUCTION: &str = "Be terse: drop filler and preambles, use fragments over full sentences, do not restate the question or already-shown code. Preserve all technical accuracy and required detail.";

pub fn enabled() -> bool {
    match std::env::var("OMNICOMPRESS_OUTPUT_SHAPER") {
        Ok(v) => {
            let lower = v.to_ascii_lowercase();
            lower == "1" || lower == "true"
        }
        Err(_) => false,
    }
}

fn ends_with_instruction(content: &str) -> bool {
    content.trim_end().ends_with(TERSE_INSTRUCTION)
}

/// Index of the first `system`-role message, if any.
fn find_system_idx(messages: &[Value]) -> Option<usize> {
    messages
        .iter()
        .position(|msg| msg.get("role").and_then(Value::as_str) == Some("system"))
}

/// True if the last block of a content array is already the terse instruction —
/// the idempotency check for multimodal/array content.
fn array_already_has_instruction(arr: &[Value]) -> bool {
    let last = match arr.last() {
        Some(l) => l,
        None => return false,
    };
    if last.get("type").and_then(Value::as_str) != Some("text") {
        return false;
    }
    last.get("text")
        .and_then(Value::as_str)
        .is_some_and(ends_with_instruction)
}

/// Append the terse instruction to a content slot, idempotently. Shared by the OpenAI
/// system-message `content` and the Anthropic top-level `system`:
/// - a string gets the instruction appended (once);
/// - an array gets a trailing `text` block (once);
/// - any other type (or `Null`, e.g. a key that was absent) is replaced with the
///   instruction as a plain string — matching the original per-surface behaviour.
fn append_instruction(slot: &mut Value) {
    match slot {
        Value::String(s) => {
            if !ends_with_instruction(s) {
                s.push_str("\n\n");
                s.push_str(TERSE_INSTRUCTION);
            }
        }
        Value::Array(arr) => {
            if !array_already_has_instruction(arr) {
                arr.push(serde_json::json!({ "type": "text", "text": TERSE_INSTRUCTION }));
            }
        }
        _ => *slot = Value::String(TERSE_INSTRUCTION.to_string()),
    }
}

pub fn shape_openai(body: &[u8]) -> Vec<u8> {
    let mut parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return body.to_vec(),
    };
    let obj = match parsed.as_object_mut() {
        Some(o) => o,
        None => return body.to_vec(),
    };
    let messages = match obj.get_mut("messages") {
        Some(Value::Array(arr)) => arr,
        _ => return body.to_vec(),
    };

    if let Some(idx) = find_system_idx(messages) {
        // Indexing a missing "content" key inserts Null, which append_instruction's
        // catch-all turns into the instruction string — the original behaviour.
        append_instruction(&mut messages[idx]["content"]);
    } else {
        messages.insert(
            0,
            serde_json::json!({ "role": "system", "content": TERSE_INSTRUCTION }),
        );
    }

    serde_json::to_vec(&parsed).unwrap_or_else(|_| body.to_vec())
}

pub fn shape_anthropic(body: &[u8]) -> Vec<u8> {
    let mut parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return body.to_vec(),
    };
    let obj = match parsed.as_object_mut() {
        Some(o) => o,
        None => return body.to_vec(),
    };

    if let Some(slot) = obj.get_mut("system") {
        append_instruction(slot);
    } else {
        obj.insert(
            "system".to_string(),
            Value::String(TERSE_INSTRUCTION.to_string()),
        );
    }

    serde_json::to_vec(&parsed).unwrap_or_else(|_| body.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn parse(bytes: &[u8]) -> Value {
        serde_json::from_slice(bytes).expect("valid json in test")
    }

    #[test]
    fn openai_no_system_inserts_first() {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-x",
            "messages": [
                {"role": "user", "content": "hi"}
            ]
        })).unwrap();
        let out = shape_openai(&body);
        let v = parse(&out);
        let msgs = v.get("messages").unwrap().as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].get("role").unwrap().as_str(), Some("system"));
        let content = msgs[0].get("content").unwrap().as_str().unwrap();
        assert_eq!(content, TERSE_INSTRUCTION);
        assert_eq!(msgs[1].get("role").unwrap().as_str(), Some("user"));
    }

    #[test]
    fn openai_existing_system_appends() {
        let body = serde_json::to_vec(&json!({
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "hi"}
            ]
        })).unwrap();
        let out = shape_openai(&body);
        let v = parse(&out);
        let msgs = v.get("messages").unwrap().as_array().unwrap();
        let content = msgs[0].get("content").unwrap().as_str().unwrap();
        assert!(content.starts_with("You are helpful."));
        assert!(content.contains(TERSE_INSTRUCTION));
    }

    #[test]
    fn openai_idempotent() {
        let body = serde_json::to_vec(&json!({
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "hi"}
            ]
        })).unwrap();
        let once = shape_openai(&body);
        let twice = shape_openai(&once);
        let v = parse(&twice);
        let content = v
            .get("messages")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .get("content")
            .unwrap()
            .as_str()
            .unwrap();
        let count = content.matches(TERSE_INSTRUCTION).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn anthropic_no_system_creates() {
        let body = serde_json::to_vec(&json!({
            "model": "claude-x",
            "messages": [
                {"role": "user", "content": "hi"}
            ]
        })).unwrap();
        let out = shape_anthropic(&body);
        let v = parse(&out);
        let system = v.get("system").unwrap().as_str().unwrap();
        assert_eq!(system, TERSE_INSTRUCTION);
    }

    #[test]
    fn anthropic_existing_system_appends() {
        let body = serde_json::to_vec(&json!({
            "system": "Original system prompt.",
            "messages": [
                {"role": "user", "content": "hi"}
            ]
        })).unwrap();
        let out = shape_anthropic(&body);
        let v = parse(&out);
        let system = v.get("system").unwrap().as_str().unwrap();
        assert!(system.starts_with("Original system prompt."));
        assert!(system.contains(TERSE_INSTRUCTION));
    }

    #[test]
    fn fail_open_invalid_json() {
        let body = b"not json";
        let out_openai = shape_openai(body);
        assert_eq!(out_openai, body.to_vec());
        let out_anthropic = shape_anthropic(body);
        assert_eq!(out_anthropic, body.to_vec());
    }

    // ── Multimodal (array) system content — previously untested; these characterise
    //    the array branches so the DRY refactor of shape_openai/shape_anthropic is
    //    guarded against behaviour drift. ──────────────────────────────────────────

    #[test]
    fn openai_system_array_appends_text_block() {
        let body = serde_json::to_vec(&json!({
            "messages": [
                {"role": "system", "content": [{"type": "text", "text": "base"}]},
                {"role": "user", "content": "hi"}
            ]
        }))
        .unwrap();
        let out = shape_openai(&body);
        let v = parse(&out);
        let blocks = v["messages"][0]["content"]
            .as_array()
            .expect("content stays an array");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks.last().unwrap()["type"], "text");
        assert_eq!(blocks.last().unwrap()["text"], TERSE_INSTRUCTION);
    }

    #[test]
    fn openai_system_array_idempotent() {
        let body = serde_json::to_vec(&json!({
            "messages": [
                {"role": "system", "content": [{"type": "text", "text": "base"}]},
                {"role": "user", "content": "hi"}
            ]
        }))
        .unwrap();
        let twice = shape_openai(&shape_openai(&body));
        let v = parse(&twice);
        let blocks = v["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2, "must not append a second instruction block");
        let n = blocks
            .iter()
            .filter(|b| b["text"] == TERSE_INSTRUCTION)
            .count();
        assert_eq!(n, 1);
    }

    #[test]
    fn openai_system_missing_content_sets_string() {
        let body = serde_json::to_vec(&json!({
            "messages": [
                {"role": "system"},
                {"role": "user", "content": "hi"}
            ]
        }))
        .unwrap();
        let out = shape_openai(&body);
        let v = parse(&out);
        assert_eq!(v["messages"][0]["content"].as_str(), Some(TERSE_INSTRUCTION));
    }

    #[test]
    fn anthropic_system_array_appends_text_block() {
        let body = serde_json::to_vec(&json!({
            "system": [{"type": "text", "text": "base"}],
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .unwrap();
        let out = shape_anthropic(&body);
        let v = parse(&out);
        let blocks = v["system"].as_array().expect("system stays an array");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks.last().unwrap()["text"], TERSE_INSTRUCTION);
    }

    #[test]
    fn anthropic_system_array_idempotent() {
        let body = serde_json::to_vec(&json!({
            "system": [{"type": "text", "text": "base"}],
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .unwrap();
        let twice = shape_anthropic(&shape_anthropic(&body));
        let v = parse(&twice);
        let blocks = v["system"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        let n = blocks
            .iter()
            .filter(|b| b["text"] == TERSE_INSTRUCTION)
            .count();
        assert_eq!(n, 1);
    }
}
