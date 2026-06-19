//! AST-based code compressor.
//!
//! Keeps the structural skeleton of a source file — imports/use/package
//! declarations and the *signatures* of functions, classes, traits, etc. — while
//! eliding the bodies (recoverable from the CCR in aggressive mode). Supported
//! languages: **Python, JavaScript, TypeScript, Rust, Go**. The right grammar is
//! chosen by parsing with each candidate and picking the one that parses cleanly.
//! Content that no supported grammar accepts (e.g. Java, C++, or malformed code)
//! falls through to `Outcome::untouched` — fail-open, no panic, no data loss.

use super::{Compressor, Outcome};
use tree_sitter::{Language, Node, Parser, Tree};

/// Per-language description of which top-level AST nodes to keep verbatim, which
/// to reduce to "signature + elided body", and which to descend into.
struct LangSpec {
    /// Stable label recorded in the `Outcome::detail`.
    name: &'static str,
    /// The tree-sitter grammar.
    language: fn() -> Language,
    /// Node kinds emitted verbatim (imports, top-level consts, type aliases…).
    keep_verbatim: &'static [&'static str],
    /// Node kinds reduced to their signature, with the `body` field replaced by
    /// `body_marker` (functions, classes, impls, traits…).
    sig_then_body: &'static [&'static str],
    /// Wrapper node kinds whose children should be processed individually
    /// (e.g. `export function …` in JS/TS).
    unwrap: &'static [&'static str],
    /// Replacement emitted in place of an elided body.
    body_marker: &'static str,
}

impl LangSpec {
    fn recognizes(&self, kind: &str) -> bool {
        self.keep_verbatim.contains(&kind)
            || self.sig_then_body.contains(&kind)
            || self.unwrap.contains(&kind)
    }
}

fn python_lang() -> Language {
    tree_sitter_python::language()
}
fn javascript_lang() -> Language {
    tree_sitter_javascript::language()
}
fn typescript_lang() -> Language {
    tree_sitter_typescript::language_typescript()
}
fn rust_lang() -> Language {
    tree_sitter_rust::language()
}
fn go_lang() -> Language {
    tree_sitter_go::language()
}

/// The supported languages, in priority order (used only to break ties when two
/// grammars both parse a snippet with zero errors).
const SPECS: &[LangSpec] = &[
    LangSpec {
        name: "python",
        language: python_lang,
        keep_verbatim: &["import_statement", "import_from_statement", "expression_statement"],
        sig_then_body: &["function_definition", "class_definition"],
        unwrap: &["decorated_definition"],
        body_marker: "\n    ...\n",
    },
    LangSpec {
        name: "rust",
        language: rust_lang,
        keep_verbatim: &[
            "use_declaration",
            "extern_crate_declaration",
            "const_item",
            "static_item",
            "type_item",
            "struct_item",
            "enum_item",
            "union_item",
            "attribute_item",
        ],
        sig_then_body: &["function_item", "impl_item", "trait_item", "mod_item"],
        unwrap: &[],
        body_marker: " { ... }\n",
    },
    LangSpec {
        name: "go",
        language: go_lang,
        keep_verbatim: &[
            "package_clause",
            "import_declaration",
            "const_declaration",
            "var_declaration",
            "type_declaration",
        ],
        sig_then_body: &["function_declaration", "method_declaration"],
        unwrap: &[],
        body_marker: " { ... }\n",
    },
    LangSpec {
        name: "typescript",
        language: typescript_lang,
        keep_verbatim: &[
            "import_statement",
            "lexical_declaration",
            "variable_declaration",
            "type_alias_declaration",
            "expression_statement",
        ],
        sig_then_body: &[
            "function_declaration",
            "generator_function_declaration",
            "class_declaration",
            "abstract_class_declaration",
            "interface_declaration",
            "enum_declaration",
        ],
        unwrap: &["export_statement"],
        body_marker: " { ... }\n",
    },
    LangSpec {
        name: "javascript",
        language: javascript_lang,
        keep_verbatim: &[
            "import_statement",
            "lexical_declaration",
            "variable_declaration",
            "expression_statement",
        ],
        sig_then_body: &[
            "function_declaration",
            "generator_function_declaration",
            "class_declaration",
        ],
        unwrap: &["export_statement"],
        body_marker: " { ... }\n",
    },
];

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

        // Pick the grammar that parses this snippet most cleanly.
        let Some((spec, tree)) = pick_spec(content) else {
            return Outcome::untouched(content);
        };
        // Only compress code we actually understand: a clean parse. Anything with
        // syntax errors in every supported grammar (unsupported language, partial
        // snippet) falls through untouched — fail-open.
        if count_errors(tree.root_node()) > 0 {
            return Outcome::untouched(content);
        }

        let out = emit_skeleton(&tree, content.as_bytes(), spec);

        if out.is_empty() || out.len() >= content.len() {
            return Outcome::untouched(content);
        }

        Outcome {
            compressed: out,
            original: Some(content.to_string()),
            detail: format!("code_ast:{}", spec.name),
        }
    }
}

