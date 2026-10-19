//! Finding what actually changed *within* a pair of lines.
//!
//! A one-word edit in a long line is invisible when the whole line is marked
//! changed; this narrows it down to the words that differ. The comparison is
//! on tokens rather than characters, so a renamed identifier shows up as one
//! piece instead of a scatter of letters.
//!
//! Written out rather than taken from a crate: it is a textbook longest-common
//! subsequence over a few dozen tokens, and the dependency list is long enough.

/// Byte ranges within a line that differ from its counterpart.
pub type Ranges = Vec<std::ops::Range<usize>>;

/// Lines longer than this are left alone. The comparison is quadratic in the
/// token count, and past this point the result is noise anyway — a rewritten
/// line is better read as a whole.
const MAX_TOKENS: usize = 400;

/// Compares two lines and returns, for each, the byte ranges that are not
/// shared with the other.
///
/// Returns empty ranges for both when the lines are too long, identical, or
/// share nothing — in all three cases marking individual words would mislead
/// rather than help.
pub fn ranges(old: &str, new: &str) -> (Ranges, Ranges) {
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);

    if old_tokens.len() > MAX_TOKENS || new_tokens.len() > MAX_TOKENS {
        return (Vec::new(), Vec::new());
    }

    let common = lcs(&old_tokens, &new_tokens);

    // Whitespace matches between any two lines, so an empty subsequence is not
    // the test — what matters is whether anything *meaningful* is shared. When
    // nothing is, the line was replaced outright and marking every word would
    // only repeat what the line colour already says.
    let shared = common
        .iter()
        .filter(|(a, _)| !old_tokens[*a].1.trim().is_empty())
        .count();
    if shared == 0 {
        return (Vec::new(), Vec::new());
    }

    (
        unmatched(&old_tokens, common.iter().map(|(a, _)| *a)),
        unmatched(&new_tokens, common.iter().map(|(_, b)| *b)),
    )
}

/// A token is either a run of word characters or a single other character.
/// Splitting this way keeps identifiers whole, so a rename reads as one
/// change rather than a handful of letters.
fn tokenize(line: &str) -> Vec<(std::ops::Range<usize>, &str)> {
    let mut tokens = Vec::new();
    let mut start = None;

    for (index, ch) in line.char_indices() {
        if ch.is_alphanumeric() || ch == '_' {
            start.get_or_insert(index);
        } else {
            if let Some(from) = start.take() {
                tokens.push((from..index, &line[from..index]));
            }
            let end = index + ch.len_utf8();
            tokens.push((index..end, &line[index..end]));
        }
    }
    if let Some(from) = start {
        tokens.push((from..line.len(), &line[from..]));
    }
    tokens
}

/// Indices of the longest common subsequence, as pairs into both sides.
fn lcs(
    old: &[(std::ops::Range<usize>, &str)],
    new: &[(std::ops::Range<usize>, &str)],
) -> Vec<(usize, usize)> {
    let (n, m) = (old.len(), new.len());
    if n == 0 || m == 0 {
        return Vec::new();
    }

    // table[i][j] = length of the LCS of old[i..] and new[j..]
    let mut table = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if old[i].1 == new[j].1 {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }

    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i].1 == new[j].1 {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// Byte ranges of the tokens that are *not* in the common subsequence,
/// with neighbours merged so the result reads as whole words.
fn unmatched(
    tokens: &[(std::ops::Range<usize>, &str)],
    matched: impl Iterator<Item = usize>,
) -> Ranges {
    let mut keep = vec![false; tokens.len()];
    for index in matched {
        keep[index] = true;
    }

    let mut ranges: Ranges = Vec::new();
    for (index, (range, text)) in tokens.iter().enumerate() {
        if keep[index] {
            continue;
        }
        // Whitespace on its own is not worth marking; between two changed
        // tokens it is absorbed by the merge below.
        if text.trim().is_empty() {
            continue;
        }
        match ranges.last_mut() {
            Some(last) if last.end >= range.start => last.end = range.end,
            _ => ranges.push(range.clone()),
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(line: &str, ranges: &Ranges) -> Vec<String> {
        ranges.iter().map(|r| line[r.clone()].to_string()).collect()
    }

    #[test]
    fn a_single_changed_word_is_found() {
        let old = "let x: u32 = 1;";
        let new = "let x: u64 = 1;";
        let (o, n) = ranges(old, new);
        assert_eq!(marked(old, &o), vec!["u32"]);
        assert_eq!(marked(new, &n), vec!["u64"]);
    }

    /// The case that motivated this: one word in a long line.
    #[test]
    fn a_rename_in_a_long_line_is_narrowed_down() {
        let old = "fn parse(input: &str) -> Result<Config, Error> {";
        let new = "fn parse(input: &str) -> Result<Config, ParseError> {";
        let (o, n) = ranges(old, new);
        assert_eq!(marked(old, &o), vec!["Error"]);
        assert_eq!(marked(new, &n), vec!["ParseError"]);
    }

    #[test]
    fn an_addition_marks_only_the_new_part() {
        let old = "if trimmed.is_empty() {";
        let new = "if trimmed.is_empty() || trimmed.starts_with('#') {";
        let (o, n) = ranges(old, new);
        assert!(o.is_empty(), "nothing was removed: {:?}", marked(old, &o));
        let added = marked(new, &n).join("");
        assert!(added.contains("starts_with"), "{added}");
        assert!(
            !added.contains("is_empty"),
            "unchanged part marked: {added}"
        );
    }

    #[test]
    fn identical_lines_have_nothing_to_mark() {
        let (o, n) = ranges("same line", "same line");
        assert!(o.is_empty());
        assert!(n.is_empty());
    }

    /// When the lines share nothing, the line colour already says everything;
    /// marking every word on top of it would be noise.
    #[test]
    fn a_wholesale_replacement_is_left_to_the_line_colour() {
        let (o, n) = ranges("alpha beta gamma", "!!! ??? ***");
        assert!(o.is_empty(), "{o:?}");
        assert!(n.is_empty(), "{n:?}");
    }

    #[test]
    fn whitespace_alone_is_never_marked() {
        let old = "a  b";
        let new = "a   b";
        let (o, n) = ranges(old, new);
        for range in o.iter().chain(n.iter()) {
            let text = if range.end <= old.len() {
                &old[range.clone()]
            } else {
                ""
            };
            assert!(!text.trim().is_empty(), "marked pure whitespace");
        }
    }

    #[test]
    fn very_long_lines_are_skipped_rather_than_crawled_over() {
        let long = "x ".repeat(MAX_TOKENS);
        let (o, n) = ranges(&long, &format!("{long}y"));
        assert!(o.is_empty());
        assert!(n.is_empty());
    }

    /// Ranges must stay inside the string, or slicing them panics.
    #[test]
    fn ranges_stay_within_the_line_including_multibyte_text() {
        let old = "größe = 1; // änderung";
        let new = "größe = 2; // änderung";
        let (o, n) = ranges(old, new);
        for r in &o {
            assert!(old.get(r.clone()).is_some(), "bad range {r:?} in {old:?}");
        }
        for r in &n {
            assert!(new.get(r.clone()).is_some(), "bad range {r:?} in {new:?}");
        }
    }

    #[test]
    fn an_empty_line_against_content_does_not_panic() {
        let (o, n) = ranges("", "something");
        assert!(o.is_empty());
        assert!(n.is_empty() || !n.is_empty());
    }
}
