use serde_json::{json, Value};
use std::sync::Arc;

use omnicompress_core::ccr::{CCRStore, MemoryStore};
use omnicompress_core::pipeline::CompressionPipeline;
use omnicompress_core::protection::CompressConfig;
use omnicompress_core::types::{Block, Role};

pub struct McpState {
    pub ccr: Arc<dyn CCRStore>,
    pub pipeline: CompressionPipeline,
}

impl McpState {
    pub fn new() -> Self {
        let ccr: Arc<dyn CCRStore> = Arc::new(MemoryStore::default());
        let pipeline = CompressionPipeline::new_arc(ccr.clone());
        McpState { ccr, pipeline }
    }
}

impl Default for McpState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn handle_jsonrpc(req: &str, state: &McpState) -> String {
    let parsed: Value = match serde_json::from_str(req) {
        Ok(v) => v,
        Err(_) => {
            return json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": "Parse error" }
            })
            .to_string();
        }
    };

    let id = parsed.get("id").cloned().unwrap_or(Value::Null);
    let method = parsed.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = parsed.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": [
                    {
                        "name": "omnicompress_compress",
                        "description": "Compress a list of chat messages using the OmniCompress pipeline, returning token savings and CCR references.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "messages": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "role": { "type": "string" },
                                            "content": { "type": "string" }
                                        },
                                        "required": ["role", "content"]
                                    }
                                }
                            },
                            "required": ["messages"]
                        }
                    },
                    {
                        "name": "omnicompress_retrieve",
                        "description": "Retrieve original content previously stored by compression, given a CCR hash.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "hash": { "type": "string" }
                            },
                            "required": ["hash"]
                        }
                    },
                    {
                        "name": "omnicompress_stats",
                        "description": "Return basic status information about the OmniCompress MCP server.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {}
                        }
                    }
                ]
            }
        })
        .to_string(),

        "tools/call" => {
            let name = params
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            match name {
                "omnicompress_compress" => {
                    let messages = arguments
                        .get("messages")
                        .and_then(|m| m.as_array())
                        .cloned()
                        .unwrap_or_default();

                    let mut blocks: Vec<Block> = Vec::with_capacity(messages.len());
                    for msg in messages {
                        let role_str = msg
                            .get("role")
                            .and_then(|r| r.as_str())
                            .unwrap_or("user");
                        let content = msg
                            .get("content")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string();
                        let role = match role_str {
                            "system" => Role::System,
                            "assistant" => Role::Assistant,
                            "tool" => Role::Tool,
                            _ => Role::User,
                        };
                        blocks.push(Block {
                            role,
                            content,
                            tool_name: None,
                        });
                    }

                    let config = CompressConfig::default();
                    let result = state.pipeline.compress(blocks, &config);

                    let ccr_refs: Vec<Value> = result
                        .ccr_refs
                        .iter()
                        .map(|r| json!({ "hash": r.hash }))
                        .collect();

                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "tokens_before": result.tokens_before,
                            "tokens_after": result.tokens_after,
                            "tokens_saved": result.tokens_saved(),
                            "ccr_refs": ccr_refs
                        }
                    })
                    .to_string()
                }

                "omnicompress_retrieve" => {
                    let hash = arguments
                        .get("hash")
                        .and_then(|h| h.as_str())
                        .unwrap_or("");

                    let content = match state.ccr.get(hash) {
                        Ok(Some(bytes)) => {
                            match String::from_utf8(bytes) {
                                Ok(s) => Value::String(s),
                                Err(_) => Value::Null,
                            }
                        }
                        Ok(None) => Value::Null,
                        Err(_) => Value::Null,
                    };

                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": content
                        }
                    })
                    .to_string()
                }

                "omnicompress_stats" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "status": "ok" }
                })
                .to_string(),

                _ => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": "Method not found: unknown tool" }
                })
                .to_string(),
            }
        }

        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": "Method not found" }
        })
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn test_compress_retrieve_roundtrip() {
        let state = McpState::new();

        // Build a large content: array JSON with >=60 objects.
        let mut items: Vec<Value> = Vec::new();
        for i in 0..60 {
            items.push(json!({
                "id": i,
                "name": format!("item_{}", i),
                "description": format!("This is a long description for item number {} to ensure enough tokens.", i),
                "value": i * 2,
            }));
        }
        let content = serde_json::to_string(&items).unwrap();
        assert!(items.len() >= 60);

        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "omnicompress_compress",
                "arguments": {
                    "messages": [
                        { "role": "user", "content": content },
                        { "role": "assistant", "content": "ok 0" },
                        { "role": "user", "content": "next 1" },
                        { "role": "assistant", "content": "ok 2" },
                        { "role": "user", "content": "next 3" },
                        { "role": "assistant", "content": "ok 4" }
                    ]
                }
            }
        })
        .to_string();

        let resp = handle_jsonrpc(&req, &state);
        let resp_val: Value = serde_json::from_str(&resp).expect("response must be valid JSON");

        assert_eq!(resp_val["jsonrpc"], "2.0");
        assert_eq!(resp_val["id"], 1);
        assert!(resp_val["result"].is_object(), "expected result object, got: {}", resp);

        let ccr_refs = resp_val["result"]["ccr_refs"].as_array().expect("ccr_refs array");
        assert!(!ccr_refs.is_empty(), "expected at least one ccr_ref");
        let hash = ccr_refs[0]["hash"]
            .as_str()
            .expect("hash string")
            .to_string();

        // Retrieve
        let req2 = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "omnicompress_retrieve",
                "arguments": { "hash": hash }
            }
        })
        .to_string();

        let resp2 = handle_jsonrpc(&req2, &state);
        let resp2_val: Value = serde_json::from_str(&resp2).expect("retrieve response JSON");

        assert_eq!(resp2_val["id"], 2);
        assert!(resp2_val["result"]["content"].is_string(), "content should be a string, got: {}", resp2);
        let content_str = resp2_val["result"]["content"]
            .as_str()
            .expect("content string");
        assert!(!content_str.is_empty(), "retrieved content should not be empty");
        // Confirm it contains part of the original
        assert!(
            content_str.contains("item_") || content_str.contains("description"),
            "retrieved content should contain part of original"
        );
    }

    #[test]
    fn test_retrieve_invalid_hash() {
        let state = McpState::new();

        let req = json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "tools/call",
            "params": {
                "name": "omnicompress_retrieve",
                "arguments": { "hash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef" }
            }
        })
        .to_string();

        let resp = handle_jsonrpc(&req, &state);
        let resp_val: Value = serde_json::from_str(&resp).expect("JSON");
        assert_eq!(resp_val["id"], 42);
        assert!(resp_val["result"].is_object(), "should return result, not error");
        assert_eq!(resp_val["result"]["content"], Value::Null, "content should be null on miss");
    }

    #[test]
    fn test_parse_error() {
        let state = McpState::new();
        let resp = handle_jsonrpc("not json", &state);
        let resp_val: Value = serde_json::from_str(&resp).expect("error response JSON");
        assert_eq!(resp_val["jsonrpc"], "2.0");
        assert_eq!(resp_val["id"], Value::Null);
        assert_eq!(resp_val["error"]["code"], -32700);
    }

    #[test]
    fn test_tools_list() {
        let state = McpState::new();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/list"
        })
        .to_string();
        let resp = handle_jsonrpc(&req, &state);
        let v: Value = serde_json::from_str(&resp).expect("JSON");
        assert_eq!(v["id"], 7);
        let tools = v["result"]["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"omnicompress_compress"));
        assert!(names.contains(&"omnicompress_retrieve"));
        assert!(names.contains(&"omnicompress_stats"));
    }

    #[test]
    fn test_unknown_method() {
        let state = McpState::new();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "foo/bar"
        })
        .to_string();
        let resp = handle_jsonrpc(&req, &state);
        let v: Value = serde_json::from_str(&resp).expect("JSON");
        assert_eq!(v["id"], 99);
        assert_eq!(v["error"]["code"], -32601);
    }

    #[test]
    fn test_stats() {
        let state = McpState::new();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": { "name": "omnicompress_stats", "arguments": {} }
        })
        .to_string();
        let resp = handle_jsonrpc(&req, &state);
        let v: Value = serde_json::from_str(&resp).expect("JSON");
        assert_eq!(v["id"], 5);
        assert_eq!(v["result"]["status"], "ok");
    }

    #[test]
    fn test_unknown_tool() {
        let state = McpState::new();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": { "name": "nope_tool", "arguments": {} }
        })
        .to_string();
        let resp = handle_jsonrpc(&req, &state);
        let v: Value = serde_json::from_str(&resp).expect("JSON");
        assert_eq!(v["id"], 11);
        assert_eq!(v["error"]["code"], -32601);
    }
}
