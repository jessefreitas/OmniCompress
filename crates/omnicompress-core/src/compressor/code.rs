//! AST-based code compressor for SP1.
//!
//! **SP1 supports Python only.** Content classified as `ContentKind::Code`
//! that is not valid Python (e.g. JavaScript, TypeScript, Rust, etc.) will
//! fail the `tree-sitter-python` parse step and fall through to
//! `Outcome::untouched` — fail-open, no panic, no data loss. Support for
//! additional languages is deferred to SP2+.

use super::{Compressor, Outcome};
use tree_sitter::{Node, Parser};

#[derive(Default)]
pub struct CodeCompressor {
    /// When `true`, code is left untouched (body elision is lossy and
    /// unrecoverable without a retrieve loop).
    pub lossless: bool,
}

impl Compressor for CodeCompressor {
    fn name(&self) -> &'static str {
        "code"
    }

    fn compress(&self, content: &str) -> Outcome {
        if self.lossless {
            return Outcome::untouched(content);
        }
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

    /// JavaScript content routed to CodeCompressor must fall through to
    /// `Outcome::untouched` (SP1 supports Python only). No panic must occur.
    #[test]
    fn javascript_falls_through_to_untouched() {
        // Build JS content long enough to exceed the min-size guard (400 chars).
        let js = "function processData(items) { return items.map(x => x * 2); }\n"
            .repeat(10);
        assert!(js.len() >= 400, "test precondition: content must exceed size guard");

        let out = CodeCompressor::default().compress(&js);

        // SP1 Python-only: non-Python code must return original=None (untouched).
        assert!(
            out.original.is_none(),
            "JavaScript must not be stored in CCR (original must be None); got original=Some(_)"
        );
    }
}
