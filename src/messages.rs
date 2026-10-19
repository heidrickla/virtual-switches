//! The `%` messages a switch printed on Enter for a privileged show line it took, as the sweeps
//! measured them, held with the line's form (`swept::said`), and when vswitch prints one. A notice
//! comes before the command's output whatever is configured; a fixed message is the whole answer
//! whatever is configured; a state message is the whole answer where the session's running
//! configuration lacks what it names. A message no pattern here reads is not held.

use crate::cli::Item;
use crate::device::Device;

/// How a held message line prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Notice,
    Fixed,
    State(Rule),
}

/// What the running configuration lacks where a state message prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// `router lisp`
    Lisp,
    /// a subscriber session or service, which vswitch never holds
    Subscriber,
    /// an `snmp-server` line other than `location` and `contact`, which leave the agent off
    SnmpAgent,
    /// `snmp-server manager`
    SnmpManager,
    /// `mpls traffic-eng fault-oam` or `mpls tp`
    FaultOam,
    /// the EIGRP or BGP autonomous system the line names, a `router eigrp` or `router bgp` running
    As,
    /// a `topology <name>` line anywhere for the topology the line names; `base` always exists
    Topology,
    /// a `topology <name>` line under a `router eigrp`, one running; `base` always exists
    EigrpTopology,
    /// a `topology <name>` line under a `router ospf`, one running; unicast `base` always exists
    OspfTopology,
    /// OSPF on the interface the line names: `ip ospf <n> area`, or a `network` of a process in
    /// the interface's VRF covering one of its addresses
    OspfInterface,
    /// the VRF the line names
    Vrf,
    /// `logging enable` under `archive`
    ConfigLogger,
    /// `service-routing xmcp listen`
    Xmcp,
    /// `spanning-tree mode mst`
    Mst,
    /// `evpn ethernet-segment` on the interface the line names
    EthernetSegment,
    /// `bgp dampening` in a running `router bgp`
    Dampening,
    /// the BGP peer group the line names, in a running `router bgp`
    PeerGroup,
    /// `cdp tlv-list`, or the one the line names
    TlvList,
}

/// The messages vswitch holds, as IOS XE prints them (the lab C9300-24T, 17.6.4); `{}` stands for
/// a value the line names.
pub const MESSAGES: &[(&str, Kind)] = &[
    (
        "% Command accepted but obsolete, unreleased or unsupported; see documentation.",
        Kind::Notice,
    ),
    (
        "% Command accepted but obsolete, see 'show (ip|ipv6) cef switching statistics [feature]'",
        Kind::Notice,
    ),
    (
        "% NOTE: This command is deprecated. Please use 'show bgp ipv6 unicast'",
        Kind::Notice,
    ),
    (
        "% Command \"show ipv6 nd suppress <...>\" is obsolete.",
        Kind::Notice,
    ),
    (
        "% Please use \"show flooding-suppression <...>\" instead.",
        Kind::Notice,
    ),
    ("% Error: Unsupported command", Kind::Fixed),
    ("% Error: ngwc not supported", Kind::Fixed),
    ("% Error: Command not supported", Kind::Fixed),
    (
        "% Error: Command is not supported on this platform",
        Kind::Fixed,
    ),
    (
        "% Error: Show version installed user-interface is not supported",
        Kind::Fixed,
    ),
    (
        "%Error: lowmem gd is not supported in this platform",
        Kind::Fixed,
    ),
    ("% Missing filename in URL.", Kind::Fixed),
    // a fixed-configuration switch has no linecards
    ("% Linecard information not found", Kind::Fixed),
    ("% LISP is not running.", Kind::State(Rule::Lisp)),
    (
        "%No active Subscriber Sessions",
        Kind::State(Rule::Subscriber),
    ),
    ("%No active services", Kind::State(Rule::Subscriber)),
    ("%SNMP agent not enabled", Kind::State(Rule::SnmpAgent)),
    ("%%SNMP manager not enabled", Kind::State(Rule::SnmpManager)),
    (
        "% MPLS Fault-OAM is not initialized.",
        Kind::State(Rule::FaultOam),
    ),
    ("%AS {} specified not configured", Kind::State(Rule::As)),
    ("%AS({}) specified not configured", Kind::State(Rule::As)),
    (
        "%Topology {} specified does not exist.",
        Kind::State(Rule::EigrpTopology),
    ),
    (
        "% IP routing table topology {} does not exist",
        Kind::State(Rule::Topology),
    ),
    (
        "%OSPF: Specified topology not found for any OSPF process",
        Kind::State(Rule::OspfTopology),
    ),
    (
        "%OSPF: OSPF not enabled on {}",
        Kind::State(Rule::OspfInterface),
    ),
    ("% No VRF named {}", Kind::State(Rule::Vrf)),
    ("% vrf {} does not exist", Kind::State(Rule::Vrf)),
    ("%Error, VRF {} does not exist", Kind::State(Rule::Vrf)),
    ("% VRF {} does not exist", Kind::State(Rule::Vrf)),
    ("% Unknown VRF", Kind::State(Rule::Vrf)),
    ("% Config Logger disabled.", Kind::State(Rule::ConfigLogger)),
    ("%XMCP server is not enabled", Kind::State(Rule::Xmcp)),
    ("% Switch is not in mst mode", Kind::State(Rule::Mst)),
    (
        "% No matching EVPN ethernet segment",
        Kind::State(Rule::EthernetSegment),
    ),
    (
        "% dampening not enabled for base",
        Kind::State(Rule::Dampening),
    ),
    ("% No such peer-group", Kind::State(Rule::PeerGroup)),
    ("%No Tlv-list exists.", Kind::State(Rule::TlvList)),
    (
        "%Tlv-list name does not exists.",
        Kind::State(Rule::TlvList),
    ),
];

