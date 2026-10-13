use std::ops::Range;

use regex::{Regex, RegexBuilder};
use vet_app::search::{LineMatcher, SearchPattern};

/// A [`SearchPattern`] compiled. A literal pattern is escaped; whole words are delimited with `\b`.
pub struct RegexMatcher {
    /// `None` for an empty pattern, which matches nothing.
    regex: Option<Regex>,
    /// The pattern matched over a whole text, where `^` and `$` stand for the ends of its lines:
    /// a line matches only if the text does. `None` where the pattern can't be read that way.
    whole: Option<Regex>,
}

impl RegexMatcher {
    /// Fails with the regex engine's message, reduced to the part fit to show.
    pub fn compile(pattern: &SearchPattern) -> Result<Self, String> {
        if pattern.text.is_empty() {
            return Ok(Self {
                regex: None,
                whole: None,
            });
        }
        let source = if pattern.regex {
            pattern.text.clone()
        } else {
            regex::escape(&pattern.text)
        };
        let source = if pattern.whole_word {
            format!(r"\b(?:{source})\b")
        } else {
            source
        };
        let regex = RegexBuilder::new(&source)
            .case_insensitive(!pattern.match_case)
            .build()
            .map_err(|e| short_message(&e))?;
        Ok(Self {
            regex: Some(regex),
            whole: whole_text(&source, pattern.match_case),
        })
    }
}

/// `source` over a whole text, a line ending at `\n`, `\r\n` or `\r` as the search splits lines.
/// Not for a pattern that anchors at the ends of the text itself (`\A`, `\z`), which a line's
/// own ends would satisfy.
fn whole_text(source: &str, match_case: bool) -> Option<Regex> {
    let hir = regex_syntax::ParserBuilder::new()
        .multi_line(true)
        .crlf(true)
        .case_insensitive(!match_case)
        .build()
        .parse(source)
        .ok()?;
    if hir.properties().look_set().contains_anchor_haystack() {
        return None;
    }
    RegexBuilder::new(source)
        .multi_line(true)
        .crlf(true)
        .case_insensitive(!match_case)
        .build()
        .ok()
}

impl LineMatcher for RegexMatcher {
    fn hits(&self, line: &str) -> Vec<Range<usize>> {
        let Some(regex) = &self.regex else {
            return Vec::new();
        };
        regex
            .find_iter(line)
            .map(|m| m.range())
            .filter(|r| !r.is_empty())
            .collect()
    }

    fn may_match(&self, text: &str) -> bool {
        match (&self.regex, &self.whole) {
            (None, _) => false,
            (Some(_), Some(whole)) => whole.is_match(text),
            (Some(_), None) => true,
        }
    }
}

