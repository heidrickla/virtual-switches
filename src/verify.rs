//! Which lines of a configuration rest on a real switch and which on the books alone: a line a capture
//! of the same platform and release held, a command a capture used in another form, or one only the
//! books give, which is the list to check on hardware (Lewis, 2026-10-05: "use the hardware verified
//! commands vs derived commands to create live hardware lists that need to be live hardware
//! verified").

use crate::check::type_traced;
use crate::cli::{Engine, Item, Outcome};
use crate::platform::Stack;
use crate::shell::Session;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Evidence {
    /// a capture of this platform and release held the line
    Held,
    /// a capture used the same command (its keywords) with other arguments
    Form,
    /// only the books give it
    Books,
}

#[derive(Debug, PartialEq)]
pub struct Line {
    /// the top-level line of the block it went into ("" at the top level)
    pub block: String,
    /// the section lines it was typed under inside its block, as typed (` address-family ipv4`)
    pub sections: Vec<String>,
    /// the line as the switch took it; a banner, its whole text through its closing delimiter
    pub line: String,
    /// its indentation as typed
    pub indent: String,
    pub evidence: Evidence,
}

/// The keywords of a line a session took, as the grammar reads it in the mode it was typed in, the
/// one it left the session in, or global configuration: its command without the arguments, and its
/// `no`. A banner's text is its argument.
fn form(engine: &Engine, line: &str, modes: &(String, String)) -> Option<Vec<String>> {
    if let Some(rest) = line.strip_prefix("banner ") {
        return Some(vec![
            "banner".into(),
            rest.split_whitespace().next()?.into(),
        ]);
    }
    [modes.0.as_str(), modes.1.as_str(), "config"]
        .iter()
        .find_map(|m| match engine.parse_redacted(m, line) {
            Outcome::Ok(p) => Some(
                p.items
                    .iter()
                    .filter_map(|i| match i {
                        Item::Kw { kw, .. } => Some(kw.to_lowercase()),
                        Item::Arg { .. } => None,
                    })
                    .collect(),
            ),
            _ => None,
        })
}

/// For each entry a session took: the line of the file it came from, and its keywords (`form`).
type Taken = Vec<(usize, Option<Vec<String>>)>;

/// A configuration typed as `check::type_config` types it, and what each entry the session took
/// came from (`Taken`); no keywords for a line taken as a capture held it.
fn typed<'e>(engine: &'e Engine, stack: Stack, config: &str) -> (Session<'e>, Taken) {
    let mut s = Session::new(engine, crate::device::Device::new(stack));
    let traced = type_traced(&mut s, config);
    let mut at = vec![(0, None); s.accepted.len()];
    for t in &traced {
        for i in t.accepted.clone() {
            let a = &s.accepted[i];
            at[i] = (
                t.typed.number,
                a.key.as_ref().and_then(|_| form(engine, &a.line, &t.modes)),
            );
        }
    }
    (s, at)
}

/// Where a line sits in the file it was typed from: the section lines between it and its block's
/// own line, each as typed, and its indentation. A section line is a line the session took above it
/// and less indented, short of the block's unindented line.
fn place(file: &[&str], number: usize, taken: &BTreeSet<usize>) -> (Vec<String>, String) {
    let depth = |l: &str| l.len() - l.trim_start().len();
    let Some(line) = number.checked_sub(1).and_then(|i| file.get(i)) else {
        return (vec![], String::new());
    };
    let indent = line[..depth(line)].to_string();
    let mut need = indent.len();
    let mut sections = vec![];
    for &n in taken.range(1..number).rev() {
        let l = file[n - 1];
        let d = depth(l);
        if need == 0 || d == 0 {
            break;
        }
        if d < need {
            sections.push(l.trim_end().to_string());
            need = d;
        }
    }
    sections.reverse();
    (sections, indent)
}

/// The commands captures used, by the kind of block they sat in: their keywords. Each capture is
/// typed on its own hardware.
pub fn capture_forms(
    engine: &Engine,
    captures: &[(&str, Stack)],
) -> BTreeSet<(String, Vec<String>)> {
    let mut forms = BTreeSet::new();
    for (text, stack) in captures {
        let (s, at) = typed(engine, stack.clone(), text);
        forms.extend(
            s.accepted
                .iter()
                .zip(at)
                .filter_map(|(a, (_, f))| f.map(|f| (a.kind(), f))),
        );
    }
    forms
}