/// The patterns a message line fits, each with where its value stands in the line.
fn fit(message: &str) -> Vec<(Kind, Option<(usize, usize)>)> {
    let m = message.trim_end();
    let mut out = vec![];
    for (pattern, kind) in MESSAGES {
        match pattern.split_once("{}") {
            None if m == *pattern => out.push((*kind, None)),
            None => {}
            Some((a, b)) if m.len() > a.len() + b.len() && m.starts_with(a) && m.ends_with(b) => {
                out.push((*kind, Some((a.len(), m.len() - b.len()))));
            }
            Some(_) => {}
        }
    }
    out
}

/// A held line's value place (`{<place>}`) and the text either side of it.
fn place(t: &str) -> Option<(&str, usize, &str)> {
    let a = t.find('{')?;
    let b = a + t[a..].find('}')?;
    let at = t[a + 1..b].parse().ok()?;
    Some((&t[..a], at, &t[b + 1..]))
}

/// How a held line prints.
pub fn kind(held: &str) -> Option<Kind> {
    match place(held.trim_end()) {
        Some((a, _, b)) => {
            let plain = format!("{a}{{}}{b}");
            MESSAGES.iter().find(|(p, _)| *p == plain).map(|(_, k)| *k)
        }
        None => fit(held).first().map(|(k, _)| *k),
    }
}

