//! What a held `?` answer depends on besides the mode: the lines that entered the submode stack.
//! `(config-router)#` answers differently under `router ospf`, `router bgp` and `router eigrp`, and
//! `(config-if)#` under a physical port, a VLAN interface and a loopback. The context is one chain of
//! those entering lines with their lab object names left out, derived here for both the corpus (from a
//! session's prompts) and the shell (from its frames), so the two cannot disagree.

use crate::live::{fits, help_key};

/// Keyword sequences after which the next word is an object's name, not a keyword.
const NAMED: &[&[&str]] = &[
    &["policy-map"],
    &["policy-map", "type", "control", "subscriber"],
    &["class-map"],
    &["class-map", "match-any"],
    &["class-map", "match-all"],
    &["class"],
    &["key", "chain"],
    &["ip", "access-list", "standard"],
    &["ip", "access-list", "extended"],
    &["ipv6", "access-list"],
    &["mac", "access-list", "extended"],
    &["route-map"],
    &["radius", "server"],
    &["tacacs", "server"],
    &["ip", "dhcp", "pool"],
    &["ipv6", "dhcp", "pool"],
    &["event", "manager", "applet"],
    &["crypto", "pki", "trustpoint"],
    &["vlan", "access-map"],
    &["template"],
    &["fallback", "profile"],
    &["vrf", "definition"],
    &["ip", "vrf"],
    &["flow", "exporter"],
    &["flow", "record"],
    &["flow", "monitor"],
];

/// First words an entering line may abbreviate (`int`, `router`), as IOS reads a unique start.
const FIRST: &[&str] = &[
    "interface",
    "router",
    "address-family",
    "line",
    "vlan",
    "policy-map",
    "class-map",
    "class",
    "key",
    "ip",
    "ipv6",
    "mac",
    "route-map",
    "radius",
    "tacacs",
    "event",
    "crypto",
    "track",
    "monitor",
    "template",
    "fallback",
    "network-policy",
    "location",
    "spanning-tree",
    "archive",
    "call-home",
    "vrf",
    "flow",
];

/// Routing protocols `router` names, abbreviable.
const PROTOCOLS: &[&str] = &[
    "ospf", "ospfv3", "eigrp", "bgp", "rip", "isis", "lisp", "mobile", "odr",
];

/// Line types, abbreviable.
const LINES: &[&str] = &["console", "vty", "aux"];

/// Interface families by the type names that belong to each, a type typed as two letters or more.
const FAMILIES: &[(&str, &[&str])] = &[
    ("vlan", &["vlan"]),
    ("loopback", &["loopback"]),
    ("port-channel", &["port-channel"]),
    ("tunnel", &["tunnel"]),
    ("app-gig", &["appgigabitethernet"]),
    (
        "physical",
        &[
            "gigabitethernet",
            "tengigabitethernet",
            "fastethernet",
            "twentyfivegige",
            "twentyfivegigabitethernet",
            "fortygigabitethernet",
            "hundredgige",
            "ethernet",
            "twogigabitethernet",
            "fivegigabitethernet",
        ],
    ),
];

/// Crawl answers that hold only in the state the crawl saw: a key's start, and the configuration
/// line whose presence ends them. The crawl's switch had AAA off, so its `aaa ?` offers `new-model`
/// alone.
pub const STATE_PREDICATES: &[(&str, &str)] = &[("aaa ", "aaa new-model")];

/// Whether a crawl answer for `key` no longer holds: a predicate names it and `configured` says the
/// session has the line that changes the answer.
pub fn stale(key: &str, configured: impl Fn(&str) -> bool) -> bool {
    STATE_PREDICATES
        .iter()
        .any(|(start, line)| key.starts_with(start) && configured(line))
}

