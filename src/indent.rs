//! Utilities for indenting source lines

use crate::args::Args;
use crate::comments::{find_comment_index, remove_comment};
use crate::format::{Pattern, State};
use crate::logging::{record_line_log, Log};
use crate::regexes::{ENV_BEGIN, ENV_END, ITEM, VERBS};
use core::cmp::max;
use log::Level;
use log::LevelFilter;
use std::path::Path;

/// Opening delimiters
const OPENS: [char; 2] = ['{', '['];
/// Closing delimiters
const CLOSES: [char; 2] = ['}', ']'];

/// Information on the indentation state of a line
#[derive(Debug, Clone)]
pub struct Indent {
    /// The indentation level of a line
    pub actual: i8,
    /// The visual indentation level of a line
    pub visual: i8,
}

impl Indent {
    /// Construct a new indentation state
    #[must_use]
    pub const fn new() -> Self {
        Self {
            actual: 0,
            visual: 0,
        }
    }
}

impl Default for Indent {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate total indentation change due to the current line
fn get_diff(
    line: &str,
    pattern: &Pattern,
    lists_begin: &[String],
    lists_end: &[String],
    no_indent_envs_begin: &[String],
    no_indent_envs_end: &[String],
) -> i8 {
    // Do not indent if line contains \verb|...|
    if pattern.contains_verb && VERBS.iter().any(|x| line.contains(x)) {
        return 0;
    }

    // Indentation for environments
    let mut diff: i8 = 0;
    if pattern.contains_env_begin && line.contains(ENV_BEGIN) {
        if no_indent_envs_begin.iter().any(|r| line.contains(r)) {
            return 0;
        }
        diff += 1;
        diff += i8::from(lists_begin.iter().any(|r| line.contains(r)));
    } else if pattern.contains_env_end && line.contains(ENV_END) {
        if no_indent_envs_end.iter().any(|r| line.contains(r)) {
            return 0;
        }
        diff -= 1;
        diff -= i8::from(lists_end.iter().any(|r| line.contains(r)));
    }

    diff
}

/// Calculate dedentation for the current line
fn get_back(
    line: &str,
    pattern: &Pattern,
    state: &State,
    lists_end: &[String],
    no_indent_envs_end: &[String],
) -> i8 {
    // Only need to dedent if indentation is present
    if state.indent.actual == 0 {
        return 0;
    }
    let mut back: i8 = 0;

    // Don't apply any indenting if a \verb|...| is present
    if pattern.contains_verb && VERBS.iter().any(|x| line.contains(x)) {
        return 0;
    }

    // Calculate dedentation for environments
    if pattern.contains_env_end && line.contains(ENV_END) {
        // Some environments are not indented
        if no_indent_envs_end.iter().any(|r| line.contains(r)) {
            return 0;
        }
        // List environments get double indents for indenting items
        for r in lists_end {
            if line.contains(r) {
                return 2;
            }
        }
        // Other environments get single indents
        back = 1;
    } else if pattern.contains_item && line.contains(ITEM) {
        // Deindent items to make the rest of item environment appear indented
        back += 1;
    }

    back
}

/// Math delimiters are lexical state, not structural indentation delimiters.
/// Ordinary parentheses never affect indentation: they are TeX text/math characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MathMode {
    #[default]
    None,
    Dollar,
    DoubleDollar,
    Paren,
    Bracket,
}

