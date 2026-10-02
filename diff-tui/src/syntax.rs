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
    highlighting::{self, FontStyle, HighlightState, Highlighter, RangedHighlightIterator, Theme},
    parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet},
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{appearance::Appearance, git::escape_controls, wrap::Code};

const PLAIN_TEXT: &str = "Plain Text";

const NON_CODE_SCOPES: [&str; 2] = ["comment", "string"];
const FUNCTION_DECLARATION_SCOPES: [&str; 2] = ["entity.name.function", "entity.type.function"];
const FUNCTION_CALL_SCOPE: [&str; 1] = ["meta.function-call"];
const TYPE_DECLARATION_SCOPES: [&str; 9] = [
    "entity.name.class",
    "entity.name.struct",
    "entity.name.enum",
    "entity.name.trait",
    "entity.name.interface",
    "entity.name.namespace",
    "entity.name.type.class",
    "entity.name.type.struct",
    "entity.name.type.enum",
];

pub struct SyntaxEngine {
    syntaxes: SyntaxSet,
    themes: [Theme; 2],
}

#[derive(Default)]
pub struct HighlightedSource {
    pub lines: HashMap<usize, Code>,
    pub enclosing_functions: HashMap<usize, String>,
}

impl SyntaxEngine {
    pub fn shared() -> &'static Self {
        static ENGINE: OnceLock<SyntaxEngine> = OnceLock::new();
        ENGINE.get_or_init(|| {
            let themes = two_face::theme::extra();
            Self {
                syntaxes: two_face::syntax::extra_newlines(),
                themes: Appearance::ALL
                    .map(|appearance| themes[appearance.palette().syntax].clone()),
            }
        })
    }

    fn theme(&self, appearance: Appearance) -> &Theme {
        &self.themes[appearance as usize]
    }

    fn syntax(&self, path: &Path, first_line: &str) -> &SyntaxReference {
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| self.syntaxes.find_syntax_by_extension(name))
            .or_else(|| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .and_then(|extension| self.syntaxes.find_syntax_by_extension(extension))
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
        wanted_lines: &BTreeSet<usize>,
        appearance: Appearance,
    ) -> Result<HighlightedSource> {
        let mut result = HighlightedSource::default();
        let Some(&last_wanted) = wanted_lines.last() else {
            return Ok(result);
        };
        let syntax = self.syntax(path, source.lines().next().unwrap_or_default());
        let highlighter = Highlighter::new(self.theme(appearance));
        let brackets = &appearance.palette().brackets;
        let mut highlight_state = HighlightState::new(&highlighter, ScopeStack::new());
        let mut parser = ParseState::new(syntax);
        let mut scopes = ScopeStack::new();
        let non_code_scopes = parse_scopes(&NON_CODE_SCOPES)?;
        let function_scopes = parse_scopes(&FUNCTION_DECLARATION_SCOPES)?;
        let call_scopes = parse_scopes(&FUNCTION_CALL_SCOPE)?;
        let type_scopes = parse_scopes(&TYPE_DECLARATION_SCOPES)?;
        let colorize_brackets = syntax.name != PLAIN_TEXT;
        let mut enclosing_function = None;
        let mut open_brackets = Vec::new();
        let lines_through_last_wanted = source.split_inclusive('\n').take(last_wanted);
        for (index, line) in lines_through_last_wanted.enumerate() {
            let number = index + 1;
            let is_wanted = wanted_lines.contains(&number);
            let ops = parser.parse_line(line, &self.syntaxes)?;
            let mut pending_ops = ops.iter().peekable();
            let mut spans = Vec::new();
            for (style, text, range) in
                RangedHighlightIterator::new(&mut highlight_state, &ops, line, &highlighter)
            {
                while let Some((_, op)) = pending_ops.next_if(|(at, _)| *at <= range.start) {
                    scopes.apply(op)?;
                }
                let span_style = terminal_style(style);
                let text = text.trim_end_matches(['\n', '\r']);
                let is_code = colorize_brackets && !has_scope(&scopes, &non_code_scopes);
                if is_code && !text.trim().is_empty() {
                    if has_scope(&scopes, &function_scopes) && !has_scope(&scopes, &call_scopes) {
                        enclosing_function = Some(escape_controls(line.trim()));
                    } else if has_scope(&scopes, &type_scopes) {
                        enclosing_function = None;
                    }
                }
                let mut plain_start = 0;
                if is_code {
                    for (offset, grapheme) in text.grapheme_indices(true) {
                        let Some(ch) = grapheme.chars().next() else {
                            continue;
                        };
                        let Some(depth) = bracket_depth(&mut open_brackets, ch) else {
                            continue;
                        };
                        if is_wanted {
                            if plain_start < offset {
                                spans.push(Span::styled(
                                    escape_controls(&text[plain_start..offset]),
                                    span_style,
                                ));
                            }
                            spans.push(Span::styled(
                                escape_controls(grapheme),
                                span_style.fg(brackets[depth % brackets.len()]),
                            ));
                            plain_start = offset + grapheme.len();
                        }
                    }
                }
                if is_wanted && plain_start < text.len() {
                    spans.push(Span::styled(
                        escape_controls(&text[plain_start..]),
                        span_style,
                    ));
                }
            }
            for (_, op) in pending_ops {
                scopes.apply(op)?;
            }
            if is_wanted {
                result.lines.insert(number, Code::styled(spans));
                if let Some(function) = &enclosing_function {
                    result.enclosing_functions.insert(number, function.clone());
                }
            }
        }
        Ok(result)
    }
}

