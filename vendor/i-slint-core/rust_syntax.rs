//! Native syntax trees with context-aware colors and incremental reparsing.
use super::{ColorSpan, SyntaxLines};
use crate::{Color, SharedString};
use alloc::{vec, vec::Vec};
use std::sync::Arc;
use tree_sitter::{InputEdit, Node, Parser, Point, Tree};

#[derive(Default)]
pub(super) struct TreeSyntax {
    parser: Option<Parser>,
    tree: Option<Tree>,
    text: SharedString,
    spans: SyntaxLines,
}

fn point(text: &str, offset: usize) -> Point {
    let before = &text[..offset];
    Point::new(
        before.bytes().filter(|byte| *byte == b'\n').count(),
        before.rsplit('\n').next().unwrap_or("").len(),
    )
}

fn edit(old: &str, new: &str) -> InputEdit {
    let mut start = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(start) || !new.is_char_boundary(start) {
        start -= 1;
    }
    let mut suffix = old.as_bytes()[start..]
        .iter()
        .rev()
        .zip(new.as_bytes()[start..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix)
        || !new.is_char_boundary(new.len() - suffix)
    {
        suffix -= 1;
    }
    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;
    InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point(old, start),
        old_end_position: point(old, old_end),
        new_end_position: point(new, new_end),
    }
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Keyword,
    Type,
    Builtin,
    Function,
    Macro,
    Field,
    Variable,
    Constant,
    Attribute,
    String,
    Number,
    Comment,
    Lifetime,
    Operator,
}

impl Kind {
    pub(super) fn rgb(self, dark: bool) -> (u8, u8, u8) {
        match (self, dark) {
            (Self::Keyword, true) => (208, 145, 242),
            (Self::Type, true) => (86, 214, 206),
            (Self::Builtin, true) => (91, 181, 251),
            (Self::Function, true) => (115, 190, 255),
            (Self::Macro, true) => (255, 190, 99),
            (Self::Field, true) => (245, 142, 145),
            (Self::Variable, true) => (204, 221, 244),
            (Self::Constant, true) => (255, 204, 100),
            (Self::Attribute, true) => (215, 158, 244),
            (Self::String, true) => (160, 213, 133),
            (Self::Number, true) => (255, 171, 130),
            (Self::Comment, true) => (166, 164, 152),
            (Self::Lifetime, true) => (218, 160, 228),
            (Self::Operator, true) => (152, 200, 218),
            (Self::Keyword, false) => (124, 46, 165),
            (Self::Type, false) => (0, 105, 107),
            (Self::Builtin, false) => (0, 88, 164),
            (Self::Function, false) => (0, 81, 156),
            (Self::Macro, false) => (145, 77, 0),
            (Self::Field, false) => (169, 38, 62),
            (Self::Variable, false) => (48, 64, 96),
            (Self::Constant, false) => (133, 86, 0),
            (Self::Attribute, false) => (123, 49, 150),
            (Self::String, false) => (48, 111, 28),
            (Self::Number, false) => (160, 64, 21),
            (Self::Comment, false) => (101, 104, 94),
            (Self::Lifetime, false) => (124, 51, 136),
            (Self::Operator, false) => (53, 91, 111),
        }
    }

    fn color(self, dark: bool) -> Color {
        let (r, g, b) = self.rgb(dark);
        Color::from_argb_u8(255, r, g, b)
    }
}