/// Each line `config` takes, with its evidence; and the captures it was weighed against.
pub fn classify(engine: &Engine, stack: Stack, config: &str) -> (Vec<Line>, Vec<String>) {
    // the commands the captures of the engine's book and release used (the corpus's forms)
    let book = crate::grammars::book_of(&engine.grammar.platform).unwrap_or_default();
    let forms: BTreeSet<(String, Vec<String>)> = crate::corpus::get()
        .forms
        .iter()
        .filter(|(b, r, ..)| b == book && *r == engine.grammar.release)
        .map(|(_, _, k, f)| (k.clone(), f.clone()))
        .collect();
    let (s, at) = typed(engine, stack, config);
    let file: Vec<&str> = config.lines().collect();
    let taken: BTreeSet<usize> = at.iter().map(|(n, _)| *n).collect();
    let lines = s
        .accepted
        .iter()
        .zip(at)
        .map(|(a, (number, f))| {
            let used = f.is_some_and(|f| forms.contains(&(a.kind(), f)));
            let evidence = match &a.key {
                None => Evidence::Held,
                Some(_) if engine.held.has_exact(&a.kind(), &a.line) => Evidence::Held,
                Some(_) if used => Evidence::Form,
                Some(_) => Evidence::Books,
            };
            // a banner is typed whole: the device keeps its text through the closing delimiter
            let line = s
                .device
                .global
                .iter()
                .find(|e| {
                    a.line.starts_with("banner ")
                        && a.key.as_deref() == Some(e.key.as_str())
                        && e.text.starts_with(a.line.as_str())
                })
                .map_or_else(|| a.line.clone(), |e| e.text.clone());
            let (sections, indent) = place(&file, number, &taken);
            Line {
                block: a.block.clone(),
                sections,
                line,
                indent,
                evidence,
            }
        })
        .collect();
    (lines, engine.held.captures.clone())
}

