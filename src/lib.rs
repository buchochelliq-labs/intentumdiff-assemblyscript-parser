//! AssemblyScript parser plugin — full-parse mode.
//!
//! Handles `.as` files (AssemblyScript — TypeScript subset that compiles to Wasm).
//! Uses tree-sitter-typescript directly (no Python grammar package needed).
//!
//! NOTE: `.as` is also used by ActionScript; content-based detection at the Python
//! layer is recommended (AssemblyScript files import from `"assemblyscript"` and/or
//! use numeric type annotations like `i32`, `f64`, `v128`).
//!
//! AssemblyScript is a strict TypeScript subset so the TypeScript grammar parses it
//! correctly.  Class/interface/enum nodes map to class-like; function/method nodes
//! to method-like.

use intentumdiff_plugin_sdk::ts_convert::{convert_ts_direct, TsDirectHooks};
use intentumdiff_plugin_sdk::tree::{SemanticNode, SemanticNodeBuilder};

wit_bindgen::generate!({
    path: "wit/plugin.wit",
    world: "parser-plugin",
});

use crate::exports::intentdiff::plugin::parser::ExamplePair;
use crate::exports::intentdiff::plugin::parser::Guest;
use crate::exports::intentdiff::plugin::parser::LanguageInfoRecord;
use crate::exports::intentdiff::plugin::parser::ParserMode;

const PLUGIN_METADATA: &str = include_str!("../plugin_metadata.info");

fn language_info_for(ids: Vec<String>) -> Vec<LanguageInfoRecord> {
    let metadata = intentumdiff_plugin_sdk::metadata::parse_plugin_metadata(PLUGIN_METADATA);
    ids.into_iter()
        .map(|language_id| {
            let info = metadata.language_or_default(&language_id);
            LanguageInfoRecord {
                language_id: info.language_id,
                language_name: info.language_name,
                language_short_name: info.language_short_name,
                monaco_language: info.monaco_language,
                default_filename: info.default_filename,
                language_file_extensions: info.language_file_extensions,
                author: metadata.author().to_string(),
                plugin_version: metadata.plugin_version().to_string(),
                last_updated: metadata.last_updated().to_string(),
            }
        })
        .collect()
}
struct AssemblyScriptParser;

const TRIVIA: &[&str] = &["comment", "line_comment", "block_comment", "html_comment"];

fn is_class_like(node_type: &str) -> bool {
    matches!(
        node_type,
        "class_declaration"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "type_alias_declaration"
            | "ambient_declaration"
            | "ambient_module_declaration"
            | "module_declaration"
            | "namespace_declaration"
    )
}

fn is_method_like(node_type: &str) -> bool {
    matches!(
        node_type,
        "function_declaration"
            | "method_definition"
            | "generator_function_declaration"
            | "arrow_function"
            | "function_signature"
            | "method_signature"
            | "abstract_method_signature"
            | "constructor_declaration"
            | "lexical_declaration"
            | "variable_declaration"
    )
}

fn label_for_ts(node: tree_sitter::Node<'_>, source: &[u8]) -> String {
    let txt = |n: tree_sitter::Node<'_>| n.utf8_text(source).unwrap_or("").to_string();
    // Leaves label with their SOURCE TEXT (#46): with the generic "(anonymous)" label a
    // numeric-literal bump left both sides identical and hashed style-only.
    if node.child_count() == 0 {
        let text = txt(node);
        if !text.trim().is_empty() {
            return text.chars().take(120).collect();
        }
    }
    for i in 0..node.child_count() {
        let c = node.child(i).unwrap();
        match c.kind() {
            "identifier"
            | "type_identifier"
            | "property_identifier"
            | "private_property_identifier"
            | "shorthand_property_identifier" => return txt(c),
            "variable_declarator" => {
                for j in 0..c.child_count() {
                    let gc = c.child(j).unwrap();
                    if matches!(
                        gc.kind(),
                        "identifier" | "type_identifier" | "property_identifier"
                    ) {
                        return txt(gc);
                    }
                }
            }
            _ => {}
        }
    }
    "(anonymous)".to_string()
}