fn identifier(node: Node<'_>, text: &str) -> Kind {
    let value = &text[node.byte_range()];
    let parent = node.parent();
    let context = parent.map_or("", |parent| parent.kind());
    let following = text[node.end_byte()..].trim_start();
    if matches!(context, "attribute" | "annotation" | "marker_annotation" | "decorator") {
        return Kind::Attribute;
    }
    if matches!(context, "macro_invocation" | "preproc_def" | "preproc_function_def")
        || (following.starts_with('!') && !following.starts_with("!="))
    {
        return Kind::Macro;
    }
    if matches!(context, "function_item" | "function_signature_item" | "function_definition"
        | "function_declaration" | "method_declaration" | "method_definition"
        | "constructor_declaration" | "function_declarator")
        && parent.is_some_and(|parent| {
            parent.child_by_field_name("name") == Some(node)
                || parent.child_by_field_name("declarator") == Some(node)
        })
    {
        return Kind::Function;
    }
    if matches!(context, "class_declaration" | "class_definition" | "interface_declaration"
        | "struct_declaration" | "enum_declaration" | "type_alias_declaration"
        | "type_spec" | "record_declaration")
        && parent.and_then(|parent| parent.child_by_field_name("name")) == Some(node)
    {
        return Kind::Type;
    }
    // The final component of a call path has a following opening paren. This
    // also covers identifiers inside unexpanded macro token trees.
    if following.starts_with('(')
        && !value.starts_with(char::is_uppercase)
    {
        return Kind::Function;
    }
    if value.chars().any(char::is_uppercase)
        && value.chars().all(|ch| !ch.is_lowercase())
        && value.len() > 1
    {
        return Kind::Constant;
    }
    if value.starts_with(char::is_uppercase) {
        return Kind::Type;
    }
    Kind::Variable
}

fn classify(node: Node<'_>, text: &str) -> Option<Kind> {
    if !node.is_named() && matches!(node.kind(),
        "int" | "float" | "double" | "char" | "bool" | "byte" | "short" | "long"
        | "void" | "string" | "number" | "boolean" | "object")
    {
        return Some(Kind::Builtin);
    }
    let kind = match node.kind() {
        "line_comment" | "block_comment" | "comment" | "documentation_comment" => Kind::Comment,
        "string_literal" | "raw_string_literal" | "char_literal" | "byte_string_literal"
        | "raw_byte_string_literal" | "string" | "character_literal" | "raw_string"
        | "template_string" | "verbatim_string_literal" | "interpreted_string_literal"
        | "rune_literal" | "raw_string_literal_content" | "string_content" => Kind::String,
        "integer_literal" | "float_literal" | "boolean_literal" | "number_literal"
        | "number" | "integer" | "float" | "decimal_integer_literal" | "hex_integer_literal"
        | "binary_integer_literal" | "octal_integer_literal" | "decimal_floating_point_literal"
        | "hex_floating_point_literal" | "real_literal" | "int_literal" | "imaginary_literal"
        | "null" | "null_literal" | "none" | "nil" | "undefined" => Kind::Number,
        "primitive_type" | "predefined_type" | "integral_type" | "floating_point_type"
        | "void_type" | "boolean_type" => Kind::Builtin,
        "type_identifier" | "namespace_identifier" | "package_identifier" => Kind::Type,
        "field_identifier" | "property_identifier" | "private_property_identifier" => {
            if text[node.end_byte()..].trim_start().starts_with('(') {
                Kind::Function
            } else {
                Kind::Field
            }
        }
        "identifier" | "metavariable" | "shorthand_property_identifier"
        | "shorthand_property_identifier_pattern" => identifier(node, text),
        "lifetime" | "label" => Kind::Lifetime,
        "self" | "Self" | "super" | "crate" | "this" | "base" => Kind::Builtin,
        "as" | "async" | "await" | "break" | "const" | "continue" | "dyn" | "else"
        | "enum" | "extern" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop"
        | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "static"
        | "struct" | "trait" | "type" | "union" | "unsafe" | "use" | "where" | "while"
        | "yield" | "gen" | "macro_rules!" | "class" | "interface" | "public" | "private"
        | "protected" | "internal" | "final" | "abstract" | "override" | "virtual" | "new"
        | "import" | "export" | "from" | "def" | "function" | "func" | "var" | "package"
        | "namespace" | "using" | "include" | "#include" | "#define" | "typedef"
        | "switch" | "case" | "default" | "try" | "catch" | "finally" | "throw" | "throws"
        | "raise" | "except" | "with" | "pass" | "assert" | "del" | "lambda" | "global"
        | "nonlocal" | "elif" | "extends" | "implements" | "instanceof" | "typeof"
        | "delete" | "do" | "defer" | "select" | "go" | "range" | "chan" | "map"
        | "readonly" | "consteval" | "constexpr" | "noexcept" | "template" | "typename"
        | "operator" | "and" | "or" | "not" | "is" => Kind::Keyword,
        "mutable_specifier" => Kind::Keyword,
        "true" | "false" => Kind::Number,
        "=" | "+" | "-" | "*" | "/" | "%" | "!" | "?" | "&" | "|" | "^" | "&&"
        | "||" | "==" | "!=" | "<=" | ">=" | "<<" | ">>" | "+=" | "-=" | "*="
        | "/=" | "::" | "->" | "=>" | ".." | "..=" => Kind::Operator,
        _ => return None,
    };
    Some(kind)
}

