//! Commands known to crash a switch, each as a lab switch measured it. The console warns after such
//! a line's answer, `probe` refuses to send one, and `explain` names one.

use crate::cli;
use crate::platform::Family;
use crate::release::Release;

/// A command that crashed a lab switch.
pub struct Hazard {
    /// the families it applies to, the one measured first
    pub families: &'static [Family],
    /// the release measured, as `show version` named it
    pub release: &'static str,
    /// the day it was measured
    pub date: &'static str,
    /// the command's keywords, whole
    pub command: &'static str,
    /// whether the command crashes with any further words too
    pub subtree: bool,
    /// shorter keywords IOS has where one of the command's stands, each a start of it: a word spelling
    /// one whole is that keyword (`show ip` is not `show ipv6`)
    pub shorter: &'static [&'static str],
    /// what the switch did
    pub effect: &'static str,
}

/// Each typed alone on one SSH session of a C9300-24T with a C9300-NM-4G at network-advantage:
/// IOSd segfaulted (`linux_iosd_image has failed (rc 139)`) and the switch reloaded with `Critical
/// software exception`. No published Cisco bug names either command.
pub const KNOWN: &[Hazard] = &[
    // the fault is in the process running the command (`SSH Process` over SSH); `show ip pim gir`,
    // `show ip igmp gir` and `show ipv6 mld gir` answer
    Hazard {
        families: &[Family::C9300],
        release: "17.6.4",
        date: "2026-10-17",
        command: "show ipv6 pim gir",
        subtree: false,
        shorter: &["i", "ip"],
        effect: "IOSd segfault and reload",
    },
    // every form tried under `bp` faults in `Sched`, the tppiosrp_bp TDL handler; the other
    // `content` forms answer
    Hazard {
        families: &[Family::C9300],
        release: "17.6.4",
        date: "2026-10-17",
        command: "show platform software tdl-database content bp",
        subtree: true,
        shorter: &[],
        effect: "IOSd segfault and reload",
    },
];

impl Hazard {
    /// What the command did on the switch measured: `crashed a Catalyst 9300 at IOS XE 17.6.4 (...)`.
    pub fn crashed(&self) -> String {
        format!("crashed {} ({})", self.measured_on(), self.effect)
    }

    /// What the command does to a switch at `release`: on the train measured, it crashes it; on
    /// another, what it did at the release measured.
    pub fn effect_at(&self, release: &str) -> String {
        let train = Release::parse(release);
        if train.is_some() && train == Release::parse(self.release) {
            format!(
                "crashes the switch ({} on {})",
                self.effect,
                self.measured_on()
            )
        } else {
            self.crashed()
        }
    }

    /// The console's warning, printed after the switch's answer to the line.
    pub fn warning(&self, release: &str) -> String {
        format!("% vswitch: this command {}", self.effect_at(release))
    }

    /// The command named with what it does at `release` and the day it was measured, for `explain`.
    pub fn described(&self, release: &str) -> String {
        format!(
            "'{}' {}, measured {}",
            self.command,
            self.effect_at(release),
            self.date
        )
    }

    fn measured_on(&self) -> String {
        let family = self.families.first().map_or("switch", |f| f.name());
        format!("a {family} at {}", crate::probe::named(self.release))
    }

    /// Whether an EXEC line's words name the command: each word a start of its keyword, in any case,
    /// and more words only under a subtree.
    fn named_by(&self, words: &[String]) -> bool {
        let keywords: Vec<&str> = self.command.split(' ').collect();
        let count = if self.subtree {
            words.len() >= keywords.len()
        } else {
            words.len() == keywords.len()
        };
        count
            && words
                .iter()
                .zip(&keywords)
                .all(|(w, k)| k.starts_with(w.as_str()) && !self.shorter.contains(&w.as_str()))
    }
}

/// An EXEC line's words as IOS reads them: before any `|`, lowercase, split on spaces and tabs.
fn words(line: &str) -> Vec<String> {
    line.split('|')
        .next()
        .unwrap_or("")
        .split(|c: char| c.is_whitespace() || c == '\u{feff}')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The EXEC line a line typed in `mode` runs: the line itself at an EXEC prompt; in configuration,
/// what follows `do` or `do-exec` (`cli::names_do_exec`), and none without one.
pub fn exec_line<'a>(mode: &str, line: &'a str) -> Option<&'a str> {
    if !cli::is_config(mode) {
        return Some(line);
    }
    let toks = cli::tokens(line);
    let first = toks.first()?;
    (first.text.eq_ignore_ascii_case("do") || cli::names_do_exec(&first.text))
        .then(|| &line[toks.get(1).map_or(line.len(), |t| t.at)..])
}

/// The known crash an EXEC line runs on a switch of `family`; with no family, on any.
pub fn find(family: Option<Family>, exec: &str) -> Option<&'static Hazard> {
    let w = words(exec);
    KNOWN
        .iter()
        .find(|h| family.is_none_or(|f| h.families.contains(&f)) && h.named_by(&w))
}

