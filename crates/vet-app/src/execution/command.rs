//! A run target's program and arguments as one editable line, and back. The line is what the
//! user types and the backend runs, so the rules are defined here and nowhere else.

/// What the user is told when the line names no program.
pub const NO_COMMAND: &str = "a run target needs a command";

/// Splits `line` into the program and its arguments. Whitespace separates words; double quotes
/// group (`\"` and `\\` are the only escapes), single quotes group literally, and adjacent
/// quoted and unquoted parts form one word. Nothing else is interpreted (no shell expansion):
/// the program is run directly. Fails with a message fit to show if the line has no program or a
/// quote is left open.
pub fn split(line: &str) -> Result<(String, Vec<String>), String> {
    let mut words: Vec<String> = Vec::new();
    // The word being read; `Some("")` after `""`, which is an empty word, not a gap.
    let mut word: Option<String> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let word = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        None => return Err("a double quote is not closed".into()),
                        Some('"') => break,
                        Some('\\') => match chars.clone().next() {
                            Some(escaped @ ('"' | '\\')) => {
                                chars.next();
                                word.push(escaped);
                            }
                            _ => word.push('\\'),
                        },
                        Some(other) => word.push(other),
                    }
                }
            }
            '\'' => {
                let word = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        None => return Err("a single quote is not closed".into()),
                        Some('\'') => break,
                        Some(other) => word.push(other),
                    }
                }
            }
            c if c.is_whitespace() => words.extend(word.take()),
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    let mut words = words.into_iter();
    match words.next() {
        Some(program) if !program.is_empty() => Ok((program, words.collect())),
        _ => Err(NO_COMMAND.into()),
    }
}

/// The line that [`split`]s into `program` and `args`.
pub fn format(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote(word: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c);
    if !word.is_empty() && word.chars().all(plain) {
        return word.to_owned();
    }
    let mut out = String::with_capacity(word.len() + 2);
    out.push('"');
    for c in word.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        let (program, args) = split(line).unwrap();
        std::iter::once(program).chain(args).collect()
    }

    #[test]
    fn whitespace_separates_words() {
        assert_eq!(words("  cargo \t run\n--bin  tool "), ["cargo", "run", "--bin", "tool"]);
        assert_eq!(split("ls").unwrap(), ("ls".to_owned(), vec![]));
    }

    #[test]
    fn double_quotes_group_and_escape_quotes_and_backslashes() {
        assert_eq!(
            words(r#"echo "a  b" "say \"hi\"" "c:\\dir" "\n""#),
            ["echo", "a  b", "say \"hi\"", "c:\\dir", "\\n"]
        );
    }

    #[test]
    fn single_quotes_are_literal() {
        assert_eq!(words(r#"sh -c 'echo "$HOME" \n'"#), ["sh", "-c", "echo \"$HOME\" \\n"]);
    }

    #[test]
    fn quoted_and_unquoted_parts_join_into_one_word() {
        assert_eq!(words(r#"a"b c"'d e'f"#), ["ab cd ef"]);
    }

    #[test]
    fn empty_quotes_make_an_empty_argument() {
        assert_eq!(words(r#"prog "" '' x"#), ["prog", "", "", "x"]);
    }

    #[test]
    fn no_shell_expansion() {
        assert_eq!(
            words("echo $HOME ~ * a|b > out"),
            ["echo", "$HOME", "~", "*", "a|b", ">", "out"]
        );
    }

    #[test]
    fn a_line_without_a_program_is_refused() {
        for line in ["", "   ", "\t\n", r#""""#, "''", r#""" x"#] {
            assert_eq!(split(line).unwrap_err(), NO_COMMAND, "{line:?}");
        }
    }

    #[test]
    fn an_open_quote_is_refused() {
        assert!(split(r#"echo "abc"#).is_err());
        assert!(split("echo 'abc").is_err());
        // An escaped quote does not close the string.
        assert!(split(r#"echo "abc\""#).is_err());
    }

    #[test]
    fn format_quotes_what_split_would_take_apart() {
        assert_eq!(
            format("cargo", &["run".into(), "--bin=a-b".into()]),
            "cargo run --bin=a-b"
        );
        assert_eq!(
            format("sh", &["-c".into(), "echo $X".into(), String::new()]),
            r#"sh -c "echo $X" """#
        );
        assert_eq!(format("p", &["a\"b\\c".into()]), r#"p "a\"b\\c""#);
    }

    #[test]
    fn format_and_split_round_trip() {
        let args: Vec<String> = [
            "",
            " ",
            "a b",
            "'",
            "\"",
            "\\",
            "\\\"",
            "line\nbreak",
            "ünï",
            "$(x)",
            "-c",
        ]
        .map(String::from)
        .to_vec();
        let line = format("my prog", &args);
        assert_eq!(split(&line).unwrap(), ("my prog".to_owned(), args));
    }
}
