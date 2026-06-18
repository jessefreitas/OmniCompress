use super::{Compressor, Outcome};
use tree_sitter::{Node, Parser};

#[derive(Default)]
pub struct CodeCompressor;

impl Compressor for CodeCompressor {
    fn name(&self) -> &'static str {
        "code"
    }

    fn compress(&self, content: &str) -> Outcome {
        // Only bother for payloads large enough to benefit (~100 tokens).
        if content.len() < 400 {
            return Outcome::untouched(content);
        }

        let mut parser = Parser::new();
        // tree-sitter-python 0.21 exports language() → Language
        if parser.set_language(&tree_sitter_python::language()).is_err() {
            return Outcome::untouched(content);
        }

        let Some(tree) = parser.parse(content, None) else {
            return Outcome::untouched(content);
        };

        let src = content.as_bytes();
        let mut out = String::new();
        let mut cursor = tree.root_node().walk();

        for child in tree.root_node().children(&mut cursor) {
            emit_top_level(child, src, &mut out);
        }

        if out.is_empty() || out.len() >= content.len() {
            return Outcome::untouched(content);
        }

        Outcome {
            compressed: out,
            original: Some(content.to_string()),
            detail: "code_ast".into(),
        }
    }
}

/// Emit the relevant portion of a top-level AST node into `out`.
///
/// - Imports and top-level expression statements are kept verbatim.
/// - Function and class definitions: only the signature line is kept;
///   the body is replaced with a marker so the reader knows it is in the CCR.
fn emit_top_level(node: Node, src: &[u8], out: &mut String) {
    match node.kind() {
        "import_statement" | "import_from_statement" | "expression_statement" => {
            let text = node.utf8_text(src).unwrap_or("");
            out.push_str(text);
            out.push('\n');
        }
        "function_definition" | "class_definition" => {
            let full = node.utf8_text(src).unwrap_or("");
            // The signature ends at the first ":\n"; everything after is the body.
            let sig = full
                .split_once(":\n")
                .map(|(head, _)| head)
                .unwrap_or(full);
            out.push_str(sig);
            out.push_str(":\n    ...\n");
        }
        // Decorators, comments, blank lines — skip silently.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressor::Compressor;

    #[test]
    fn keeps_signatures_drops_bodies() {
        let code = "import os\n\ndef alpha(x):\n    y = x + 1\n    return y\n\ndef beta(a, b):\n    return a * b\n"
            .repeat(6);
        let out = CodeCompressor::default().compress(&code);
        assert!(out.original.is_some());
        assert!(out.compressed.contains("def alpha(x)"), "mantém assinatura");
        assert!(out.compressed.contains("import os"), "mantém imports");
        assert!(out.compressed.len() < code.len());
    }
}
