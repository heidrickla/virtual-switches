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
    /// an EID table for LISP instance 0 under a running `router lisp`: `instance-id 0` with an
    /// IPv4 service, or `eid-table <vrf> instance-id 0`; a line naming another instance or LISP
    /// router lacks nothing
    LispInstance,
    /// the EID table of the VRF the line names, `default` where it names none, under a running
    /// `router lisp`: an `eid-table` line for it in an instance's IPv4 service, or one naming its
    /// instance
    LispEidTable,
    /// an instance's Ethernet service on the VLAN the line names (`eid-table vlan <n>`), under a
    /// running `router lisp`
    LispVlan,
    /// `ddt` under a running `router lisp`
    LispDdt,
    /// a `prefix-list` under a running `router lisp`, the one the line names where it names one
    LispPrefixList,
    /// a subscriber session or service, which vswitch never holds
    Subscriber,
    /// an `snmp-server` line other than `location` and `contact`, which leave the agent off
    SnmpAgent,
    /// `snmp-server manager`
    SnmpManager,
    /// `mpls traffic-eng fault-oam` or `mpls tp`
    FaultOam,
    /// the EIGRP autonomous system the line names in the line's family and VRF, an EIGRP process
    /// running; the BGP one, a `router bgp` running
    As,
    /// a `topology <name>` line in the line's family (IPv4, unicast or multicast) and VRF for the
    /// topology the line names; `base` always exists
    Topology,
    /// a `topology <name>` line in a running `router eigrp`'s address family of the line's family,
    /// VRF and autonomous system; `base` always exists
    EigrpTopology,
    /// a `topology <name>` line in a running `router ospf`, in its multicast address family for a
    /// multicast line and outside it for a unicast one; unicast `base` always exists
    OspfTopology,
    /// OSPF on the interface the line names: `ip ospf <n> area`, or a `network` of a process in
    /// the interface's VRF covering one of its addresses
    OspfInterface,
    /// the VRF the line names
    Vrf,
    /// `logging enable` under `archive`'s `log config`
    ConfigLogger,
    /// `service-routing xmcp listen`
    Xmcp,
    /// `spanning-tree mode mst`
    Mst,
    /// `evpn ethernet-segment` on the interface the line names
    EthernetSegment,
    /// `bgp dampening` for the base table, in a running `router bgp`: directly under it or in its
    /// global IPv4 unicast address family
    Dampening,
    /// the BGP peer group the line names, in a running `router bgp`: defined outside any VRF
    /// address family, or in the VRF's the line names
    PeerGroup,
    /// `cdp tlv-list`, or the one the line names
    TlvList,
    /// `address-family l2vpn evpn` in a running `router bgp`, for an EVPN line; another family's
    /// line lacks nothing
    EvpnTable,
    /// the EVPN instance the line names (`l2vpn evpn instance <n>`), a `router bgp` running
    Evi,
    /// the IPv4 prefix the line names in the global CEF table (`show::cef_prefixes`); a line in
    /// another VRF lacks nothing
    CefPrefix,
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
    (
        "% Warning: Use of this command may result in packet drops",
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
    // a line's own arguments, which the lab C9200L-48T-4G (17.6.4) refused alike on another
    // configuration
    (
        "% Error: Invalid timestamp. Input must be in the correct format with leading and \
         trailing zeroes",
        Kind::Fixed,
    ),
    (
        "% Error: Interval too large for reverse. Supported maximum interval is 600 seconds!",
        Kind::Fixed,
    ),
    ("% Error: Invalid uuid. Requires prefix [0x]", Kind::Fixed),
    (
        "% Error: Failed to add options for trace filter command, invalid input",
        Kind::Fixed,
    ),
    ("% Error: Unknown Protocol", Kind::Fixed),
    ("% Error: Unknown protocol", Kind::Fixed),
    ("% Error: Unknown interface {}", Kind::Fixed),
    ("% Failed : No such seat", Kind::Fixed),
    ("%error opening {}", Kind::Fixed),
    (
        "%Error processing {}: Unable to allocate memory",
        Kind::Fixed,
    ),
    // a fixed-configuration switch has no linecards
    ("% Linecard information not found", Kind::Fixed),
    ("% LISP is not running.", Kind::State(Rule::Lisp)),
    // the C9200 images on an empty `router lisp` (the lab C9200L-48T-4G, 17.6.4 and 17.15.6; the
    // first EID table text is 17.15's, the second 17.6's)
    (
        "% Could not find EID table instance ID 0 in LISP 0.",
        Kind::State(Rule::LispInstance),
    ),
    (
        "% Could not find EID or RLOC table for specified VRF {}.",
        Kind::State(Rule::LispEidTable),
    ),
    (
        "% Could not find EID table for specified VRF {}.",
        Kind::State(Rule::LispEidTable),
    ),
    ("% Invalid VRF name!", Kind::State(Rule::Vrf)),
    ("% Invalid VLAN ID!", Kind::State(Rule::LispVlan)),
    (
        "% DDT functionality is currently NOT ENABLED.",
        Kind::State(Rule::LispDdt),
    ),
    (
        "% There is no prefix list to display.",
        Kind::State(Rule::LispPrefixList),
    ),
    (
        "% Prefix list {} does not exist.",
        Kind::State(Rule::LispPrefixList),
    ),
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
    // a `router bgp` with no EVPN family or instance (the lab C9200L-48T-4G, 17.6.4 and 17.15.6)
    ("% Network not in table", Kind::State(Rule::EvpnTable)),
    ("% Unknown EVI", Kind::State(Rule::Evi)),
    // the lab C9200L-48T-4G (17.6.4 and 17.15.6)
    ("%Prefix not found", Kind::State(Rule::CefPrefix)),
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
/// message names; a value the form holds as a keyword stays as printed. An ending message is held
/// only where it ends the lines; the notices before a line no pattern reads, or before an ending
/// message another line follows, are held alone (`show bgp ipv6 2001:DB8::/32` printed the obsolete
/// notice, `% Network not in table` and a deprecation notice on the lab C9200L-48T-4G, 17.15.6).
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
    // how many notices came before the first ending message
    let mut notices = None;
    for m in raw {
        let read = fit(m)
            .into_iter()
            .find_map(|(k, slot)| held(m, slot).map(|t| (t, k)));
        match read {
            Some((t, Kind::Notice)) if notices.is_none() => out.push(t),
            Some((t, k)) if !matches!(k, Kind::Notice) => {
                notices.get_or_insert(out.len());
                out.push(t);
            }
            // the notices before a line no pattern reads stand; an ending message does not
            _ => {
                out.truncate(notices.unwrap_or(out.len()));
                return out;
            }
        }
    }
    out
}

