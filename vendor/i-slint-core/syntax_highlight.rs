// RPad: cached, color-only syntax spans. Font metrics and source byte offsets stay unchanged.
use crate::{Color, SharedString};
#[path = "rust_syntax.rs"]
mod rust_syntax;
use alloc::vec::Vec;
use std::{ops::Range, rc::Rc, sync::Arc};
use syntect::{
    highlighting::{HighlightIterator, HighlightState, Highlighter, StyleModifier, Theme, ThemeItem, ThemeSet},
    parsing::{ParseState, ScopeStack, SyntaxSet},
};


#[derive(Clone, Debug, PartialEq)]
pub struct ColorSpan {
    pub range: Range<usize>,
    pub color: Color,
}

struct Line {
    text: SharedString,
    parse: Rc<ParseState>,
    highlight: Rc<HighlightState>,
    spans: Arc<Vec<ColorSpan>>,
}

std::thread_local! {
    static SYNTAXES: SyntaxSet = {
        let mut builder = SyntaxSet::load_defaults_nonewlines().into_builder();
        builder.add(syntect::parsing::SyntaxDefinition::load_from_str(
            include_str!("rpad-config.sublime-syntax"), false, None).expect("embedded config grammar"));
        for source in [include_str!("rpad-lua.sublime-syntax"), include_str!("rpad-pawn.sublime-syntax"), include_str!("rpad-gdscript.sublime-syntax"), include_str!("rpad-cmake.sublime-syntax"), include_str!("rpad-glsl.sublime-syntax"), include_str!("rpad-wgsl.sublime-syntax"), include_str!("rpad-gdshader.sublime-syntax")] {
            builder.add(syntect::parsing::SyntaxDefinition::load_from_str(source, false, None)
                .expect("embedded language grammar"));
        }
        builder.build()
    };
    static THEMES: [Theme; 2] = [make_theme(false), make_theme(true)];
}

// A shared, scope-based palette covers all Syntect grammars, not just Rust.
fn make_theme(dark: bool) -> Theme {
    use rust_syntax::Kind;
    let defaults = ThemeSet::load_defaults();
    let mut theme = defaults.themes[if dark { "base16-ocean.dark" } else { "InspiredGitHub" }].clone();
    theme.scopes.clear();
    for (scope, kind) in [
        ("comment, punctuation.definition.comment", Kind::Comment),
        ("string, punctuation.definition.string", Kind::String),
        ("constant.character.escape, constant.other.placeholder", Kind::Number),
        ("constant.numeric, constant.language", Kind::Number),
        ("constant.other, variable.other.constant, entity.name.constant", Kind::Constant),
        ("keyword, storage, storage.modifier", Kind::Keyword),
        ("keyword.operator", Kind::Operator),
        ("storage.type, support.type, support.class", Kind::Builtin),
        ("entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, entity.name.trait, entity.name.namespace", Kind::Type),
        ("entity.name.function, support.function, variable.function", Kind::Function),
        ("entity.name.function.macro, support.function.macro, meta.preprocessor, keyword.control.directive", Kind::Macro),
        ("entity.name.tag", Kind::Type),
        ("entity.other.attribute-name, variable.annotation", Kind::Attribute),
        ("variable, variable.parameter, variable.other", Kind::Variable),
        ("variable.other.member, variable.other.property, entity.name.field, support.variable.property", Kind::Field),
        ("support.constant, support.variable", Kind::Builtin),
        ("markup.heading, markup.bold, markup.italic", Kind::Keyword),
        ("markup.raw, markup.inline.raw, markup.fenced_code", Kind::String),
        ("markup.underline.link, string.other.link", Kind::Function),
        ("markup.inserted", Kind::String),
        ("markup.deleted, invalid", Kind::Field),
    ] {
        let (r, g, b) = kind.rgb(dark);
        theme.scopes.push(ThemeItem {
            scope: scope.parse().expect("embedded scope selector"),
            style: StyleModifier {
                foreground: Some(syntect::highlighting::Color { r, g, b, a: 255 }),
                background: None,
                font_style: None,
            },
        });
    }
    theme
}

