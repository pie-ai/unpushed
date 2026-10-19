//! Turning a diff into coloured lines.
//!
//! Three readings are laid on top of each other, in order of how quickly they
//! need to register:
//!
//! 1. **Added or removed** — the whole line is green or red. This is what you
//!    scan for, so nothing is allowed to compete with it. An earlier version
//!    coloured only a one-character marker and left the content in syntax
//!    colours; added, removed and context lines then looked alike at a glance.
//! 2. **What changed inside the line** — the differing words are brightened
//!    and emboldened, so a one-word edit in a long line is not a hunt.
//! 3. **Language** — syntax colouring, but only on context lines, where the
//!    first two readings do not apply. That keeps the surrounding code easy to
//!    follow without muddying the signal.
//!
//! The syntax and theme sets are loaded once on first use, not at startup:
//! they cost both time and a few megabytes, and most sessions never open a
//! diff at all.
//!
//! `syntect` is pulled in without default features, using `fancy-regex`
//! instead of the C library Oniguruma. That keeps the dependency tree free of
//! C code, which is what makes the cross-compiled `armv7` build work at all.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
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

/// Renders every line of a diff, in the same order.
pub fn diff(path: &str, lines: &[DiffLine]) -> Vec<Highlighted> {
    let h = highlighter();
    let syntax = syntax_for(h, path);
    let mut context = HighlightLines::new(syntax, &h.theme);
    let word_ranges = intra_line_changes(lines);

    lines
        .iter()
        .enumerate()
        .map(|(index, line)| match line.kind {
            LineKind::Header => vec![(Style::default().fg(Color::Cyan), line.text.clone())],
            LineKind::Context => colour(&mut context, h, &line.text),
            LineKind::Removed => {
                marked(&line.text, &word_ranges[index], Color::Red, Color::LightRed)
            }
            LineKind::Added => marked(
                &line.text,
                &word_ranges[index],
                Color::Green,
                Color::LightGreen,
            ),
        })
        .collect()
}

/// Splits a line into plain and emphasised pieces.
fn marked(text: &str, ranges: &crate::worddiff::Ranges, base: Color, accent: Color) -> Highlighted {
    let plain = Style::default().fg(base);
    if ranges.is_empty() {
        return vec![(plain, text.to_string())];
    }
    let strong = Style::default().fg(accent).add_modifier(Modifier::BOLD);

    let mut out = Vec::new();
    let mut at = 0;
    for range in ranges {
        if range.start > at {
            out.push((plain, text[at..range.start].to_string()));
        }
        out.push((strong, text[range.clone()].to_string()));
        at = range.end;
    }
    if at < text.len() {
        out.push((plain, text[at..].to_string()));
    }
    out
}

/// Pairs removed lines with the added ones that replaced them and works out
/// which words differ.
///
/// Runs of removed lines are followed by runs of added ones; pairing them in
/// order is what git itself does when it shows a replacement. Where the runs
/// are of different lengths the surplus lines have no counterpart and stay
/// plain — a line that was only added or only removed has nothing to compare
/// against.
fn intra_line_changes(lines: &[DiffLine]) -> Vec<crate::worddiff::Ranges> {
    let mut ranges: Vec<crate::worddiff::Ranges> = vec![Vec::new(); lines.len()];
    let mut index = 0;

    while index < lines.len() {
        if lines[index].kind != LineKind::Removed {
            index += 1;
            continue;
        }
        let removed_from = index;
        while index < lines.len() && lines[index].kind == LineKind::Removed {
            index += 1;
        }
        let added_from = index;
        while index < lines.len() && lines[index].kind == LineKind::Added {
            index += 1;
        }

        let pairs = (added_from - removed_from).min(index - added_from);
        for offset in 0..pairs {
            let (old, new) = (removed_from + offset, added_from + offset);
            let (a, b) = crate::worddiff::ranges(&lines[old].text, &lines[new].text);
            ranges[old] = a;
            ranges[new] = b;
        }
    }
    ranges
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

    /// Headers are not code; they get their own colour rather than being fed
    /// to the syntax parser.
    #[test]
    fn headers_get_their_own_colour() {
        let lines = vec![line(LineKind::Header, "@@ -1,3 +1,3 @@")];
        let out = diff("main.rs", &lines);
        assert_eq!(out[0].len(), 1);
        assert_eq!(out[0][0].0.fg, Some(Color::Cyan));
    }

    /// The reading that has to register first: a glance must separate added
    /// from removed from context.
    #[test]
    fn added_and_removed_lines_are_coloured_whole() {
        let lines = vec![
            line(LineKind::Removed, "let x = 1;"),
            line(LineKind::Added, "let y = 2;"),
        ];
        let out = diff("main.rs", &lines);
        assert!(out[0]
            .iter()
            .all(|(s, _)| matches!(s.fg, Some(Color::Red) | Some(Color::LightRed))));
        assert!(out[1]
            .iter()
            .all(|(s, _)| matches!(s.fg, Some(Color::Green) | Some(Color::LightGreen))));
    }

    #[test]
    fn the_changed_words_are_picked_out_within_the_line() {
        let lines = vec![
            line(LineKind::Removed, "fn parse() -> Result<Config, Error> {"),
            line(
                LineKind::Added,
                "fn parse() -> Result<Config, ParseError> {",
            ),
        ];
        let out = diff("main.rs", &lines);

        let strong: Vec<&str> = out[1]
            .iter()
            .filter(|(s, _)| s.add_modifier.contains(Modifier::BOLD))
            .map(|(_, text)| text.as_str())
            .collect();
        assert_eq!(strong, vec!["ParseError"], "whole line was: {:?}", out[1]);
    }

    /// Context lines are where syntax colouring still earns its place.
    #[test]
    fn context_lines_keep_their_language_colours() {
        let lines = vec![line(LineKind::Context, "fn main() { let x = 1; }")];
        let out = diff("main.rs", &lines);
        assert!(out[0].len() > 1, "not split into tokens: {:?}", out[0]);
        let colours: std::collections::HashSet<_> =
            out[0].iter().filter_map(|(s, _)| s.fg).collect();
        assert!(colours.len() > 1, "only one colour: {colours:?}");
    }

    /// Runs of unequal length have surplus lines with no counterpart; those
    /// must stay plain rather than being compared to the wrong line.
    #[test]
    fn unpaired_lines_are_left_without_word_marks() {
        let lines = vec![
            line(LineKind::Removed, "only removed"),
            line(LineKind::Added, "only removed"),
            line(LineKind::Added, "an extra line"),
        ];
        let out = diff("main.rs", &lines);
        assert!(
            out[2]
                .iter()
                .all(|(s, _)| !s.add_modifier.contains(Modifier::BOLD)),
            "surplus line was marked: {:?}",
            out[2]
        );
    }
}