/// What a line prints with the message lines its form holds (`template`): the notices, then the
/// messages that end the answer where each holds in `d`, else the command's own output `out`; and
/// whether the messages stood in place of the output. `items` is the line's parse by the form.
pub fn print(held: &[String], items: &[Item], d: &Device, out: Vec<String>) -> (Vec<String>, bool) {
    let mut config = Config::read(
        &crate::show::configuration(d),
        d.stack.management().is_some(),
    );
    config.cef = crate::show::cef_prefixes(d);
    print_by(held, items, &config, out)
}

/// What a LISP show (`show lisp ...`, `show ip|ipv6 lisp ...`) prints with no `router lisp`: the
/// notices its form holds, then `% LISP is not running.`, whatever it prints where one runs. A VRF
/// or VLAN the line names bare is read first, where the form holds its message (`% Invalid VRF
/// name!`, `% Invalid VLAN ID!`), and `show lisp extranet summary` is incomplete. The lab
/// C9200L-48T-4G (17.15.6, network-advantage) printed these for every LISP line of its sweep,
/// typed with no `router lisp`. None for another line, or where `router lisp` runs.
pub fn lisp_stopped(held: &[String], items: &[Item], d: &Device) -> Option<Vec<String>> {
    let values: Vec<&str> = items
        .iter()
        .map(|i| match i {
            Item::Kw { kw, .. } => kw.as_str(),
            Item::Arg { value, .. } => value.as_str(),
        })
        .collect();
    if !matches!(
        values.as_slice(),
        ["show", "lisp", ..] | ["show", "ip" | "ipv6", "lisp", ..]
    ) {
        return None;
    }
    let config = Config::read(
        &crate::show::configuration(d),
        d.stack.management().is_some(),
    );
    if config.top("router lisp") {
        return None;
    }
    if values == ["show", "lisp", "extranet", "summary"] {
        return Some(vec!["% Incomplete command.".into(), String::new()]);
    }
    let mut said = vec![];
    for t in held {
        let at = place(t).map(|(_, at, _)| at);
        let filled = match place(t) {
            Some((a, at, b)) => format!("{a}{}{b}", values.get(at)?),
            None => t.clone(),
        };
        match kind(t) {
            Some(Kind::Notice) => said.push(filled),
            Some(Kind::State(r @ (Rule::Vrf | Rule::LispVlan)))
                if config.lacks(r, items, &values, at) =>
            {
                said.push(filled);
                return Some(said);
            }
            _ => {}
        }
    }
    said.push("% LISP is not running.".into());
    Some(said)
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

/// A line printed under a top-level line, with the submode lines it stands in below that one,
/// outermost first (` address-family ipv4 vrf V` for `  bgp dampening` under `router bgp 1`).
struct Line {
    text: String,
    within: Vec<String>,
}

/// The running configuration's top-level lines, each with the lines printed under it, whether
/// the switch has a management port (its `Mgmt-vrf`), and the prefixes its global CEF table holds.
struct Config {
    sections: Vec<(String, Vec<Line>)>,
    management: bool,
    cef: std::collections::BTreeSet<(std::net::Ipv4Addr, u8)>,
}

/// What a submode chain names: its address family (`ipv6` where a line of it says so, else
/// `ipv4`), whether it is multicast, a service family, and its VRF.
struct Family<'a> {
    ipv6: bool,
    multicast: bool,
    service: bool,
    vrf: Option<&'a str>,
}

