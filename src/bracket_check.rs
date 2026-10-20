//! Optional, non-mutating source-level delimiter diagnostics.
//! This is a lexical checker, not a complete TeX parser.

use std::path::Path;

/// One delimiter issue in the original source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// 1-based source line.
    pub line: usize,
    /// 1-based character column.
    pub column: usize,
    /// Explanation.
    pub message: String,
}

/// Independent matching scope. Delimiters never match across math boundaries.
#[derive(Debug)]
struct Scope {
    name: String,
    opened_at: (usize, usize),
    brackets: Vec<(char, usize, usize)>,
}

fn flush_scope(scope: &mut Scope, errors: &mut Vec<Diagnostic>) {
    for (ch, line, column) in scope.brackets.drain(..) {
        errors.push(Diagnostic { line, column, message: format!("Unclosed opening '{ch}'") });
    }
}

fn is_math_environment(name: &str) -> bool {
    matches!(name, "math" | "displaymath" | "equation" | "equation*" | "align" | "align*"
        | "alignat" | "alignat*" | "flalign" | "flalign*" | "gather" | "gather*"
        | "multline" | "multline*" | "split" | "aligned" | "alignedat" | "gathered"
        | "cases" | "dcases" | "matrix" | "pmatrix" | "bmatrix" | "Bmatrix"
        | "vmatrix" | "Vmatrix" | "smallmatrix")
}

/// Lexically check bracket nesting, with a separate stack for each math block.
/// This is not a full TeX parser. Math boundaries must be well formed.
#[must_use]
pub fn check_brackets(source: &str, verbatims: &[String]) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let mut scopes = vec![Scope { name: String::new(), opened_at: (0, 0), brackets: Vec::new() }];
    let mut verbatim: Option<String> = None;
    for (li, line) in source.lines().enumerate() {
        let number = li + 1;
        let trimmed = line.trim_start();
        if let Some(ref env) = verbatim {
            if trimmed.starts_with(&format!("\\end{{{env}}}")) {
                verbatim = None;
            }
            continue;
        }
        if let Some(name) = verbatims.iter().find(|name| trimmed.starts_with(&format!("\\begin{{{name}}}"))) {
            verbatim = Some(name.clone());
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '%' { break; }
            if c == '\\' {
                if i + 1 < chars.len() && !chars[i + 1].is_alphabetic() {
                    let next = chars[i + 1];
                    let marker = match next { '(' => Some("\\("), ')' => Some("\\("), '[' => Some("\\["), ']' => Some("\\["), _ => None };
                    if let Some(name) = marker {
                        if next == '(' || next == '[' {
                            scopes.push(Scope {name: name.to_string(), opened_at: (number, i + 1), brackets: Vec::new()});
                        } else if scopes.last().is_some_and(|s| s.name == name) {
                            let mut scope = scopes.pop().expect("math scope exists");
                            flush_scope(&mut scope, &mut errors);
                        }
                    }
                    // Escaped punctuation is not a bracket for ordinary matching.
                    i += 2;
                    continue;
                }
                let start = i + 1;
                i = start;
                while i < chars.len() && (chars[i].is_alphabetic() || chars[i] == '@') { i += 1; }
                let command: String = chars[start..i].iter().collect();
                if (command == "begin" || command == "end") && i < chars.len() && chars[i] == '{' {
                    if let Some(end) = (i+1..chars.len()).find(|&j| chars[j] == '}') {
                        let name: String = chars[i+1..end].iter().collect();
                        if is_math_environment(&name) {
                            let marker = format!("env:{name}");
                            if command == "begin" {
                                scopes.push(Scope { name: marker, opened_at: (number, start), brackets: Vec::new() });
                            } else if scopes.last().is_some_and(|s| s.name == marker) {
                                let mut scope = scopes.pop().expect("math scope exists");
                                flush_scope(&mut scope, &mut errors);
                            }
                        }
                        // Environment name braces are TeX syntax, not bracket content.
                        i = end + 1;
                    }
                }
                continue;
            }
            if c == '$' {
                let double = i + 1 < chars.len() && chars[i + 1] == '$';
                let name = if double { "$$" } else { "$" };
                if scopes.last().is_some_and(|s| s.name == name) {
                    let mut scope = scopes.pop().expect("math scope exists");
                    flush_scope(&mut scope, &mut errors);
                } else if scopes.last().is_some_and(|s| s.name.is_empty()) {
                    scopes.push(Scope {name: name.to_string(), opened_at: (number, i + 1), brackets: Vec::new()});
                }
                i += if double { 2 } else { 1 };
                continue;
            }
            if matches!(c, '{' | '[' | '(') {
                scopes.last_mut().expect("document scope exists").brackets.push((c, number, i + 1));
            } else if matches!(c, '}' | ']' | ')') {
                let expected = match c { '}' => '{', ']' => '[', ')' => '(', _ => unreachable!() };
                let stack = &mut scopes.last_mut().expect("document scope exists").brackets;
                match stack.last().copied() {
                    Some((opener, _, _)) if opener == expected => { stack.pop(); }
                    Some((opener, open_line, open_col)) => {
                        errors.push(Diagnostic {line: number, column: i + 1, message: format!("Mismatched closing '{c}': expected closing for '{opener}' opened at {open_line}:{open_col}")});
                    }
                    None => errors.push(Diagnostic {line: number, column: i + 1, message: format!("Extra closing '{c}'")}),
                }
            }
            i += 1;
        }
    }
    for mut scope in scopes {
        flush_scope(&mut scope, &mut errors);
    }
    errors.sort_by_key(|d| (d.line, d.column));
    errors
}

