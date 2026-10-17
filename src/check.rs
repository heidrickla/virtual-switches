//! A configuration typed into a switch line by line, as a console boots it, and the same configuration
//! compared across two IOS XE releases: the lines an upgrade breaks, the lines it newly takes, and the
//! lines vswitch takes at neither.

use crate::cli::Engine;
use crate::platform::Stack;
use crate::{device, shell};

/// One configuration line as typed: its number in the file, the line, the switch's error when it
/// refused it, and whether it was taken only because a real switch held it (`live`).
#[derive(Clone, Debug, PartialEq)]
pub struct Typed {
    pub number: usize,
    pub line: String,
    pub error: Option<String>,
    pub held: bool,
}

/// Types a configuration in configuration mode: a capture's comments and top-level `version` line are
/// skipped (the version chose the release the engine runs), a banner's text is typed as it is, blank
/// lines, `!` lines and `end` among it, and `end` stops it. Returns the session, still in
/// configuration mode, and every line typed with its outcome.
pub fn type_config<'a>(
    engine: &'a Engine,
    stack: Stack,
    config: &str,
) -> (shell::Session<'a>, Vec<Typed>) {
    let mut s = shell::Session::new(engine, device::Device::new(stack));
    let typed = type_into(&mut s, config);
    (s, typed)
}

/// Types a configuration into a session in configuration mode, as `type_config` does; a lab types a
/// switch's files one after another into one session.
pub fn type_into(s: &mut shell::Session<'_>, config: &str) -> Vec<Typed> {
    type_traced(s, config)
        .into_iter()
        .map(|t| t.typed)
        .collect()
}

/// A line `type_traced` typed: its outcome, the entries it added to the session's `accepted`, and the
/// mode the session was in before it and after it.
pub struct Traced {
    pub typed: Typed,
    pub accepted: std::ops::Range<usize>,
    pub modes: (String, String),
}

/// `type_into`, each line typed with what it added to the session and the modes around it.
pub fn type_traced(s: &mut shell::Session<'_>, config: &str) -> Vec<Traced> {
    if !crate::cli::is_config(s.mode()) {
        s.type_line("enable");
        s.type_line("configure terminal");
    }
    // a switch's own printed configuration keeps its layout and its last change
    if let Some(layout) = device::Layout::of(config) {
        s.device.layout = Some(layout);
        if let Some(t) = config
            .lines()
            .find_map(|l| l.trim().strip_prefix("! Last configuration change at "))
            .and_then(crate::shell::parse_stamp)
        {
            s.device.last_change = Some(t);
        }
    }
    // a kept configuration was swept of its secrets: `<removed>` stands for each value
    let was = std::mem::replace(&mut s.redacted, true);
    let mut typed = Vec::new();
    for (i, line) in config.lines().enumerate() {
        let l = line.trim_end();
        let t = l.trim_start();
        // a banner's text is text: the session reads every line of it
        let text = s.in_banner();
        if !text
            && (t.is_empty()
                || t.starts_with('!')
                // the configuration's own version line; ` version 2` under `router rip` is a command
                || (l == t && t.starts_with("version "))
                || t.starts_with("Building configuration")
                || t.starts_with("Current configuration"))
        {
            continue;
        }
        if !text && t == "end" {
            break;
        }
        let before = s.accepted.len();
        let mode = s.mode().to_string();
        let error = match s.type_line(l) {
            (_, shell::Result::Ok) => None,
            (o, shell::Result::Error) => Some(
                o.iter()
                    .rev()
                    .find(|x| x.starts_with('%'))
                    .cloned()
                    .unwrap_or_default(),
            ),
        };
        let held = s.accepted.len() > before && s.accepted.last().is_some_and(|a| a.key.is_none());
        typed.push(Traced {
            typed: Typed {
                number: i + 1,
                line: t.to_string(),
                error,
                held,
            },
            accepted: before..s.accepted.len(),
            modes: (mode, s.mode().to_string()),
        });
        if s.closed {
            break;
        }
    }
    s.redacted = was;
    typed
}

/// The lines of one configuration typed at two releases, by outcome.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// taken at the first release, refused at the second, with the second's error
    pub breaks: Vec<Typed>,
    /// refused at the first release, taken at the second, with the first's error
    pub gains: Vec<Typed>,
    /// refused at both, with the second's error
    pub neither: Vec<Typed>,
    /// taken at the first release only because a real switch held it there, refused by the second
    /// release's books: no capture of the second release says whether it breaks
    pub unproven: Vec<Typed>,
    /// lines one release's session never typed, having left configuration mode before them (an
    /// `exit` the other release read in a submode): those the first did not reach, and the second
    pub unreached: (Vec<Typed>, Vec<Typed>),
    /// lines typed at both releases
    pub lines: usize,
}