impl<'a> Family<'a> {
    fn of(chain: impl IntoIterator<Item = &'a str>) -> Family<'a> {
        let mut f = Family {
            ipv6: false,
            multicast: false,
            service: false,
            vrf: None,
        };
        for l in chain {
            let w: Vec<&str> = l.split_whitespace().collect();
            f.ipv6 |= w.contains(&"ipv6");
            f.multicast |= w.contains(&"multicast");
            f.service |= w.first() == Some(&"service-family");
            if let Some(i) = w.iter().position(|x| *x == "vrf") {
                f.vrf = w.get(i + 1).copied().or(f.vrf);
            }
        }
        f
    }
}

impl Config {
    fn read(config: &[String], management: bool) -> Config {
        let mut sections: Vec<(String, Vec<Line>)> = vec![];
        // the submode lines open in the section, by indentation
        let mut open: Vec<(usize, String)> = vec![];
        for l in config {
            let t = l.trim();
            if t.is_empty() || t.starts_with('!') {
                continue;
            }
            match sections.last_mut() {
                Some(s) if l.starts_with(' ') => {
                    let indent = l.len() - l.trim_start_matches(' ').len();
                    while open.last().is_some_and(|(i, _)| *i >= indent) {
                        open.pop();
                    }
                    let within = open.iter().map(|(_, o)| o.clone()).collect();
                    s.1.push(Line {
                        text: t.to_string(),
                        within,
                    });
                    open.push((indent, t.to_string()));
                }
                _ => {
                    sections.push((t.to_string(), vec![]));
                    open.clear();
                }
            }
        }
        Config {
            sections,
            management,
            cef: Default::default(),
        }
    }

    fn top(&self, prefix: &str) -> bool {
        self.sections.iter().any(|(h, _)| h.starts_with(prefix))
    }

    fn has(&self, header: &str) -> bool {
        self.sections.iter().any(|(h, _)| h == header)
    }

    /// The sections whose header begins `prefix`.
    fn under<'c>(&'c self, prefix: &'c str) -> impl Iterator<Item = &'c (String, Vec<Line>)> {
        self.sections
            .iter()
            .filter(move |(h, _)| h.starts_with(prefix))
    }

    /// Each line under the sections whose header begins `prefix`, with its chain: the header, then
    /// the submode lines it stands in.
    fn lines_under<'c>(&'c self, prefix: &'c str) -> impl Iterator<Item = (&'c str, Vec<&'c str>)> {
        self.under(prefix).flat_map(|(h, lines)| {
            lines.iter().map(move |l| {
                let chain = std::iter::once(h.as_str())
                    .chain(l.within.iter().map(String::as_str))
                    .collect();
                (l.text.as_str(), chain)
            })
        })
    }