fn parse_scopes(names: &[&str]) -> Result<Vec<Scope>> {
    Ok(names
        .iter()
        .map(|name| Scope::new(name))
        .collect::<Result<_, _>>()?)
}

fn has_scope(stack: &ScopeStack, prefixes: &[Scope]) -> bool {
    stack
        .as_slice()
        .iter()
        .any(|scope| prefixes.iter().any(|prefix| prefix.is_prefix_of(*scope)))
}

fn terminal_style(style: highlighting::Style) -> Style {
    let fg = style.foreground;
    let mut terminal = Style::new().fg(Color::Rgb(fg.r, fg.g, fg.b));
    if style.font_style.contains(FontStyle::BOLD) {
        terminal = terminal.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        terminal = terminal.add_modifier(Modifier::ITALIC);
    }
    terminal
}

fn bracket_depth(open_brackets: &mut Vec<char>, ch: char) -> Option<usize> {
    let opening = match ch {
        '(' | '[' | '{' => {
            open_brackets.push(ch);
            return Some(open_brackets.len() - 1);
        }
        ')' => '(',
        ']' => '[',
        '}' => '{',
        _ => return None,
    };
    if open_brackets.last() != Some(&opening) {
        return None;
    }
    open_brackets.pop();
    Some(open_brackets.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_context_ignores_comments_strings_calls_and_resets_at_new_classes() {
        let source = concat!(
            "class Client {\n",
            "    private fun approve() {\n",
            "        /*\n",
            "        fun misleadingComment() {}\n",
            "        */\n",
            "        val message = \"\"\"\n",
            "        fun misleadingString() {}\n",
            "        \"\"\"\n",
            "        send(message)\n",
            "    }\n",
            "}\n",
            "class Other {\n",
            "    val amount = 10\n",
            "}\n",
        );
        let result = SyntaxEngine::shared()
            .highlight(
                Path::new("client.kt"),
                source,
                &BTreeSet::from([9, 13]),
                Appearance::Dark,
            )
            .unwrap();
        assert_eq!(result.enclosing_functions[&9], "private fun approve() {");
        assert!(!result.enclosing_functions.contains_key(&13));
    }

    #[test]
    fn highlights_keywords_and_multiline_comment_context() {
        let engine = SyntaxEngine::shared();
        let wanted = BTreeSet::from([4, 6]);
        let source = "/*\nfirst\nsecond\nfn actually_a_comment() {}\n*/\nfn actual_code() {}\n";
        let result = engine
            .highlight(Path::new("main.rs"), source, &wanted, Appearance::Dark)
            .unwrap();
        let comment = result.lines[&4].wrap(100);
        let code = result.lines[&6].wrap(100);
        assert_ne!(comment[0].spans[0].style.fg, code[0].spans[0].style.fg);
        assert_eq!(result.lines[&4].text, "fn actually_a_comment() {}");
    }
}