/// The message lines the switch printed before any output for a line (`raw`), as its form holds
/// them: notices, then the fixed or state messages that end the answer. A value is written
/// `{<place>}`, its place the one value token of `form` whose word of the line (`words`) the
/// message names; a value the form holds as a keyword stays as printed. Notices before a line no
/// pattern reads are held alone; an ending message is held only where it ends the lines.
pub fn template(raw: &[String], form: &[&str], words: &[&str]) -> Vec<String> {
    let names = |i: usize, value: &str| {
        words[i] == value
            || crate::cli::interface_name(words[i], None)
                .is_some_and(|(full, num, _)| format!("{full}{num}") == value)
    };
    let held = |m: &String, slot: Option<(usize, usize)>| -> Option<String> {
        let Some((a, b)) = slot else {
            return Some(m.clone());
        };
        let value = &m[a..b];
        let n = form.len().min(words.len());
        let at: Vec<usize> = (0..n)
            .filter(|&i| form[i].starts_with('<') && form[i] != "<line>" && names(i, value))
            .collect();
        match at[..] {
            [i] => Some(format!("{}{{{i}}}{}", &m[..a], &m[b..])),
            [] if (0..n)
                .any(|i| !form[i].starts_with('<') && words[i].eq_ignore_ascii_case(value)) =>
            {
                Some(m.clone())
            }
            _ => None,
        }
    };
    let mut out = vec![];
    let mut ended = false;
    for m in raw {
        let read = fit(m)
            .into_iter()
            .find_map(|(k, slot)| held(m, slot).map(|t| (t, k)));
        let Some((t, k)) = read else {
            // the notices before a line no pattern reads stand; an ending message does not
            return if ended { vec![] } else { out };
        };
        match k {
            Kind::Notice if ended => return vec![],
            Kind::Notice => out.push(t),
            _ => {
                ended = true;
                out.push(t);
            }
        }
    }
    out
}

/// What a line prints with the message lines its form holds (`template`): the notices, then the
/// messages that end the answer where each holds in `d`, else the command's own output `out`; and
/// whether the messages stood in place of the output. `items` is the line's parse by the form.
pub fn print(held: &[String], items: &[Item], d: &Device, out: Vec<String>) -> (Vec<String>, bool) {
    let config = Config::read(
        &crate::show::configuration(d),
        d.stack.management().is_some(),
    );
    print_by(held, items, &config, out)
}

/// `print` on a configuration read.
fn print_by(
    held: &[String],
    items: &[Item],
    config: &Config,
    out: Vec<String>,
) -> (Vec<String>, bool) {
    let values: Vec<&str> = items
        .iter()
        .map(|i| match i {
            Item::Kw { kw, .. } => kw.as_str(),
            Item::Arg { value, .. } => value.as_str(),
        })
        .collect();
    let mut said = vec![];
    let mut enders = vec![];
    for t in held {
        let filled = match place(t) {
            Some((a, at, b)) => match values.get(at) {
                Some(v) => (format!("{a}{v}{b}"), Some(at)),
                None => return (out, false),
            },
            None => (t.clone(), None),
        };
        match kind(t) {
            Some(Kind::Notice) => said.push(filled.0),
            Some(k) => enders.push((k, filled)),
            None => return (out, false),
        }
    }
    let stands = !enders.is_empty()
        && enders.iter().all(|(k, (_, at))| match k {
            Kind::State(r) => config.lacks(*r, items, &values, *at),
            _ => true,
        });
    if stands {
        said.extend(enders.into_iter().map(|(_, (text, _))| text));
        return (said, true);
    }
    said.extend(out);
    (said, false)
}

/// The running configuration's top-level lines, each with the lines printed under it, and whether
/// the switch has a management port (its `Mgmt-vrf`).
struct Config {
    sections: Vec<(String, Vec<String>)>,
    management: bool,
}

impl Config {
    fn read(config: &[String], management: bool) -> Config {
        let mut sections: Vec<(String, Vec<String>)> = vec![];
        for l in config {
            let t = l.trim();
            if t.is_empty() || t.starts_with('!') {
                continue;
            }
            match sections.last_mut() {
                Some(s) if l.starts_with(' ') => s.1.push(t.to_string()),
                _ => sections.push((t.to_string(), vec![])),
            }
        }
        Config {
            sections,
            management,
        }
    }

    fn top(&self, prefix: &str) -> bool {
        self.sections.iter().any(|(h, _)| h.starts_with(prefix))
    }

    fn has(&self, header: &str) -> bool {
        self.sections.iter().any(|(h, _)| h == header)
    }

    /// The sections whose header begins `prefix`.
    fn under<'c>(&'c self, prefix: &'c str) -> impl Iterator<Item = &'c (String, Vec<String>)> {
        self.sections
            .iter()
            .filter(move |(h, _)| h.starts_with(prefix))
    }