/// Parse `content` with every supported grammar and return the spec + tree with
/// the fewest syntax errors (ties broken by how many top-level nodes the spec
/// recognises, then by `SPECS` order). Returns `None` if nothing parses.
fn pick_spec(content: &str) -> Option<(&'static LangSpec, Tree)> {
    let mut best: Option<(usize, usize, &'static LangSpec, Tree)> = None;
    for spec in SPECS {
        let mut parser = Parser::new();
        if parser.set_language(&(spec.language)()).is_err() {
            continue;
        }
        let Some(tree) = parser.parse(content, None) else {
            continue;
        };
        let errors = count_errors(tree.root_node());
        let matched = count_matched_top_level(&tree, spec);
        let better = match &best {
            None => true,
            Some((best_err, best_matched, _, _)) => {
                errors < *best_err || (errors == *best_err && matched > *best_matched)
            }
        };
        if better {
            best = Some((errors, matched, spec, tree));
        }
    }
    best.map(|(_, _, spec, tree)| (spec, tree))
}

/// Count error/missing nodes anywhere in the subtree.
fn count_errors(node: Node) -> usize {
    let mut total = usize::from(node.is_error() || node.is_missing());
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        total += count_errors(child);
    }
    total
}

/// Count top-level children whose kind this spec knows how to handle.
fn count_matched_top_level(tree: &Tree, spec: &LangSpec) -> usize {
    let root = tree.root_node();
    let mut cursor = root.walk();
    root.children(&mut cursor)
        .filter(|c| spec.recognizes(c.kind()))
        .count()
}

/// Build the skeleton string by walking the top-level nodes.
fn emit_skeleton(tree: &Tree, src: &[u8], spec: &LangSpec) -> String {
    let mut out = String::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        emit_node(child, src, spec, &mut out);
    }
    out
}