fn emit(
    rows: &mut [Vec<ColorSpan>],
    starts: &[usize],
    text: &str,
    node: Node<'_>,
    color: Color,
) {
    let range = node.byte_range();
    if range.is_empty() || range.end > text.len() {
        return;
    }
    let first = starts.partition_point(|start| *start <= range.start) - 1;
    let last = starts.partition_point(|start| *start < range.end);
    for row in first..last {
        let start = starts[row];
        let end = starts.get(row + 1).map_or(text.len(), |next| next - 1);
        // Syntect and the renderer both use paragraph-relative UTF-8 offsets.
        let end = if end > start && text.as_bytes()[end - 1] == b'\r' {
            end - 1
        } else {
            end
        };
        let lo = range.start.max(start);
        let hi = range.end.min(end);
        if lo < hi && text.is_char_boundary(lo) && text.is_char_boundary(hi) {
            rows[row].push(ColorSpan {
                range: lo - start..hi - start,
                color,
            });
        }
    }
}

fn highlight(tree: &Tree, text: &str, dark: bool) -> Vec<Vec<ColorSpan>> {
    let mut starts = vec![0];
    starts.extend(
        text.bytes()
            .enumerate()
            .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
    );
    let mut rows = vec![Vec::new(); starts.len()];
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        // Atomic tokens such as strings and comments must not leak colors from
        // their nested syntax nodes. The iterative walk avoids stack overflow.
        if let Some(kind) = classify(node, text) {
            emit(&mut rows, &starts, text, node, kind.color(dark));
        } else if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return rows;
            }
        }
    }
}

pub(super) fn language(extension: &str) -> Option<tree_sitter::Language> {
    Some(match extension {
        "rs" => tree_sitter_rust::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "h" | "hpp" | "hh" | "hxx" | "cpp" | "cc" | "cxx" => tree_sitter_cpp::LANGUAGE.into(),
        "cs" | "csx" => tree_sitter_c_sharp::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "js" | "jsx" | "mjs" | "cjs" => tree_sitter_javascript::LANGUAGE.into(),
        "ts" | "mts" | "cts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "py" | "pyw" | "pyi" => tree_sitter_python::LANGUAGE.into(),
        _ => return None,
    })
}

impl TreeSyntax {
    pub fn update(&mut self, text: &SharedString, dark: bool, language: tree_sitter::Language) -> Option<SyntaxLines> {
        if self.parser.is_none() {
            let mut parser = Parser::new();
            parser.set_language(&language).ok()?;
            self.parser = Some(parser);
        }
        let same_text = self.text.as_str().as_ptr() == text.as_str().as_ptr()
            && self.text.len() == text.len();
        if !same_text || self.tree.is_none() {
            if let Some(tree) = &mut self.tree {
                tree.edit(&edit(&self.text, text));
            }
            self.tree = self.parser.as_mut()?.parse(text.as_bytes(), self.tree.as_ref());
            self.text = text.clone();
        }
        let rows = highlight(self.tree.as_ref()?, text, dark);
        // Reuse unchanged row allocations so the existing shaping memo can
        // retain glyph layouts even when a syntax tree was updated.
        let previous = &self.spans;
        self.spans = Arc::new(
            rows.into_iter()
                .enumerate()
                .map(|(index, spans)| {
                    previous
                        .get(index)
                        .filter(|old| old.as_ref() == &spans)
                        .cloned()
                        .unwrap_or_else(|| Arc::new(spans))
                })
                .collect(),
        );
        Some(self.spans.clone())
    }
}