/// Render a compact, aligned diagnostic table for each source file.
#[must_use]
pub fn format_diagnostics(file: &Path, diagnostics: &[Diagnostic]) -> String {
    if diagnostics.is_empty() {
        return String::new();
    }

    // The diagnostic text always contains the offending symbol in single quotes.
    // Retain any additional mismatch explanation in the Description column.
    let rows: Vec<(usize, usize, String, String)> = diagnostics.iter().map(|d| {
        let (description, symbol) = if let Some((before, rest)) = d.message.split_once('\'') {
            if let Some((symbol, after)) = rest.split_once('\'') {
                (format!("{}{}", before.trim_end(), after), format!("'{}'", symbol))
            } else {
                (d.message.clone(), String::new())
            }
        } else {
            (d.message.clone(), String::new())
        };
        (d.line, d.column, description, symbol)
    }).collect();

    let row_width = diagnostics.iter().map(|d| d.line.to_string().len()).max().unwrap_or(0).max(4);
    let col_width = diagnostics.iter().map(|d| d.column.to_string().len()).max().unwrap_or(0).max(4);
    let desc_width = rows.iter().map(|r| r.2.chars().count()).max().unwrap_or(0).max("Description".len());
    let mut report = format!("{}\n", file.display());
    report.push_str(&format!("{:>row_width$}  {:>col_width$}  {:<desc_width$}  Symbol\n", "Row#", "Col#", "Description"));
    for (line, column, description, symbol) in rows {
        report.push_str(&format!("{line:>row_width$}  {column:>col_width$}  {description:<desc_width$}  {symbol}\n"));
    }
    report.push('\n');
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_extra_closers() {
        let d = check_brackets("a ) b ] c }", &[]);
        assert_eq!(d.len(), 3);
        assert!(d.iter().all(|d| d.message.contains("Extra")));
    }
    #[test]
    fn reports_unclosed_openers() {
        let d = check_brackets("{([", &[]);
        assert_eq!(d.len(), 3);
    }
    #[test]
    fn skips_comments_and_escaped_delimiters() {
        assert!(check_brackets(r"\{ \} \% % ({[", &[]).is_empty());
    }
    #[test]
    fn skips_verbatim_content() {
        assert!(check_brackets("\\begin{verbatim}\n({[\n\\end{verbatim}", &["verbatim".to_string()]).is_empty());
    }
    #[test]
    fn reports_line_and_column() {
        let diagnostics = check_brackets("text\n  (\n", &[]);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 2);
        assert_eq!(diagnostics[0].column, 3);
        let report = format_diagnostics(Path::new("example.tex"), &diagnostics);
        assert!(report.contains("example.tex\n"));
        assert!(report.contains("Row#  Col#"));
        assert!(report.contains("2     3"));
        assert!(report.contains("Unclosed opening"));
        assert!(report.contains("'('"));
    }
    #[test]
    fn math_brackets_do_not_match_document_brackets() {
        let d = check_brackets("\\[\n(x\n\\]\n)\n", &[]);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].line, d[0].column), (2, 1));
        assert_eq!((d[1].line, d[1].column), (4, 1));
    }
    #[test]
    fn dollar_scopes_are_independent() {
        let d = check_brackets("$( $ )", &[]);
        assert_eq!(d.len(), 2);
    }
    #[test]
    fn math_environment_isolated_from_following_text() {
        let d = check_brackets("\\begin{align}\n(x\n\\end{align}\n)\n", &[]);
        assert_eq!(d.len(), 2);
    }
    #[test]
    fn nested_math_parentheses_match_inside_one_block() {
        assert!(check_brackets("\\[ (a[b] + {c}) \\]", &[]).is_empty());
    }
    #[test]
    fn crossed_brackets_in_same_scope_fail() {
        assert!(!check_brackets("\\[ ([)] \\]", &[]).is_empty());
    }
    #[test]
    fn dollar_escape_does_not_start_math() {
        assert!(check_brackets(r"\$ (a) \$", &[]).is_empty());
    }
    #[test]
    fn checks_math_brackets() {
        assert_eq!(check_brackets("$x=(1$", &[]).len(), 1);
    }
}