    /// The family of each `topology <name>` line under the sections whose header begins `prefix`,
    /// the header itself too where it is one.
    fn topologies<'c>(&'c self, prefix: &'c str, name: &'c str) -> Vec<Family<'c>> {
        let defines = |l: &str| {
            let mut w = l.split_whitespace();
            w.next() == Some("topology") && w.next() == Some(name)
        };
        let headers = self
            .under(prefix)
            .filter(|(h, _)| defines(h))
            .map(|(h, _)| Family::of([h.as_str()]));
        let lines = self
            .lines_under(prefix)
            .filter(|(l, _)| defines(l))
            .map(|(_, chain)| Family::of(chain));
        headers.chain(lines).collect()
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
            Rule::LispInstance => {
                // the line's numbers, the instance and LISP router it names, are all 0
                let zero = items.iter().zip(values).all(|(i, v)| {
                    !matches!(i, Item::Arg { .. })
                        || !v.bytes().all(|b| b.is_ascii_digit())
                        || *v == "0"
                });
                zero && self.top("router lisp")
                    && !self.lines_under("router lisp").any(|(l, chain)| {
                        l.starts_with("eid-table ") && l.ends_with(" instance-id 0")
                            || l == "service ipv4" && chain.last() == Some(&"instance-id 0")
                    })
            }
            Rule::LispEidTable => {
                let vrf = after("vrf").unwrap_or("default");
                let table = if vrf == "default" {
                    "eid-table default".to_string()
                } else {
                    format!("eid-table vrf {vrf}")
                };
                self.top("router lisp")
                    && !self.lines_under("router lisp").any(|(l, chain)| {
                        l.starts_with(&format!("{table} instance-id "))
                            || l == table && chain.contains(&"service ipv4")
                    })
            }
            // read before `router lisp` runs (the lab C9200L-48T-4G, 17.15.6)
            Rule::LispVlan => after("vlan").is_some_and(|v| {
                let table = format!("eid-table vlan {v}");
                !self.lines_under("router lisp").any(|(l, _)| l == table)
            }),
            Rule::LispDdt => {
                self.top("router lisp")
                    && !self.lines_under("router lisp").any(|(l, chain)| {
                        chain.len() == 1 && l.split_whitespace().next() == Some("ddt")
                    })
            }
            Rule::LispPrefixList => {
                let list = |l: &str| match named("prefix-list") {
                    Some(name) => l == format!("prefix-list {name}"),
                    None => l.starts_with("prefix-list "),
                };
                self.top("router lisp")
                    && !self
                        .lines_under("router lisp")
                        .any(|(l, chain)| chain.len() == 1 && list(l))
            }
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
                    // the AS in the line's family and VRF: a classic process (`router eigrp <n>`,
                    // `ipv6 router eigrp <n>`), or a named one's address or service family
                    let ipv6 = has_kw("ipv6");
                    let service = has_kw("service-family");
                    let vrf = after("vrf");
                    let classic = !service
                        && vrf.is_none()
                        && self.has(&format!(
                            "{}router eigrp {n}",
                            if ipv6 { "ipv6 " } else { "" }
                        ));
                    let family = self.lines_under("router eigrp ").any(|(l, chain)| {
                        let w: Vec<&str> = l.split_whitespace().collect();
                        let f = Family::of(chain.into_iter().chain([l]));
                        matches!(w.first(), Some(&"address-family" | &"service-family"))
                            && w.windows(2)
                                .any(|p| p[0] == "autonomous-system" && p[1] == n)
                            && f.ipv6 == ipv6
                            && f.service == service
                            && f.vrf == vrf
                    });
                    (self.top("router eigrp ") || self.top("ipv6 router eigrp "))
                        && !classic
                        && !family
                } else if has_kw("bgp") {
                    self.top("router bgp ") && !self.has(&format!("router bgp {n}"))
                } else {
                    false
                }
            }
            // an IPv4 routing table: unicast or multicast as the line, in its VRF or none
            Rule::Topology => named("topology").is_some_and(|name| {
                let (multicast, vrf) = (has_kw("multicast"), after("vrf"));
                name != "base"
                    && !self
                        .topologies("", name)
                        .iter()
                        .any(|f| !f.ipv6 && f.multicast == multicast && f.vrf == vrf)
            }),
            Rule::EigrpTopology => named("topology").is_some_and(|name| {
                let ipv6 = has_kw("ipv6");
                let vrf = after("vrf");
                let asn = eigrp_as(items, values);
                name != "base"
                    && self.top("router eigrp ")
                    && !self.lines_under("router eigrp ").any(|(l, chain)| {
                        let mut w = l.split_whitespace();
                        let family = chain
                            .iter()
                            .rev()
                            .find(|c| c.starts_with("address-family "));
                        w.next() == Some("topology")
                            && w.next() == Some(name)
                            && family.is_some_and(|a| {
                                let f = Family::of([*a]);
                                let words: Vec<&str> = a.split_whitespace().collect();
                                f.ipv6 == ipv6
                                    && f.vrf == vrf
                                    && asn.is_none_or(|n| {
                                        words
                                            .windows(2)
                                            .any(|p| p[0] == "autonomous-system" && p[1] == n)
                                    })
                            })
                    })
            }),
            // the unicast base topology runs with the process; the multicast one is configured in
            // its multicast address family, any process's
            Rule::OspfTopology => named("topology").is_some_and(|name| {
                let multicast = has_kw("multicast");
                (name != "base" || multicast)
                    && self.top("router ospf ")
                    && !self
                        .topologies("router ospf ", name)
                        .iter()
                        .any(|f| !f.ipv6 && f.multicast == multicast)
            }),
            Rule::OspfInterface => value.is_some_and(|name| self.ospf_off(name)),
            // a LISP EID table names its VRF bare (`show lisp eid-table <vrf>`)
            Rule::Vrf => named("vrf")
                .or_else(|| after("eid-table"))
                .or_else(|| after("locator-table"))
                .is_some_and(|name| {
                    let mgmt = name == "Mgmt-vrf" && self.management;
                    !mgmt
                        && !self.sections.iter().any(|(h, _)| {
                            h.strip_prefix("vrf definition ") == Some(name)
                                || h.strip_prefix("ip vrf ") == Some(name)
                        })
                }),
            Rule::ConfigLogger => !self.lines_under("archive").any(|(l, chain)| {
                l == "logging enable" && chain.last().is_some_and(|c| *c == "log config")
            }),
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
                        .any(|l| l.starts_with("evpn ethernet-segment"))
                }),
            // the base table's: directly under `router bgp` or in its global IPv4 unicast family
            Rule::Dampening => {
                let base = |chain: &[&str]| match chain {
                    [_] => true,
                    [_, af] => matches!(*af, "address-family ipv4" | "address-family ipv4 unicast"),
                    _ => false,
                };
                self.top("router bgp ")
                    && !self
                        .lines_under("router bgp ")
                        .any(|(l, chain)| l.starts_with("bgp dampening") && base(&chain))
            }
            // a peer group of the VRF the line names, else one outside every VRF
            Rule::PeerGroup => after("peer-group").is_some_and(|g| {
                let defined = format!("neighbor {g} peer-group");
                let vrf = after("vrf");
                self.top("router bgp ")
                    && !self
                        .lines_under("router bgp ")
                        .any(|(l, chain)| l == defined && Family::of(chain).vrf == vrf)
            }),
            Rule::TlvList => match after("tlv-list") {
                Some(name) if name != "*" => !self.has(&format!("cdp tlv-list {name}")),
                _ => !self.top("cdp tlv-list "),
            },
            Rule::EvpnTable => {
                has_kw("evpn")
                    && self.top("router bgp ")
                    && !self
                        .lines_under("router bgp ")
                        .any(|(l, _)| l == "address-family l2vpn evpn")
            }
            Rule::Evi => after("evi").is_some_and(|n| {
                let header = format!("l2vpn evpn instance {n}");
                self.top("router bgp ")
                    && !self
                        .sections
                        .iter()
                        .any(|(h, _)| *h == header || h.starts_with(&format!("{header} ")))
            }),
            Rule::CefPrefix => {
                let prefix = values.iter().find_map(|v| {
                    let (a, n) = v.split_once('/')?;
                    Some((a.parse().ok()?, n.parse().ok()?))
                });
                after("vrf").is_none_or(|v| v == "Default")
                    && prefix.is_some_and(|p| !self.cef.contains(&p))
            }
        }
    }

    /// An interface's own lines, not a submode's, none where it has no block.
    fn interface(&self, name: &str) -> impl Iterator<Item = &str> {
        let header = format!("interface {name}");
        self.sections
            .iter()
            .filter(move |(h, _)| *h == header)
            .flat_map(|(_, lines)| lines.iter())
            .filter(|l| l.within.is_empty())
            .map(|l| l.text.as_str())
    }

    /// Whether OSPF is off on an interface: an OSPF process runs, the interface has no `ip ospf <n>
    /// area` line, and no `network` of a process in its VRF covers an address it holds. An address
    /// vswitch cannot read (DHCP, unnumbered) leaves it unknown where such a `network` exists.
    fn ospf_off(&self, name: &str) -> bool {
        let lines: Vec<&str> = self.interface(name).collect();
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
            .flat_map(|(_, lines)| lines.iter().filter(|l| l.within.is_empty()))
            .filter_map(|l| {
                let w: Vec<&str> = l.text.split_whitespace().collect();
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

/// The autonomous system a `show eigrp address-family` line names: the number between its family
/// (and VRF) and `topology`.
fn eigrp_as<'v>(items: &[Item], values: &[&'v str]) -> Option<&'v str> {
    let family = items
        .iter()
        .position(|i| matches!(i, Item::Kw { kw, .. } if kw == "ipv4" || kw == "ipv6"))?;
    let end = items
        .iter()
        .position(|i| matches!(i, Item::Kw { kw, .. } if kw == "topology"))?;
    let between = values.get(family + 1..end)?;
    let mut skip = false;
    between.iter().copied().find(|v| {
        let vrf = *v == "vrf";
        let named = std::mem::replace(&mut skip, vrf);
        !vrf && !named && v.bytes().all(|b| b.is_ascii_digit())
    })
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
            // a line's own arguments refused (the lab C9300-24T and C9200L-48T-4G, 17.6.4)
            assert!(stands(
                "% Error: Unknown interface 10",
                "show widget name <word>",
                "show widget name 10",
                config
            ));
            assert!(stands(
                "%error opening flash:",
                "show format flash:",
                "show format flash:",
                config
            ));
            assert!(!stands(
                "% Warning: Use of this command may result in packet drops",
                "show widget",
                "show widget",
                config
            ));
        }
        assert_eq!(
            template(
                &lines(&["%Error processing flash:: Unable to allocate memory"]),
                &["show", "file", "<word>"],
                &["show", "file", "flash:"]
            ),
            ["%Error processing {2}: Unable to allocate memory"]
        );
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
            // the lab C9200L-48T-4G (17.6.4 and 17.15.6) on an empty `router lisp`; with no LISP
            // process, and with each table defined, what it prints is unmeasured
            (
                "% Could not find EID table instance ID 0 in LISP 0.",
                "show ip lisp database",
                "show ip lisp database",
                "router lisp\n instance-id 3\n  service ipv4",
                &[
                    "router lisp\n instance-id 0\n  service ipv4\n   eid-table default",
                    "router lisp\n eid-table default instance-id 0",
                    "",
                ],
            ),
            (
                "% Could not find EID or RLOC table for specified VRF default.",
                "show lisp eid-table default ipv4",
                "show lisp eid-table default ipv4",
                "router lisp\n instance-id 3\n  service ipv4\n   eid-table vrf RED",
                &[
                    "router lisp\n instance-id 3\n  service ipv4\n   eid-table default",
                    "router lisp\n eid-table default instance-id 3",
                    "",
                ],
            ),
            (
                "% Could not find EID table for specified VRF RED.",
                "show ip lisp eid-table vrf <word> database",
                "show ip lisp eid-table vrf RED database",
                "router lisp\n instance-id 3\n  service ipv4\n   eid-table default",
                &["router lisp\n instance-id 3\n  service ipv4\n   eid-table vrf RED"],
            ),
            (
                "% Invalid VRF name!",
                "show lisp eid-table <word>",
                "show lisp eid-table 10",
                "router lisp",
                &["vrf definition 10"],
            ),
            (
                "% Invalid VLAN ID!",
                "show lisp eid-table vlan <number:1-4094>",
                "show lisp eid-table vlan 1",
                "router lisp\n instance-id 3\n  service ethernet\n   eid-table vlan 10",
                // read before `router lisp` runs: no configuration holds the VLAN (the lab
                // C9200L-48T-4G, 17.15.6)
                &["router lisp\n instance-id 3\n  service ethernet\n   eid-table vlan 1"],
            ),
            (
                "% DDT functionality is currently NOT ENABLED.",
                "show lisp ddt",
                "show lisp ddt",
                "router lisp",
                &["router lisp\n ddt", ""],
            ),
            (
                "% There is no prefix list to display.",
                "show lisp prefix-list",
                "show lisp prefix-list",
                "router lisp",
                &["router lisp\n prefix-list wired\n  192.168.0.0/16", ""],
            ),
            (
                "% Prefix list 10 does not exist.",
                "show lisp prefix-list <word>",
                "show lisp prefix-list 10",
                "router lisp\n prefix-list wired",
                &["router lisp\n prefix-list 10"],
            ),
            // a `router bgp` with neither an EVPN family nor the instance (the lab C9200L-48T-4G,
            // 17.6.4 and 17.15.6)
            (
                "% Network not in table",
                "show bgp l2vpn evpn route-type ethernet-segment",
                "show bgp l2vpn evpn route-type ethernet-segment",
                "router bgp 1\n address-family ipv4",
                &["router bgp 1\n address-family l2vpn evpn", ""],
            ),
            (
                "% Unknown EVI",
                "show bgp l2vpn evpn evi <number:1-65535> detail",
                "show bgp l2vpn evpn evi 1 detail",
                "router bgp 1\nl2vpn evpn instance 2 vlan-based",
                &["router bgp 1\nl2vpn evpn instance 1 vlan-based", ""],
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
        // a LISP line naming another instance, and a BGP line outside EVPN, lack nothing
        assert!(!stands(
            "% Could not find EID table instance ID 0 in LISP 0.",
            "show ip lisp instance-id <number:0-16777200>",
            "show ip lisp instance-id 5",
            "router lisp"
        ));
        assert!(!stands(
            "% Network not in table",
            "show bgp ipv4 unicast <word>",
            "show bgp ipv4 unicast 10.1.1.0/24",
            "router bgp 1"
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
    fn a_cef_prefix_message_reads_the_switch_s_table() {
        // the lab C9200L-48T-4G (17.6.4 and 17.15.6) printed it for a down SVI's subnet, and the
        // prefix once a Null0 route installed it
        let form = ["show", "ip", "cef", "<word>", "detail"];
        let words = ["show", "ip", "cef", "10.1.1.0/24", "detail"];
        let held = template(&lines(&["%Prefix not found"]), &form, &words);
        let out = || vec!["output".to_string()];
        let mut config = Config::read(&[], true);
        assert!(print_by(&held, &parse(&form, &words), &config, out()).1);
        config.cef.insert(([10, 1, 1, 0].into(), 24));
        assert!(!print_by(&held, &parse(&form, &words), &config, out()).1);
        // `Default` is the global table; another VRF's is unread
        let config = Config::read(&[], true);
        let form = ["show", "ip", "cef", "vrf", "<word>", "<word>"];
        for (vrf, stood) in [("Default", true), ("RED", false)] {
            let words = ["show", "ip", "cef", "vrf", vrf, "10.1.1.0/24"];
            let held = template(&lines(&["%Prefix not found"]), &form, &words);
            let said = print_by(&held, &parse(&form, &words), &config, out());
            assert_eq!(said.1, stood, "{vrf}");
        }
    }

    /// A state message reads the context it names: the address family, VRF, autonomous system and
    /// submode a line holds, never a like line in another.
    #[test]
    fn a_state_message_reads_the_configuration_context_it_names() {
        // (switch's lines, form, line, configurations lacking it, configurations holding it)
        type Case<'a> = (&'a str, &'a str, &'a str, &'a [&'a str], &'a [&'a str]);
        let cases: &[Case] = &[
            (
                &format!("{NOTICE}\n% dampening not enabled for base"),
                "show bgp dampening parameters",
                "show bgp dampening parameters",
                &[
                    "router bgp 1\n address-family ipv4 vrf V\n  bgp dampening",
                    "router bgp 1\n address-family ipv6\n  bgp dampening",
                ],
                &[
                    "router bgp 1\n address-family ipv4\n  bgp dampening",
                    "router bgp 1\n address-family ipv4 unicast\n  bgp dampening",
                ],
            ),
            (
                "%AS 7 specified not configured",
                "show ip eigrp <number:1-65535> timers",
                "show ip eigrp 7 timers",
                &[
                    "router eigrp 9\nipv6 router eigrp 7",
                    "router eigrp N\n address-family ipv6 unicast autonomous-system 7",
                    "router eigrp N\n address-family ipv4 unicast vrf V autonomous-system 7",
                    "router eigrp N\n service-family ipv4 autonomous-system 7",
                ],
                &["router eigrp 9\n address-family ipv4 vrf V autonomous-system 3\nrouter eigrp 7"],
            ),
            (
                "%AS 7 specified not configured",
                "show ipv6 eigrp <number:1-65535> timers",
                "show ipv6 eigrp 7 timers",
                &["router eigrp 7"],
                &[
                    "router eigrp 9\nipv6 router eigrp 7",
                    "router eigrp N\n address-family ipv6 unicast autonomous-system 7",
                ],
            ),
            (
                "%AS 7 specified not configured",
                "show eigrp service-family ipv4 <number:1-65535> clients",
                "show eigrp service-family ipv4 7 clients",
                &["router eigrp N\n address-family ipv4 unicast autonomous-system 7"],
                &["router eigrp N\n service-family ipv4 autonomous-system 7"],
            ),
            (
                "% IP routing table topology BLUE does not exist",
                "show ip route topology <word>",
                "show ip route topology BLUE",
                &[
                    "global-address-family ipv6\n topology BLUE",
                    "global-address-family ipv4 multicast\n topology BLUE",
                ],
                &["global-address-family ipv4\n topology BLUE"],
            ),
            (
                "%Topology BLUE specified does not exist.",
                "show eigrp address-family ipv4 topology <word>",
                "show eigrp address-family ipv4 topology BLUE",
                &[
                    "router eigrp N\n address-family ipv6 unicast autonomous-system 2\n  topology BLUE tid 10",
                    "router eigrp N\n service-family ipv4 autonomous-system 2\n  topology BLUE tid 10",
                ],
                &[
                    "router eigrp N\n address-family ipv4 unicast autonomous-system 2\n  topology BLUE tid 10",
                ],
            ),
            (
                "%OSPF: Specified topology not found for any OSPF process",
                "show ip ospf rib topology <word>",
                "show ip ospf rib topology BLUE",
                &["router ospf 1\n address-family ipv4 multicast\n  topology BLUE"],
                &["router ospf 1\n address-family ipv4 unicast\n  topology BLUE"],
            ),
            (
                "%OSPF: Specified topology not found for any OSPF process",
                "show ip ospf rib multicast topology <word>",
                "show ip ospf rib multicast topology base",
                &["router ospf 1\n topology base"],
                &["router ospf 2\nrouter ospf 1\n address-family ipv4 multicast\n  topology base"],
            ),
            (
                &format!("{NOTICE}\n% No such peer-group"),
                "show bgp peer-group <word>",
                "show bgp peer-group PG",
                &["router bgp 1\n address-family ipv4 vrf V\n  neighbor PG peer-group"],
                &["router bgp 1\n neighbor PG peer-group"],
            ),
            (
                "% Config Logger disabled.",
                "show archive log config all",
                "show archive log config all",
                &["archive\n logging enable\n log config\n  notify syslog"],
                &["archive\n path flash:saved\n log config\n  notify syslog\n  logging enable"],
            ),
        ];
        for (said, form, line, lacking, holding) in cases {
            for config in *lacking {
                assert!(stands(said, form, line, config), "{line} on {config:?}");
            }
            for config in *holding {
                assert!(!stands(said, form, line, config), "{line} on {config:?}");
            }
        }
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
        assert_eq!(
            template(
                &lines(&[NOTICE, "% LISP is not running.", NOTICE]),
                &form,
                &form
            ),
            [NOTICE]
        );
        assert_eq!(
            template(
                &lines(&[NOTICE, "% LISP is not running.", "% Something else"]),
                &form,
                &form
            ),
            [NOTICE]
        );
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