/// Compares the same configuration typed at two releases, line by line: each line of the file
/// with itself.
pub fn compare(from: &[Typed], to: &[Typed]) -> Report {
    let by_number = |v: &[Typed]| -> std::collections::BTreeMap<usize, Typed> {
        v.iter().map(|t| (t.number, t.clone())).collect()
    };
    let (mut first, second) = (by_number(from), by_number(to));
    let mut r = Report::default();
    for (n, b) in second {
        let Some(a) = first.remove(&n) else {
            r.unreached.0.push(b);
            continue;
        };
        r.lines += 1;
        match (&a.error, &b.error) {
            (None, Some(_)) if a.held && !b.held => r.unproven.push(b),
            (None, Some(_)) => r.breaks.push(b),
            (Some(_), None) => r.gains.push(a),
            (Some(_), Some(_)) => r.neither.push(b),
            (None, None) => {}
        }
    }
    r.unreached.1 = first.into_values().collect();
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replayed_configuration_takes_the_sweep_s_placeholder_where_a_value_was() {
        let r = crate::release::Release::parse("17.15").unwrap();
        let mut e = Engine::new(crate::grammars::grammar("c9300", r).unwrap());
        // the books alone, so no capture's held line takes these
        e.held = crate::live::Held::default();
        let stack = || crate::platform::parse_stack("C9300-24T").unwrap();
        let config = "enable secret 9 <removed>\nline vty 0 4\n password 7 <removed>\nend\n";
        let (_, typed) = type_config(&e, stack(), config);
        assert!(typed.iter().all(|t| t.error.is_none()), "{typed:?}");
        // an interface is never a value the sweep took out
        let (_, typed) = type_config(&e, stack(), "interface <removed>\nend\n");
        assert!(typed[0].error.is_some());
        // replay leaves the session as it found it
        let (s, _) = type_config(&e, stack(), config);
        assert!(!s.redacted);
    }

    /// The 9300 books at 17.15, no capture's held line among them.
    fn books() -> Engine {
        let r = crate::release::Release::parse("17.15").unwrap();
        let mut e = Engine::new(crate::grammars::grammar("c9300", r).unwrap());
        e.held = crate::live::Held::default();
        e
    }

    #[test]
    fn a_banner_s_text_is_typed_whole_and_the_configuration_goes_on_after_it() {
        let e = books();
        let stack = || crate::platform::parse_stack("C9300-24T").unwrap();
        let banner = |s: &shell::Session<'_>, kind: &str| {
            s.device
                .global
                .iter()
                .find(|g| g.key == format!("banner {kind}"))
                .map(|g| g.text.clone())
                .unwrap_or_default()
        };
        // a `!` line, a blank line and a line reading `end` are its text
        let config = "hostname lab\nbanner motd ^C\n!!! Authorized access only !!!\n\nend\n^C\nntp server 10.1.1.1\nend\n";
        let (s, typed) = type_config(&e, stack(), config);
        assert_eq!(
            banner(&s, "motd"),
            "banner motd ^C\n!!! Authorized access only !!!\n\nend\n^C"
        );
        assert!(!s.in_banner());
        assert!(
            typed
                .iter()
                .any(|t| t.line == "ntp server 10.1.1.1" && t.error.is_none()),
            "{typed:?}"
        );
        // its delimiter on a line of `!` closes it
        let config = "banner login #\nAuthorized use only\n!!!!!!!!#\nntp server 10.1.1.1\n";
        let (s, typed) = type_config(&e, stack(), config);
        assert!(!s.in_banner());
        assert!(
            banner(&s, "login").contains("\nAuthorized use only\n!!!!!!!!"),
            "{}",
            banner(&s, "login")
        );
        assert!(
            typed
                .iter()
                .any(|t| t.line == "ntp server 10.1.1.1" && t.error.is_none()),
            "{typed:?}"
        );
    }

    #[test]
    fn a_version_line_under_a_block_is_a_command() {
        let e = books();
        let stack = crate::platform::parse_stack("C9300-24T").unwrap();
        let config = "version 17.15\nrouter rip\n version 2\n network 10.0.0.0\nend\n";
        let (_, typed) = type_config(&e, stack, config);
        let lines: Vec<&str> = typed.iter().map(|t| t.line.as_str()).collect();
        assert_eq!(lines, ["router rip", "version 2", "network 10.0.0.0"]);
        assert!(typed.iter().all(|t| t.error.is_none()), "{typed:?}");
    }

    #[test]
    fn lines_pair_by_their_place_in_the_file_and_one_release_may_not_reach_some() {
        // the second release's session left configuration mode after line 5
        let from = typed(&[
            ("a", true),
            ("b", true),
            ("c", false),
            ("d", true),
            ("e", true),
            ("f", false),
            ("g", false),
        ]);
        let to = typed(&[
            ("a", true),
            ("b", true),
            ("c", true),
            ("d", true),
            ("e", true),
        ]);
        let r = compare(&from, &to);
        assert_eq!(r.lines, 5);
        assert_eq!(r.gains.len(), 1);
        let unreached: Vec<&str> = r.unreached.1.iter().map(|t| t.line.as_str()).collect();
        assert_eq!(unreached, ["f", "g"]);
        assert!(r.unreached.0.is_empty());
    }

    fn typed(lines: &[(&str, bool)]) -> Vec<Typed> {
        lines
            .iter()
            .enumerate()
            .map(|(i, (l, ok))| Typed {
                number: i + 1,
                line: l.to_string(),
                error: (!ok).then(|| "% Invalid input detected at '^' marker.".to_string()),
                held: false,
            })
            .collect()
    }

    #[test]
    fn lines_sort_by_what_the_upgrade_does_to_them() {
        let from = typed(&[("a", true), ("b", true), ("c", false), ("d", false)]);
        let to = typed(&[("a", true), ("b", false), ("c", true), ("d", false)]);
        let r = compare(&from, &to);
        assert_eq!(r.lines, 4);
        assert_eq!(
            r.breaks.iter().map(|t| t.line.as_str()).collect::<Vec<_>>(),
            ["b"]
        );
        assert_eq!(
            r.gains.iter().map(|t| t.line.as_str()).collect::<Vec<_>>(),
            ["c"]
        );
        assert_eq!(
            r.neither
                .iter()
                .map(|t| t.line.as_str())
                .collect::<Vec<_>>(),
            ["d"]
        );
        // taken at the first release only on a capture's word: not a break
        let mut from = typed(&[("e", true)]);
        from[0].held = true;
        let r = compare(&from, &typed(&[("e", false)]));
        assert!(r.breaks.is_empty());
        assert_eq!(r.unproven.len(), 1);
    }
}