/// Calculate delimiter-based indentation, ignoring math contents, including
/// math spans extending across lines. Escaped dollars and escaped braces are
/// handled as literal characters. Environments are tracked separately.
fn get_diff_back_delim(line: &str, mode: &mut MathMode) -> (i8, i8) {
    let mut diff: i8 = 0;
    let mut back: i8 = 0;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Backslash escaping and math delimiters (\(, \), \[ and \]).
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            match (*mode, next) {
                (MathMode::None, b'(') => *mode = MathMode::Paren,
                (MathMode::Paren, b')') => *mode = MathMode::None,
                (MathMode::None, b'[') => *mode = MathMode::Bracket,
                (MathMode::Bracket, b']') => *mode = MathMode::None,
                _ => {}
            }
            i += 2;
            continue;
        }
        if bytes[i] == b'$' && matches!(*mode, MathMode::None | MathMode::Dollar | MathMode::DoubleDollar) {
            if *mode == MathMode::DoubleDollar {
                if bytes.get(i + 1) == Some(&b'$') {
                    *mode = MathMode::None;
                    i += 2;
                    continue;
                }
            } else if *mode == MathMode::Dollar {
                *mode = MathMode::None;
                i += 1;
                continue;
            } else if bytes.get(i + 1) == Some(&b'$') {
                *mode = MathMode::DoubleDollar;
                i += 2;
                continue;
            } else {
                *mode = MathMode::Dollar;
                i += 1;
                continue;
            }
        }
        if *mode == MathMode::None {
            let c = line[i..].chars().next().expect("valid UTF-8 character");
            diff -= i8::from(OPENS.contains(&c));
            diff += i8::from(CLOSES.contains(&c));
            back = max(diff, back);
            i += c.len_utf8();
        } else {
            // Math text cannot alter surrounding delimiter indentation.
            i += line[i..].chars().next().expect("valid UTF-8 character").len_utf8();
        }
    }
    (-diff, back)
}

/// Math environments count as environments, but their formula content should
/// not contribute delimiter indentation.
fn math_environment(line: &str, prefix: &str) -> Option<&'static str> {
    const ENVS: [&str; 12] = [
        "math", "displaymath", "equation", "equation*", "align", "align*",
        "gather", "gather*", "multline", "multline*", "flalign", "flalign*",
    ];
    ENVS.into_iter().find(|env| line.contains(&format!("{prefix}{{{env}}}")))
}

/// Calculate indentation properties of the current line
#[allow(clippy::too_many_arguments)]
fn get_indent(
    line: &str,
    prev_indent: &Indent,
    pattern: &Pattern,
    state: &State,
    lists_begin: &[String],
    lists_end: &[String],
    no_indent_envs_begin: &[String],
    no_indent_envs_end: &[String],
    math_mode: &mut MathMode,
    math_env: &mut Option<String>,
) -> Indent {
    let mut diff = get_diff(
        line,
        pattern,
        lists_begin,
        lists_end,
        no_indent_envs_begin,
        no_indent_envs_end,
    );
    let mut back =
        get_back(line, pattern, state, lists_end, no_indent_envs_end);
    // The opening/closing environment still controls indentation normally.
    let closing_math = math_env.as_ref().is_some_and(|name| line.contains(&format!("\\end{{{name}}}")));
    let opening_math = math_env.is_none().then(|| math_environment(line, "\\begin")).flatten();
    let diff_back_delim = if math_env.is_some() || opening_math.is_some() {
        (0, 0)
    } else {
        get_diff_back_delim(line, math_mode)
    };
    if closing_math { *math_env = None; }
    if let Some(name) = opening_math {
        if !line.contains(&format!("\\end{{{name}}}")) {
            *math_env = Some(name.to_owned());
        }
    }
    diff += diff_back_delim.0;
    back += diff_back_delim.1;
    let actual = prev_indent.actual + diff;
    let visual = prev_indent.actual - back;
    Indent { actual, visual }
}

