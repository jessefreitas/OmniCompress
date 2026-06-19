use std::collections::HashMap;

use crate::types::Block;

const DEDUP_MARKER: &str = "[omnicompress: identical to a later message, omitted]";

pub fn dedup_repeated(messages: Vec<Block>, min_len: usize) -> Vec<Block> {
    let mut last_index: HashMap<String, usize> = HashMap::new();

    for (i, block) in messages.iter().enumerate() {
        if block.content.len() >= min_len {
            last_index.insert(block.content.clone(), i);
        }
    }

    messages
        .into_iter()
        .enumerate()
        .map(|(i, mut block)| {
            if block.content.len() >= min_len {
                if let Some(&last) = last_index.get(&block.content) {
                    if last != i {
                        block.content = DEDUP_MARKER.to_string();
                    }
                }
            }
            block
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::types::{Block, Role};
    use super::dedup_repeated;

    fn make_block(role: Role, content: &str) -> Block {
        Block {
            role,
            content: content.to_string(),
            tool_name: None,
        }
    }

    #[test]
    fn test_dedup_long_identical() {
        let long_content = "This is a very long content that definitely exceeds the minimum length required for deduplication.";
        let messages = vec![
            make_block(Role::User, long_content),
            make_block(Role::Assistant, "Different content here."),
            make_block(Role::User, long_content),
        ];
        
        let result = dedup_repeated(messages, 20);
        
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].content, "[omnicompress: identical to a later message, omitted]");
        assert_eq!(result[1].content, "Different content here.");
        assert_eq!(result[2].content, long_content);
    }

    #[test]
    fn test_no_dedup_short() {
        let messages = vec![
            make_block(Role::User, "ok"),
            make_block(Role::Assistant, "ok"),
        ];
        
        let result = dedup_repeated(messages, 10);
        
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "ok");
        assert_eq!(result[1].content, "ok");
    }

    #[test]
    fn test_no_repetition() {
        let messages = vec![
            make_block(Role::User, "First long message that is over the limit"),
            make_block(Role::Assistant, "Second long message that is also over the limit"),
        ];
        
        let result = dedup_repeated(messages, 10);
        
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "First long message that is over the limit");
        assert_eq!(result[1].content, "Second long message that is also over the limit");
    }
}