fn convert_ts(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    id_prefix: &str,
    parent_class: Option<&str>,
) -> Option<SemanticNode> {
    convert_ts_direct(
        node,
        source,
        id_prefix,
        parent_class,
        &TsDirectHooks {
            is_trivia: &|kind| TRIVIA.contains(&kind),
            class_label: &|n, s| is_class_like(n.kind()).then(|| label_for_ts(n, s)),
            // Statements and literal leaves are semantic (#46): the old class/method-only
            // predicate dropped whole bodies from the tree, so a numeric-literal bump
            // hashed style-only (the corpus mutation finding).
            keep_childless: &|n| {
                is_class_like(n.kind())
                    || is_method_like(n.kind())
                    || matches!(
                        n.kind(),
                        // Tree-sitter exposes binary operators as anonymous leaf tokens.
                        // Without them, n*n -> n**2 leaves an identical return_statement
                        // scaffold and the semantic engine cannot report the expression edit.
                        "*" | "**"
                            | "number"
                            | "string"
                            | "template_string"
                            | "true"
                            | "false"
                            | "null"
                            | "regex"
                            | "expression_statement"
                            | "return_statement"
                            | "lexical_declaration"
                            | "variable_declaration"
                            | "if_statement"
                            | "for_statement"
                            | "while_statement"
                            | "do_statement"
                            | "throw_statement"
                            | "try_statement"
                            | "break_statement"
                            | "continue_statement"
                    )
            },
            unwrap_single: &|n, _| n.kind() == "export_statement",
            label: &|n, s| label_for_ts(n, s),
            is_method_like: &|n| is_method_like(n.kind()),
        },
    )
}