    /// Whether a `topology <name>` line stands in the sections whose header begins `prefix`.
    fn topology(&self, prefix: &str, name: &str) -> bool {
        let defines = |l: &str| {
            let mut w = l.split_whitespace();
            w.next() == Some("topology") && w.next() == Some(name)
        };
        self.under(prefix)
            .any(|(h, lines)| defines(h) || lines.iter().any(|l| defines(l)))
    }

    /// Whether the configuration lacks what rule `r` names for a line: its parse `items`, their
    /// words `values`, `at` the place of the value the message names.
    fn lacks(&self, r: Rule, items: &[Item], values: &[&str], at: Option<usize>) -> bool {
        // the word after a keyword of the line
        let after = |kw: &str| -> Option<&str> {
            items
                .iter()
                .position(|i| matches!(i, Item::Kw { kw: k, .. } if k == kw))
                .and_then(|i| values.get(i + 1).copied())
        };
        let value = at.and_then(|i| values.get(i).copied());
        let named = |kw: &str| value.or_else(|| after(kw));
        let has_kw = |kw: &str| {
            items
                .iter()
                .any(|i| matches!(i, Item::Kw { kw: k, .. } if k == kw))
        };
        match r {
            Rule::Lisp => !self.top("router lisp"),
            Rule::Subscriber => true,
            Rule::SnmpAgent => self.sections.iter().all(|(h, _)| {
                !h.starts_with("snmp-server ")
                    || h.starts_with("snmp-server location ")
                    || h.starts_with("snmp-server contact ")
            }),
            Rule::SnmpManager => !self.top("snmp-server manager"),
            Rule::FaultOam => !self.top("mpls traffic-eng fault-oam") && !self.top("mpls tp"),
            Rule::As => {
                let Some(n) = value else { return false };
                if has_kw("eigrp") {
                    // the AS in any form IOS runs EIGRP by: classic, named, IPv6
                    let numbered = |h: &str, lines: &[String]| {
                        h.split_whitespace().nth(2) == Some(n)
                            || lines.iter().any(|l| {
                                let w: Vec<&str> = l.split_whitespace().collect();
                                w.windows(2)
                                    .any(|p| p[0] == "autonomous-system" && p[1] == n)
                            })
                    };
                    self.top("router eigrp ")
                        && !self
                            .under("router eigrp ")
                            .chain(self.under("ipv6 router eigrp "))
                            .any(|(h, lines)| numbered(h.trim_start_matches("ipv6 "), lines))
                } else if has_kw("bgp") {
                    self.top("router bgp ") && !self.has(&format!("router bgp {n}"))
                } else {
                    false
                }
            }
            Rule::Topology => {
                named("topology").is_some_and(|name| name != "base" && !self.topology("", name))
            }
            Rule::EigrpTopology => named("topology").is_some_and(|name| {
                name != "base" && self.top("router eigrp ") && !self.topology("router eigrp ", name)
            }),
            // the unicast base topology runs with the process; the multicast one is configured
            Rule::OspfTopology => named("topology").is_some_and(|name| {
                (name != "base" || has_kw("multicast"))
                    && self.top("router ospf ")
                    && !self.topology("router ospf ", name)
            }),
            Rule::OspfInterface => value.is_some_and(|name| self.ospf_off(name)),
            Rule::Vrf => named("vrf").is_some_and(|name| {
                let mgmt = name == "Mgmt-vrf" && self.management;
                !mgmt
                    && !self.sections.iter().any(|(h, _)| {
                        h.strip_prefix("vrf definition ") == Some(name)
                            || h.strip_prefix("ip vrf ") == Some(name)
                    })
            }),
            Rule::ConfigLogger => !self
                .under("archive")
                .any(|(_, lines)| lines.iter().any(|l| l == "logging enable")),
            Rule::Xmcp => !self.top("service-routing xmcp"),
            Rule::Mst => !self.top("spanning-tree mode mst"),
            Rule::EthernetSegment => items
                .iter()
                .find_map(|i| match i {
                    Item::Arg { name, value, .. } if name.starts_with("interface:") => {
                        Some(value.as_str())
                    }
                    _ => None,
                })
                .is_some_and(|p| {
                    !self
                        .interface(p)
                        .iter()
                        .any(|l| l.starts_with("evpn ethernet-segment"))
                }),
            Rule::Dampening => {
                self.top("router bgp ")
                    && !self
                        .under("router bgp ")
                        .any(|(_, lines)| lines.iter().any(|l| l.starts_with("bgp dampening")))
            }
            Rule::PeerGroup => after("peer-group").is_some_and(|g| {
                let defined = format!("neighbor {g} peer-group");
                self.top("router bgp ")
                    && !self
                        .under("router bgp ")
                        .any(|(_, lines)| lines.contains(&defined))
            }),
            Rule::TlvList => match after("tlv-list") {
                Some(name) if name != "*" => !self.has(&format!("cdp tlv-list {name}")),
                _ => !self.top("cdp tlv-list "),
            },
        }
    }

