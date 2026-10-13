//! The part of a failed restore command's output worth showing. The whole output
//! is in the debug log.
//!
//! npm prints what is wrong first and then its usage text for the command, so the
//! last lines of its output say nothing about the cause. Its error is taken from
//! the first `npm error` line on, without the usage block and with a long list of
//! packages cut short. The output of any other tool is shown as its first and
//! last lines: a tool names the failing step early or late.

/// Lines kept from the start of a tool's output, and from its end.
const HEAD_LINES: usize = 8;
const TAIL_LINES: usize = 4;

/// Packages listed of one kind (`Missing:`, `Invalid:`) in an npm error.
const LISTED_PACKAGES: usize = 5;

/// What npm puts before each line of an error.
const NPM_ERROR: &str = "npm error";

/// The excerpt of `text`, the output of `program`.
pub(super) fn of(program: &str, text: &str) -> String {
    let lines: Vec<String> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .collect();
    let lines = match program {
        "npm" => npm(&lines).unwrap_or(lines),
        _ => lines,
    };
    head_and_tail(lines).join("\n")
}

/// Whether `npm ci` refused to install because the lock file does not match `package.json`.
pub(super) fn npm_lock_out_of_sync(text: &str) -> bool {
    text.contains("code EUSAGE") && text.contains("in sync")
}

fn head_and_tail(mut lines: Vec<String>) -> Vec<String> {
    if lines.len() <= HEAD_LINES + TAIL_LINES {
        return lines;
    }
    let omitted = lines.len() - HEAD_LINES - TAIL_LINES;
    let tail = lines.split_off(lines.len() - TAIL_LINES);
    lines.truncate(HEAD_LINES);
    lines.push(format!("… {omitted} lines omitted …"));
    lines.extend(tail);
    lines
}

/// The error lines of npm's output, or `None` if it has none.
fn npm(lines: &[String]) -> Option<Vec<String>> {
    let first = lines.iter().position(|l| content(l).is_some())?;
    let lines: Vec<String> = lines[first..].iter().filter(|l| !is_bare(l)).cloned().collect();
    Some(cap_lists(without_usage(lines)))
}

/// The text of an `npm error` line.
fn content(line: &str) -> Option<&str> {
    line.strip_prefix(NPM_ERROR).map(str::trim)
}

fn is_bare(line: &str) -> bool {
    content(line).is_some_and(str::is_empty)
}

/// `lines` without the usage block that follows an error: from the command's
/// description, the line before `Usage:`, through `Run "npm help <command>" for more info`.
fn without_usage(mut lines: Vec<String>) -> Vec<String> {
    let Some(usage) = lines.iter().position(|l| content(l) == Some("Usage:")) else {
        return lines;
    };
    // Without the closing line the block runs up to npm's pointer to its log, or to the end.
    let end = lines[usage..]
        .iter()
        .position(|l| content(l).is_some_and(|c| c.starts_with("Run \"npm help ")))
        .map(|at| usage + at + 1)
        .or_else(|| {
            lines[usage..]
                .iter()
                .position(|l| content(l).is_some_and(|c| c.starts_with("A complete log")))
                .map(|at| usage + at)
        })
        .unwrap_or(lines.len());
    lines.drain(usage.saturating_sub(1)..end);
    lines
}

