// RPad: cached, color-only syntax spans. Font metrics and source byte offsets stay unchanged.
use crate::{Color, SharedString};
use alloc::vec::Vec;
use std::{ops::Range, rc::Rc};
use syntect::{
    highlighting::{HighlightIterator, HighlightState, Highlighter, ThemeSet},
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
    spans: Rc<Vec<ColorSpan>>,
}

std::thread_local! {
    static SYNTAXES: SyntaxSet = {
        let mut builder = SyntaxSet::load_defaults_nonewlines().into_builder();
        builder.add(syntect::parsing::SyntaxDefinition::load_from_str(
            include_str!("rpad-config.sublime-syntax"), false, None).expect("embedded config grammar"));
        for source in [include_str!("rpad-pawn.sublime-syntax"), include_str!("rpad-gdscript.sublime-syntax"), include_str!("rpad-cmake.sublime-syntax")] {
            builder.add(syntect::parsing::SyntaxDefinition::load_from_str(source, false, None)
                .expect("embedded language grammar"));
        }
        builder.build()
    };
    static THEMES: ThemeSet = ThemeSet::load_defaults();
}

pub type SyntaxLines = Rc<Vec<Rc<Vec<ColorSpan>>>>;

#[derive(Default)]
pub struct SyntaxCache {
    filename: SharedString,
    dark: bool,
    text: SharedString,
    lines: Vec<Line>,
    output: SyntaxLines,
}

impl SyntaxCache {
    pub fn update(
        &mut self,
        text: &SharedString,
        filename: SharedString,
        dark: bool,
    ) -> SyntaxLines {
        if filename.is_empty() {
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
                let syntax = syntaxes
                    .find_syntax_by_extension(extension)
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
                let theme = &themes.themes[if dark {
                    "base16-ocean.dark"
                } else {
                    "InspiredGitHub"
                }];
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
                            spans: Rc::new(spans),
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
                self.output = Rc::new(self.lines.iter().map(|line| line.spans.clone()).collect());
                self.output.clone()
            })
        })
    }
}
