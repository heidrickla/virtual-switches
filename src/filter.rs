//! Output modifiers typed after `|`: `include`, `exclude`, `begin`, `section` and `count`, each with
//! an IOS regular expression.

use crate::cli::{Outcome, tokens};
use regex::Regex;

#[derive(Debug)]
pub enum Modifier {
    Include(Regex),
    Exclude(Regex),
    Begin(Regex),
    /// `section` and `section include`: each section a line of which matches
    Section(Regex),
    /// `section exclude`: each section no line of which matches
    SectionExclude(Regex),
    Count(Regex),
}

const KEYWORDS: &[&str] = &[
    "append", "begin", "count", "exclude", "format", "include", "redirect", "section", "tee",
];

/// The characters `_` matches besides the start and end of the line.
const DELIMITERS: &str = ",{}() ";

/// An IOS regular expression: `_` matches a comma, a brace, a parenthesis, a space, or the start or
/// end of the line; inside a bracket expression, which holds characters only, the characters. An
/// expression ending in a lone `\` is incomplete, as any other malformed one.
fn ios_regex(pattern: &str) -> Option<Regex> {
    let mut out = String::new();
    let mut escaped = false;
    // inside `[...]`: whether a member is read yet (a `]` before one is a member) and whether a `^`
    // may still negate it
    let mut class: Option<(bool, bool)> = None;
    for c in pattern.chars() {
        if escaped {
            out.push('\\');
            out.push(c);
            escaped = false;
            if class.is_some() {
                class = Some((true, false));
            }
        } else if c == '\\' {
            escaped = true;
        } else if let Some((member, negatable)) = class {
            if c == ']' && member {
                class = None;
                out.push(c);
            } else if c == '^' && negatable {
                class = Some((false, false));
                out.push(c);
            } else {
                class = Some((true, false));
                if c == '_' {
                    out.push_str(DELIMITERS);
                } else {
                    out.push(c);
                }
            }
        } else if c == '[' {
            class = Some((false, true));
            out.push(c);
        } else if c == '_' {
            out.push_str(&format!("(?:^|$|[{DELIMITERS}])"));
        } else {
            out.push(c);
        }
    }
    if escaped {
        return None;
    }
    Regex::new(&out).ok()
}

/// The modifier in `text`, the part of a line after its `|`, which starts at byte `at` of the line.
/// A refusal is the error IOS prints, its caret placed in the whole line.
pub fn parse(text: &str, at: usize) -> Result<Modifier, Outcome> {
    let toks = tokens(text);
    let Some(word) = toks.first() else {
        return Err(Outcome::Incomplete);
    };
    let typed = word.text.to_lowercase();
    let matches: Vec<&str> = KEYWORDS
        .iter()
        .copied()
        .filter(|k| k.starts_with(&typed))
        .collect();
    let keyword = match matches.as_slice() {
        [one] => *one,
        [] => return Err(Outcome::Invalid(at + word.at)),
        _ => return Err(Outcome::Ambiguous),
    };
    // `section [include | exclude] <expression>`: a word either begins is that keyword
    let sub = match (keyword, toks.get(1)) {
        ("section", Some(t)) => {
            let typed = t.text.to_lowercase();
            ["include", "exclude"]
                .into_iter()
                .find(|k| k.starts_with(&typed))
        }
        _ => None,
    };
    // the expression is the rest of the line as typed, spaces included
    let Some(first) = toks.get(if sub.is_some() { 2 } else { 1 }) else {
        return Err(Outcome::Incomplete);
    };
    let pattern = text[first.at..].trim_end();
    let re = ios_regex(pattern).ok_or(Outcome::Invalid(at + first.at))?;
    Ok(match (keyword, sub) {
        ("include", _) => Modifier::Include(re),
        ("exclude", _) => Modifier::Exclude(re),
        ("begin", _) => Modifier::Begin(re),
        ("section", Some("exclude")) => Modifier::SectionExclude(re),
        ("section", _) => Modifier::Section(re),
        ("count", _) => Modifier::Count(re),
        // append, format, redirect and tee write files or reformat; the switch keeps no files
        _ => return Err(Outcome::Invalid(at + word.at)),
    })
}

/// The lines of `output` the modifier lets through.
pub fn apply(m: &Modifier, output: Vec<String>) -> Vec<String> {
    match m {
        Modifier::Include(re) => output.into_iter().filter(|l| re.is_match(l)).collect(),
        Modifier::Exclude(re) => output.into_iter().filter(|l| !re.is_match(l)).collect(),
        Modifier::Begin(re) => output.into_iter().skip_while(|l| !re.is_match(l)).collect(),
        Modifier::Count(re) => vec![format!(
            "Number of lines which match regexp = {}",
            output.iter().filter(|l| re.is_match(l)).count()
        )],
        Modifier::Section(re) => sections(output, |lines| lines.iter().any(|l| re.is_match(l))),
        Modifier::SectionExclude(re) => {
            sections(output, |lines| !lines.iter().any(|l| re.is_match(l)))
        }
    }
}