/// `lines` with at most `LISTED_PACKAGES` of each run of `Missing:` or `Invalid:` lines.
fn cap_lists(lines: Vec<String>) -> Vec<String> {
    let kind = |line: &str| {
        let content = content(line)?;
        ["Missing:", "Invalid:"].into_iter().find(|k| content.starts_with(k))
    };
    let mut kept = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let run = lines[at..]
            .iter()
            .take_while(|l| kind(l).is_some() && kind(l) == kind(&lines[at]))
            .count();
        if run == 0 {
            kept.push(lines[at].clone());
            at += 1;
            continue;
        }
        kept.extend(lines[at..at + run.min(LISTED_PACKAGES)].iter().cloned());
        if run > LISTED_PACKAGES {
            kept.push(format!("{NPM_ERROR} … and {} more", run - LISTED_PACKAGES));
        }
        at += run;
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    /// npm 11.6.0 refusing `npm ci` for a lock file that lacks packages.
    fn out_of_sync() -> String {
        let mut lines = vec![
            "npm warn Unknown project config \"x\"",
            "npm error code EUSAGE",
            "npm error",
            "npm error `npm ci` can only install packages when your package.json and package-lock.json or npm-shrinkwrap.json are in sync. Please update your lock file with `npm install` before continuing.",
            "npm error",
        ];
        let missing: Vec<String> = (0..20)
            .map(|i| format!("npm error Missing: pkg-{i}@1.0.{i} from lock file"))
            .collect();
        lines.extend(missing.iter().map(String::as_str));
        lines.extend([
            "npm error",
            "npm error Clean install a project",
            "npm error",
            "npm error Usage:",
            "npm error npm ci",
            "npm error",
            "npm error Options:",
            "npm error [--install-strategy <hoisted|nested|shallow|linked>] [--legacy-bundling]",
            "npm error [--global-style] [--omit <dev|optional|peer> [--omit <dev|optional|peer> ...]]",
            "npm error [--include <prod|dev|peer|optional> [--include <prod|dev|peer|optional> ...]]",
            "npm error [--strict-peer-deps] [--foreground-scripts] [--ignore-scripts] [--no-audit]",
            "npm error [--no-bin-links] [--no-fund] [--dry-run]",
            "npm error [-w|--workspace <workspace-name> [-w|--workspace <workspace-name> ...]]",
            "npm error [-ws|--workspaces] [--include-workspace-root] [--install-links]",
            "npm error",
            "npm error aliases: clean-install, ic, install-clean, isntall-clean",
            "npm error",
            "npm error Run \"npm help ci\" for more info",
            "npm error A complete log of this run can be found in: /home/u/.npm/_logs/2025-01-01-debug-0.log",
        ]);
        lines.join("\n") + "\n"
    }

    #[test]
    fn npms_error_is_shown_without_its_usage_text_and_with_long_lists_cut_short() {
        let excerpt = of("npm", &out_of_sync());
        let lines: Vec<&str> = excerpt.lines().collect();
        assert_eq!(lines[0], "npm error code EUSAGE");
        assert!(lines[1].contains("are in sync"), "{excerpt}");
        assert_eq!(lines[2], "npm error Missing: pkg-0@1.0.0 from lock file");
        assert_eq!(lines[6], "npm error Missing: pkg-4@1.0.4 from lock file");
        assert_eq!(lines[7], "npm error … and 15 more");
        assert!(
            lines.last().unwrap().contains("A complete log of this run"),
            "{excerpt}"
        );
        for gone in [
            "Clean install",
            "Usage:",
            "Options:",
            "aliases:",
            "npm help ci",
            "npm warn",
        ] {
            assert!(!excerpt.contains(gone), "{gone} in {excerpt}");
        }
        assert_eq!(lines.len(), 9, "{excerpt}");
    }

    #[test]
    fn an_npm_error_without_usage_text_is_kept_as_it_is() {
        let text = "npm error code E404\nnpm error 404 Not Found - GET https://registry/x\nnpm error\nnpm error A complete log of this run can be found in: /l\n";
        assert_eq!(
            of("npm", text),
            "npm error code E404\nnpm error 404 Not Found - GET https://registry/x\nnpm error A complete log of this run can be found in: /l"
        );
    }

    #[test]
    fn a_usage_block_without_its_closing_line_ends_at_npms_pointer_to_its_log() {
        let text = "npm error code EUSAGE\nnpm error why\nnpm error\nnpm error Install\nnpm error Usage:\nnpm error npm i\nnpm error A complete log of this run can be found in: /l\n";
        assert_eq!(
            of("npm", text),
            "npm error code EUSAGE\nnpm error why\nnpm error A complete log of this run can be found in: /l"
        );
    }

    #[test]
    fn npm_output_without_an_error_line_is_cut_like_any_tool_output() {
        let text: String = (0..30).map(|i| format!("line {i}\n")).collect();
        let excerpt = of("npm", &text);
        assert!(
            excerpt.starts_with("line 0\n") && excerpt.ends_with("line 29"),
            "{excerpt}"
        );
    }

    #[test]
    fn another_tools_output_keeps_its_first_and_last_lines() {
        let text: String = (0..30).map(|i| format!("line {i}\n\n")).collect();
        let excerpt = of("pnpm", &text);
        let lines: Vec<&str> = excerpt.lines().collect();
        assert_eq!(lines.len(), HEAD_LINES + 1 + TAIL_LINES);
        assert_eq!(lines[0], "line 0");
        assert_eq!(lines[HEAD_LINES - 1], "line 7");
        assert_eq!(lines[HEAD_LINES], "… 18 lines omitted …");
        assert_eq!(lines[HEAD_LINES + 1], "line 26");
        assert_eq!(*lines.last().unwrap(), "line 29");
    }

    #[test]
    fn short_output_is_kept_whole() {
        assert_eq!(of("cargo", "a\n\nb\nc\n"), "a\nb\nc");
        assert_eq!(of("cargo", "\n"), "");
    }

    #[test]
    fn only_npms_refusal_over_the_lock_file_counts_as_out_of_sync() {
        assert!(npm_lock_out_of_sync(&out_of_sync()));
        assert!(!npm_lock_out_of_sync("npm error code E404\nnpm error 404 Not Found\n"));
        assert!(!npm_lock_out_of_sync(
            "npm error code EUSAGE\nnpm error `npm ci` can only install with an existing package-lock.json\n"
        ));
    }
}