/// Escape source code and tooltips for safe inclusion in an HTML document.
fn escape_html(text: &str) -> String {
    let mut escaped = String::new();
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Render a single original source file with every source line numbered.
/// Offending delimiter characters are red and carry diagnostic tooltips.
#[must_use]
pub fn format_html_source(file: &Path, source: &str, diagnostics: &[Diagnostic]) -> String {
    let mut html = format!("<section><h2>{}</h2><div class=\"source\">\n", escape_html(&file.display().to_string()));
    // `lines()` preserves interior blank lines; an empty file still gets line 1.
    let lines: Vec<&str> = if source.is_empty() { vec![""] } else { source.lines().collect() };
    for (index, line) in lines.iter().enumerate() {
        let line_number = index + 1;
        let faulty_line = diagnostics.iter().any(|d| d.line == line_number);
        let line_class = if faulty_line { "lineno bad-line" } else { "lineno" };
        html.push_str(&format!("<div class=\"line\"><span class=\"{line_class}\">{line_number}</span><span class=\"code\">"));
        for (offset, c) in line.chars().enumerate() {
            let column = offset + 1;
            let issues: Vec<&Diagnostic> = diagnostics.iter().filter(|d| d.line == line_number && d.column == column).collect();
            if issues.is_empty() {
                html.push_str(&escape_html(&c.to_string()));
            } else {
                let messages = issues.iter().map(|d| d.message.as_str()).collect::<Vec<_>>().join("; ");
                html.push_str(&format!("<span class=\"bad\" title=\"{} (line {line_number}, column {column})\">{}</span>", escape_html(&messages), escape_html(&c.to_string())));
            }
        }
        html.push_str("</span></div>\n");
    }
    html.push_str("</div></section>\n");
    html
}

/// HTML document wrapper. Source files are never modified by this renderer.
#[must_use]
pub fn html_start() -> &'static str {
    "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>tex-fmt bracket report</title><style>body{font-family:system-ui,sans-serif;margin:1.5em}h2{font-size:1em;overflow-wrap:anywhere}.source{font:14px/1.5 Consolas,'Courier New',monospace;overflow-x:auto;border:1px solid #ccc;padding:0.6em 0}.line{display:flex;min-width:max-content;white-space:pre}.lineno{position:sticky;left:0;flex:none;text-align:right;width:5em;padding-right:1em;margin-right:1em;color:#666;background:#fff;border-right:1px solid #ddd;user-select:none}.code{white-space:pre}.lineno.bad-line{color:#c00;font-weight:bold}.bad{color:#c00;background:#ffe3e3;font-weight:bold;text-decoration:underline;text-decoration-color:#c00}</style></head><body><h1>Bracket diagnostics</h1>\n"
}

/// Close the HTML document.
#[must_use]
pub fn html_end() -> &'static str { "</body></html>\n" }

#[cfg(test)]
mod html_tests {
    use super::*;
    #[test]
    fn includes_all_line_numbers_and_marks_original_character() {
        let source = "a<(\n\nline 3 ]";
        let diagnostics = check_brackets(source, &[]);
        let html = format_html_source(Path::new("a.tex"), source, &diagnostics);
        assert!(html.contains("&lt;"));
        assert!(html.contains("class=\"lineno\">1"));
        assert!(html.contains("class=\"lineno\">2"));
        assert!(html.contains("class=\"lineno\">3"));
        assert!(html.contains("line 1, column 3"));
        assert!(html.contains("line 3, column 8"));
        assert!(html.contains("class=\"bad\""));
        assert!(html.contains("class=\"lineno bad-line\">1"));
        assert!(html.contains("class=\"lineno\">2"));
        assert!(html.contains("class=\"lineno bad-line\">3"));
    }
}