/// A syntax error is printed with the pattern and a marker under it; its last line says what is
/// wrong.
fn short_message(error: &regex::Error) -> String {
    let message = error.to_string();
    let last = message.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default();
    last.trim().strip_prefix("error: ").unwrap_or(last.trim()).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(text: &str, match_case: bool, whole_word: bool, regex: bool) -> SearchPattern {
        SearchPattern {
            text: text.into(),
            match_case,
            whole_word,
            regex,
        }
    }

    fn hits(p: &SearchPattern, line: &str) -> Vec<Range<usize>> {
        RegexMatcher::compile(p).unwrap().hits(line)
    }

    #[test]
    fn a_plain_pattern_is_literal_and_ignores_case_unless_asked() {
        let line = "a.b A.B axb";
        assert_eq!(hits(&pattern("a.b", false, false, false), line), [0..3, 4..7]);
        assert_eq!(hits(&pattern("a.b", true, false, false), line), vec![0..3]);
    }

    #[test]
    fn a_regex_pattern_is_rust_syntax() {
        assert_eq!(hits(&pattern(r"a.b", true, false, true), "a.b axb"), [0..3, 4..7]);
        assert_eq!(hits(&pattern(r"\d+", true, false, true), "x12 y3"), [1..3, 5..6]);
    }

    #[test]
    fn whole_word_keeps_a_whole_alternation_inside_the_boundaries() {
        let p = pattern("ab|cd", true, true, true);
        assert_eq!(hits(&p, "ab abc cd xcd"), [0..2, 7..9]);
    }

    /// The same queries are run by the find bar's matcher (`ui/src/lib/search/query.test.ts`): what a
    /// plain query finds must not depend on which of the two runs it. Each row: text, [match case,
    /// whole word, regex], a line, and the text of the matches.
    #[test]
    fn plain_queries_find_what_the_find_bars_matcher_finds() {
        type Row = (&'static str, [bool; 3], &'static str, &'static [&'static str]);
        let rows: [Row; 10] = [
            (
                "über",
                [false, true, false],
                "über Über überall xüber über",
                &["über", "Über", "über"],
            ),
            ("Straße", [true, true, false], "Straße Straßen", &["Straße"]),
            ("é", [false, true, false], "café é", &["é"]),
            ("delay", [false, true, false], "max_delay delay delays", &["delay"]),
            ("ÜBER", [false, false, false], "über", &["über"]),
            ("ab|cd", [true, true, true], "ab abc cd xcd", &["ab", "cd"]),
            ("日本", [false, true, false], "日本語 日本", &["日本"]),
            ("a.b", [true, false, false], "a.b axb", &["a.b"]),
            ("a b", [false, true, false], "a b, a bc", &["a b"]),
            (".foo", [false, true, false], "x.foo .foo", &[".foo"]),
        ];
        for (text, [match_case, whole_word, regex], line, expected) in rows {
            let found: Vec<&str> = hits(&pattern(text, match_case, whole_word, regex), line)
                .into_iter()
                .map(|r| &line[r])
                .collect();
            assert_eq!(found, expected, "{text:?} in {line:?}");
        }
    }

    /// What the search does with a text a matcher rules out must be what matching its lines
    /// one by one would have found: nothing.
    #[test]
    fn a_text_is_ruled_out_only_if_none_of_its_lines_match() {
        let texts = [
            "",
            "alpha\nbeta\n",
            "use a;\nlet x;\n",
            "let x;\nuse a;",
            "x\ruse a\r\nend",
            "ab\ncd",
            "foo bar\n  baz",
            "é\nÜBER\n",
        ];
        let patterns = [
            pattern("use", false, false, false),
            pattern("^use", true, false, true),
            pattern("a;$", true, false, true),
            pattern(r"\Ause", true, false, true),
            pattern(r"a;\z", true, false, true),
            pattern(r"(?-m)^let", true, false, true),
            pattern("ab|cd", true, true, true),
            pattern(r"b\s+c", true, false, true),
            pattern(r"\bbaz\b", true, false, true),
            pattern("über", false, false, false),
            pattern(r"a.*b", false, false, true),
            pattern("^$", true, false, true),
        ];
        let mut ruled_out = 0;
        for p in &patterns {
            let matcher = RegexMatcher::compile(p).unwrap();
            for text in texts {
                let lines = text.split(['\n', '\r']);
                let found = lines.clone().any(|line| !matcher.hits(line).is_empty());
                if !matcher.may_match(text) {
                    assert!(!found, "{:?} was ruled out of {text:?}", p.text);
                    ruled_out += 1;
                }
            }
        }
        assert!(ruled_out > 20, "the matcher rules out almost nothing");
    }

    #[test]
    fn empty_matches_and_an_empty_pattern_find_nothing() {
        assert_eq!(
            hits(&pattern("x*", true, false, true), "abc"),
            Vec::<Range<usize>>::new()
        );
        assert_eq!(hits(&pattern("", true, false, true), "abc"), Vec::<Range<usize>>::new());
    }

    #[test]
    fn an_invalid_pattern_fails_with_a_one_line_message() {
        let error = RegexMatcher::compile(&pattern("a(", true, false, true)).err().unwrap();
        assert!(!error.contains('\n'), "{error}");
        assert!(error.contains("unclosed group"), "{error}");
    }
}