/// A crawl's help key: the line typed before `?` as `help_key` has it, with each address and MAC
/// address characterized, so a site address typed at the console finds the lab's answer and no address
/// is stored.
pub fn key(text: &str) -> String {
    help_key(text)
        .split(' ')
        .map(|w| {
            if fits("«ipv4»", w) {
                "«ipv4»"
            } else if fits("«mac»", w) {
                "«mac»"
            } else {
                w
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A word as IOS reads an abbreviation of one of `known`: itself when it is one, the one it begins
/// when it is two letters or more and begins just one, else as typed.
fn expand(word: &str, known: &[&str]) -> String {
    let w = word.to_lowercase();
    if known.contains(&w.as_str()) || w.len() < 2 {
        return w;
    }
    let mut starts = known.iter().filter(|k| k.starts_with(w.as_str()));
    match (starts.next(), starts.next()) {
        (Some(k), None) => k.to_string(),
        _ => w,
    }
}

/// The family of an interface type typed as `letters`, and `other` for one not listed.
fn family(letters: &str) -> &'static str {
    let l = letters.to_lowercase();
    if l.len() < 2 {
        return "other";
    }
    FAMILIES
        .iter()
        .find(|(_, types)| types.iter().any(|t| t.starts_with(l.as_str())))
        .map_or("other", |(f, _)| f)
}

/// An interface name's letters and the rest of it: `GigabitEthernet0/3` is `GigabitEthernet` and `0/3`.
fn split_name(name: &str) -> (&str, &str) {
    let at = name
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(name.len());
    name.split_at(at)
}

/// Whether a word is an interface name (a listed type and a number), as `Vlan10` or `Gi1/0/1`.
fn interface_like(w: &str) -> bool {
    let (letters, rest) = split_name(w);
    !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '/' | '.' | '-'))
        && family(letters) != "other"
}

/// `interface <name>` or `interface range <names>` as one family, with a subinterface flagged by its
/// dot and the management port by its number.
fn interface_segment(rest: &[&str]) -> String {
    let range = rest
        .first()
        .is_some_and(|w| w.len() >= 3 && "range".starts_with(&w.to_lowercase()));
    let names = if range { &rest[1..] } else { rest };
    let joined: String = names.iter().take(2).copied().collect::<Vec<_>>().join("");
    let (letters, number) = split_name(&joined);
    let mut fam = family(letters).to_string();
    // the management port is GigabitEthernet0/0; a 2960's front ports are 0/3 and the like
    if fam == "physical"
        && "gigabitethernet".starts_with(&letters.to_lowercase())
        && number == "0/0"
    {
        fam = "mgmt".into();
    }
    let sub = if number.contains('.') && !range {
        " sub"
    } else {
        ""
    };
    if range {
        format!("interface range {fam}")
    } else {
        format!("interface {fam}{sub}")
    }
}

/// `router <protocol>`, the value of `router ospf 1` and `router bgp 65000` left out. EIGRP's numeric
/// AS and its named mode differ.
fn router_segment(rest: &[&str]) -> String {
    let Some(first) = rest.first() else {
        return "router".into();
    };
    let proto = expand(first, PROTOCOLS);
    match (proto.as_str(), rest.get(1)) {
        ("eigrp", Some(v)) if v.chars().all(|c| c.is_ascii_digit()) => "router eigrp as".into(),
        ("eigrp", Some(_)) => "router eigrp named".into(),
        _ => format!("router {proto}"),
    }
}

/// `address-family` and its keywords, a VRF's name and an AS number left out.
fn family_segment(rest: &[&str]) -> String {
    let mut out = vec!["address-family".to_string()];
    let mut skip = false;
    for w in rest {
        if std::mem::take(&mut skip) {
            continue;
        }
        let w = w.to_lowercase();
        if w.chars().all(|c| c.is_ascii_digit()) || fits("«ipv4»", &w) {
            continue;
        }
        skip = w == "vrf";
        out.push(w);
    }
    out.join(" ")
}

/// Any other entering line, its object names and numbers characterized (`policy-map «word»`,
/// `track «number» interface «word» line-protocol`).
fn generic_segment(toks: &[&str]) -> String {
    let mut out: Vec<String> = vec![];
    for (i, t) in toks.iter().enumerate() {
        let lower = t.to_lowercase();
        let before: Vec<&str> = out.iter().map(String::as_str).collect();
        let continues = NAMED
            .iter()
            .any(|e| e.len() > i && e[..i] == before[..] && e[i] == lower);
        let named = !continues && NAMED.iter().any(|e| e.len() == i && e[..] == before[..]);
        out.push(if named {
            "«word»".into()
        } else if lower.chars().all(|c| c.is_ascii_digit()) {
            "«number»".into()
        } else if fits("«ipv4»", &lower) {
            "«ipv4»".into()
        } else if fits("«mac»", &lower) {
            "«mac»".into()
        } else if interface_like(t) {
            "«word»".into()
        } else if t.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && t.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            lower
        } else {
            "«word»".into()
        });
    }
    out.join(" ")
}

