use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContentKind {
    Json,
    /// Line-oriented tabular data: NDJSON/JSONL or CSV/TSV.
    Tabular,
    Code,
    Log,
    Prose,
    Diff,
    Unknown,
}

/// A content block within a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub role: Role,
    pub content: String,
    /// Originating tool name, when applicable (e.g. "Bash", "search_memories").
    pub tool_name: Option<String>,
}

impl Block {
    /// Constructor for plain text blocks.
    pub fn from_text(role: Role, content: &str) -> Self {
        Block {
            role,
            content: content.to_string(),
            tool_name: None,
        }
    }

    /// Constructor for tool-result blocks.
    pub fn tool(role: Role, content: &str, tool: &str) -> Self {
        Block {
            role,
            content: content.to_string(),
            tool_name: Some(tool.to_string()),
        }
    }

    /// Returns the text content of the block.
    pub fn text(&self) -> &str {
        &self.content
    }
}

pub type Hash = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CcrRef {
    pub hash: Hash,
    pub original_tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub unit: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressResult {
    pub messages: Vec<Block>,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub transforms: Vec<Transform>,
    pub ccr_refs: Vec<CcrRef>,
}

impl CompressResult {
    pub fn tokens_saved(&self) -> usize {
        self.tokens_before.saturating_sub(self.tokens_after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_text_len_counts_chars() {
        let b = Block::from_text(Role::User, "abc");
        assert_eq!(b.text(), "abc");
        assert!(matches!(b.role, Role::User));
    }

    #[test]
    fn compress_result_saved_is_diff() {
        let r = CompressResult {
            messages: vec![],
            tokens_before: 100,
            tokens_after: 30,
            transforms: vec![],
            ccr_refs: vec![],
        };
        assert_eq!(r.tokens_saved(), 70);
    }
}
