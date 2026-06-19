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

    // Find existing system message
    let mut system_idx: Option<usize> = None;
    for (i, msg) in messages.iter().enumerate() {
        if let Some(role) = msg.get("role").and_then(|r| r.as_str()) {
            if role == "system" {
                system_idx = Some(i);
                break;
            }
        }
    }

    if let Some(idx) = system_idx {
        let msg = &mut messages[idx];
        match msg.get_mut("content") {
            Some(Value::String(s)) => {
                if !ends_with_instruction(s) {
                    s.push_str("\n\n");
                    s.push_str(TERSE_INSTRUCTION);
                }
            }
            Some(Value::Array(arr)) => {
                // Multimodal content: check last string part for idempotency.
                let mut already_present = false;
                if let Some(last) = arr.last() {
                    if let Some(t) = last.get("type").and_then(|v| v.as_str()) {
                        if t == "text" {
                            if let Some(txt) = last.get("text").and_then(|v| v.as_str()) {
                                if ends_with_instruction(txt) {
                                    already_present = true;
                                }
                            }
                        }
                    }
                }
                if !already_present {
                    let new_part = serde_json::json!({
                        "type": "text",
                        "text": TERSE_INSTRUCTION
                    });
                    arr.push(new_part);
                }
            }
            _ => {
                // content missing or unexpected type: set as string.
                let new_content = TERSE_INSTRUCTION.to_string();
                msg["content"] = Value::String(new_content);
            }
        }
    } else {
        // Insert at the beginning of the array.
        let system_msg = serde_json::json!({
            "role": "system",
            "content": TERSE_INSTRUCTION
        });
        messages.insert(0, system_msg);
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

    match obj.get_mut("system") {
        Some(Value::String(s)) => {
            if !ends_with_instruction(s) {
                s.push_str("\n\n");
                s.push_str(TERSE_INSTRUCTION);
            }
        }
        Some(Value::Array(arr)) => {
            // Anthropic system can be an array of content blocks.
            let mut already_present = false;
            if let Some(last) = arr.last() {
                if let Some(t) = last.get("type").and_then(|v| v.as_str()) {
                    if t == "text" {
                        if let Some(txt) = last.get("text").and_then(|v| v.as_str()) {
                            if ends_with_instruction(txt) {
                                already_present = true;
                            }
                        }
                    }
                }
            }
            if !already_present {
                let new_block = serde_json::json!({
                    "type": "text",
                    "text": TERSE_INSTRUCTION
                });
                arr.push(new_block);
            }
        }
        Some(_) => {
            // Unexpected type: overwrite with string.
            obj.insert(
                "system".to_string(),
                Value::String(TERSE_INSTRUCTION.to_string()),
            );
        }
        None => {
            obj.insert(
                "system".to_string(),
                Value::String(TERSE_INSTRUCTION.to_string()),
            );
        }
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
}