fn enrich(spans: &mut Vec<ColorSpan>, line: &str, range: Range<usize>, dark: bool) {
    use rust_syntax::Kind;
    let part = &line[range.clone()];
    let mut start = None;
    let mut append = |lo: usize, hi: usize| {
        let word = &part[lo..hi];
        // Do not treat number fragments or pieces of a longer grammar token as names.
        if !word.chars().next().is_some_and(|ch| ch == '_' || ch.is_alphabetic()) {
            return;
        }
        let from = range.start + lo;
        let to = range.start + hi;
        let is_ident = |ch: char| ch == '_' || ch.is_alphanumeric();
        if line[..from].chars().next_back().is_some_and(is_ident)
            || line[to..].chars().next().is_some_and(is_ident) {
            return;
        }
        let following = line[to..].trim_start();
        let previous = line[..from].trim_end();
        let kind = if following.starts_with('(') {
            Kind::Function
        } else if word.len() > 1 && word.chars().any(char::is_uppercase)
            && !word.chars().any(char::is_lowercase) {
            Kind::Constant
        } else if word.starts_with(char::is_uppercase) {
            Kind::Type
        } else if previous.ends_with('.') || previous.ends_with("->") {
            Kind::Field
        } else {
            Kind::Variable
        };
        let (r, g, b) = kind.rgb(dark);
        spans.push(ColorSpan { range: from..to, color: Color::from_argb_u8(255, r, g, b) });
    };
    for (index, ch) in part.char_indices() {
        if ch == '_' || ch.is_alphanumeric() {
            start.get_or_insert(index);
        } else if let Some(lo) = start.take() {
            append(lo, index);
        }
    }
    if let Some(lo) = start {
        append(lo, part.len());
    }
}

pub type SyntaxLines = Arc<Vec<Arc<Vec<ColorSpan>>>>;

pub const ASYNC_THRESHOLD: usize = 1024 * 1024;

#[derive(Default)]
pub struct SyntaxCache {
    filename: SharedString,
    dark: bool,
    text: SharedString,
    lines: Vec<Line>,
    output: SyntaxLines,
    tree: Option<rust_syntax::TreeSyntax>,
}

impl SyntaxCache {
    pub fn update(
        &mut self,
        text: &SharedString,
        filename: SharedString,
        dark: bool,
    ) -> SyntaxLines {
        if filename.is_empty() {
            self.tree = None;
            self.filename = Default::default();
            self.text = Default::default();
            self.lines.clear();
            self.output = Default::default();
            return self.output.clone();
        }

        if self.filename == filename && self.dark == dark && self.text.as_ptr() == text.as_ptr() {
            return self.output.clone();
        }

        let extension = if filename
            .rsplit(['/', '\\'])
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("CMakeLists.txt"))
        {
            "cmake".into()
        } else {
            filename
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
        };
        if let Some(language) = rust_syntax::language(&extension) {
            if self.filename != filename {
                self.tree = None;
            }
            let parser = self.tree.get_or_insert_with(rust_syntax::TreeSyntax::default);
            if let Some(spans) = parser.update(text, dark, language) {
                self.lines.clear();
                self.filename = filename;
                self.dark = dark;
                self.text = text.clone();
                self.output = spans;
                return self.output.clone();
            }
        } else {
            self.tree = None;
        }
        let first_line = || text.lines().next().unwrap_or("");
        if matches!(extension.as_str(), "txt" | "log" | "text")
            || (filename == "Untitled"
                && !first_line().starts_with("#!")
                && !first_line().starts_with('<'))
        {
            self.lines.clear();
            self.filename = filename;
            self.text = text.clone();
            self.dark = dark;
            self.output = Default::default();
            return self.output.clone();
        }

