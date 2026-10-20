# Read-only bracket diagnostics

`--check-brackets` performs diagnostics **without beautifying or modifying** the `.tex` file. When enabled, the diagnostic command returns before reaching the normal formatter and file writer.

## Print report to console

```powershell
tex-fmt --check-brackets "HW8.tex"
```

## Save report to a specified file

```powershell
tex-fmt --check-brackets --brackets-log "brackets.log" "HW8.tex"
```

The `--brackets-log PATH` option alone also enables bracket checking. The output file is created or replaced; the LaTeX source is never modified by either option.

## Beautify normally

```powershell
tex-fmt "HW8.tex"
```

To beautify and then check, run **two separate commands**. The diagnostic locations refer to the source as it exists at the time of the check.

Example report:

```text
HW8.tex: line 376, column 28: Unclosed opening '('
HW8.tex: line 382, column 41: Extra closing ']'
```

Rows/lines and columns are 1-based; columns count Unicode characters rather than bytes. If no issues exist, the output says `No bracket issues found.`.

This is a lexical diagnostic, not a full TeX grammar parser. Literal brackets in prose, macro constructs, and custom catcodes may produce false positives. Escaped punctuation, comments, and configured verbatim environments are skipped.

Regression test: `cargo test bracket_check::tests`.