/// Calculates the indent for `line` based on its contents.
/// This functions saves the calculated [Indent], which might be
/// negative, to the given [State], and then ensures that the returned
/// [Indent] is non-negative.
#[allow(clippy::too_many_arguments)]
pub fn calculate_indent(
    line: &str,
    state: &mut State,
    logs: &mut Vec<Log>,
    file: &Path,
    args: &Args,
    pattern: &Pattern,
    lists_begin: &[String],
    lists_end: &[String],
    no_indent_envs_begin: &[String],
    no_indent_envs_end: &[String],
) -> Indent {
    // Calculate the new indent by first removing the comment from the line
    // (if there is one) to ignore diffs from characters in there.
    let comment_index = find_comment_index(line, pattern);
    let line_strip = remove_comment(line, comment_index);
    let mut math_mode = state.math_mode;
    let mut math_env = state.math_env.clone();
    let mut indent = get_indent(
        line_strip,
        &state.indent,
        pattern,
        state,
        lists_begin,
        lists_end,
        no_indent_envs_begin,
        no_indent_envs_end,
        &mut math_mode,
        &mut math_env,
    );
    state.math_mode = math_mode;
    state.math_env = math_env;

    // Record the indent to the logs.
    if args.verbosity == LevelFilter::Trace {
        record_line_log(
            logs,
            Level::Trace,
            file,
            state.linum_new,
            state.linum_old,
            line,
            &format!(
                "Indent: actual = {}, visual = {}:",
                indent.actual, indent.visual
            ),
        );
    }

    // Save the indent to the state. Note, this indent might be negative;
    // it is saved without correction so that this is
    // not forgotten for the next iterations.
    state.indent = indent.clone();

    // Update the last zero-indented line for use in error messages.
    if indent.visual == 0 && state.linum_new > state.linum_last_zero_indent {
        state.linum_last_zero_indent = state.linum_new;
    }

    // However, we can't negatively indent a line.
    // So we log the negative indent and reset the values to 0.
    if (indent.visual < 0) || (indent.actual < 0) {
        record_line_log(
            logs,
            Level::Warn,
            file,
            state.linum_new,
            state.linum_old,
            line,
            "Indent is negative.",
        );
        indent.actual = indent.actual.max(0);
        indent.visual = indent.visual.max(0);

        // If this is the first negatively indented line, record in the state
        if state.linum_first_negative_indent.is_none() {
            state.linum_first_negative_indent = Some(state.linum_new);
        }
    }

    indent
}

/// Apply the given indentation to a line
#[must_use]
pub fn apply_indent(
    line: &str,
    indent: &Indent,
    args: &Args,
    indent_char: &str,
) -> String {
    let first_non_whitespace = line.chars().position(|c| !c.is_whitespace());

    // If line is blank, return an empty line
    if first_non_whitespace.is_none() {
        return String::new();
    }

    // If line is correctly indented, return it directly
    #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
    let n_indent_chars = (indent.visual * args.tabsize as i8) as usize;
    if first_non_whitespace == Some(n_indent_chars) {
        return line.into();
    }

    // Otherwise, allocate enough memory to fit line with the added
    // indentation and insert the appropriate string slices
    let trimmed_line = line.trim_start();
    let mut new_line =
        String::with_capacity(trimmed_line.len() + n_indent_chars);
    for idx in 0..n_indent_chars {
        new_line.insert_str(idx, indent_char);
    }
    new_line.insert_str(n_indent_chars, trimmed_line);
    new_line
}

#[cfg(test)]
mod math_indent_tests {
    use super::{get_diff_back_delim, MathMode};

    #[test]
    fn ignores_parentheses_outside_math() {
        let mut mode = MathMode::None;
        assert_eq!(get_diff_back_delim("n(H(X_1)", &mut mode), (0, 0));
    }

    #[test]
    fn ignores_inline_math_and_resumes_afterwards() {
        let mut mode = MathMode::None;
        assert_eq!(get_diff_back_delim("$\\frac{a}{b(c}$", &mut mode), (0, 0));
        assert_eq!(mode, MathMode::None);
        assert_eq!(get_diff_back_delim("{text}", &mut mode), (0, 0));
    }

    #[test]
    fn multiline_display_math_keeps_indent_clean() {
        let mut mode = MathMode::None;
        assert_eq!(get_diff_back_delim("\\[", &mut mode), (0, 0));
        assert_eq!(mode, MathMode::Bracket);
        assert_eq!(get_diff_back_delim("\\frac{a}{(b}", &mut mode), (0, 0));
        assert_eq!(get_diff_back_delim("\\]", &mut mode), (0, 0));
        assert_eq!(mode, MathMode::None);
    }

    #[test]
    fn escaped_dollar_does_not_start_math() {
        let mut mode = MathMode::None;
        assert_eq!(get_diff_back_delim(r"\$ {hello", &mut mode), (1, 0));
        assert_eq!(mode, MathMode::None);
    }

    #[test]
    fn double_dollar_multiline() {
        let mut mode = MathMode::None;
        assert_eq!(get_diff_back_delim("$$", &mut mode), (0, 0));
        assert_eq!(mode, MathMode::DoubleDollar);
        assert_eq!(get_diff_back_delim("[ { (", &mut mode), (0, 0));
        assert_eq!(get_diff_back_delim("$$", &mut mode), (0, 0));
        assert_eq!(mode, MathMode::None);
    }
}