/// Emit one node according to the language spec.
///
/// - `unwrap` kinds: recurse into children (e.g. `export …`).
/// - `keep_verbatim` kinds: copy the source text as-is.
/// - `sig_then_body` kinds: copy the text up to the `body` field, then the
///   elision marker. If there is no `body` field, keep the node whole.
/// - everything else (comments, decorators, blank lines): skipped.
fn emit_node(node: Node, src: &[u8], spec: &LangSpec, out: &mut String) {
    let kind = node.kind();

    if spec.unwrap.contains(&kind) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            emit_node(child, src, spec, out);
        }
        return;
    }

    if spec.keep_verbatim.contains(&kind) {
        if let Ok(text) = node.utf8_text(src) {
            out.push_str(text);
            out.push('\n');
        }
        return;
    }

    if spec.sig_then_body.contains(&kind) {
        if let Some(body) = node.child_by_field_name("body") {
            let sig = src
                .get(node.start_byte()..body.start_byte())
                .and_then(|b| std::str::from_utf8(b).ok())
                .unwrap_or("")
                .trim_end();
            out.push_str(sig);
            out.push_str(spec.body_marker);
        } else if let Ok(text) = node.utf8_text(src) {
            // No body to elide (e.g. an abstract signature) — keep it whole.
            out.push_str(text);
            out.push('\n');
        }
    }
    // Other kinds are intentionally skipped.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compressor::Compressor;

    fn compress(code: &str) -> Outcome {
        CodeCompressor::default().compress(code)
    }

    #[test]
    fn python_keeps_signatures_drops_bodies() {
        let code = "import os\n\ndef alpha(x):\n    y = x + 1\n    return y\n\ndef beta(a, b):\n    return a * b\n"
            .repeat(6);
        let out = compress(&code);
        assert_eq!(out.detail, "code_ast:python");
        assert!(out.original.is_some());
        assert!(out.compressed.contains("def alpha(x)"), "mantém assinatura");
        assert!(out.compressed.contains("import os"), "mantém imports");
        assert!(!out.compressed.contains("y = x + 1"), "elide corpo");
        assert!(out.compressed.len() < code.len());
    }

    #[test]
    fn javascript_keeps_signatures_drops_bodies() {
        let js = "import fs from 'fs';\n\nfunction processData(items) {\n  const out = items.map(x => x * 2);\n  return out;\n}\n\nclass Widget {\n  render() { return 1; }\n}\n"
            .repeat(4);
        let out = compress(&js);
        assert!(out.original.is_some(), "JS deve comprimir agora");
        assert!(out.detail.starts_with("code_ast:"), "detail: {}", out.detail);
        assert!(out.compressed.contains("function processData(items)"), "assinatura JS");
        assert!(!out.compressed.contains("items.map(x => x * 2)"), "elide corpo JS");
        assert!(out.compressed.len() < js.len());
    }

    #[test]
    fn typescript_keeps_signatures_and_interfaces() {
        let ts = "import { A } from './a';\n\ninterface User {\n  id: number;\n  name: string;\n}\n\nfunction greet(u: User): string {\n  const msg = `hi ${u.name}`;\n  return msg;\n}\n"
            .repeat(4);
        let out = compress(&ts);
        assert!(out.original.is_some(), "TS deve comprimir");
        assert!(out.compressed.contains("function greet(u: User): string"), "assinatura TS");
        assert!(!out.compressed.contains("hi ${u.name}"), "elide corpo TS");
        assert!(out.compressed.len() < ts.len());
    }

    #[test]
    fn rust_keeps_signatures_drops_bodies() {
        let rs = "use std::collections::HashMap;\n\npub fn add(a: i32, b: i32) -> i32 {\n    let s = a + b;\n    s\n}\n\nimpl Foo {\n    fn bar(&self) -> u8 {\n        42\n    }\n}\n"
            .repeat(4);
        let out = compress(&rs);
        assert_eq!(out.detail, "code_ast:rust");
        assert!(out.original.is_some());
        assert!(out.compressed.contains("pub fn add(a: i32, b: i32) -> i32"), "assinatura Rust");
        assert!(out.compressed.contains("use std::collections::HashMap"), "mantém use");
        assert!(!out.compressed.contains("let s = a + b"), "elide corpo Rust");
        assert!(out.compressed.len() < rs.len());
    }

    #[test]
    fn go_keeps_signatures_drops_bodies() {
        let go = "package main\n\nimport \"fmt\"\n\nfunc Add(a int, b int) int {\n\ts := a + b\n\treturn s\n}\n\nfunc greet(name string) {\n\tfmt.Println(name)\n}\n"
            .repeat(4);
        let out = compress(&go);
        assert_eq!(out.detail, "code_ast:go");
        assert!(out.original.is_some());
        assert!(out.compressed.contains("func Add(a int, b int) int"), "assinatura Go");
        assert!(out.compressed.contains("package main"), "mantém package");
        assert!(!out.compressed.contains("s := a + b"), "elide corpo Go");
        assert!(out.compressed.len() < go.len());
    }

    /// An unsupported language (Java) parses cleanly in NO supported grammar, so
    /// it must fall through to `untouched` — fail-open, no panic, no data loss.
    #[test]
    fn unsupported_language_falls_through() {
        let java = "public class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
            .repeat(8);
        assert!(java.len() >= 400);
        let out = compress(&java);
        assert!(
            out.original.is_none(),
            "Java não suportado deve passar intacto; detail={}",
            out.detail
        );
    }
}
