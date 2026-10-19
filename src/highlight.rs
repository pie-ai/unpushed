//! Syntax highlighting for the diff view.
//!
//! The syntax and theme sets are loaded once on first use, not at startup:
//! they cost both time and a few megabytes, and most sessions never open a
//! diff at all.
//!
//! `syntect` is pulled in without default features, using `fancy-regex`
//! instead of the C library Oniguruma. That keeps the dependency tree free of
//! C code, which is what makes the cross-compiled `armv7` build work at all.

use std::sync::OnceLock;

use ratatui::style::{Color, Style};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

use crate::commits::{DiffLine, LineKind};

struct Highlighter {
    syntaxes: SyntaxSet,
    theme: Theme,
}

fn highlighter() -> &'static Highlighter {
    static HIGHLIGHTER: OnceLock<Highlighter> = OnceLock::new();
    HIGHLIGHTER.get_or_init(|| {
        let themes = ThemeSet::load_defaults();
        Highlighter {
            syntaxes: SyntaxSet::load_defaults_newlines(),
            // Mid-tone foreground colours that stay legible on both dark and
            // light terminals. Only the foreground is used; the theme's
            // background is ignored so the terminal's own shows through.
            theme: themes.themes["base16-ocean.dark"].clone(),
        }
    })
}

/// A diff line broken into coloured pieces.
pub type Highlighted = Vec<(Style, String)>;

/// Colours the content of each diff line according to the file's language.
///
/// Returns one entry per input line, in the same order.
pub fn diff(path: &str, lines: &[DiffLine]) -> Vec<Highlighted> {
    let h = highlighter();
    let syntax = syntax_for(h, path);

    // Two independent states: a diff interleaves the before and after
    // versions, and feeding both through one parser leaves it in a state that
    // belongs to neither. Removed lines continue the "before" file, added
    // lines the "after"; context lines belong to both and advance each.
    let mut before = HighlightLines::new(syntax, &h.theme);
    let mut after = HighlightLines::new(syntax, &h.theme);

    lines
        .iter()
        .map(|line| match line.kind {
            // Headers are not code and would only confuse the parser.
            LineKind::Header => vec![(Style::default(), line.text.clone())],
            LineKind::Removed => colour(&mut before, h, &line.text),
            LineKind::Added => colour(&mut after, h, &line.text),
            LineKind::Context => {
                let rendered = colour(&mut before, h, &line.text);
                // Keep the other state moving too, so it stays in step.
                let _ = colour(&mut after, h, &line.text);
                rendered
            }
        })
        .collect()
}

fn syntax_for<'a>(h: &'a Highlighter, path: &str) -> &'a SyntaxReference {
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    h.syntaxes
        .find_syntax_by_extension(extension)
        .or_else(|| {
            // Files like `Makefile` or `Dockerfile` carry the name, not an
            // extension.
            std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| h.syntaxes.find_syntax_by_token(n))
        })
        .unwrap_or_else(|| h.syntaxes.find_syntax_plain_text())
}

fn colour(state: &mut HighlightLines, h: &Highlighter, text: &str) -> Highlighted {
    // syntect's parsers expect a trailing newline; without one, the last token
    // of every line is swallowed.
    let owned = format!("{text}\n");
    let mut out = Vec::new();
    for piece in LinesWithEndings::from(&owned) {
        match state.highlight_line(piece, &h.syntaxes) {
            Ok(ranges) => {
                for (style, part) in ranges {
                    let part = part.trim_end_matches(['\n', '\r']);
                    if part.is_empty() {
                        continue;
                    }
                    out.push((
                        Style::default().fg(Color::Rgb(
                            style.foreground.r,
                            style.foreground.g,
                            style.foreground.b,
                        )),
                        part.to_string(),
                    ));
                }
            }
            // A parser failure must not cost the user their diff; show the
            // line without colour instead.
            Err(_) => out.push((Style::default(), text.to_string())),
        }
    }
    if out.is_empty() {
        out.push((Style::default(), String::new()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: LineKind, text: &str) -> DiffLine {
        DiffLine {
            kind,
            text: text.to_string(),
        }
    }

    #[test]
    fn every_input_line_yields_exactly_one_output_line() {
        let lines = vec![
            line(LineKind::Header, "@@ -1,3 +1,3 @@"),
            line(LineKind::Context, "fn main() {"),
            line(LineKind::Removed, "    println!(\"old\");"),
            line(LineKind::Added, "    println!(\"new\");"),
            line(LineKind::Context, "}"),
        ];
        assert_eq!(diff("main.rs", &lines).len(), lines.len());
    }

    #[test]
    fn code_is_broken_into_several_coloured_pieces() {
        let lines = vec![line(LineKind::Context, "fn main() { let x = 1; }")];
        let out = diff("main.rs", &lines);
        assert!(out[0].len() > 1, "expected several spans, got {:?}", out[0]);
        assert!(
            out[0].iter().any(|(style, _)| style.fg.is_some()),
            "nothing was coloured"
        );
    }

    /// Reassembling the pieces must give back the original line, or the view
    /// would quietly drop characters.
    #[test]
    fn the_text_survives_highlighting_unchanged() {
        for (path, text) in [
            ("main.rs", "fn main() { let x = 1; }"),
            ("x.py", "def f(a, b):  # note"),
            ("notes.txt", "plain text, no language"),
            ("weird.zzz", "unknown extension"),
            ("Makefile", "all: build"),
        ] {
            let lines = vec![line(LineKind::Context, text)];
            let joined: String = diff(path, &lines)[0]
                .iter()
                .map(|(_, part)| part.as_str())
                .collect();
            assert_eq!(joined, text, "mangled for {path}");
        }
    }

    #[test]
    fn an_unknown_extension_still_renders() {
        let lines = vec![line(LineKind::Context, "whatever this is")];
        let out = diff("file.unknownext", &lines);
        assert_eq!(out.len(), 1);
        assert!(!out[0].is_empty());
    }

    #[test]
    fn empty_lines_do_not_vanish() {
        let lines = vec![line(LineKind::Context, "")];
        let out = diff("main.rs", &lines);
        assert_eq!(out.len(), 1);
    }

    /// Headers are not code; colouring them would feed junk to the parser.
    #[test]
    fn headers_are_left_uncoloured() {
        let lines = vec![line(LineKind::Header, "@@ -1,3 +1,3 @@")];
        let out = diff("main.rs", &lines);
        assert_eq!(out[0].len(), 1);
        assert_eq!(out[0][0].0.fg, None);
    }
}