/// The lines only the books give, as a script to type on a switch: each under its block's line and
/// the section lines it was typed under, indented as typed; a banner whole.
pub fn script(lines: &[Line]) -> Vec<String> {
    let mut out = vec![];
    // the block and section lines the last line printed is typed under
    let mut at: Vec<String> = vec![];
    for l in lines.iter().filter(|l| l.evidence == Evidence::Books) {
        if l.block.is_empty() {
            out.extend(l.line.lines().map(str::to_string));
            at = vec![l.line.clone()];
            continue;
        }
        let path: Vec<String> = std::iter::once(l.block.clone())
            .chain(l.sections.iter().cloned())
            .collect();
        let same = at.iter().zip(&path).take_while(|(a, b)| a == b).count();
        if same < path.len() {
            out.extend(path[same..].iter().cloned());
        } else if at.len() > path.len() {
            // out of a section: its block's line again takes the switch back
            out.push(path[path.len() - 1].clone());
        }
        at = path;
        let indent = if l.indent.is_empty() { " " } else { &l.indent };
        out.push(format!("{indent}{}", l.line));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::Release;

    fn engine() -> Engine {
        Engine::new(crate::grammars::grammar("c9400", Release::parse("17.5").unwrap()).unwrap())
    }

    #[test]
    fn a_line_rests_on_a_capture_a_command_form_or_the_books() {
        let e = engine();
        let stack = crate::platform::parse_stack("1=C9400-LC-48T,3=C9400-SUP-1XL").unwrap();
        let config = "\
service call-home
hostname lab
interface GigabitEthernet1/0/5
 description bmc
 switchport mode trunk
 spanning-tree portfast trunk
 udld port aggressive
!
end
";
        let (lines, captures) = classify(&e, stack, config);
        assert_eq!(captures.len(), 2);
        let ev = |l: &str| lines.iter().find(|x| x.line == l).unwrap().evidence;
        // the books do not carry `service call-home`; the cores held it
        assert_eq!(ev("service call-home"), Evidence::Held);
        assert_eq!(ev("switchport mode trunk"), Evidence::Held);
        assert_eq!(ev("spanning-tree portfast trunk"), Evidence::Held);
        // the cores have hostnames and descriptions, of their own
        assert_eq!(ev("hostname lab"), Evidence::Form);
        assert_eq!(ev("description bmc"), Evidence::Form);
        // neither core runs UDLD
        assert_eq!(ev("udld port aggressive"), Evidence::Books);
        assert_eq!(
            script(&lines),
            ["interface GigabitEthernet1/0/5", " udld port aggressive"]
        );
    }

    #[test]
    fn a_form_is_the_same_command_with_other_arguments() {
        let e = engine();
        let stack = crate::platform::parse_stack("1=C9400-LC-48T,3=C9400-SUP-1XL").unwrap();
        let config = "ip route 10.9.9.0 255.255.255.0 10.0.0.1\ninterface GigabitEthernet1/0/5\n spanning-tree bpduguard disable\n duplex half\n!\nend\n";
        let (lines, _) = classify(&e, stack, config);
        let ev = |l: &str| lines.iter().find(|x| x.line == l).unwrap().evidence;
        // the cores route other prefixes
        assert_eq!(
            ev("ip route 10.9.9.0 255.255.255.0 10.0.0.1"),
            Evidence::Form
        );
        // the cores enable BPDU guard and run full duplex: another keyword is another command
        assert_eq!(ev("spanning-tree bpduguard disable"), Evidence::Books);
        assert_eq!(ev("duplex half"), Evidence::Books);
    }

    #[test]
    fn a_line_from_the_books_is_scripted_under_the_sections_it_was_typed_in() {
        let e = engine();
        let stack = crate::platform::parse_stack("1=C9400-LC-48T,3=C9400-SUP-1XL").unwrap();
        let config = "\
vrf definition Mgmt-vrf
 !
 address-family ipv4
  route-target export 1:1
 exit-address-family
!
end
";
        let (lines, _) = classify(&e, stack, config);
        assert_eq!(
            script(&lines),
            [
                "vrf definition Mgmt-vrf",
                " address-family ipv4",
                "  route-target export 1:1",
            ]
        );
        // out of a section, its block's line again; into another, that section's line
        let line = |sections: &[&str], text: &str| Line {
            block: "vrf definition lab".into(),
            sections: sections.iter().map(|s| s.to_string()).collect(),
            line: text.into(),
            indent: " ".repeat(sections.len() + 1),
            evidence: Evidence::Books,
        };
        let lines = [
            line(&[" address-family ipv4"], "route-target export 1:1"),
            line(&[" address-family ipv6"], "route-target export 1:2"),
            line(&[], "rd 1:1"),
        ];
        assert_eq!(
            script(&lines),
            [
                "vrf definition lab",
                " address-family ipv4",
                "  route-target export 1:1",
                " address-family ipv6",
                "  route-target export 1:2",
                "vrf definition lab",
                " rd 1:1"
            ]
        );
    }

    #[test]
    fn a_banner_from_the_books_is_scripted_whole() {
        let e = Engine::new(
            crate::grammars::grammar("c9300", crate::release::Release::parse("17.12").unwrap())
                .unwrap(),
        );
        let stack = crate::platform::parse_stack("C9300X-24Y").unwrap();
        let config = "hostname lab\nbanner motd ^C\nAuthorized only\n\nSecond line\n^C\nntp server 10.1.1.1\nend\n";
        let (lines, _) = classify(&e, stack, config);
        assert_eq!(
            script(&lines),
            [
                "hostname lab",
                "banner motd ^C",
                "Authorized only",
                "",
                "Second line",
                "^C",
                "ntp server 10.1.1.1"
            ]
        );
    }

    #[test]
    fn with_no_capture_every_line_is_the_books() {
        // 17.12: no 9300 capture holds that release
        let e = Engine::new(
            crate::grammars::grammar("c9300", crate::release::Release::parse("17.12").unwrap())
                .unwrap(),
        );
        let stack = crate::platform::parse_stack("C9300X-24Y").unwrap();
        let (lines, captures) = classify(&e, stack, "hostname lab\nvlan 10\n name x\nend\n");
        assert!(captures.is_empty());
        assert!(lines.iter().all(|l| l.evidence == Evidence::Books));
        assert_eq!(script(&lines), ["hostname lab", "vlan 10", " name x"]);
    }
}