/// The context one entering line adds to its chain: its words with the lab's names and numbers left
/// out, an interface as its family, a router as its protocol.
pub fn segment(line: &str) -> String {
    let toks: Vec<&str> = line.split_whitespace().collect();
    let Some(first) = toks.first() else {
        return String::new();
    };
    let head = expand(first, FIRST);
    let rest = &toks[1..];
    match head.as_str() {
        "interface" => interface_segment(rest),
        "router" => router_segment(rest),
        "address-family" => family_segment(rest),
        _ => {
            let mut words: Vec<String> = toks.iter().map(|w| w.to_string()).collect();
            words[0] = head.clone();
            if head == "line"
                && let Some(kind) = rest.first()
            {
                words[1] = expand(kind, LINES);
            }
            generic_segment(&words.iter().map(String::as_str).collect::<Vec<_>>())
        }
    }
}

/// The context of the submodes entered from global configuration, outermost first: one chain, empty
/// in EXEC, privileged EXEC and global configuration.
pub fn of<S: AsRef<str>>(entering: &[S]) -> String {
    entering
        .iter()
        .map(|l| segment(l.as_ref()))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" > ")
}

/// A session's mode stack as its prompts show it, so each answer takes the context it was asked in.
/// Entered at `configure terminal` and left at `end`, `exit` and a prompt back in EXEC.
#[derive(Debug, Default)]
pub struct Stack {
    /// (mode, the line that entered it), global configuration first
    frames: Vec<(String, String)>,
}

fn is_exec(mode: &str) -> bool {
    matches!(mode, "exec" | "privileged")
}

/// Words that open a block of their own, which a session may type inside another one of them.
const SIBLINGS: &[&str] = &["interface", "router", "line", "vlan"];

impl Stack {
    /// The context of a prompt in `mode`: empty in EXEC, none when the stack does not end in that
    /// mode (a session that began inside a submode).
    pub fn context(&self, mode: &str) -> Option<String> {
        if is_exec(mode) {
            return Some(String::new());
        }
        if mode == "config" && self.frames.is_empty() {
            return Some(String::new());
        }
        let (top, _) = self.frames.last()?;
        (top == mode).then(|| {
            let lines: Vec<&str> = self.frames[1..].iter().map(|(_, l)| l.as_str()).collect();
            of(&lines)
        })
    }