/// The sections of `output` that `keep` takes, each whole: a section is a line at the left margin and
/// the indented lines under it.
fn sections(output: Vec<String>, keep: impl Fn(&[String]) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < output.len() {
        let mut end = i + 1;
        if !output[i].starts_with(' ') {
            while end < output.len() && output[end].starts_with(' ') {
                end += 1;
            }
        }
        if keep(&output[i..end]) {
            out.extend_from_slice(&output[i..end]);
        }
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    fn run(m: &str, text: &str) -> Vec<String> {
        apply(&parse(m, 0).unwrap(), lines(text))
    }

    const CONFIG: &str = "hostname s1\n!\ninterface Vlan1\n no ip address\n shutdown\n!\ninterface Vlan10\n ip address 10.0.0.1 255.255.255.0\n!\nline con 0\n stopbits 1\nno line auto-consolidation\nend";

    #[test]
    fn include_exclude_begin_and_count_filter_by_an_ios_expression() {
        assert_eq!(
            run("include Vlan", CONFIG),
            ["interface Vlan1", "interface Vlan10"]
        );
        assert_eq!(
            run("i Vlan1_", CONFIG),
            ["interface Vlan1"],
            "_ is the end of the line"
        );
        assert_eq!(run("exclude ^ [a-z]", CONFIG).len(), 9);
        assert_eq!(run("begin ^line", CONFIG)[0], "line con 0");
        assert_eq!(
            run("count address", CONFIG),
            ["Number of lines which match regexp = 2"]
        );
    }

    #[test]
    fn a_section_is_a_matching_line_and_the_lines_under_it() {
        assert_eq!(
            run("section line", CONFIG),
            ["line con 0", " stopbits 1", "no line auto-consolidation"]
        );
        // a line under a section shows its whole section
        assert_eq!(
            run("section 10.0.0.1", CONFIG),
            ["interface Vlan10", " ip address 10.0.0.1 255.255.255.0"]
        );
    }

    #[test]
    fn section_include_and_exclude_keep_or_drop_the_sections_that_match() {
        let vlan10 = ["interface Vlan10", " ip address 10.0.0.1 255.255.255.0"];
        assert_eq!(run("section include Vlan10", CONFIG), vlan10);
        assert_eq!(run("section i Vlan10", CONFIG), vlan10);
        assert_eq!(
            run("section exclude interface", CONFIG),
            [
                "hostname s1",
                "!",
                "!",
                "!",
                "line con 0",
                " stopbits 1",
                "no line auto-consolidation",
                "end"
            ]
        );
        // a word neither keyword begins is the expression
        assert_eq!(run("section interface Vlan10", CONFIG), vlan10);
        assert!(matches!(
            parse("section include", 0),
            Err(Outcome::Incomplete)
        ));
    }

    #[test]
    fn an_underscore_in_a_bracket_expression_is_a_delimiter_character() {
        let text = " description rack8::b\n description rack8 b\n description rack8_b";
        assert!(run("include 8:[_]", text).is_empty());
        assert_eq!(run("include 8[_]b", text), [" description rack8 b"]);
        assert_eq!(run("include 8[^_]b", text), [" description rack8_b"]);
        // `]` first in a class is a member, and `_` after it still a delimiter
        assert_eq!(run("include 8[]_]b", text), [" description rack8 b"]);
        assert_eq!(run("include 8_b", text), [" description rack8 b"]);
    }

    #[test]
    fn an_expression_ending_in_a_lone_backslash_is_refused() {
        assert!(matches!(parse("include \\", 0), Err(Outcome::Invalid(8))));
        assert!(matches!(parse("count x\\", 0), Err(Outcome::Invalid(6))));
        assert_eq!(run("include 8\\.1", "a 8.1\na 801"), ["a 8.1"]);
    }

    #[test]
    fn a_modifier_is_refused_as_ios_refuses_it() {
        assert!(matches!(parse("bogus x", 4), Err(Outcome::Invalid(4))));
        assert!(matches!(parse("include", 0), Err(Outcome::Incomplete)));
        assert!(matches!(parse("e x", 0), Ok(Modifier::Exclude(_))));
        assert!(matches!(parse("tee x", 2), Err(Outcome::Invalid(2))));
    }
}
