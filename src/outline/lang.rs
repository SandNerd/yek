//! Per-language configuration for outline extraction.
//!
//! Languages are described as *data*: a node-kind classification table plus a
//! visibility rule and an elision style. Adding a language is adding a grammar
//! dependency and a few match arms — no new control flow.

use std::path::Path;

use super::{Handling, SymbolKind};

/// A source language yek can outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    Tsx,
    Python,
}

/// How to decide whether a declaration is part of the public API.
#[derive(Clone, Copy)]
pub(crate) enum VisibilityRule {
    /// Public iff the node has a direct child of this kind (e.g. Rust `pub`).
    Marker(&'static str),
    /// Public iff wrapped by a parent node of this kind (e.g. TS `export_statement`).
    Parent(&'static str),
    /// Always public (e.g. Python — no `pub`/`export` keyword).
    Public,
}

/// How elided bodies are rendered.
#[derive(Clone, Copy)]
pub(crate) enum ElisionStyle {
    /// C-like: ` { /* … N lines … */ }`.
    Braces,
    /// Python-style: `: # ... N lines elided ...`.
    PythonStyle,
}

impl Language {
    /// All languages compiled into this build.
    pub fn all() -> &'static [Language] {
        &[
            Language::Rust,
            Language::TypeScript,
            Language::Tsx,
            Language::Python,
        ]
    }

    /// Canonical lowercase name, used by `--outline-languages` and `--json`.
    pub fn name(self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::Python => "python",
        }
    }

    /// Parse a user-supplied language name (for `--outline-languages`).
    pub fn from_name(name: &str) -> Option<Language> {
        match name.trim().to_ascii_lowercase().as_str() {
            "rust" | "rs" => Some(Language::Rust),
            "typescript" | "ts" => Some(Language::TypeScript),
            "tsx" => Some(Language::Tsx),
            "python" | "py" => Some(Language::Python),
            _ => None,
        }
    }

    /// The tree-sitter grammar for this language.
    pub(crate) fn ts_language(self) -> tree_sitter::Language {
        match self {
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
        }
    }

    /// Map a grammar node kind to a symbol kind and how to render its body.
    /// Returns `None` for nodes that are not outline-worthy declarations.
    pub(crate) fn classify(self, node_kind: &str) -> Option<(SymbolKind, Handling)> {
        match self {
            Language::Rust => match node_kind {
                "function_item" => Some((SymbolKind::Function, Handling::Elide)),
                // Bodyless trait method / associated declarations: show verbatim.
                "function_signature_item" => Some((SymbolKind::Function, Handling::ShowFull)),
                "associated_type" => Some((SymbolKind::Type, Handling::ShowFull)),
                // `macro_definition` has no `body` field, so its rules cannot be
                // elided generically yet; show it verbatim rather than pretending.
                "macro_definition" => Some((SymbolKind::Macro, Handling::ShowFull)),
                "struct_item" => Some((SymbolKind::Struct, Handling::ShowFull)),
                "enum_item" => Some((SymbolKind::Enum, Handling::ShowFull)),
                "union_item" => Some((SymbolKind::Union, Handling::ShowFull)),
                "type_item" => Some((SymbolKind::Type, Handling::ShowFull)),
                "const_item" => Some((SymbolKind::Const, Handling::ShowFull)),
                "static_item" => Some((SymbolKind::Static, Handling::ShowFull)),
                "use_declaration" => Some((SymbolKind::Import, Handling::ShowFull)),
                "trait_item" => Some((SymbolKind::Trait, Handling::Recurse)),
                "impl_item" => Some((SymbolKind::Impl, Handling::Recurse)),
                "mod_item" => Some((SymbolKind::Module, Handling::Recurse)),
                _ => None,
            },
            Language::TypeScript | Language::Tsx => classify_typescript(node_kind),
            Language::Python => classify_python(node_kind),
        }
    }

    pub(crate) fn visibility(self) -> VisibilityRule {
        match self {
            Language::Rust => VisibilityRule::Marker("visibility_modifier"),
            // `export function …` / `export class …` wraps the declaration in
            // an `export_statement` parent rather than attaching a child marker.
            Language::TypeScript | Language::Tsx => VisibilityRule::Parent("export_statement"),
            // Python has no `pub`/`export` keyword; all top-level names are
            // effectively public.
            Language::Python => VisibilityRule::Public,
        }
    }

    pub(crate) fn elision(self) -> ElisionStyle {
        match self {
            Language::Rust | Language::TypeScript | Language::Tsx => ElisionStyle::Braces,
            Language::Python => ElisionStyle::PythonStyle,
        }
    }
}

/// Shared classification for Python nodes.
fn classify_python(node_kind: &str) -> Option<(SymbolKind, Handling)> {
    match node_kind {
        "function_definition" => Some((SymbolKind::Function, Handling::Elide)),
        "class_definition" => Some((SymbolKind::Struct, Handling::Recurse)),
        // `typed_assignment` covers class-body annotated variables like
        // `host: str = "localhost"` in dataclasses and regular classes.
        "typed_assignment" => Some((SymbolKind::Static, Handling::ShowFull)),
        // Decorators are not outline-worthy standalone declarations.
        "decorated_definition" => {
            // A decorated function/class is handled by classifying its inner
            // definition; we skip the decorator wrapper itself.
            None
        }
        _ => None,
    }
}

/// Shared classification for TypeScript and TSX (same declaration node kinds).
fn classify_typescript(node_kind: &str) -> Option<(SymbolKind, Handling)> {
    match node_kind {
        "function_declaration" | "generator_function_declaration" => {
            Some((SymbolKind::Function, Handling::Elide))
        }
        "method_definition" => Some((SymbolKind::Function, Handling::Elide)),
        // Bodyless signatures (interfaces / abstract classes): show verbatim.
        "method_signature" | "abstract_method_signature" => {
            Some((SymbolKind::Function, Handling::ShowFull))
        }
        "class_declaration" | "abstract_class_declaration" => {
            Some((SymbolKind::Struct, Handling::Recurse))
        }
        // Closest existing kind: interfaces expose a public surface like traits.
        "interface_declaration" => Some((SymbolKind::Trait, Handling::Recurse)),
        "type_alias_declaration" => Some((SymbolKind::Type, Handling::ShowFull)),
        "enum_declaration" => Some((SymbolKind::Enum, Handling::ShowFull)),
        _ => None,
    }
}

/// Detect a supported language from a file path's extension.
pub fn detect_language(rel_path: &str) -> Option<Language> {
    let ext = Path::new(rel_path).extension()?.to_str()?;
    match ext {
        "rs" => Some(Language::Rust),
        "ts" => Some(Language::TypeScript),
        "tsx" => Some(Language::Tsx),
        "py" => Some(Language::Python),
        _ => None,
    }
}
