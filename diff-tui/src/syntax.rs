use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
    sync::OnceLock,
};

use anyhow::Result;
use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};
use two_face::re_exports::syntect::{
    highlighting::{FontStyle, HighlightState, Highlighter, RangedHighlightIterator, Theme},
    parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet},
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{git::clean, wrap::Code};

const BRACKET_COLORS: [Color; 6] = [
    Color::Rgb(235, 203, 139),
    Color::Rgb(180, 142, 173),
    Color::Rgb(136, 192, 208),
    Color::Rgb(208, 135, 112),
    Color::Rgb(129, 161, 193),
    Color::Rgb(163, 190, 140),
];

pub struct SyntaxEngine {
    syntaxes: SyntaxSet,
    theme: Theme,
}

impl SyntaxEngine {
    pub fn shared() -> &'static Self {
        static ENGINE: OnceLock<SyntaxEngine> = OnceLock::new();
        ENGINE.get_or_init(|| Self {
            syntaxes: two_face::syntax::extra_newlines(),
            theme: two_face::theme::extra()[two_face::theme::EmbeddedThemeName::Nord].clone(),
        })
    }

    fn syntax(&self, path: &Path, first_line: &str) -> &SyntaxReference {
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| self.syntaxes.find_syntax_by_extension(name))
            .or_else(|| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .and_then(|ext| self.syntaxes.find_syntax_by_extension(ext))
            })
            .or_else(|| self.syntaxes.find_syntax_by_first_line(first_line))
            .unwrap_or_else(|| self.syntaxes.find_syntax_plain_text())
    }

    pub fn language(&self, path: &Path) -> String {
        self.syntax(path, "").name.clone()
    }

    pub fn highlight(
        &self,
        path: &Path,
        source: &str,
        wanted: &BTreeSet<usize>,
    ) -> Result<HashMap<usize, Code>> {
        let mut result = HashMap::new();
        let Some(last) = wanted.last().copied() else {
            return Ok(result);
        };
        let syntax = self.syntax(path, source.lines().next().unwrap_or_default());
        let highlighter = Highlighter::new(&self.theme);
        let mut state = HighlightState::new(&highlighter, ScopeStack::new());
        let mut parser = ParseState::new(syntax);
        let mut scopes = ScopeStack::new();
        let excluded = [Scope::new("comment")?, Scope::new("string")?];
        let mut brackets = Vec::new();
        let rainbow = syntax.name != "Plain Text";
        // Parse skipped lines too, so comments, strings and bracket depth retain context.
        for (index, line) in source.split_inclusive('\n').take(last).enumerate() {
            let ops = parser.parse_line(line, &self.syntaxes)?;
            let mut pending = ops.iter().peekable();
            let visible = wanted.contains(&(index + 1));
            let mut spans = Vec::new();
            for (style, text, range) in
                RangedHighlightIterator::new(&mut state, &ops, line, &highlighter)
            {
                while pending.peek().is_some_and(|(at, _)| *at <= range.start) {
                    scopes.apply(&pending.next().unwrap().1)?;
                }
                let fg = style.foreground;
                let mut terminal = Style::new().fg(Color::Rgb(fg.r, fg.g, fg.b));
                if style.font_style.contains(FontStyle::BOLD) {
                    terminal = terminal.add_modifier(Modifier::BOLD);
                }
                if style.font_style.contains(FontStyle::ITALIC) {
                    terminal = terminal.add_modifier(Modifier::ITALIC);
                }
                let text = text.trim_end_matches(['\n', '\r']);
                let mut start = 0;
                if rainbow
                    && !scopes
                        .as_slice()
                        .iter()
                        .any(|scope| excluded.iter().any(|prefix| prefix.is_prefix_of(*scope)))
                {
                    for (offset, grapheme) in text.grapheme_indices(true) {
                        let ch = grapheme.chars().next().unwrap();
                        let depth = match ch {
                            '(' | '[' | '{' => {
                                let depth = brackets.len();
                                brackets.push(ch);
                                depth
                            }
                            ')' | ']' | '}' => {
                                let opening = match ch {
                                    ')' => '(',
                                    ']' => '[',
                                    _ => '{',
                                };
                                if brackets.last() != Some(&opening) {
                                    continue;
                                }
                                brackets.pop();
                                brackets.len()
                            }
                            _ => continue,
                        };
                        if visible {
                            if start < offset {
                                spans.push(Span::styled(clean(&text[start..offset]), terminal));
                            }
                            spans.push(Span::styled(
                                clean(grapheme),
                                terminal.fg(BRACKET_COLORS[depth % BRACKET_COLORS.len()]),
                            ));
                            start = offset + grapheme.len();
                        }
                    }
                }
                if visible && start < text.len() {
                    spans.push(Span::styled(clean(&text[start..]), terminal));
                }
            }
            for (_, op) in pending {
                scopes.apply(op)?;
            }
            if visible {
                result.insert(index + 1, Code::styled(spans));
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_common_languages_without_language_servers() {
        let engine = SyntaxEngine::shared();
        for extension in [
            "rs", "kt", "kts", "scala", "java", "ts", "tsx", "js", "py", "go", "php", "rb", "c",
            "cpp", "cs", "swift", "sh", "json", "yaml", "toml", "sql", "html", "css",
        ] {
            assert_ne!(
                engine.language(Path::new(&format!("example.{extension}"))),
                "Plain Text",
                "{extension}"
            );
        }
        assert_eq!(
            engine.language(Path::new("unknown.extensionzzz")),
            "Plain Text"
        );
    }

    #[test]
    fn highlights_keywords_and_multiline_comment_context() {
        let engine = SyntaxEngine::shared();
        let wanted = BTreeSet::from([4, 6]);
        let source = "/*\nfirst\nsecond\nfn actually_a_comment() {}\n*/\nfn actual_code() {}\n";
        let result = engine
            .highlight(Path::new("main.rs"), source, &wanted)
            .unwrap();
        let comment = result[&4].wrap(100);
        let code = result[&6].wrap(100);
        assert_ne!(comment[0].spans[0].style.fg, code[0].spans[0].style.fg);
        assert_eq!(result[&4].text, "fn actually_a_comment() {}");
    }
}