fn process_impl(source: &str) -> String {
    let mut parser = tree_sitter::Parser::new();
    let lang = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    if parser.set_language(&lang).is_err() {
        return r#"{"error":"Failed to load TypeScript grammar"}"#.to_string();
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return r#"{"error":"Parse failed"}"#.to_string(),
    };
    let root = tree.root_node();
    match convert_ts(root, source.as_bytes(), "0", None) {
        Some(n) => serde_json::to_string(&n).unwrap_or_else(|e| format!(r#"{{"error":"{}"}}"#, e)),
        None => r#"{"error":"Empty semantic tree"}"#.to_string(),
    }
}

impl Guest for AssemblyScriptParser {
    fn get_parser_mode() -> ParserMode {
        ParserMode::FullParse
    }
    fn grammar_id() -> String {
        "assemblyscript".to_string()
    }
    fn detect_language(filename: String, content: String) -> String {
        if !filename.to_lowercase().ends_with(".as") {
            return String::new();
        }
        // AssemblyScript heuristics vs ActionScript (.as conflict):
        // AS files typically import from "assemblyscript" or use Wasm numeric types.
        if content.contains("\"assemblyscript\"")
            || content.contains(": i32")
            || content.contains(": i64")
            || content.contains(": f32")
            || content.contains(": f64")
            || content.contains(": v128")
            || content.contains("@external")
            || content.contains("@inline")
        {
            return "assemblyscript".to_string();
        }
        String::new()
    }
    fn preprocess_source(source: String) -> String {
        source
    }
    fn example(_language: String) -> ExamplePair {
        ExamplePair {
            old: "export function add(a: i32, b: i32): i32 {\n  return a + b;\n}\n\nexport function square(n: i32): i32 {\n  return n * n;\n}\n".to_string(),
            new: "export function add(a: i32, b: i32): i32 {\n  return a + b;\n}\n\nexport function square(n: i32): i32 {\n  return n ** 2;\n}\n\nexport function cube(n: i32): i32 {\n  return n ** 3;\n}\n".to_string(),
        }
    }
    fn process(input: String, _language: String, _filename: String) -> String {
        process_impl(&input)
    }
    fn trivia_node_types() -> Vec<String> {
        TRIVIA.iter().map(|s| s.to_string()).collect()
    }
    fn language_ids() -> Vec<String> {
        vec!["assemblyscript".to_string()]
    }
    fn language_info() -> Vec<LanguageInfoRecord> {
        language_info_for(Self::language_ids())
    }
    fn priority() -> i32 {
        0
    }
}

export!(AssemblyScriptParser);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exports::intentdiff::plugin::parser::Guest;
    use intentumdiff_plugin_sdk::testing as t;

    #[test]
    fn grammar_id_nonempty() {
        assert!(!AssemblyScriptParser::grammar_id().is_empty());
    }

    #[test]
    fn language_ids_contain_grammar_id() {
        let gid = AssemblyScriptParser::grammar_id();
        let ids = AssemblyScriptParser::language_ids();
        assert!(
            ids.contains(&gid),
            "language_ids {:?} must contain {:?}",
            ids,
            gid
        );
    }

    #[test]
    fn detect_language_known_ext() {
        let r = AssemblyScriptParser::detect_language(
            "test.as".to_string(),
            "export function add(a: i32, b: i32): i32 { return a + b; }".to_string(),
        );
        assert_eq!(r.as_str(), "assemblyscript");
    }

    #[test]
    fn detect_language_unknown_ext() {
        let r = AssemblyScriptParser::detect_language(
            "test.xyz_notareal_ext_9z8y".to_string(),
            "".to_string(),
        );
        assert_eq!(r.as_str(), "");
    }

    #[test]
    fn process_impl_empty_returns_valid_json() {
        let out = process_impl("");
        t::assert_valid_json(&out, "process(empty)");
    }

    #[test]
    fn process_impl_whitespace_returns_valid_json() {
        let out = process_impl("   \n  ");
        t::assert_valid_json(&out, "process(whitespace)");
    }


    /// Operator identity must survive in every arithmetic family, not just
    /// the original multiplication/exponent test. These are real expressions.
    #[test]
    fn arithmetic_operator_replacements_preserve_semantic_identity() {
        fn find_token<'a>(
            node: &'a serde_json::Value, token: &str,
        ) -> Option<&'a serde_json::Value> {
            if node.get("label").and_then(serde_json::Value::as_str) == Some(token) {
                return Some(node);
            }
            node.get("children")
                .and_then(serde_json::Value::as_array)
                .and_then(|children| children.iter().find_map(|child| find_token(child, token)))
        }

        for (old_expr, new_expr, old_op, new_op) in [
            ("n + 1", "n - 1", "+", "-"),
            ("n / 2", "n % 2", "/", "%"),
            ("n << 1", "n >> 1", "<<", ">>"),
            ("n & 1", "n | 1", "&", "|"),
        ] {
            let old = format!("export function f(n: i32): i32 {{ return {old_expr}; }}");
            let new = format!("export function f(n: i32): i32 {{ return {new_expr}; }}");
            let old_tree: serde_json::Value =
                serde_json::from_str(&process_impl(&old)).expect("valid old parse");
            let new_tree: serde_json::Value =
                serde_json::from_str(&process_impl(&new)).expect("valid new parse");
            assert!(
                find_token(&old_tree, old_op).is_some(),
                "old operator {old_op:?} was lost for {old_expr:?}: {old_tree}"
            );
            assert!(
                find_token(&new_tree, new_op).is_some(),
                "new operator {new_op:?} was lost for {new_expr:?}: {new_tree}"
            );
            assert_ne!(
                old_tree.get("structural_hash"), new_tree.get("structural_hash"),
                "operator-only edit {old_expr:?} -> {new_expr:?} was masked"
            );
        }
    }

    /// Regression for Core #191: anonymous TypeScript operator terminals must survive
    /// semantic conversion; changing arithmetic operators is not formatting.
    #[test]
    fn binary_operator_tokens_are_preserved_in_source_tree() {
        fn has_token(node: &serde_json::Value, token: &str) -> bool {
            node.get("label").and_then(serde_json::Value::as_str) == Some(token)
                || node.get("children")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|children| children.iter().any(|child| has_token(child, token)))
        }

        let old = "export function square(n: i32): i32 { return n * n; }\n";
        let new = "export function square(n: i32): i32 { return n ** 2; }\n";
        let old_tree: serde_json::Value =
            serde_json::from_str(&process_impl(old)).expect("old parser tree JSON");
        let new_tree: serde_json::Value =
            serde_json::from_str(&process_impl(new)).expect("new parser tree JSON");
        assert!(has_token(&old_tree, "*"), "old multiply operator is absent: {old_tree}");
        assert!(has_token(&new_tree, "**"), "new exponent operator is absent: {new_tree}");
        assert_ne!(
            old_tree.get("structural_hash"),
            new_tree.get("structural_hash"),
            "operator change must not retain the same structural hash"
        );
    }
}
