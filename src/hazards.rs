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
    /// the command's words, whole: its keywords, and at `port` the interface it crashed on
    pub command: &'static str,
    /// which word of `command` names an interface, where the crash was measured on that interface
    /// alone; a line names it as IOS reads one, abbreviated and in one word or two (`tu 0`)
    pub port: Option<usize>,
    /// whether the command crashes with any further words too
    pub subtree: bool,
    /// shorter keywords IOS has where one of the command's stands, each a start of it: a word spelling
    /// one whole is that keyword (`show ip` is not `show ipv6`)
    pub shorter: &'static [&'static str],
    /// what the switch did
    pub effect: &'static str,
}

/// Each crashed a lab switch at network-advantage over SSH: IOSd segfaulted (`linux_iosd_image has
/// failed (rc 139)`) and the switch reloaded with `Critical software exception`.
pub const KNOWN: &[Hazard] = &[
    // a C9300-24T with a C9300-NM-4G, each typed alone on one SSH session; no published Cisco bug
    // names either
    // the fault is in the process running the command (`SSH Process` over SSH); `show ip pim gir`,
    // `show ip igmp gir` and `show ipv6 mld gir` answer
    Hazard {
        families: &[Family::C9300],
        release: "17.6.4",
        date: "2026-10-17",
        command: "show ipv6 pim gir",
        port: None,
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
        port: None,
        subtree: true,
        shorter: &[],
        effect: "IOSd segfault and reload",
    },
    // a C9200L-48T-4G, each the last command of its crashinfo file in a show sweep: the same forms
    // on the other interfaces swept answered, and at 17.6.4 the Tunnel0 forms closed the session
    // without a reload
    max_power("show interfaces Tunnel0 transceiver max-power"),
    max_power("show interfaces Tunnel0 transceiver max-power violation"),
    max_power("show interfaces GigabitEthernet0/0 transceiver max-power"),
    max_power("show interfaces GigabitEthernet0/0 transceiver max-power violation"),
];

/// A `transceiver max-power` form that crashed the lab C9200L-48T-4G at 17.15.6, on the interface
/// it names (the third word); the 9200 books cover the 9200, 9200L and 9200 B.
const fn max_power(command: &'static str) -> Hazard {
    Hazard {
        families: &[Family::C9200L, Family::C9200, Family::C9200B],
        release: "17.15.6",
        date: "2026-10-21",
        command,
        port: Some(2),
        subtree: false,
        shorter: &[],
        effect: "IOSd segfault and reload",
    }
}

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
    /// the interface at `port` by type and number, and more words only under a subtree.
    fn named_by(&self, words: &[String]) -> bool {
        let mut at = 0;
        for (i, k) in self.command.split(' ').enumerate() {
            let used = if self.port == Some(i) {
                port_named(k, words.get(at), words.get(at + 1))
            } else {
                words
                    .get(at)
                    .filter(|w| k.starts_with(w.as_str()) && !self.shorter.contains(&w.as_str()))
                    .map(|_| 1)
            };
            match used {
                Some(n) => at += n,
                None => return false,
            }
        }
        self.subtree || at == words.len()
    }
}

/// How many of the words `first` and `second` name the interface `measured` (`Tunnel0`): its type by
/// IOS's abbreviations and its number, each part equal by value; none where they name another.
fn port_named(measured: &str, first: Option<&String>, second: Option<&String>) -> Option<usize> {
    let (ty, number, _) = cli::interface_name(measured, None)?;
    let (typed, n, used) = cli::interface_name(first?, second.map(String::as_str))?;
    let parts =
        |n: &str| -> Option<Vec<u32>> { n.split(['/', '.']).map(|p| p.parse().ok()).collect() };
    (typed == ty && n.contains('.') == number.contains('.') && parts(&n)? == parts(&number)?)
        .then_some(used)
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

    const C9200L: Option<Family> = Some(Family::C9200L);

    #[test]
    fn a_c9200_line_is_a_hazard_on_the_interface_measured_alone() {
        for line in [
            "show interfaces Tunnel0 transceiver max-power",
            "sh int tu0 trans max",
            "SHOW INTERFACES TUNNEL 0 TRANSCEIVER MAX-POWER",
            "show interfaces Tunnel0 transceiver max-power violation | include x",
            "show interfaces GigabitEthernet0/0 transceiver max-power",
            "sh int gi0/0 tr max-power violation",
            "show interfaces gigabitethernet 0/0 transceiver max-power",
            "show interfaces Gi00/0 transceiver max-power",
        ] {
            for family in [Family::C9200L, Family::C9200, Family::C9200B] {
                assert!(
                    typed(Some(family), "privileged", line).is_some(),
                    "{family:?}: {line}"
                );
            }
            assert!(typed(None, "privileged", line).is_some(), "{line}");
            assert!(typed(C9300, "privileged", line).is_none(), "{line}");
        }
        assert!(typed(C9200L, "config-if", "do show int Tu0 transceiver max-power").is_some());
        assert!(typed(C9200L, "config", "show int Tu0 transceiver max-power").is_none());
        // the forms the switch answered, other interfaces and other words
        for line in [
            "show interfaces GigabitEthernet1/0/1 transceiver max-power",
            "show interfaces GigabitEthernet1/0/1 transceiver max-power violation",
            "show interfaces Gi1/1/1 transceiver max-power",
            "show interfaces Tunnel1 transceiver max-power",
            "show interfaces Gi0/0.1 transceiver max-power",
            "show interfaces Gi0 transceiver max-power",
            "show interfaces Vlan0 transceiver max-power",
            "show interfaces Tunnel0 transceiver",
            "show interfaces Tunnel0 transceiver max-power violation detail",
            "show interfaces transceiver max-power",
            "show interfaces t0 transceiver max-power",
        ] {
            assert!(typed(C9200L, "privileged", line).is_none(), "{line}");
        }
    }

    #[test]
    fn a_c9200_hazard_s_warning_and_explain_name_its_interface_and_release() {
        let h = typed(C9200L, "privileged", "sh int tu0 trans max").unwrap();
        assert_eq!(h.command, "show interfaces Tunnel0 transceiver max-power");
        assert_eq!(
            h.warning("17.15"),
            "% vswitch: this command crashes the switch (IOSd segfault and reload on a Catalyst 9200L at IOS XE 17.15.6)"
        );
        assert_eq!(
            h.warning("17.6"),
            "% vswitch: this command crashed a Catalyst 9200L at IOS XE 17.15.6 (IOSd segfault and reload)"
        );
        assert_eq!(
            h.described("17.15"),
            "'show interfaces Tunnel0 transceiver max-power' crashes the switch (IOSd segfault and reload on a Catalyst 9200L at IOS XE 17.15.6), measured 2026-10-21"
        );
    }

    /// Every keyword a hazard's book offers at its release where one of its keywords stands, and
    /// which is a start of it, is in its `shorter`: IOS reads it as itself. `show interface`, which
    /// the books spell beside `interfaces`, is not one: IOS lists `interfaces` alone and vswitch
    /// takes `show interface` as it.
    #[test]
    fn a_shorter_keyword_beside_a_hazard_s_is_listed() {
        for h in KNOWN {
            let book = crate::grammars::book(h.families.first().copied());
            let engine = cli::Engine::new(
                crate::grammars::grammar(book, Release::parse(h.release).unwrap()).unwrap(),
            );
            let keywords: Vec<&str> = h.command.split(' ').collect();
            for (i, k) in keywords.iter().enumerate() {
                // the books' `show interface` is IOS's `show interfaces` (output.md)
                if h.port == Some(i) || (i == 1 && *k == "interfaces") {
                    continue;
                }
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