/// The known crash a line typed in `mode` runs on a switch of `family`; with no family, on any.
pub fn typed(family: Option<Family>, mode: &str, line: &str) -> Option<&'static Hazard> {
    find(family, exec_line(mode, line)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const C9300: Option<Family> = Some(Family::C9300);

    #[test]
    fn a_line_is_read_as_ios_reads_it() {
        for line in [
            "show ipv6 pim gir",
            "sh ipv6 pim gi",
            "SHOW IPV6 PIM GIR",
            "sh  ipv6\tpim gir ",
            "show ipv6 pim gir | include up",
            "show ipv6 pim gir|count",
            "show platform software tdl-database content bp",
            "show platform software tdl-database content bp config local",
            "show platform software tdl-database content bp config nodes",
            "show platform software tdl-database content bp config nodes 0",
            "sh plat soft tdl con bp oper local",
            "show platform software tdl-database content bp oper nodes",
        ] {
            for mode in ["exec", "privileged"] {
                assert!(typed(C9300, mode, line).is_some(), "{mode}: {line}");
            }
        }
        // from configuration only through `do` or `do-exec`, whole or abbreviated
        for line in [
            "do show ipv6 pim gir",
            "DO sh ipv6 pim gir | inc x",
            "do-exec show platform software tdl-database content bp config nodes 0",
            "do-e sh plat soft tdl con bp oper local",
        ] {
            for mode in ["config", "config-if", "config-vlan"] {
                assert!(typed(C9300, mode, line).is_some(), "{mode}: {line}");
            }
        }
        assert!(typed(C9300, "config", "show ipv6 pim gir").is_none());
        assert!(typed(C9300, "privileged", "do show ipv6 pim gir").is_none());
    }

    #[test]
    fn the_forms_that_answer_and_other_families_are_not_hazards() {
        for line in [
            "show ip pim gir",
            "show i pim gir",
            "show ip igmp gir",
            "show ipv6 mld gir",
            "show ipv6 pim",
            "show ipv6 pim gir detail",
            "show ipv6 pim girx",
            "show platform software tdl-database content",
            "show platform software tdl-database content cfgmgr status",
            "show platform software tdl-database content cfgmgr version",
            "show platform software tdl-database content fpmd access-lists",
            "show platform software tdl-database content fpmd policers",
            "show platform software tdl-database content ftm",
            "show platform software tdl-database content ios access-list",
            "show platform software tdl-database content ios device data",
            "show platform software tdl-database content ios interfaces",
            "show platform software tdl-database content ios sensors",
            "show platform software tdl-database content ompd summary",
            "show platform software tdl-database content vdaemon control-statistics",
            "show platform software tdl-database content bpx",
            "show | show ipv6 pim gir",
        ] {
            assert!(typed(C9300, "privileged", line).is_none(), "{line}");
        }
        for family in [
            Family::C9200,
            Family::C9200L,
            Family::C2960,
            Family::C2960X,
            Family::C9300X,
        ] {
            for line in [
                "show ipv6 pim gir",
                "show platform software tdl-database content bp config nodes 0",
            ] {
                assert!(
                    typed(Some(family), "privileged", line).is_none(),
                    "{family:?}: {line}"
                );
            }
        }
        // a family not known is any family
        assert!(typed(None, "privileged", "show ipv6 pim gir").is_some());
    }

    #[test]
    fn the_warning_says_crashes_on_the_train_measured_and_crashed_elsewhere() {
        let h = typed(C9300, "privileged", "show ipv6 pim gir").unwrap();
        assert_eq!(
            h.warning("17.6"),
            "% vswitch: this command crashes the switch (IOSd segfault and reload on a Catalyst 9300 at IOS XE 17.6.4)"
        );
        assert_eq!(h.warning("17.6.4"), h.warning("17.6"));
        assert_eq!(
            h.warning("17.15"),
            "% vswitch: this command crashed a Catalyst 9300 at IOS XE 17.6.4 (IOSd segfault and reload)"
        );
        assert_eq!(h.warning("16.12"), h.warning("17.15"));
        // `explain` names the command and the day
        assert_eq!(
            h.described("17.15"),
            "'show ipv6 pim gir' crashed a Catalyst 9300 at IOS XE 17.6.4 (IOSd segfault and reload), measured 2026-10-17"
        );
    }

    /// Every keyword the 9300 book offers where one of a hazard's keywords stands, and which is a
    /// start of it, is in its `shorter`: IOS reads it as itself.
    #[test]
    fn a_shorter_keyword_beside_a_hazard_s_is_listed() {
        let engine = cli::Engine::new(
            crate::grammars::grammar("c9300", Release::parse("17.6").unwrap()).unwrap(),
        );
        for h in KNOWN {
            let keywords: Vec<&str> = h.command.split(' ').collect();
            for (i, k) in keywords.iter().enumerate() {
                let before = cli::tokens(&keywords[..i].join(" "));
                for next in engine.next("privileged", &before).0 {
                    if let cli::Next::Kw { kw, .. } = next {
                        let kw = kw.to_lowercase();
                        if kw != *k && k.starts_with(kw.as_str()) {
                            assert!(h.shorter.contains(&kw.as_str()), "{}: {kw}", h.command);
                        }
                    }
                }
            }
        }
    }
}