    /// What typing `command` at a prompt in `mode` did, given the mode of the next prompt: a `?`
    /// and a refused line change nothing.
    pub fn advance(&mut self, mode: &str, command: &str, refused: bool, next: Option<&str>) {
        let command = command.trim();
        let Some(next) = next else { return };
        if command.is_empty() || command.ends_with('?') || refused {
            return;
        }
        if is_exec(next) {
            self.frames.clear();
            return;
        }
        if is_exec(mode) {
            self.frames = vec![(next.to_string(), command.to_string())];
            return;
        }
        if self.frames.is_empty() {
            if mode != "config" {
                return;
            }
            self.frames.push(("config".into(), String::new()));
        }
        let first = command
            .split_whitespace()
            .next()
            .map(str::to_lowercase)
            .unwrap_or_default();
        if first.starts_with("exit-") || (first.len() >= 3 && "exit".starts_with(first.as_str())) {
            self.frames.pop();
            while self.frames.last().is_some_and(|(m, _)| m != next) {
                self.frames.pop();
            }
        } else if SIBLINGS.contains(&expand(&first, FIRST).as_str()) && self.frames.len() > 1 {
            // a block of its own opens from global configuration, whichever submode it is typed in
            self.frames.truncate(1);
            self.frames.push((next.to_string(), command.to_string()));
        } else if next == mode {
            // an ordinary line of the mode
        } else if let Some(i) = self.frames.iter().rposition(|(m, _)| m == next) {
            self.frames.truncate(i + 1);
        } else {
            self.frames.push((next.to_string(), command.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routers_interfaces_and_address_families_take_their_own_contexts() {
        let cases = [
            ("router ospf 1", "router ospf"),
            ("router ospf 7 vrf RED", "router ospf"),
            ("router bgp 65000", "router bgp"),
            ("router eigrp 1", "router eigrp as"),
            ("router eigrp NAMED", "router eigrp named"),
            ("router rip", "router rip"),
            ("addr ipv4 unicast", "address-family ipv4 unicast"),
            ("interface GigabitEthernet1/0/1", "interface physical"),
            ("int gi1/0/2", "interface physical"),
            ("interface TenGigabitEthernet1/1/1", "interface physical"),
            ("interface GigabitEthernet0/0", "interface mgmt"),
            ("interface GigabitEthernet0/3", "interface physical"),
            ("interface Vlan10", "interface vlan"),
            ("interface Loopback0", "interface loopback"),
            ("interface Port-channel1", "interface port-channel"),
            ("interface Tunnel5", "interface tunnel"),
            ("interface AppGigabitEthernet1/0/1", "interface app-gig"),
            (
                "interface GigabitEthernet1/0/1.100",
                "interface physical sub",
            ),
            (
                "interface range GigabitEthernet1/0/5 - 6",
                "interface range physical",
            ),
            ("interface range Vlan 10 - 11", "interface range vlan"),
            ("address-family ipv4", "address-family ipv4"),
            ("address-family ipv4 vrf RED", "address-family ipv4 vrf"),
            (
                "address-family ipv4 unicast autonomous-system 1",
                "address-family ipv4 unicast autonomous-system",
            ),
            ("policy-map TEST", "policy-map «word»"),
            ("class class-default", "class «word»"),
            ("key chain TEST", "key chain «word»"),
            ("key 1", "key «number»"),
            ("class-map match-any TEST", "class-map match-any «word»"),
            ("class-map TEST", "class-map «word»"),
            (
                "ip access-list extended TEST",
                "ip access-list extended «word»",
            ),
            (
                "ip access-list standard voice",
                "ip access-list standard «word»",
            ),
            (
                "route-map TEST permit 10",
                "route-map «word» permit «number»",
            ),
            ("line vty 0 4", "line vty «number» «number»"),
            ("line con 0", "line console «number»"),
            ("icmp-echo 10.1.1.1", "icmp-echo «ipv4»"),
            (
                "track 1 interface GigabitEthernet0/3 line-protocol",
                "track «number» interface «word» line-protocol",
            ),
            ("ipv6 dhcp pool TEST6", "ipv6 dhcp pool «word»"),
            ("vlan 10", "vlan «number»"),
            (
                "vlan access-map TESTMAP 10",
                "vlan access-map «word» «number»",
            ),
            (
                "spanning-tree mst configuration",
                "spanning-tree mst configuration",
            ),
        ];
        for (line, want) in cases {
            assert_eq!(segment(line), want, "{line}");
        }
    }

    #[test]
    fn a_context_names_no_lab_object_and_two_names_of_one_kind_agree() {
        for (a, b) in [
            ("policy-map TEST", "policy-map voice-out"),
            ("router ospf 1", "router ospf 100"),
            (
                "interface GigabitEthernet1/0/1",
                "interface TenGigabitEthernet2/1/4",
            ),
            ("key chain TEST", "key chain Corp-Keys"),
            ("line vty 0 4", "line vty 5 15"),
        ] {
            assert_eq!(segment(a), segment(b), "{a} / {b}");
        }
        let distinct = [
            "router ospf 1",
            "router bgp 1",
            "router eigrp 1",
            "router eigrp NAMED",
            "interface Vlan10",
            "interface Loopback0",
            "interface GigabitEthernet1/0/1",
            "interface GigabitEthernet0/0",
            "interface GigabitEthernet1/0/1.5",
            "interface range GigabitEthernet1/0/1 - 2",
            "interface Port-channel1",
            "interface Tunnel1",
            "interface AppGigabitEthernet1/0/1",
            "line console 0",
            "line vty 0 4",
        ];
        let all: std::collections::BTreeSet<String> = distinct.iter().map(|l| segment(l)).collect();
        assert_eq!(all.len(), distinct.len(), "{all:?}");
        assert!(
            !of(&["policy-map TESTPOLICY", "class TESTCLASS"]).contains("TEST"),
            "no lab name enters a key"
        );
    }

    #[test]
    fn a_chain_runs_from_the_outermost_entering_line() {
        assert_eq!(of::<&str>(&[]), "");
        assert_eq!(
            of(&["router bgp 65000", "address-family ipv4"]),
            "router bgp > address-family ipv4"
        );
        assert_eq!(
            of(&["policy-map TEST", "class class-default"]),
            "policy-map «word» > class «word»"
        );
    }

    #[test]
    fn a_help_key_keeps_its_words_and_characterizes_addresses() {
        assert_eq!(key("ARP  10.1.1.1 "), "arp «ipv4» ");
        assert_eq!(key("arp 10.1.1.1 0011.2233.4455 "), "arp «ipv4» «mac» ");
        assert_eq!(
            key("access-list 100 permit ip 255.255.255.0"),
            "access-list 100 permit ip «ipv4»"
        );
        assert_eq!(key("ip "), "ip ");
        assert_eq!(key(" "), "");
    }

    fn walk(steps: &[(&str, &str, Option<&str>)]) -> Vec<Option<String>> {
        let mut s = Stack::default();
        steps
            .iter()
            .map(|(mode, command, next)| {
                let c = s.context(mode);
                s.advance(mode, command, false, *next);
                c
            })
            .collect()
    }

    #[test]
    fn a_session_s_prompts_give_each_probe_the_context_it_was_asked_in() {
        let got = walk(&[
            ("config", "end", Some("privileged")),
            ("privileged", "configure terminal", Some("config")),
            ("config", "router ospf 1", Some("config-router")),
            ("config-router", " ?", Some("config-router")),
            (
                "config-router",
                "address-family ipv4",
                Some("config-router-af"),
            ),
            ("config-router-af", " ?", Some("config-router-af")),
            (
                "config-router-af",
                "exit-address-family",
                Some("config-router"),
            ),
            ("config-router", "exit", Some("config")),
            ("config", "interface Vlan10", Some("config-if")),
            ("config-if", " ?", Some("config-if")),
            ("config-if", "interface Loopback0", Some("config-if")),
            ("config-if", " ?", Some("config-if")),
            (
                "config-if",
                "interface range Gi1/0/5 - 6",
                Some("config-if-range"),
            ),
            ("config-if-range", " ?", Some("config-if-range")),
            ("config-if-range", "end", Some("privileged")),
            ("privileged", "show ?", Some("privileged")),
        ]);
        let want: Vec<Option<&str>> = vec![
            Some(""),
            Some(""),
            Some(""),
            Some("router ospf"),
            Some("router ospf"),
            Some("router ospf > address-family ipv4"),
            Some("router ospf > address-family ipv4"),
            Some("router ospf"),
            Some(""),
            Some("interface vlan"),
            Some("interface vlan"),
            Some("interface loopback"),
            Some("interface loopback"),
            Some("interface range physical"),
            Some("interface range physical"),
            Some(""),
        ];
        let got: Vec<Option<&str>> = got.iter().map(|c| c.as_deref()).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn a_session_that_begins_inside_a_submode_has_no_context_until_it_leaves() {
        let got = walk(&[
            ("config-if", " ?", Some("config-if")),
            ("config-if", "exit", Some("config")),
            ("config", "end", Some("privileged")),
            ("privileged", "configure terminal", Some("config")),
            ("config", "line vty 0 4", Some("config-line")),
            ("config-line", " ?", Some("config-line")),
        ]);
        assert_eq!(got[0], None);
        assert_eq!(got[5].as_deref(), Some("line vty «number» «number»"));
    }

    #[test]
    fn a_refused_line_and_a_probe_change_no_context() {
        let mut s = Stack::default();
        s.advance("privileged", "configure terminal", false, Some("config"));
        s.advance("config", "router bogus", true, Some("config"));
        s.advance("config", "ip ?", false, Some("config"));
        assert_eq!(s.context("config").as_deref(), Some(""));
    }

    #[test]
    fn a_state_predicate_ends_a_crawl_answer_only_when_its_line_is_configured() {
        assert!(!stale("aaa ", |_| false));
        assert!(stale("aaa ", |l| l == "aaa new-model"));
        assert!(stale("aaa authentication ", |l| l == "aaa new-model"));
        assert!(!stale("ip ", |_| true));
    }
}