    /// An interface's lines, none where it has no block.
    fn interface(&self, name: &str) -> &[String] {
        let header = format!("interface {name}");
        self.sections
            .iter()
            .find(|(h, _)| *h == header)
            .map_or(&[], |(_, lines)| lines.as_slice())
    }

    /// Whether OSPF is off on an interface: an OSPF process runs, the interface has no `ip ospf <n>
    /// area` line, and no `network` of a process in its VRF covers an address it holds. An address
    /// vswitch cannot read (DHCP, unnumbered) leaves it unknown where such a `network` exists.
    fn ospf_off(&self, name: &str) -> bool {
        let lines = self.interface(name);
        if !self.top("router ospf ")
            || lines
                .iter()
                .any(|l| l.starts_with("ip ospf ") && l.split_whitespace().any(|w| w == "area"))
        {
            return false;
        }
        let vrf = lines.iter().find_map(|l| {
            l.strip_prefix("vrf forwarding ")
                .or_else(|| l.strip_prefix("ip vrf forwarding "))
        });
        let v4 = |w: &str| w.parse::<std::net::Ipv4Addr>().ok().map(u32::from);
        let networks: Vec<(u32, u32)> = self
            .under("router ospf ")
            .filter(|(h, _)| {
                let w: Vec<&str> = h.split_whitespace().collect();
                let theirs = if w.get(3) == Some(&"vrf") {
                    w.get(4).copied()
                } else {
                    None
                };
                theirs == vrf
            })
            .flat_map(|(_, lines)| lines.iter())
            .filter_map(|l| {
                let w: Vec<&str> = l.split_whitespace().collect();
                match w[..] {
                    ["network", a, m, "area", _] => Some((v4(a)?, v4(m)?)),
                    _ => None,
                }
            })
            .collect();
        if networks.is_empty() {
            return true;
        }
        let unknown = lines.iter().any(|l| {
            [
                "ip address dhcp",
                "ip address negotiated",
                "ip address pool",
                "ip unnumbered",
            ]
            .iter()
            .any(|p| l.starts_with(p))
        });
        if unknown {
            return false;
        }
        let addresses: Vec<u32> = lines
            .iter()
            .filter_map(|l| {
                let w: Vec<&str> = l.split_whitespace().collect();
                match w[..] {
                    ["ip", "address", a, _, ..] => v4(a),
                    _ => None,
                }
            })
            .collect();
        !addresses
            .iter()
            .any(|a| networks.iter().any(|(n, w)| a & !w == n & !w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    const NOTICE: &str =
        "% Command accepted but obsolete, unreleased or unsupported; see documentation.";

    #[test]
    fn a_held_message_names_its_value_by_the_place_of_the_form_s_value() {
        let form = ["show", "widget", "<number:1-10>", "detail"];
        let words = ["show", "widget", "7", "detail"];
        assert_eq!(
            template(&lines(&["%AS 7 specified not configured"]), &form, &words),
            ["%AS {2} specified not configured"]
        );
        // a value no value of the form holds, or two of them, gives no line
        assert!(template(&lines(&["%AS 8 specified not configured"]), &form, &words).is_empty());
        let twice = ["show", "<number:1-10>", "<number:1-10>"];
        assert!(
            template(
                &lines(&["%AS 7 specified not configured"]),
                &twice,
                &["show", "7", "7"]
            )
            .is_empty()
        );
        // an interface the message writes in full
        assert_eq!(
            template(
                &lines(&["%OSPF: OSPF not enabled on GigabitEthernet1/0/2"]),
                &["show", "<interface:GigabitEthernet>"],
                &["show", "Gi1/0/2"]
            ),
            ["%OSPF: OSPF not enabled on {1}"]
        );
        // a value the form holds as a keyword stays as printed
        assert_eq!(
            template(
                &lines(&["% IP routing table topology blue does not exist"]),
                &["show", "topology", "blue"],
                &["show", "topology", "blue"]
            ),
            ["% IP routing table topology blue does not exist"]
        );
    }

    /// A line's parse by a form, word for word: a value token's word as its value.
    fn parse(form: &[&str], words: &[&str]) -> Vec<Item> {
        form.iter()
            .zip(words)
            .enumerate()
            .map(|(tok, (f, w))| match f.strip_prefix('<') {
                Some(name) => Item::Arg {
                    name: name.trim_end_matches('>').to_string(),
                    value: w.to_string(),
                    tok,
                    typed: true,
                },
                None => Item::Kw {
                    kw: f.to_string(),
                    tok,
                    exact: true,
                },
            })
            .collect()
    }

    /// Whether the switch's `%` lines `said` for `line`, held by `form`, print in place of the
    /// output on `config`; where they do not, the output follows the notices.
    fn stands(said: &str, form: &str, line: &str, config: &str) -> bool {
        let form: Vec<&str> = form.split(' ').collect();
        let words: Vec<&str> = line.split(' ').collect();
        let raw = lines(&said.split('\n').collect::<Vec<_>>());
        let held = template(&raw, &form, &words);
        assert_eq!(held.len(), raw.len(), "{said}");
        let config = Config::read(&lines(&config.lines().collect::<Vec<_>>()), true);
        let (out, stood) = print_by(&held, &parse(&form, &words), &config, vec!["output".into()]);
        if stood {
            assert_eq!(out, raw, "{line}");
        } else {
            let notices = raw.iter().filter(|l| kind(l) == Some(Kind::Notice));
            let want: Vec<String> = notices.cloned().chain(["output".to_string()]).collect();
            assert_eq!(out, want, "{line}");
        }
        stood
    }

    #[test]
    fn a_fixed_message_prints_whatever_is_configured_and_a_notice_comes_before_the_output() {
        for config in ["", "router lisp\nspanning-tree mode mst"] {
            assert!(stands(
                "% Error: Unsupported command",
                "show widget",
                "show widget",
                config
            ));
            assert!(stands(
                &format!("{NOTICE}\n% Missing filename in URL."),
                "show widget file <word>",
                "show widget file flash:",
                config
            ));
            assert!(!stands(NOTICE, "show widget", "show widget", config));
        }
    }

    #[test]
    fn a_state_message_prints_where_the_configuration_lacks_what_it_names() {
        // (switch's lines, form, line, a configuration lacking it, configurations holding it)
        let cases: &[(&str, &str, &str, &str, &[&str])] = &[
            (
                "% LISP is not running.",
                "show lisp site",
                "show lisp site",
                "router ospf 1",
                &["router lisp\n locator-set LS"],
            ),
            (
                "%SNMP agent not enabled",
                "show snmp contact",
                "show snmp contact",
                "snmp-server location here\nsnmp-server contact someone",
                &["snmp-server community c RO"],
            ),
            (
                "%%SNMP manager not enabled",
                "show snmp pending",
                "show snmp pending",
                "",
                &["snmp-server manager"],
            ),
            (
                "% MPLS Fault-OAM is not initialized.",
                "show mpls fault-oam summary",
                "show mpls fault-oam summary",
                "",
                &["mpls traffic-eng fault-oam", "mpls tp"],
            ),
            (
                "%AS 7 specified not configured",
                "show ip eigrp <number:1-65535> timers",
                "show ip eigrp 7 timers",
                "router eigrp 9",
                &[
                    "router eigrp 7",
                    "router eigrp NAMED\n address-family ipv4 unicast autonomous-system 7",
                    "router eigrp 9\nipv6 router eigrp 7",
                    // no EIGRP at all: what the switch prints there is unmeasured
                    "",
                ],
            ),
            (
                "%AS(7) specified not configured",
                "show widget bgp <number:1-65535>",
                "show widget bgp 7",
                "router bgp 9",
                &["router bgp 7", ""],
            ),
            (
                "% IP routing table topology BLUE does not exist",
                "show ip route topology <word>",
                "show ip route topology BLUE",
                "",
                &[
                    "router eigrp NAMED\n service-family ipv4 autonomous-system 2\n  topology BLUE tid 10",
                ],
            ),
            (
                "%Topology BLUE specified does not exist.",
                "show eigrp address-family ipv4 topology <word>",
                "show eigrp address-family ipv4 topology BLUE",
                "router eigrp NAMED\n address-family ipv4 unicast autonomous-system 2",
                &[
                    "router eigrp NAMED\n address-family ipv4 unicast autonomous-system 2\n  topology BLUE tid 10",
                    "",
                ],
            ),
            (
                "%OSPF: Specified topology not found for any OSPF process",
                "show ip ospf rib topology <word>",
                "show ip ospf rib topology BLUE",
                "router ospf 1\nrouter eigrp NAMED\n service-family ipv4 autonomous-system 2\n  topology BLUE tid 10",
                &["router ospf 1\n topology BLUE", ""],
            ),
            (
                "%OSPF: OSPF not enabled on Vlan9",
                "show ip ospf interface <interface:Vlan>",
                "show ip ospf interface Vlan9",
                "interface Vlan9\n ip address 198.51.100.1 255.255.255.0\nrouter ospf 1\n network 192.0.2.0 0.0.0.255 area 0",
                &[
                    "interface Vlan9\n ip address 192.0.2.1 255.255.255.0\nrouter ospf 1\n network 192.0.2.0 0.0.0.255 area 0",
                    "interface Vlan9\n ip ospf 1 area 0\nrouter ospf 1",
                    // an address vswitch cannot read
                    "interface Vlan9\n ip address dhcp\nrouter ospf 1\n network 192.0.2.0 0.0.0.255 area 0",
                    "interface Vlan9\n ip address 198.51.100.1 255.255.255.0",
                ],
            ),
            (
                "% No VRF named BLUE",
                "show vrf <word>",
                "show vrf BLUE",
                "vrf definition RED",
                &["vrf definition BLUE", "ip vrf BLUE"],
            ),
            (
                &format!("{NOTICE}\n% Unknown VRF"),
                "show bgp vrf <word> summary",
                "show bgp vrf BLUE summary",
                "",
                &["vrf definition BLUE\n address-family ipv4"],
            ),
            (
                "% Config Logger disabled.",
                "show archive log config all",
                "show archive log config all",
                "archive\n path flash:saved",
                &["archive\n log config\n  logging enable"],
            ),
            (
                "%XMCP server is not enabled",
                "show service-routing xmcp server",
                "show service-routing xmcp server",
                "",
                &["service-routing xmcp listen"],
            ),
            (
                "% Switch is not in mst mode",
                "show spanning-tree mst service instance",
                "show spanning-tree mst service instance",
                "spanning-tree mode rapid-pvst",
                &["spanning-tree mode mst"],
            ),
            (
                "% No matching EVPN ethernet segment",
                "show l2vpn evpn ethernet-segment interface <interface:Port-channel>",
                "show l2vpn evpn ethernet-segment interface Port-channel1",
                "interface Port-channel1",
                &["interface Port-channel1\n evpn ethernet-segment 1"],
            ),
            (
                &format!("{NOTICE}\n% dampening not enabled for base"),
                "show bgp dampening parameters",
                "show bgp dampening parameters",
                "router bgp 1",
                &["router bgp 1\n bgp dampening", ""],
            ),
            (
                &format!("{NOTICE}\n% No such peer-group"),
                "show bgp peer-group <word>",
                "show bgp peer-group PG",
                "router bgp 1\n neighbor OTHER peer-group",
                &["router bgp 1\n neighbor PG peer-group", ""],
            ),
            (
                "%No Tlv-list exists. ",
                "show cdp tlv-list *",
                "show cdp tlv-list *",
                "",
                &["cdp tlv-list T"],
            ),
            (
                "%Tlv-list name does not exists.",
                "show cdp tlv-list <word>",
                "show cdp tlv-list T",
                "cdp tlv-list U",
                &["cdp tlv-list T"],
            ),
        ];
        for (said, form, line, lacking, holding) in cases {
            assert!(stands(said, form, line, lacking), "{line} on {lacking:?}");
            for config in *holding {
                assert!(!stands(said, form, line, config), "{line} on {config:?}");
            }
        }
        // a base topology: OSPF's unicast one and EIGRP's run with the process
        let ospf = "%OSPF: Specified topology not found for any OSPF process";
        let base = "show ip ospf rib multicast topology base";
        let form = "show ip ospf rib multicast topology <word>";
        assert!(stands(ospf, form, base, "router ospf 1"));
        let (form, base) = (
            "show ip ospf rib topology <word>",
            "show ip ospf rib topology base",
        );
        assert!(!stands(ospf, form, base, "router ospf 1"));
        assert!(!stands(
            "%Topology base specified does not exist.",
            "show eigrp address-family ipv4 topology <word>",
            "show eigrp address-family ipv4 topology base",
            "router eigrp NAMED\n address-family ipv4 unicast autonomous-system 2"
        ));
        // vswitch holds no subscriber session
        assert!(stands(
            "%No active Subscriber Sessions",
            "show sss session",
            "show sss session",
            ""
        ));
        // the management port's VRF stands on a switch with one
        let form = ["show", "vrf", "<word>"];
        let words = ["show", "vrf", "Mgmt-vrf"];
        let held = template(&lines(&["% No VRF named Mgmt-vrf"]), &form, &words);
        let out = || vec!["output".to_string()];
        let with = Config::read(&[], true);
        assert!(!print_by(&held, &parse(&form, &words), &with, out()).1);
        let without = Config::read(&[], false);
        assert!(print_by(&held, &parse(&form, &words), &without, out()).1);
    }

    #[test]
    fn notices_stand_before_a_line_no_pattern_reads_and_an_ending_message_ends_the_lines() {
        let form = ["show", "widget"];
        assert_eq!(
            template(&lines(&[NOTICE, "% LISP is not running."]), &form, &form),
            [NOTICE, "% LISP is not running."]
        );
        assert_eq!(
            template(&lines(&[NOTICE, "% Something else"]), &form, &form),
            [NOTICE]
        );
        assert!(template(&lines(&["% LISP is not running.", NOTICE]), &form, &form).is_empty());
        assert!(
            template(
                &lines(&["% LISP is not running.", "% Something else"]),
                &form,
                &form
            )
            .is_empty()
        );
        assert!(template(&lines(&["% Something else"]), &form, &form).is_empty());
        // the switch's spacing is kept, the pattern read without it
        assert_eq!(
            template(&lines(&["%No Tlv-list exists. "]), &form, &form),
            ["%No Tlv-list exists. "]
        );
        assert_eq!(
            kind("%No Tlv-list exists. "),
            Some(Kind::State(Rule::TlvList))
        );
        assert_eq!(
            kind("%AS({3}) specified not configured"),
            Some(Kind::State(Rule::As))
        );
        assert_eq!(kind("% Error: Unsupported command"), Some(Kind::Fixed));
        assert_eq!(kind("% Something else"), None);
    }
}