        SYNTAXES.with(|syntaxes| {
            THEMES.with(|themes| {
                let extension = match extension.as_str() {
                    "hpp" | "hh" | "hxx" | "cc" | "cxx" => "cpp",
                    "ts" | "tsx" | "jsx" | "mjs" | "cjs" => "js",
                    "yml" => "yaml",
                    "csx" => "cs",
                    "vue" | "svelte" => "html",
                    "sh" | "bash" | "zsh" => "sh",
                    other => other,
                };
                let syntax = (if extension == "lua" {
                    syntaxes.find_syntax_by_name("RPad Lua")
                } else {
                    syntaxes.find_syntax_by_extension(extension)
                })
                    .or_else(|| syntaxes.find_syntax_by_name(&filename))
                    .or_else(|| {
                        syntaxes.find_syntax_by_first_line(text.lines().next().unwrap_or(""))
                    });
                let syntax = syntax.filter(|syntax| syntax.name != "Plain Text");
                let Some(syntax) = syntax else {
                    self.lines.clear();
                    self.filename = filename;
                    self.text = text.clone();
                    self.dark = dark;
                    self.output = Default::default();
                    return self.output.clone();
                };
                let theme = &themes[usize::from(dark)];
                let highlighter = Highlighter::new(theme);
                let default_color =
                    theme
                        .settings
                        .foreground
                        .unwrap_or(syntect::highlighting::Color {
                            r: 255,
                            g: 255,
                            b: 255,
                            a: 255,
                        });
                let enrich_identifiers = matches!(extension,
                    "rs" | "c" | "cpp" | "cs" | "go" | "java" | "js" | "py"
                    | "lua" | "rb" | "php" | "sh" | "bash" | "zsh" | "sma" | "sp" | "pwn"
                    | "gd" | "glsl" | "vert" | "frag" | "wgsl" | "gdshader" | "cmake"
                    | "swift" | "kt" | "kts" | "scala" | "pl" | "r" | "sql" | "zig"
                    | "nim" | "dart" | "haskell" | "hs" | "el" | "clj" | "ex" | "exs");
                let initial_parse = ParseState::new(syntax);
                let initial_highlight = HighlightState::new(&highlighter, ScopeStack::new());
                let input: Vec<_> = text
                    .split('\n')
                    .map(|line| line.strip_suffix('\r').unwrap_or(line))
                    .collect();
                let same_mode = self.filename == filename && self.dark == dark;
                let mut prefix = 0;
                let mut suffix = 0;

                if same_mode {
                    while prefix < input.len().min(self.lines.len())
                        && input[prefix] == self.lines[prefix].text.as_str()
                    {
                        prefix += 1;
                    }
                    while suffix < input.len().min(self.lines.len()) - prefix
                        && input[input.len() - 1 - suffix]
                            == self.lines[self.lines.len() - 1 - suffix].text.as_str()
                    {
                        suffix += 1;
                    }
                }

                let old_suffix = self.lines.len() - suffix;
                let mut old_before = if old_suffix == 0 {
                    (
                        Rc::new(initial_parse.clone()),
                        Rc::new(initial_highlight.clone()),
                    )
                } else {
                    (
                        self.lines[old_suffix - 1].parse.clone(),
                        self.lines[old_suffix - 1].highlight.clone(),
                    )
                };
                let tail = self.lines.split_off(old_suffix);
                self.lines.truncate(prefix);
                let (mut parse, mut highlight) = self
                    .lines
                    .last()
                    .map(|line| (line.parse.as_ref().clone(), line.highlight.as_ref().clone()))
                    .unwrap_or((initial_parse, initial_highlight));

                let mut shared_parse = Rc::new(parse.clone());
                let mut shared_highlight = Rc::new(highlight.clone());
                let mut parse_line =
                    |text: &str, parse: &mut ParseState, highlight: &mut HighlightState| {
                        let mut next_parse = parse.clone();
                        let mut next_highlight = highlight.clone();
                        let mut spans = Vec::new();
                        if let Ok(operations) = next_parse.parse_line(text, syntaxes) {
                            let mut offset = 0;
                            for (style, part) in HighlightIterator::new(
                                &mut next_highlight,
                                &operations,
                                text,
                                &highlighter,
                            ) {
                                let end = offset + part.len();
                                if style.foreground != default_color && offset != end {
                                    let color = style.foreground;
                                    spans.push(ColorSpan {
                                        range: offset..end,
                                        color: Color::from_argb_u8(
                                            color.a, color.r, color.g, color.b,
                                        ),
                                    });
                                } else if enrich_identifiers && offset != end {
                                    // Only fill unstyled grammar gaps. The parser's comment and
                                    // string scopes already have explicit colors.
                                    enrich(&mut spans, text, offset..end, dark);
                                }
                                offset = end;
                            }
                            *parse = next_parse;
                            *highlight = next_highlight;
                        }
                        if *shared_parse != *parse {
                            shared_parse = Rc::new(parse.clone());
                        }
                        if *shared_highlight != *highlight {
                            shared_highlight = Rc::new(highlight.clone());
                        }
                        Line {
                            text: text.into(),
                            parse: shared_parse.clone(),
                            highlight: shared_highlight.clone(),
                            spans: Arc::new(spans),
                        }
                    };

                for line in &input[prefix..input.len() - suffix] {
                    self.lines
                        .push(parse_line(line, &mut parse, &mut highlight));
                }

                let mut tail = tail.into_iter();
                while let Some(old) = tail.next() {
                    if parse == *old_before.0 && highlight == *old_before.1 {
                        self.lines.push(old);
                        self.lines.extend(tail);
                        break;
                    }
                    old_before = (old.parse, old.highlight);
                    self.lines
                        .push(parse_line(&old.text, &mut parse, &mut highlight));
                }

                self.filename = filename;
                self.dark = dark;
                self.text = text.clone();
                self.output = Arc::new(self.lines.iter().map(|line| line.spans.clone()).collect());
                self.output.clone()
            })
        })
    }
}
