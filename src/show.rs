//! Output of EXEC commands. Formats follow Catalyst 9300 output in Cisco's examples where one
//! exists; the README lists each format's source and the ones that are best effort.

use crate::cli::Parse;
use crate::device::{Block, Device, Link};
use crate::shell::Session;

/// Global lines IOS writes after the interfaces. `no logging console` and `logging buffered` stay
/// near the top, as the cores and c9200 17.15 capture 27 print them.
const AFTER_INTERFACES: &[&str] = &[
    "ip default-gateway",
    "ip forward-protocol",
    "no ip forward-protocol",
    "ip http",
    "no ip http",
    "ip route",
    "ip ssh",
    "no ip ssh",
    "ip scp",
    "logging host",
    "logging origin-id",
    "logging trap",
    "logging source-interface",
    "logging facility",
    "snmp-server",
    "snmp ",
    // numbered lists, where named lists' blocks print (six 2960-Pluses, 15.2(6)E1)
    "access-list ",
];

/// Classic IOS before 15.2 (12.2, 15.0), whose output some rules set apart (the lab C2960G at
/// 15.0(2)SE11).
pub(crate) fn before_15_2(d: &Device) -> bool {
    crate::release::Release::parse(&d.release)
        .zip(crate::release::Release::parse("15.2(2)E"))
        .is_some_and(|(r, e)| r.classic() && r < e)
}

/// Global lines IOS writes last, after the line blocks (the cores' running configurations).
const LAST: &[&str] = &["ntp"];

/// Global lines IOS writes after the VLAN blocks, before the class maps and the interfaces
/// (c9200 17.15 capture 27; c2960x 15.2(7)E capture 8's `no cdp run` precedes its interfaces; the IE-3300s' `lldp run`).
const AFTER_VLANS: &[&str] = &["cdp ", "no cdp ", "lldp "];

/// Global lines IOS writes after `control-plane`, before the banners (c9200 17.15 capture 27).
const AFTER_CONTROL_PLANE: &[&str] = &["privilege "];

/// Blocks IOS writes after the VLAN blocks (c9200 17.15 capture 27).
const AFTER_VLAN_BLOCKS: &[&str] = &["class-map ", "policy-map "];

/// Blocks IOS writes among the global lines after the interfaces, before `snmp-server`
/// (c9200 17.15 capture 27).
const BEFORE_SNMP: &[&str] = &["ip access-list "];

/// Blocks IOS writes after the interfaces' global lines and before the line blocks.
const BEFORE_LINES: &[&str] = &["control-plane"];

/// Blocks IOS writes after the last interface, before the global lines that follow the interfaces
/// (`ip forward-protocol`, `ip route`); no capture holds one.
const AFTER_INTERFACE_BLOCKS: &[&str] = &["router "];

/// One-time actions IOS does not keep in the running configuration (the RSA key pair goes to
/// private NVRAM).
const NOT_STORED: &[&str] = &["crypto key generate"];

/// A `version` line typed or pasted, which every capture holds: the configuration prints its own,
/// the release the switch runs.
fn typed_version(text: &str) -> bool {
    text.starts_with("version ")
}

/// Whether the running configuration has a place of its own for a block: the VRFs, VLANs,
/// interfaces and lines, and the blocks named above. Any other prints among the global blocks in the
/// order typed.
pub fn has_place(header: &str) -> bool {
    vlan_ids_of(header).is_some()
        || ["interface ", "line ", "vrf definition "]
            .iter()
            .chain(AFTER_VLAN_BLOCKS)
            .chain(AFTER_INTERFACE_BLOCKS)
            .chain(BEFORE_SNMP)
            .chain(BEFORE_LINES)
            .any(|p| header.starts_with(p))
}

/// The VLANs a VLAN block's header names (`vlan 10`, `vlan 11,65-66`). The other blocks a `vlan`
/// line opens (`vlan access-map`, `vlan configuration`) name none: the running configuration keeps
/// them in every VTP mode.
pub fn vlan_ids_of(header: &str) -> Option<&str> {
    let list = header.strip_prefix("vlan ")?.trim();
    (!list.is_empty()
        && list
            .chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == '-'))
    .then_some(list)
}

/// The first VLAN a VLAN block names, which orders the blocks.
fn first_vlan(header: &str) -> u32 {
    vlan_ids_of(header)
        .and_then(|l| crate::generate::vlan_ids(l).first().copied())
        .unwrap_or(0)
}

/// The VLANs the VLAN blocks define, and a range a switch keeps as a global line (`vlan 65-66` on
/// an IE 3000 at 15.2(6)E).
pub fn defined_vlans(d: &Device) -> std::collections::BTreeSet<u32> {
    let blocks = d
        .blocks
        .iter()
        .filter_map(|b| vlan_ids_of(&b.header))
        .flat_map(crate::generate::vlan_ids);
    let global = d
        .global
        .iter()
        .filter_map(|e| crate::generate::vlan_definition(&e.text))
        .flatten();
    blocks.chain(global).collect()
}

/// Commands whose next word is a name, which is no part of the command.
const NAMED: &[&str] = &["hostname", "username", "description"];

/// A configuration line's command as IOS orders its lines: the first word, and the second where it
/// is a keyword (`service timestamps`, `switchport access`); a `no` line orders as its command.
pub(crate) fn command(text: &str) -> String {
    let t = text.trim();
    let t = t.strip_prefix("no ").unwrap_or(t);
    let mut words = t.split_whitespace();
    let first = words.next().unwrap_or_default();
    let keyword = |w: &str| {
        w.starts_with(|c: char| c.is_ascii_lowercase())
            && w.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    match words.next() {
        Some(w) if keyword(w) && !NAMED.contains(&first) => format!("{first} {w}"),
        _ => first.to_string(),
    }
}

/// Which commands real switches printed before which, each by its number (`Orders::numbers`): the
/// commands a capture printed, and each pair some capture printed in that order and none the other
/// way round.
#[derive(Default)]
struct Precedence {
    known: std::collections::HashSet<usize>,
    pairs: std::collections::HashSet<(usize, usize)>,
}

impl Precedence {
    fn of(p: &crate::corpus::Precedence) -> Self {
        Precedence {
            known: p.known.iter().copied().collect(),
            pairs: p.pairs.iter().copied().collect(),
        }
    }

    /// Whether IOS prints command `a` before command `b`.
    fn before(&self, a: usize, b: usize) -> bool {
        self.pairs.contains(&(a, b))
    }

    /// Whether a capture printed command `a`.
    fn knows(&self, a: usize) -> bool {
        self.known.contains(&a)
    }
}

/// IOS's printing order as the captures show it.
struct Orders {
    /// each command a capture printed, by number
    numbers: std::collections::HashMap<String, usize>,
    /// top-level lines by book and release, from that release's captures
    release: std::collections::HashMap<(String, String), Precedence>,
    /// top-level lines by family, classic IOS then IOS XE, from every capture of the family
    family: [Precedence; 2],
    /// the lines of interface blocks and of line blocks, from every capture
    interface: Precedence,
    line: Precedence,
    /// commands whose site values the corpus characterized, by number (`live::fits`)
    fitted: Vec<(String, usize)>,
    /// the `!` lines between two top-level commands that a release's captures printed most often,
    /// by book, release and the two commands' numbers
    release_gaps: std::collections::HashMap<(String, String, usize, usize), usize>,
    /// the same over every release of a family, classic IOS then IOS XE
    family_gaps: [std::collections::HashMap<(usize, usize), usize>; 2],
}

impl Orders {
    fn number(&self, cmd: &str) -> Option<usize> {
        self.numbers.get(cmd).copied().or_else(|| {
            self.fitted
                .iter()
                .find(|(p, _)| crate::live::fits(p, cmd))
                .map(|(_, n)| *n)
        })
    }
}

/// The orders, read from the corpus once.
fn orders() -> &'static Orders {
    static ORDERS: std::sync::OnceLock<Orders> = std::sync::OnceLock::new();
    ORDERS.get_or_init(|| {
        let c = crate::corpus::get();
        Orders {
            numbers: c
                .numbers
                .iter()
                .enumerate()
                .map(|(n, cmd)| (cmd.clone(), n))
                .collect(),
            release: c
                .release_order
                .iter()
                .map(|(b, r, p)| ((b.clone(), r.clone()), Precedence::of(p)))
                .collect(),
            family: [
                Precedence::of(&c.family_order[0]),
                Precedence::of(&c.family_order[1]),
            ],
            interface: Precedence::of(&c.interface_order),
            line: Precedence::of(&c.line_order),
            fitted: c
                .numbers
                .iter()
                .enumerate()
                .filter(|(_, cmd)| cmd.contains('«'))
                .map(|(n, cmd)| (cmd.clone(), n))
                .collect(),
            release_gaps: most_often(
                c.gaps
                    .iter()
                    .map(|(b, r, x, y, n, t)| ((b.clone(), r.clone(), *x, *y), *n, *t)),
            ),
            family_gaps: [true, false].map(|classic| {
                most_often(
                    c.gaps
                        .iter()
                        .filter(|(_, r, ..)| {
                            crate::release::Release::parse(r).is_some_and(|r| r.classic())
                                == classic
                        })
                        .map(|(_, _, x, y, n, t)| ((*x, *y), *n, *t)),
                )
            }),
        }
    })
}

/// Each key's value seen the most times, the smaller value on a tie.
fn most_often<K: Eq + std::hash::Hash + Clone>(
    rows: impl Iterator<Item = (K, usize, usize)>,
) -> std::collections::HashMap<K, usize> {
    let mut counts: std::collections::HashMap<(K, usize), usize> = Default::default();
    for (k, n, times) in rows {
        *counts.entry((k, n)).or_default() += times;
    }
    let mut best: std::collections::HashMap<K, (usize, usize)> = Default::default();
    for ((k, n), times) in counts {
        let b = best.entry(k).or_insert((n, times));
        if times > b.1 || (times == b.1 && n < b.0) {
            *b = (n, times);
        }
    }
    best.into_iter().map(|(k, (n, _))| (k, n)).collect()
}

/// The `!` lines between top-level lines as IOS prints them: as many as the switch's release, or
/// failing that its family, printed most often between the same two commands; as built where no
/// capture printed the pair. A banner's text and indented lines stay as they are.
fn separate(d: &Device, body: Vec<String>) -> Vec<String> {
    let o = orders();
    let book = crate::grammars::book(d.stack.family());
    let classic = crate::release::Release::parse(&d.release)
        .map_or_else(|| d.stack.classic(), |r| r.classic());
    let gap = |a: usize, b: usize| {
        o.release_gaps
            .get(&(book.to_string(), d.release.clone(), a, b))
            .or_else(|| o.family_gaps[usize::from(!classic)].get(&(a, b)))
            .copied()
    };
    let mut out = Vec::with_capacity(body.len());
    let mut before: Option<Option<usize>> = None;
    let mut bangs = 0;
    let mut closes: Option<String> = None;
    for l in body {
        if let Some(d) = &closes {
            if l.contains(d.as_str()) {
                closes = None;
            }
            out.push(l);
            continue;
        }
        if before.is_none() {
            before = l.starts_with("version ").then(|| o.number(&command(&l)));
            out.push(l);
            continue;
        }
        if l == "!" {
            bangs += 1;
            continue;
        }
        if l.is_empty() || l.starts_with([' ', '\t']) {
            out.extend(std::iter::repeat_n("!".to_string(), bangs));
            bangs = 0;
            out.push(l);
            continue;
        }
        let this = o.number(&command(&l));
        let n = match (before.flatten(), this) {
            (Some(a), Some(b)) => gap(a, b).unwrap_or(bangs),
            _ => bangs,
        };
        out.extend(std::iter::repeat_n("!".to_string(), n));
        bangs = 0;
        before = Some(this);
        closes = crate::live::banner_opens(&l);
        out.push(l);
    }
    out.extend(std::iter::repeat_n("!".to_string(), bangs));
    out
}

/// `items` in IOS's printing order: each after every item whose command one of `order` puts before
/// its own, otherwise as typed. An item indented deeper than the first, or whose command no capture
/// printed, keeps its place after the item before it.
fn in_ios_order<T>(items: Vec<T>, text: impl Fn(&T) -> &str, order: &[&Precedence]) -> Vec<T> {
    let depth = |s: &str| s.len() - s.trim_start().len();
    let base = items.first().map_or(0, |i| depth(text(i)));
    let mut units: Vec<(Option<usize>, Vec<T>)> = vec![];
    for item in items {
        let known = orders()
            .number(&command(text(&item)))
            .filter(|&n| order.iter().any(|p| p.knows(n)));
        let stays = depth(text(&item)) > base || known.is_none();
        match units.last_mut() {
            Some((_, unit)) if stays => unit.push(item),
            _ => units.push((known, vec![item])),
        }
    }
    let n = units.len();
    let first: Vec<Vec<bool>> = (0..n)
        .map(|a| {
            (0..n)
                .map(|b| match (units[a].0, units[b].0) {
                    (Some(x), Some(y)) => x != y && order.iter().any(|p| p.before(x, y)),
                    _ => false,
                })
                .collect()
        })
        .collect();
    // how many units not yet placed must come before each
    let mut waits: Vec<usize> = (0..n)
        .map(|b| (0..n).filter(|&a| first[a][b]).count())
        .collect();
    let mut left: Vec<usize> = (0..n).collect();
    let mut placed = vec![];
    while !left.is_empty() {
        // the first unit typed that waits on none; a cycle the captures leave takes the first
        let at = left.iter().position(|&u| waits[u] == 0).unwrap_or(0);
        let u = left.remove(at);
        for &w in &left {
            if first[u][w] {
                waits[w] = waits[w].saturating_sub(1);
            }
        }
        placed.push(u);
    }
    let mut units: Vec<Option<Vec<T>>> = units.into_iter().map(|(_, u)| Some(u)).collect();
    placed
        .into_iter()
        .flat_map(|u| units[u].take().unwrap_or_default())
        .collect()
}

/// The top-level orders that apply to a switch: its release's captures, then its family's.
fn top_order(d: &Device) -> Vec<&'static Precedence> {
    let o = orders();
    let book = crate::grammars::book(d.stack.family());
    let classic = crate::release::Release::parse(&d.release)
        .map_or_else(|| d.stack.classic(), |r| r.classic());
    o.release
        .iter()
        .find(|((b, r), _)| b == book && *r == d.release)
        .map(|(_, p)| p)
        .into_iter()
        .chain([&o.family[usize::from(!classic)]])
        .collect()
}

/// The sibling order IOS prints `privilege exec level` commands in, by the command they follow ("" at
/// the top): its parse tree's, as captures printed it (a 2960 at 15.2(6)E and a 9200 at 17.15 alike)
/// and the lab C9300-24T (17.6.4) printed `show ip ...` before `show version`.
const PRIVILEGE_SIBLINGS: &[(&str, &[&str])] = &[
    (
        "",
        &[
            "rcommand",
            "connect",
            "telnet",
            "tunnel",
            "access-enable",
            "traceroute",
            "systat",
            "ping",
            "where",
            "resume",
            "name-connection",
            "login",
            "terminal",
            "show",
            "clear",
        ],
    ),
    ("show", &["ip", "version"]),
];

fn entry_text<'x>(e: &'x &&crate::device::Entry) -> &'x str {
    e.text.as_str()
}

/// `privilege exec level` lines put in their own places in the order IOS walks its parse tree: a
/// command's longer forms before it, siblings as `PRIVILEGE_SIBLINGS` orders them, else as typed
/// (derived: no switch has shown where an unlisted sibling goes). Other lines stay.
fn privilege_tree<T>(items: Vec<T>, text: impl Fn(&T) -> &str) -> Vec<T> {
    fn walk(paths: &[(usize, Vec<String>)], at: &[String], out: &mut Vec<usize>) {
        let mut kids: Vec<(&str, usize)> = vec![];
        for (i, p) in paths {
            if p.len() > at.len() && p.starts_with(at) && !kids.iter().any(|k| k.0 == p[at.len()]) {
                kids.push((&p[at.len()], *i));
            }
        }
        let known = PRIVILEGE_SIBLINGS
            .iter()
            .find(|(c, _)| *c == at.join(" "))
            .map_or(&[][..], |(_, k)| *k);
        kids.sort_by_key(|(w, i)| (known.iter().position(|k| k == w).unwrap_or(usize::MAX), *i));
        for (w, _) in kids {
            let mut next = at.to_vec();
            next.push(w.to_string());
            walk(paths, &next, out);
            out.extend(paths.iter().filter(|(_, p)| *p == next).map(|(i, _)| *i));
        }
    }
    let paths: Vec<(usize, Vec<String>)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let rest = text(e).strip_prefix("privilege exec level ")?;
            let (_, command) = rest.split_once(' ')?;
            Some((
                i,
                command.split_whitespace().map(str::to_lowercase).collect(),
            ))
        })
        .collect();
    if paths.len() < 2 {
        return items;
    }
    let mut order = vec![];
    walk(&paths, &[], &mut order);
    let mut items: Vec<Option<T>> = items.into_iter().map(Some).collect();
    let walked: Vec<T> = order.iter().filter_map(|&i| items[i].take()).collect();
    for ((slot, _), item) in paths.iter().zip(walked) {
        items[*slot] = Some(item);
    }
    items.into_iter().flatten().collect()
}

/// The comment IOS XE prints above `contact-email-addr sch-smart-licensing@cisco.com`.
pub const CALL_HOME_NOTE: [&str; 2] = [
    "! If contact email address in call-home is configured as sch-smart-licensing@cisco.com",
    "! the email address configured in Cisco Smart License Portal will be used as contact email address to send SCH notifications.",
];

/// A block's lines as IOS prints them: an interface's and a line's in IOS's order, an access list's
/// by sequence number as its release prints them (`acl::running`), any other block's as typed (a
/// policy map's order is its meaning).
fn block_lines(b: &Block, classic: bool) -> Vec<String> {
    if let Some((kind, _)) = crate::acl::of(&b.header) {
        return crate::acl::running(kind, &b.lines, classic);
    }
    // a pubkey chain prints its users two spaces in and their keys three, typed or replayed (the
    // lab C9300, 17.6.4, and the lab C2960G, 15.0(2)SE11)
    if b.header == "ip ssh pubkey-chain" {
        return b
            .lines
            .iter()
            .map(|e| {
                let t = e.text.trim_start();
                let user = t.starts_with("username ") || t.starts_with("server ");
                format!("{}{t}", if user { "  " } else { "   " })
            })
            .collect();
    }
    // IOS XE explains the smart licensing address above it, and no other (the lab C9300, 17.6.4;
    // every capture's call-home at 17.5 to 17.9)
    if b.header == "call-home" && !classic {
        return b
            .lines
            .iter()
            .flat_map(|e| {
                let note = e.text.trim() == "contact-email-addr sch-smart-licensing@cisco.com";
                let notes = note.then_some(CALL_HOME_NOTE).into_iter().flatten();
                notes
                    .map(|n| format!(" {n}"))
                    .chain([format!(" {}", e.text)])
            })
            .collect();
    }
    let order = if b.header.starts_with("interface ") {
        Some(&orders().interface)
    } else if b.header.starts_with("line ") {
        Some(&orders().line)
    } else {
        None
    };
    let lines: Vec<&crate::device::Entry> = b.lines.iter().collect();
    let lines = match order {
        Some(o) => in_ios_order(lines, |e| e.text.as_str(), &[o]),
        None => lines,
    };
    lines.iter().map(|e| format!(" {}", e.text)).collect()
}

/// `line con 0`, `line aux 0`, then the vty ranges by first line, as every capture prints them.
fn line_order(header: &str) -> (u8, u32) {
    let mut words = header.split_whitespace().skip(1);
    let kind = match words.next() {
        Some("con") => 0,
        Some("aux") => 1,
        Some("vty") => 2,
        _ => 3,
    };
    (kind, words.next().and_then(|n| n.parse().ok()).unwrap_or(0))
}

fn interface_order(header: &str, d: &Device) -> (u8, usize) {
    let name = header.trim_start_matches("interface ");
    let num = |prefix: &str| {
        name.strip_prefix(prefix)
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(0)
    };
    // Null0 first (the lab C9300, 17.6.4), then loopbacks (c9200 17.15 capture 27), port-channels,
    // the ports and the VLAN interfaces
    if name.starts_with("Null") {
        return (0, num("Null"));
    }
    if name.starts_with("Loopback") {
        return (1, num("Loopback"));
    }
    if name.starts_with("Port-channel") {
        return (2, num("Port-channel"));
    }
    if let Some(i) = d.stack.physical().iter().position(|p| p == name) {
        return (3, i);
    }
    if name.starts_with("Vlan") {
        return (4, num("Vlan"));
    }
    (5, 0)
}

/// The running configuration as `show running-config` prints it, a blank line after `end` (the lab
/// C9300, 17.6.4).
pub fn running_config(d: &Device) -> Vec<String> {
    let body = configuration(d);
    let bytes: usize = body.iter().map(|l| l.len() + 1).sum();
    let mut out = vec![
        "Building configuration...".to_string(),
        String::new(),
        format!("Current configuration : {bytes} bytes"),
    ];
    out.extend(body);
    out.push(String::new());
    out
}

/// The configuration's lines from the first `!` to `end`, as `write memory` keeps them.
pub fn configuration(d: &Device) -> Vec<String> {
    let mut body = vec!["!".to_string()];
    // the train the switch runs (`version 17.5`, a classic `version 15.2`); the 26.x book documents 26.1
    if !d.release.is_empty() {
        let train = if d.release == "26.x" {
            "26.1".to_string()
        } else {
            crate::release::Release::parse(&d.release)
                .map_or_else(|| d.release.clone(), |r| r.config_version())
        };
        body.push(format!("version {train}"));
    }
    let starts = |text: &str, list: &[&str]| list.iter().any(|a| text.starts_with(a));
    // a VTP server or client keeps the VLANs and its VTP settings in vlan.dat
    let in_config = d.vlans_in_running_config();
    let kept: Vec<_> = d
        .global
        .iter()
        .filter(|e| !starts(&e.text, NOT_STORED) && !typed_version(&e.text))
        // `vtp version` prints whatever the mode (six C9200s at 17.9 and 17.12 print `vtp version
        // 1` with no `vtp mode` line)
        .filter(|e| in_config || !e.text.starts_with("vtp ") || e.text.starts_with("vtp version "))
        .filter(|e| !d.held_top.contains(&e.text))
        .collect();
    // banners print after the global lines that follow the interfaces, before the line blocks
    let (banners, kept): (Vec<_>, Vec<_>) =
        kept.into_iter().partition(|e| e.key.starts_with("banner "));
    // each section's lines in IOS's order whatever order they were typed in (`service` lines before
    // `hostname`)
    let top = top_order(d);
    let ordered = |lines: Vec<_>| {
        privilege_tree(
            in_ios_order(lines, |e: &&&crate::device::Entry| e.text.as_str(), &top),
            entry_text,
        )
    };
    let last = ordered(kept.iter().filter(|e| starts(&e.text, LAST)).collect());
    // IOS 12.2 writes its `ip ssh` lines after the VLAN blocks, before the interfaces (four 2960s at
    // 12.2(44)SE and 12.2(55)SE; c2960 12.2(55)SE capture 2's follow its `vlan 535`), 15.2 and IOS XE after the
    // interfaces
    // 12.2 and 15.0 write `ip ssh` before the interfaces (C2960G at 15.0(2)SE11); 15.2 after them
    let ssh_first = before_15_2(d);
    let early_ssh =
        |text: &str| ssh_first && (text.starts_with("ip ssh") || text.starts_with("no ip ssh"));
    // 12.2's `logging 198.51.100.14` is the `logging host` form
    let after_interfaces = |text: &str| {
        (starts(text, AFTER_INTERFACES)
            || text
                .strip_prefix("logging ")
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit())))
            && !early_ssh(text)
    };
    let after = ordered(kept.iter().filter(|e| after_interfaces(&e.text)).collect());
    let after_vlans = ordered(
        kept.iter()
            .filter(|e| starts(&e.text, AFTER_VLANS) || early_ssh(&e.text))
            .collect(),
    );
    let after_cp = ordered(
        kept.iter()
            .filter(|e| starts(&e.text, AFTER_CONTROL_PLANE))
            .collect(),
    );
    let before = ordered(
        kept.iter()
            .filter(|e| {
                !after_interfaces(&e.text)
                    && !early_ssh(&e.text)
                    && ![LAST, AFTER_VLANS, AFTER_CONTROL_PLANE]
                        .iter()
                        .any(|l| starts(&e.text, l))
            })
            .collect(),
    );
    let before = spanning_vlans_in_order(before, |e| e.text.as_str());
    let block = |body: &mut Vec<String>, b: &Block| {
        // a `vlan configuration` with nothing under it is not kept (the lab C9300, 17.6.4)
        if b.lines.is_empty() && b.header.starts_with("vlan configuration ") {
            return;
        }
        // a VLAN with no name ends its line with a space (the lab C9300, 17.6.4; 15 captures at
        // 15.2, 17.6 and 17.15); a named one does not
        if b.lines.is_empty()
            && b.header
                .strip_prefix("vlan ")
                .is_some_and(|v| v.chars().all(|c| c.is_ascii_digit()))
        {
            body.push(format!("{} ", b.header));
        } else {
            body.push(b.header.clone());
        }
        body.extend(block_lines(b, d.stack.classic()));
        body.push("!".into());
    };
    // the VRF definitions follow the hostname (the cores' running configurations, the guides'
    // `show running-config` example)
    let vrfs: Vec<&Block> = d
        .blocks
        .iter()
        .filter(|b| b.header.starts_with("vrf definition "))
        .collect();
    let at_hostname = before
        .iter()
        .position(|e| e.text.starts_with("hostname "))
        .map_or(0, |i| i + 1);
    for e in &before[..at_hostname] {
        body.push(e.text.clone());
    }
    if !vrfs.is_empty() {
        body.push("!".into());
        for b in &vrfs {
            block(&mut body, b);
        }
    }
    for e in &before[at_hostname..] {
        body.push(e.text.clone());
    }
    body.push("!".into());
    let mut vlans: Vec<&Block> = d
        .blocks
        .iter()
        .filter(|b| in_config && vlan_ids_of(&b.header).is_some())
        .collect();
    vlans.sort_by_key(|b| first_vlan(&b.header));
    // IOS lists every interface the stack has, those never configured as an empty block
    let unconfigured: Vec<Block> = d
        .stack
        .physical()
        .into_iter()
        .map(|p| format!("interface {p}"))
        .filter(|h| !d.blocks.iter().any(|b| &b.header == h))
        .map(|header| Block {
            header,
            lines: vec![],
        })
        .collect();
    let mut ifaces: Vec<&Block> = d
        .blocks
        .iter()
        .filter(|b| b.header.starts_with("interface "))
        // Null0 with nothing under it is not printed (the lab C9300, 17.6.4)
        .filter(|b| !(b.header.starts_with("interface Null") && b.lines.is_empty()))
        .chain(&unconfigured)
        .collect();
    ifaces.sort_by_key(|b| interface_order(&b.header, d));
    let mut lines: Vec<&Block> = d
        .blocks
        .iter()
        .filter(|b| b.header.starts_with("line "))
        .collect();
    lines.sort_by_key(|b| line_order(&b.header));
    let late: Vec<&Block> = d
        .blocks
        .iter()
        .filter(|b| starts(&b.header, BEFORE_LINES))
        .collect();
    let routing: Vec<&Block> = d
        .blocks
        .iter()
        .filter(|b| starts(&b.header, AFTER_INTERFACE_BLOCKS) && !d.held_top.contains(&b.header))
        .collect();
    let others: Vec<&Block> = d
        .blocks
        .iter()
        .filter(|b| {
            vlan_ids_of(&b.header).is_none()
                && !b.header.starts_with("interface ")
                && !b.header.starts_with("line ")
                && !b.header.starts_with("vrf definition ")
                && !starts(&b.header, BEFORE_LINES)
                && !starts(&b.header, BEFORE_SNMP)
                && !starts(&b.header, AFTER_INTERFACE_BLOCKS)
                && !d.held_top.contains(&b.header)
        })
        .collect();
    let acls: Vec<&Block> = d
        .blocks
        .iter()
        .filter(|b| starts(&b.header, BEFORE_SNMP) && !d.held_top.contains(&b.header))
        .collect();
    let (mut post, pre): (Vec<&Block>, Vec<&Block>) = others
        .into_iter()
        .partition(|b| starts(&b.header, AFTER_VLAN_BLOCKS));
    // every class map, then every policy map (the 62 captures that hold them)
    post.sort_by_key(|b| {
        AFTER_VLAN_BLOCKS
            .iter()
            .position(|p| b.header.starts_with(p))
    });
    for b in pre.iter().chain(&vlans) {
        block(&mut body, b);
    }
    for e in &after_vlans {
        body.push(e.text.clone());
    }
    for b in post.iter().chain(&ifaces).chain(&routing) {
        block(&mut body, b);
    }
    let (snmp, rest): (Vec<&&&crate::device::Entry>, Vec<_>) =
        after.iter().partition(|e| e.text.starts_with("snmp"));
    let (numbered, rest): (Vec<&&&crate::device::Entry>, Vec<_>) = rest
        .into_iter()
        .partition(|e| e.text.starts_with("access-list "));
    for e in rest.iter().chain(&numbered) {
        body.push(e.text.clone());
    }
    for b in &acls {
        block(&mut body, b);
    }
    for e in &snmp {
        body.push(e.text.clone());
    }
    if !after.is_empty() {
        body.push("!".into());
    }
    for b in &late {
        block(&mut body, b);
    }
    for e in &after_cp {
        body.push(e.text.clone());
    }
    for e in &banners {
        body.extend(e.text.lines().map(str::to_string));
        body.push("!".into());
    }
    for b in &lines {
        body.push(b.header.clone());
        body.extend(block_lines(b, d.stack.classic()));
    }
    body.push("!".into());
    for e in &last {
        body.push(e.text.clone());
    }
    if !last.is_empty() {
        body.push("!".into());
    }
    place_held(d, &mut body);
    body.push("end".into());
    separate(d, body)
}

/// IOS writes the `spanning-tree vlan` lines in VLAN order whatever order they were typed in (the lab
/// C9300, 17.6.4: `vlan 1 priority 24576` before `vlan 10 priority 4096`); the other lines keep their
/// places.
fn spanning_vlans_in_order<T>(items: Vec<T>, text: impl Fn(&T) -> &str) -> Vec<T> {
    let first = |t: &str| -> Option<u32> {
        let list = t
            .strip_prefix("spanning-tree vlan ")?
            .split_whitespace()
            .next()?;
        crate::generate::vlan_ids(list).first().copied()
    };
    let places: Vec<usize> = (0..items.len())
        .filter(|&i| first(text(&items[i])).is_some())
        .collect();
    let mut picked: Vec<(u32, usize)> = places
        .iter()
        .filter_map(|&i| Some((first(text(&items[i]))?, i)))
        .collect();
    picked.sort();
    let mut slots: Vec<Option<T>> = items.into_iter().map(Some).collect();
    let moved: Vec<Option<T>> = picked.iter().map(|&(_, i)| slots[i].take()).collect();
    for (&p, m) in places.iter().zip(moved) {
        slots[p] = m;
    }
    slots.into_iter().flatten().collect()
}

/// Puts the lines and blocks taken as a real switch held them where that switch printed them:
/// after the top-level line before them in its capture, or the nearest earlier one present.
fn place_held(d: &Device, body: &mut Vec<String>) {
    let units = d
        .global
        .iter()
        .filter(|e| d.held_top.contains(&e.text) && !typed_version(&e.text))
        .map(|e| vec![e.text.clone()])
        .chain(
            d.blocks
                .iter()
                .filter(|b| d.held_top.contains(&b.header))
                .map(|b| {
                    let mut u = vec![b.header.clone()];
                    u.extend(block_lines(b, d.stack.classic()));
                    u.push("!".into());
                    u
                }),
        );
    for unit in units.collect::<Vec<_>>() {
        let mut anchor = d.held_after.get(&unit[0]);
        let mut at = None;
        // anchors from a session and from captures can point back at each other
        let mut seen = std::collections::BTreeSet::new();
        // an anchor whose setting changed still marks the place (`vtp mode off` held, `vtp mode
        // transparent` typed: the lab C9300, 17.6.4, kept the crypto blocks after it)
        let same_command = |a: &str, l: &str| {
            let (aw, lw): (Vec<&str>, Vec<&str>) = (
                a.split_whitespace().collect(),
                l.split_whitespace().collect(),
            );
            aw.len() >= 3 && aw.len() == lw.len() && aw[..aw.len() - 1] == lw[..lw.len() - 1]
        };
        while let Some(a) = anchor {
            if let Some(i) = body
                .iter()
                .position(|l| crate::live::fits(a, l))
                .or_else(|| {
                    body.iter()
                        .position(|l| !l.starts_with(' ') && same_command(a, l))
                })
            {
                at = Some(i);
                break;
            }
            if !seen.insert(a) {
                break;
            }
            anchor = d.held_after.get(a);
        }
        let to = match at {
            // past the anchor's indented lines and the `!` closing its block
            Some(i) => {
                let mut j = i + 1;
                while j < body.len() && body[j].starts_with([' ', '\t']) {
                    j += 1;
                }
                if j > i + 1 && body.get(j).is_some_and(|l| l == "!") {
                    j += 1;
                }
                j
            }
            None => body.len(),
        };
        body.splice(to..to, unit);
    }
}

/// What a VTP server or client keeps in vlan.dat, outside the running configuration: its VTP lines
/// and VLANs, as lines that type them back. Empty in transparent mode, where the configuration holds
/// them.
pub fn vlan_database(d: &Device) -> Vec<String> {
    if d.vlans_in_running_config() {
        return vec![];
    }
    let mut out: Vec<String> = d
        .global
        .iter()
        .filter(|e| e.text.starts_with("vtp "))
        .map(|e| e.text.clone())
        .collect();
    let mut vlans: Vec<&Block> = d
        .blocks
        .iter()
        .filter(|b| vlan_ids_of(&b.header).is_some())
        .collect();
    vlans.sort_by_key(|b| first_vlan(&b.header));
    for b in vlans {
        out.push(b.header.clone());
        out.extend(b.lines.iter().map(|e| format!(" {}", e.text)));
    }
    out
}

/// What an EXEC command prints. The line as written out picks the output: the books list some forms
/// as commands of their own (`show running-config interface`).
pub fn render(s: &Session, name: &str, p: &Parse, _line: &str) -> Vec<String> {
    let running = p.canonical().starts_with("show running-config");
    match name {
        _ if running => {
            let all = running_config(&s.device);
            // `show running-config interface <name>`: that block alone
            let words: Vec<String> = p
                .canonical()
                .split_whitespace()
                .map(str::to_string)
                .collect();
            if words.len() > 2 && words[2].eq_ignore_ascii_case("interface") {
                let want = format!("interface {}", words[3..].join(""));
                let mut body: Vec<String> = vec!["!".into()];
                if let Some(b) = s
                    .device
                    .blocks
                    .iter()
                    .find(|b| b.header.eq_ignore_ascii_case(&want))
                {
                    body.push(b.header.clone());
                    body.extend(block_lines(b, s.device.stack.classic()));
                } else if s.device.stack.has(&words[3..].join("")) {
                    // a port never configured is an empty block
                    body.push(want);
                }
                body.push("end".into());
                let bytes: usize = body.iter().map(|l| l.len() + 1).sum();
                let mut out = vec![
                    "Building configuration...".to_string(),
                    String::new(),
                    format!("Current configuration : {bytes} bytes"),
                ];
                out.extend(body);
                out.push(String::new());
                return out;
            }
            all
        }
        n if n.starts_with("show") => {
            // the books spell some commands `show interface ...`, which IOS reads as `interfaces`
            let c = p
                .canonical()
                .replacen("show interface ", "show interfaces ", 1);
            let view = s.lab.as_ref();
            if let Some(out) = crate::spanning::show(&s.device, view, &c) {
                return out;
            }
            // the switch as the lab leaves it: ports BPDU guard err-disabled, and their far ends
            // without a link
            let eff = crate::spanning::effective(&s.device, view);
            let d: &Device = &eff;
            // every platform keeps IOS's trailing spaces (`act/unsup `): every family's show tech
            // has them; only a terminal log loses them
            match c.as_str() {
                "show interfaces status" => interfaces_status(d),
                "show interfaces status err-disabled" => interfaces_status_errdisabled(d),
                "show vlan brief" => vlan_brief(d, s.width),
                "show vlan" => vlan(d, s.width),
                "show interfaces switchport" => interfaces_switchport(d),
                // one port: `show interfaces Gi1/0/1 switchport`
                _ if c.starts_with("show interfaces ")
                    && c.ends_with(" switchport")
                    && one_port(&c).is_some_and(|p| interface_switchport(d, &p).is_some()) =>
                {
                    one_port(&c)
                        .and_then(|p| interface_switchport(d, &p))
                        .unwrap_or_default()
                }
                "show etherchannel summary" => etherchannel_summary(d),
                "show interfaces trunk" => {
                    let sp = crate::spanning::Spanning::of(&s.device, view);
                    interfaces_trunk_with(d, &|p: &str, v: u32| {
                        crate::spanning::forwards(&sp, p, v)
                    })
                }
                "show ip interface brief" => ip_interface_brief(d),
                "show power inline" => power_inline(d),
                "show vtp status" => vtp_status(d),
                // the configuration saved or booted from, as NVRAM keeps it (`startup`); a switch
                // with none answers as IOS does with its NVRAM erased
                "show startup-config" => match &s.saved {
                    Some(lines) => {
                        crate::startup::show(lines, crate::grammars::book(s.device.stack.family()))
                    }
                    None => vec!["startup-config is not present".into()],
                },
                // IPv4 access lists, all or the one named (c9300 17.6 capture 1 and the lab 2960G, 2026-10-07)
                _ if c == "show ip access-lists" || c.starts_with("show ip access-lists ") => {
                    access_lists(&s.device, c.split_whitespace().nth(3))
                }
                _ if c == "show access-lists" || c.starts_with("show access-lists ") => {
                    access_lists(&s.device, c.split_whitespace().nth(2))
                }
                _ => vec![format!("% vswitch has no output model for '{c}' yet")],
            }
        }
        _ => vec![],
    }
}

fn config_of<'d>(d: &'d Device, name: &str) -> Vec<&'d str> {
    d.config_of(name)
}

/// `show ip access-lists [<name>]`: each IPv4 access list as created, or the one named; nothing
/// when there is none (the lab 2960G with none configured).
pub fn access_lists(d: &Device, name: Option<&str>) -> Vec<String> {
    d.blocks
        .iter()
        .filter_map(|b| crate::acl::of(&b.header).map(|(kind, n)| (kind, n, b)))
        .filter(|(_, n, _)| name.is_none_or(|want| want == *n))
        .flat_map(|(kind, n, b)| crate::acl::show(kind, n, &b.lines, d.stack.classic()))
        .collect()
}

/// A port's link while the port runs: not shut down and not a supervisor uplink that is off.
pub(crate) fn up<'d>(d: &'d Device, name: &str) -> Option<&'d Link> {
    let l = d.links.get(name)?;
    let off = config_of(d, name).contains(&"shutdown")
        || d.stack.uplink_off(name, &enabled_uplinks(d)).is_some();
    (!off).then_some(l)
}

/// A bundled member's link and name: a port-channel runs while one of its members is bundled.
fn bundle<'d>(d: &'d Device, channel: &str) -> Option<(String, &'d Link)> {
    let n = channel.strip_prefix("Port-channel")?;
    if config_of(d, channel).contains(&"shutdown") {
        return None;
    }
    let member = format!("channel-group {n} mode ");
    d.stack.physical().into_iter().find_map(|p| {
        let l = up(d, &p).filter(|l| l.bundled)?;
        config_of(d, &p)
            .iter()
            .any(|c| c.starts_with(&member))
            .then_some((p, l))
    })
}

/// A member's port-channel number and whether it runs LACP (`channel-group <n> mode active`).
pub(crate) fn channel_of(d: &Device, p: &str) -> Option<(String, bool)> {
    let c = config_of(d, p);
    let mut words = value(&c, "channel-group ")?.split_whitespace();
    let n = words.next()?.to_string();
    Some((n, matches!(words.nth(1), Some("active" | "passive"))))
}

/// An LACP member with a link it does not bundle: IOS XE suspends it unless its port-channel has
/// `no port-channel standalone-disable` (the lab C9300, 17.6.4); classic IOS leaves it stand-alone.
pub(crate) fn suspended(d: &Device, p: &str) -> bool {
    !d.stack.classic()
        && up(d, p).is_some_and(|l| !l.bundled)
        && channel_of(d, p).is_some_and(|(n, lacp)| {
            lacp && !config_of(d, &format!("Port-channel{n}"))
                .contains(&"no port-channel standalone-disable")
        })
}

/// A port-channel no member is bundled in: no VLAN (`unassigned`), its native VLAN 0, whatever its
/// mode or members (the lab C9300, 17.6.4, and the lab C2960G, 15.0(2)SE11); classic IOS from 15.2
/// shows the access VLAN (a 2960-X capture at 15.2).
fn channel_down(d: &Device, p: &str) -> bool {
    p.starts_with("Port-channel")
        && bundle(d, p).is_none()
        && (!d.stack.classic() || before_15_2(d))
}

/// The speed a port's link runs (Mb/s): its transceiver's, a copper link's negotiated speed, else
/// the copper port's own.
pub(crate) fn link_mbps(name: &str, l: &Link) -> u32 {
    match (l.optic, l.speed) {
        (Some(o), _) => o.mbps,
        (None, Some(s)) => s,
        (None, None) if name.starts_with("TenGigabitEthernet") => 10_000,
        (None, None) if name.starts_with("FastEthernet") => 100,
        (None, None) => 1000,
    }
}

/// Whether a switch port with a link carries VLAN `id`: as its access VLAN, or in a trunk's allowed list.
fn carries(d: &Device, name: &str, l: &Link, id: u32) -> bool {
    let c = config_of(d, name);
    if c.contains(&"no switchport") {
        return false;
    }
    if l.trunk {
        value(&c, "switchport trunk allowed vlan ")
            .is_none_or(|list| crate::generate::vlan_ids(list).contains(&id))
    } else {
        value(&c, "switchport access vlan ")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1)
            == id
    }
}

pub(crate) fn value<'a>(lines: &[&'a str], prefix: &str) -> Option<&'a str> {
    lines.iter().find_map(|l| l.strip_prefix(prefix))
}

/// `TwentyFiveGigE1/0/1` as IOS's tables print it: `Twe1/0/1`.
pub fn short(name: &str) -> String {
    for (full, abbreviation) in crate::cli::interface_types() {
        if let Some(rest) = name.strip_prefix(full)
            && rest.starts_with(|c: char| c.is_ascii_digit())
        {
            return format!("{abbreviation}{rest}");
        }
    }
    name.to_string()
}

/// The front-panel ports in IOS order, then the port-channels by number; never the management port.
fn ports(d: &Device) -> Vec<String> {
    let mut out: Vec<String> = d
        .stack
        .physical()
        .into_iter()
        .filter(|p| Some(p.as_str()) != d.stack.management())
        .collect();
    let mut channels: Vec<(u32, String)> = d
        .blocks
        .iter()
        .filter_map(|b| b.header.strip_prefix("interface Port-channel"))
        .filter_map(|n| n.parse().ok().map(|v| (v, format!("Port-channel{n}"))))
        .collect();
    channels.sort();
    out.extend(channels.into_iter().map(|(_, n)| n));
    out
}

/// `speed 10000` as the Speed column prints it: `10G`.
fn speed(s: &str) -> String {
    match s.parse::<u32>() {
        Ok(v) if v >= 10_000 && v % 1000 == 0 => format!("{}G", v / 1000),
        _ => s.to_string(),
    }
}

/// The supervisor QSFP ports `enable` has turned on, as (slot, port).
pub fn enabled_uplinks(d: &Device) -> std::collections::BTreeSet<(u32, u32)> {
    d.blocks
        .iter()
        .filter(|b| b.lines.iter().any(|e| e.text == "enable"))
        .filter_map(|b| {
            let name = b.header.strip_prefix("interface ")?;
            let num = &name[name.find(|c: char| c.is_ascii_digit())?..];
            let parts: Vec<u32> = num.split('/').filter_map(|p| p.parse().ok()).collect();
            match parts[..] {
                [slot, 0, port] => Some((slot, port)),
                _ => None,
            }
        })
        .collect()
}

/// `show interfaces status err-disabled`, laid out as the lab C9300 printed it (17.6.4):
/// BPDU guard the only reason modelled; the 12-character name cut is inferred.
fn interfaces_status_errdisabled(d: &Device) -> Vec<String> {
    let mut out = vec![
        "Port         Name         Status       Reason               Err-disabled Vlans"
            .to_string(),
        String::new(),
    ];
    for p in ports(d)
        .into_iter()
        .filter(|p| d.errdisabled.contains_key(p))
    {
        let c = config_of(d, &p);
        let name: String = value(&c, "description ")
            .unwrap_or("")
            .chars()
            .take(12)
            .collect();
        out.push(format!(
            "{:<13}{name:<13}{:<13}bpduguard",
            short(&p),
            "err-disabled"
        ));
    }
    out
}

/// A transceiver's Type in `show interfaces status`: the IE-3300 leaves off the Catalysts' ` SFP`
/// (`1000BaseLX`, ie3300 17.6 capture 1).
fn optic_type(d: &Device, o: &crate::platform::Optic) -> &'static str {
    match d.stack.family() {
        Some(crate::platform::Family::IE3300) => o.shows.strip_suffix(" SFP").unwrap_or(o.shows),
        _ => o.shows,
    }
}

/// `show interfaces status`, laid out as the cores print it (Catalyst 9400, 17.5): a port is
/// `disabled` when shut down, `inactive` when it is a supervisor uplink that is off
/// (`Stack::uplink_off`), `suspended` when IOS XE suspends an LACP member, and `notconnect`
/// otherwise, no port having a link; an AppGigabitEthernet port is always connected. The Vlan
/// column is the access VLAN, as on a port that is not trunking, whether or not the VLAN exists
/// (the lab C9300, 17.6.4, and the lab C2960G: never `inactive` for a deleted VLAN).
/// Classic IOS (c2960x 15.2(7)E capture 8, 2960-X, 15.2(7)E0a): a 10-column Port, an empty SFP slot `auto auto Not
/// Present`, a linked 1G optic autonegotiated, no Type on a port-channel, and FastEthernet0 last.
pub fn interfaces_status(d: &Device) -> Vec<String> {
    use crate::platform::Media;
    let classic = d.stack.classic();
    let port_width = if classic { 9 } else { 12 };
    let row = |p: &str, n: &str, st: &str, v: &str, du: &str, sp: &str, ty: &str| {
        // an empty Type keeps the space before it (the lab C2960G's Po1)
        format!("{p:<port_width$} {n:<18} {st:<12} {v:<10} {du:>6} {sp:>6} {ty}")
    };
    // a 2960-Plus at 15.2(6)E1 keeps a space after Type (c2960 15.2(6)E capture 5); the C2960G at 15.0(2)SE11
    // does not
    let header = row("Port", "Name", "Status", "Vlan", "Duplex", "Speed", "Type");
    let mut out = vec![
        String::new(),
        if d.stack.family().is_some_and(|f| f.classic_two_part()) && !before_15_2(d) {
            format!("{header} ")
        } else {
            header
        },
    ];
    let enabled = enabled_uplinks(d);
    // a module slot's ports show only where the module is fitted (c9300 17.15 capture 1's NM-8X: Te1/1/1-8)
    for p in ports(d).into_iter().filter(|p| d.stack.fitted(p)) {
        let c = config_of(d, &p);
        let name: String = value(&c, "description ")
            .unwrap_or("")
            .chars()
            .take(18)
            .collect();
        let app = p.starts_with("AppGigabitEthernet");
        // a port's own link, or a port-channel's bundled member's
        let link = up(d, &p).map(|l| (p.clone(), l)).or_else(|| bundle(d, &p));
        let held = suspended(d, &p);
        let status = if c.contains(&"shutdown") {
            "disabled"
        } else if d.errdisabled.contains_key(&p) {
            // BPDU guard heard a BPDU (the lab C9300, 17.6.4); its far end prints `notconnect`
            "err-disabled"
        } else if d.stack.uplink_off(&p, &enabled).is_some() {
            "inactive"
        } else if held {
            "suspended"
        } else if app || link.is_some() {
            "connected"
        } else {
            "notconnect"
        };
        let vlan = if c.contains(&"no switchport") {
            "routed"
        } else if channel_down(d, &p) {
            "unassigned"
        } else if link.as_ref().is_some_and(|(_, l)| l.trunk) {
            "trunk"
        } else {
            value(&c, "switchport access vlan ").unwrap_or("1")
        };
        // an application hosting port: up as the cores' (9400, 17.5, Type `unknown`) and the lab
        // C9300's (17.6.4), shut as six 9300s' at 17.15
        let (mut duplex, mut sp, mut ty) = if app && status == "disabled" {
            ("auto", "auto".to_string(), "App-hosting port")
        } else if app && d.stack.family() == Some(crate::platform::Family::C9400) {
            ("a-full", "a-1000".to_string(), "unknown")
        } else if app {
            ("a-full", "a-1000".to_string(), "App-hosting port")
        } else if p.starts_with("Port-channel") {
            ("auto", "auto".into(), if classic { "" } else { "N/A" })
        } else {
            match d.stack.media(&p) {
                // a 2960's or 2960-Plus's Fast Ethernet ports (c2960 15.2(6)E capture 5, 15.2(6)E1)
                Media::Copper if p.starts_with("FastEthernet") => {
                    ("auto", "auto".into(), "10/100BaseTX")
                }
                Media::Copper => ("auto", "auto".into(), "10/100/1000BaseTX"),
                Media::Sfp | Media::SfpPlus if classic => ("auto", "auto".into(), "Not Present"),
                // an empty slot on a fixed switch reads `auto` (c9300 17.15 capture 5's C9300-24S, 17.15), a
                // 9400 card's its native speed (the cores, 17.5)
                Media::Sfp if d.stack.family() != Some(crate::platform::Family::C9400) => {
                    ("auto", "auto".into(), "unknown")
                }
                Media::Sfp => ("auto", "a-1000".into(), "unknown"),
                Media::SfpPlus => ("auto", "a-10G".into(), "unknown"),
                Media::Uplink | Media::Other => ("auto", "auto".into(), "unknown"),
            }
        };
        // with a link: a transceiver's fixed speed, an autonegotiated copper speed, a bundle's
        // speed (the cores: `full 10G SFP-10GBase-SR`, `a-full a-1000`, `a-full a-10G N/A`)
        match &link {
            // a bundle's speed, fixed where its members' is set (c2960x 15.2(7)E capture 7's Po3: `a-full 1000`)
            Some((m, l)) if p.starts_with("Port-channel") => {
                duplex = "a-full";
                let set = value(&config_of(d, m), "speed ").is_some_and(|s| s != "auto");
                let speed = crate::platform::speed_text(link_mbps(m, l));
                sp = if set { speed } else { format!("a-{speed}") };
            }
            Some((_, l)) => match l.optic {
                Some(o) if o.negotiates || classic && o.mbps <= 1000 => {
                    duplex = "a-full";
                    sp = format!("a-{}", crate::platform::speed_text(o.mbps));
                    ty = optic_type(d, o);
                }
                Some(o) => {
                    duplex = "full";
                    sp = crate::platform::speed_text(o.mbps);
                    ty = optic_type(d, o);
                }
                // a far end on half duplex negotiates it (c9200 17.15 capture 7's `a-half a-100`)
                None => {
                    duplex = if l.half { "a-half" } else { "a-full" };
                    sp = format!("a-{}", crate::platform::speed_text(link_mbps(&p, l)));
                    // a dual-purpose uplink linked by copper runs its RJ-45 side
                    if d.stack.dual_purpose(&p) {
                        ty = "10/100/1000BaseTX";
                    }
                }
            },
            // an err-disabled port still names its transceiver (the lab C9300, 17.6.4)
            None => {
                if let Some(Some(o)) = d.errdisabled.get(&p) {
                    ty = optic_type(d, o);
                }
            }
        }
        // a suspended member negotiates its speed and not its duplex (the lab C9300's `auto a-1000`)
        if held {
            duplex = "auto";
        }
        match value(&c, "speed ") {
            Some("auto") | None => {}
            Some("nonegotiate") => sp = "auto".into(),
            Some(s) => sp = speed(s),
        }
        if let Some(x) = value(&c, "duplex ").filter(|x| *x != "auto") {
            duplex = x;
        }
        out.push(row(&short(&p), &name, status, vlan, duplex, &sp, ty));
    }
    // the 2960-X's management port, last; a 2960, 2960-Plus or 2960-C has none
    if let (true, Some(m)) = (classic, d.stack.management()) {
        let c = config_of(d, m);
        let name: String = value(&c, "description ")
            .unwrap_or("")
            .chars()
            .take(18)
            .collect();
        let status = if c.contains(&"shutdown") {
            "disabled"
        } else {
            "notconnect"
        };
        out.push(row(
            &short(m),
            &name,
            status,
            "routed",
            "auto",
            "auto",
            "10/100BaseTX",
        ));
    }
    out
}

/// `show interfaces trunk`, laid out as the command reference's example: each port trunking now (a
/// port-channel in place of its members), its DTP mode, the allowed VLANs, those the VLAN database
/// has, and those forwarding. Nothing while no port trunks.
#[cfg(test)]
pub fn interfaces_trunk(d: &Device) -> Vec<String> {
    interfaces_trunk_with(d, &|_: &str, _: u32| true)
}

/// `show interfaces trunk` with spanning tree's state: the last list keeps the VLANs a port forwards
/// in (`forwards`), `none` on a port blocked in all of them (the lab C9300, 17.6.4).
pub fn interfaces_trunk_with(d: &Device, forwards: &dyn Fn(&str, u32) -> bool) -> Vec<String> {
    let mut defined = defined_vlans(d);
    defined.insert(1);
    let mut rows: Vec<(String, Vec<&str>)> = vec![];
    for p in ports(d) {
        let link = up(d, &p).or_else(|| bundle(d, &p).map(|(_, l)| l));
        let member = !p.starts_with("Port-channel") && link.is_some_and(|l| l.bundled);
        // a suspended LACP member does not trunk (the lab C9300, 17.6.4)
        if link.is_some_and(|l| l.trunk) && !member && !suspended(d, &p) {
            rows.push((p.clone(), config_of(d, &p)));
        }
    }
    if rows.is_empty() {
        return vec![];
    }
    let allowed = |c: &[&str]| -> std::collections::BTreeSet<u32> {
        value(c, "switchport trunk allowed vlan ")
            .map_or_else(|| (1..=4094).collect(), crate::generate::vlan_ids)
    };
    // two VLANs in a row print as a range here (`460-461`, c2960x 15.2(7)E capture 8), where the running
    // configuration's list writes `460,461`
    let list = |set: &std::collections::BTreeSet<u32>| {
        let v: Vec<u32> = set.iter().copied().collect();
        let mut out = vec![];
        let mut i = 0;
        while i < v.len() {
            let mut j = i;
            while j + 1 < v.len() && v[j + 1] == v[j] + 1 {
                j += 1;
            }
            out.push(if j > i {
                format!("{}-{}", v[i], v[j])
            } else {
                v[i].to_string()
            });
            i = j + 1;
        }
        if out.is_empty() {
            "none".to_string()
        } else {
            out.join(",")
        }
    };
    // the Port column: 12 on classic IOS (c2960x 15.2(7)E capture 8), 15 on IOS XE (c9200 17.6 capture 1, C9200L 17.6.4)
    let w = if d.stack.classic() { 12 } else { 15 };
    // IOS XE pads the header's last column (the lab C9300, 17.6.4)
    let pad = if d.stack.classic() { "" } else { "  " };
    let mut out = vec![
        String::new(),
        format!(
            "{:<w$}Mode             Encapsulation  Status        Native vlan{pad}",
            "Port"
        ),
    ];
    for (p, c) in &rows {
        let mode = match value(c, "switchport mode ") {
            Some("dynamic desirable") => "desirable",
            Some("trunk") => "on",
            _ => "auto",
        };
        // `switchport trunk native vlan tag` sets tagging, not the VLAN
        let native = c
            .iter()
            .filter_map(|l| l.strip_prefix("switchport trunk native vlan "))
            .find(|v| v.parse::<u32>().is_ok())
            .unwrap_or("1");
        out.push(format!(
            "{:<w$}{mode:<17}{:<15}{:<14}{native}",
            short(p),
            "802.1q",
            "trunking"
        ));
    }
    for title in [
        "Vlans allowed on trunk",
        "Vlans allowed and active in management domain",
        "Vlans in spanning tree forwarding state and not pruned",
    ] {
        out.push(String::new());
        out.push(format!("{:<w$}{title}", "Port"));
        for (p, c) in &rows {
            let mut set = allowed(c);
            if title != "Vlans allowed on trunk" {
                set.retain(|v| defined.contains(v));
            }
            if title.starts_with("Vlans in spanning tree") {
                set.retain(|v| forwards(p, *v));
            }
            out.push(format!("{:<w$}{}", short(p), list(&set)));
        }
    }
    out
}

/// A PoE port's admin state and the milliwatts it may draw: `power inline never` is off, `static`
/// reserves its maximum, and a `max` lowers the port's own (PoE+ 30 W, UPOE 60 W, UPOE+ 90 W).
pub fn poe_setting(lines: &[&str], kind_max: u32) -> (&'static str, u32) {
    let max = |prefix: &str| {
        lines
            .iter()
            .find_map(|l| l.strip_prefix(prefix))
            .and_then(|v| v.parse().ok())
    };
    if lines.contains(&"power inline never") {
        ("off", kind_max)
    } else if let Some(mw) = max("power inline static max ") {
        ("static", mw)
    } else if lines.contains(&"power inline static") {
        ("static", kind_max)
    } else {
        ("auto", max("power inline auto max ").unwrap_or(kind_max))
    }
}

/// `show power inline`, laid out as the 9300 command reference's example (a C9300-48UN, 610 W): a
/// row per member whose budget is known, the watts its default power supply gives, those static
/// ports reserve and those left; then each PoE port, off with no powered device, its allocation and
/// its maximum. Nothing on a stack without PoE.
pub fn power_inline(d: &Device) -> Vec<String> {
    // an IE-3300 with no PoE prints its own empty table, the same on all six (17.6.4)
    if d.stack.family() == Some(crate::platform::Family::IE3300)
        && d.stack
            .members
            .iter()
            .all(|m| crate::platform::model(&m.model).is_some_and(|x| x.poe.is_none()))
    {
        return [
            "Available: n/a(w)  Used: n/a(w)  Remaining: n/a",
            "",
            "Interface Admin  Oper       Power   Device              Class Max",
            "                            (Watts)                            ",
            "--------- ------ ---------- ------- ------------------- ----- ----",
            "--------- ------ ---------- ---------- ---------- ------ ----- ",
            "Totals:          0    on    0.0       ",
            "",
        ]
        .map(String::from)
        .to_vec();
    }
    let mut modules = vec![];
    let mut rows = vec![];
    for m in &d.stack.members {
        let Some(model) = crate::platform::model(&m.model) else {
            continue;
        };
        let Some(poe) = model.poe else { continue };
        let mut reserved = 0;
        for &(ty, lo, hi) in model.ports {
            for port in lo..=hi {
                let name = if model.family.two_part() {
                    format!("{ty}0/{port}")
                } else {
                    format!("{ty}{}/0/{port}", m.number)
                };
                let (admin, mw) = poe_setting(&d.config_of(&name), poe.kind.max_mw());
                let power = if admin == "static" { mw } else { 0 };
                reserved += power;
                rows.push(format!(
                    "{:<10}{admin:<7}{:<11}{:<8}{:<20}{:<6}{:.1} ",
                    short(&name),
                    "off",
                    format!("{:.1}", power as f64 / 1000.0),
                    "n/a",
                    "n/a",
                    mw as f64 / 1000.0
                ));
            }
        }
        if let Some(w) = poe.watts {
            let used = reserved as f64 / 1000.0;
            modules.push(format!(
                "{:<6}{:>11.1}{used:>11.1}{:>12.1}",
                m.number,
                w as f64,
                w as f64 - used
            ));
        }
    }
    // a 2960, 2960-Plus or 2960-C without PoE prints its totals and an empty table (c2960 15.2(6)E capture 5)
    let none = rows.is_empty() && d.stack.family().is_some_and(|f| f.two_part());
    if rows.is_empty() && !none {
        return vec![];
    }
    let mut out = if none {
        vec![
            "Available:0.0(w)  Used:0.0(w)  Remaining:0.0(w)".to_string(),
            String::new(),
        ]
    } else {
        vec![String::new()]
    };
    if !modules.is_empty() {
        out.extend(
            [
                "Module   Available     Used     Remaining",
                "          (Watts)     (Watts)    (Watts) ",
                "------   ---------   --------   ---------",
            ]
            .map(String::from),
        );
        out.extend(modules);
    }
    out.extend(
        [
            "Interface Admin  Oper       Power   Device              Class Max",
            "                            (Watts)                            ",
            "--------- ------ ---------- ------- ------------------- ----- ----",
        ]
        .map(String::from),
    );
    out.extend(rows);
    out
}

/// The VLANs `show vlan` lists, with each one's name and status: 1, those defined, 1002-1005.
fn vlan_list(d: &Device) -> Vec<(u32, String, &'static str)> {
    let mut vlans: Vec<(u32, String, &str)> = vec![(1, "default".into(), "active")];
    for b in &d.blocks {
        let Some(ids) = vlan_ids_of(&b.header) else {
            continue;
        };
        let name = b.lines.iter().find_map(|e| e.text.strip_prefix("name "));
        let status = if b.lines.iter().any(|e| e.text == "shutdown") {
            "act/lshut"
        } else {
            "active"
        };
        for id in crate::generate::vlan_ids(ids) {
            if id == 1 || id >= 1002 || vlans.iter().any(|v| v.0 == id) {
                continue;
            }
            let shown = name.map_or(format!("VLAN{id:04}"), str::to_string);
            vlans.push((id, shown, status));
        }
    }
    // a range kept as a global line defines its VLANs, named by default
    for id in d
        .global
        .iter()
        .filter_map(|e| crate::generate::vlan_definition(&e.text))
        .flatten()
    {
        if id != 1 && id < 1002 && !vlans.iter().any(|v| v.0 == id) {
            vlans.push((id, format!("VLAN{id:04}"), "active"));
        }
    }
    for (id, name) in [
        (1002, "fddi-default"),
        (1003, "token-ring-default"),
        (1004, "fddinet-default"),
        (1005, "trnet-default"),
    ] {
        vlans.push((id, name.into(), "act/unsup"));
    }
    vlans.sort_by_key(|v| v.0);
    vlans
}

/// `show vlan brief`, laid out as the cores print it: each VLAN's access ports, comma-separated,
/// wrapped between ports before `width` columns under the Ports column.
pub fn vlan_brief(d: &Device, width: usize) -> Vec<String> {
    let mut out = vec![
        String::new(),
        "VLAN Name                             Status    Ports".to_string(),
        "---- -------------------------------- --------- -------------------------------"
            .to_string(),
    ];
    let vlans = vlan_list(d);
    let members: Vec<(String, u32)> = ports(d)
        .into_iter()
        .filter_map(|p| {
            let c = config_of(d, &p);
            // a module port with no module fitted (c9300 17.15 capture 5's Te1/1/1-8) is not listed
            if c.contains(&"no switchport") || !d.stack.fitted(&p) {
                return None;
            }
            // a trunking port and a bundle's member are not listed; the bundle is, unless it trunks,
            // and only once a member is bundled (c2960x 15.2(7)E capture 8's Po1, every member down, is not)
            let link = up(d, &p).or_else(|| bundle(d, &p).map(|(_, l)| l));
            if link.is_some_and(|l| l.trunk || (l.bundled && !p.starts_with("Port-channel"))) {
                return None;
            }
            if p.starts_with("Port-channel") && link.is_none() {
                return None;
            }
            let id = value(&c, "switchport access vlan ")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            Some((short(&p), id))
        })
        .collect();
    for (id, name, status) in vlans {
        let name: String = name.chars().take(32).collect();
        let mut line = format!("{id:<4} {name:<32} {status:<9} ");
        let mut first = true;
        for (p, _) in members.iter().filter(|(_, v)| *v == id) {
            if first {
                line.push_str(p);
                first = false;
            } else if line.len() + 2 + p.len() > width {
                out.push(line);
                line = format!("{:48}{p}", "");
            } else {
                line.push_str(", ");
                line.push_str(p);
            }
        }
        out.push(line);
    }
    out
}

/// The port a `show interfaces <port> <what>` line names, as IOS names it.
fn one_port(c: &str) -> Option<String> {
    let words: Vec<&str> = c.split_whitespace().collect();
    let middle = &words[2..words.len() - 1];
    let (full, num, used) = crate::cli::interface_name(middle.first()?, middle.get(1).copied())?;
    (used == middle.len()).then(|| format!("{full}{num}"))
}

/// `show interfaces switchport`, as the 39 show techs print it: a block per switchport, blank lines
/// between. A module port with no module fitted gives its mode alone; a routed port is not listed.
/// The IOS XE Catalysts add `Vepa Enabled` and `App Interface`; the IE-3300 does not.
pub fn interfaces_switchport(d: &Device) -> Vec<String> {
    let blocks: Vec<Vec<String>> = switchport_blocks(d).into_iter().map(|(_, b)| b).collect();
    blocks.join(&String::new())
}

/// `show interfaces <port> switchport`: the port's block alone, with no blank line after it (the lab
/// C9300, 17.6.4); `None` for a port the full form does not list.
pub fn interface_switchport(d: &Device, port: &str) -> Option<Vec<String>> {
    switchport_blocks(d)
        .into_iter()
        .find(|(p, _)| p.eq_ignore_ascii_case(port))
        .map(|(_, b)| b)
}

/// Each switch port and its `show interfaces switchport` block.
fn switchport_blocks(d: &Device) -> Vec<(String, Vec<String>)> {
    let vlans = vlan_list(d);
    let named = |id: u32| match vlans.iter().find(|v| v.0 == id) {
        Some((_, name, _)) => format!("{id} ({name})"),
        None => format!("{id} (Inactive)"),
    };
    let mut blocks: Vec<(String, Vec<String>)> = vec![];
    for p in ports(d) {
        let c = config_of(d, &p);
        // a routed port is not listed (the 9300s' AppGigabitEthernet1/0/1, and the slot ports of
        // kinds the module lacks, which IOS made routed)
        if c.contains(&"no switchport") {
            continue;
        }
        let mut b = vec![format!("Name: {}", short(&p)), "Switchport: Enabled".into()];
        // a tunnel and a private-VLAN port name their mode as the configuration guides print it
        let admin = match value(&c, "switchport mode ") {
            Some("access") => "static access",
            Some("trunk") => "trunk",
            Some("dynamic desirable") => "dynamic desirable",
            Some("dot1q-tunnel") => "tunnel",
            Some(m) if m.starts_with("private-vlan ") => m,
            _ => "dynamic auto",
        };
        b.push(format!("Administrative Mode: {admin}"));
        if !d.stack.fitted(&p) {
            // c9300 17.15 capture 5's Te1/1/1-8, a C9300-24S with no module
            b.push(String::new());
            blocks.push((p, b));
            continue;
        }
        let link = up(d, &p).or_else(|| bundle(d, &p).map(|(_, l)| l));
        // an application hosting port runs as an access port whatever its mode (the lab C9300, 17.6.4)
        let app = p.starts_with("AppGigabitEthernet") && !c.contains(&"shutdown");
        let oper = match link {
            _ if app => "static access",
            None => "down",
            Some(l) if l.trunk => "trunk",
            Some(_) => "static access",
        };
        // a bundled member names its port-channel; IOS XE calls an LACP member it has not bundled,
        // linked or not, suspended (the lab C9300, 17.6.4)
        let channel = channel_of(d, &p).filter(|_| !p.starts_with("Port-channel"));
        let mut held = false;
        match channel {
            Some((n, _)) if link.is_some_and(|l| l.bundled) => {
                b.push(format!("Operational Mode: {oper} (member of bundle Po{n})"));
            }
            Some((n, true)) if !d.stack.classic() && (link.is_none() || suspended(d, &p)) => {
                held = true;
                b.push(format!(
                    "Operational Mode: down (suspended member of bundle Po{n})"
                ));
            }
            _ => b.push(format!("Operational Mode: {oper}")),
        }
        b.push("Administrative Trunking Encapsulation: dot1q".into());
        if app {
            b.push("Operational Trunking Encapsulation: native".into());
        } else if let Some(l) = link.filter(|_| !held) {
            let enc = if l.trunk { "dot1q" } else { "native" };
            b.push(format!("Operational Trunking Encapsulation: {enc}"));
        }
        // a static mode sends no DTP: access, tunnel and private-VLAN ports
        let negotiates = matches!(admin, "trunk" | "dynamic auto" | "dynamic desirable")
            && !c.contains(&"switchport nonegotiate");
        b.push(format!(
            "Negotiation of Trunking: {}",
            if negotiates { "On" } else { "Off" }
        ));
        // the first number a line of the command gives (`native vlan tag` gives none)
        let number = |prefix: &str, default: u32| {
            c.iter()
                .filter_map(|l| l.strip_prefix(prefix))
                .find_map(|v| v.parse().ok())
                .unwrap_or(default)
        };
        if channel_down(d, &p) {
            b.push("Access Mode VLAN: unassigned".into());
            b.push("Trunking Native Mode VLAN: 0 (Inactive)".into());
        } else {
            b.push(format!(
                "Access Mode VLAN: {}",
                named(number("switchport access vlan ", 1))
            ));
            b.push(format!(
                "Trunking Native Mode VLAN: {}",
                named(number("switchport trunk native vlan ", 1))
            ));
        }
        // the IE-3300 tags no native VLAN by default (ie3300 17.6 capture 1)
        let ie = d.stack.family() == Some(crate::platform::Family::IE3300);
        b.push(format!(
            "Administrative Native VLAN tagging: {}",
            if ie { "disabled" } else { "enabled" }
        ));
        // a VLAN by number and name; `dot1p`, `untagged` and `none` as written
        let voice = match value(&c, "switchport voice vlan ") {
            Some(v) => v.parse().map_or_else(|_| v.to_string(), named),
            None => "none".to_string(),
        };
        b.push(format!("Voice VLAN: {voice}"));
        b.extend(
            [
                "Administrative private-vlan host-association: none ",
                "Administrative private-vlan mapping: none ",
                "Administrative private-vlan trunk native VLAN: none",
                "Administrative private-vlan trunk Native VLAN tagging: enabled",
                "Administrative private-vlan trunk encapsulation: dot1q",
                "Administrative private-vlan trunk normal VLANs: none",
                "Administrative private-vlan trunk associations: none",
                "Administrative private-vlan trunk mappings: none",
                "Operational private-vlan: none",
            ]
            .map(String::from),
        );
        let allowed = match value(&c, "switchport trunk allowed vlan ") {
            None | Some("all") => "ALL".to_string(),
            Some("none") => "NONE".to_string(),
            Some(list) => crate::target::ios_list(&crate::generate::vlan_ids(list)),
        };
        b.push(format!("Trunking VLANs Enabled: {allowed}"));
        // the VLANs VTP may prune (2-1001) the pruning lines leave eligible, each read in turn as
        // the allowed list reads its `all`, `none`, `add`, `remove` and `except`
        let mut eligible: Option<std::collections::BTreeSet<u32>> = None;
        for spec in c
            .iter()
            .filter_map(|l| l.strip_prefix("switchport trunk pruning vlan "))
        {
            let now = eligible.as_ref().map(crate::target::ios_list);
            eligible = Some(
                crate::target::allowed_after(now.as_deref(), spec)
                    .into_iter()
                    .filter(|v| (2..=1001).contains(v))
                    .collect(),
            );
        }
        let pruning = match eligible {
            None => "2-1001".to_string(),
            Some(set) if set.is_empty() => "NONE".to_string(),
            Some(set) => crate::target::ios_list(&set),
        };
        b.push(format!("Pruning VLANs Enabled: {pruning}"));
        // a port-channel has no capture lines (the lab C9300, 17.6.4, and the lab C2960G)
        if !p.starts_with("Port-channel") {
            b.push("Capture Mode Disabled".into());
            b.push("Capture VLANs Allowed: ALL".into());
        }
        b.push(String::new());
        let flag = |line: &str, on: &str, off: &str| {
            (if c.contains(&line) { on } else { off }).to_string()
        };
        b.push(format!(
            "Protected: {}",
            flag("switchport protected", "true", "false")
        ));
        b.push(format!(
            "Unknown unicast blocked: {}",
            flag("switchport block unicast", "enabled", "disabled")
        ));
        b.push(format!(
            "Unknown multicast blocked: {}",
            flag("switchport block multicast", "enabled", "disabled")
        ));
        if !d.stack.classic() && !ie {
            b.push("Vepa Enabled: false".into());
            b.push("App Interface: false".into());
        }
        b.push("Appliance trust: none".into());
        blocks.push((p, b));
    }
    blocks
}

/// `show vlan`, as every show tech prints it: the brief table, each VLAN's type, SAID (100000 and
/// its number) and MTU, then the remote SPAN and private VLAN tables, empty while vswitch models
/// neither.
pub fn vlan(d: &Device, width: usize) -> Vec<String> {
    let mut out = vlan_brief(d, width);
    out.push(String::new());
    out.push(
        "VLAN Type  SAID       MTU   Parent RingNo BridgeNo Stp  BrdgMode Trans1 Trans2".into(),
    );
    out.push(
        "---- ----- ---------- ----- ------ ------ -------- ---- -------- ------ ------".into(),
    );
    for (id, _, _) in vlan_list(d) {
        let (kind, stp) = match id {
            1002 => ("fddi", "-"),
            1003 => ("tr", "-"),
            1004 => ("fdnet", "ieee"),
            1005 => ("trnet", "ibm"),
            _ => ("enet", "-"),
        };
        out.push(format!(
            "{id:<4} {kind:<5} {:<10} 1500  -      -      -        {stp:<4} -        0      0   ",
            100_000 + id
        ));
    }
    let rule = "-".repeat(78);
    out.extend(
        [
            "",
            "Remote SPAN VLANs",
            &rule,
            "",
            "",
            "Primary Secondary Type              Ports",
            "------- --------- ----------------- ------------------------------------------",
            "",
        ]
        .map(String::from),
    );
    out
}

/// `show etherchannel summary`, laid out as both C9200Ls print it: every port-channel and its
/// members, Layer 2 (`S`) or, with `no switchport`, Layer 3 (`R`); a member bundled (`P`), linked
/// but not bundled (`I`) or down (`D`), and the port-channel in use (`U`) while a member is bundled.
pub fn etherchannel_summary(d: &Device) -> Vec<String> {
    let mut groups: std::collections::BTreeMap<u32, (&str, Vec<String>)> = Default::default();
    for p in d.stack.physical() {
        let c = config_of(d, &p);
        let Some(spec) = value(&c, "channel-group ") else {
            continue;
        };
        let mut words = spec.split_whitespace();
        let Some(n) = words.next().and_then(|n| n.parse().ok()) else {
            continue;
        };
        let protocol = match words.nth(1) {
            Some("active" | "passive") => "LACP",
            Some("desirable" | "auto") => "PAgP",
            _ => "-",
        };
        // with a link: bundled, suspended (`suspended`), else stand-alone
        let flag = match up(d, &p) {
            Some(l) if l.bundled => "P",
            Some(_) if suspended(d, &p) => "s",
            Some(_) => "I",
            None => "D",
        };
        let g = groups.entry(n).or_insert((protocol, vec![]));
        g.0 = protocol;
        g.1.push(format!("{}({flag})", short(&p)));
    }
    for b in &d.blocks {
        if let Some(n) = b
            .header
            .strip_prefix("interface Port-channel")
            .and_then(|n| n.parse().ok())
        {
            groups.entry(n).or_insert(("-", vec![]));
        }
    }
    let classic = d.stack.classic();
    // the legend in groups as both C9200Ls print it; classic IOS from 15.2 adds N and m
    // (c2960x 15.2(7)E capture 8)
    let mut legend = vec![
        "Flags:  D - down        P - bundled in port-channel",
        "        I - stand-alone s - suspended",
        "        H - Hot-standby (LACP only)",
        "        R - Layer3      S - Layer2",
    ];
    // before 15.2 classic IOS prints IOS XE's legend without Auto LAG (the C2960G, 15.0(2)SE11)
    let old = before_15_2(d);
    if classic && !old {
        legend.extend([
            "        U - in use      N - not in use, no aggregation",
            "        f - failed to allocate aggregator",
        ]);
    } else {
        legend.push("        U - in use      f - failed to allocate aggregator");
    }
    legend.extend(["", "        M - not in use, minimum links not met"]);
    if classic && !old {
        legend.push("        m - not in use, port not aggregated due to minimum links not met");
    }
    legend.extend([
        "        u - unsuitable for bundling",
        "        w - waiting to be aggregated",
        "        d - default port",
        "",
    ]);
    if !old {
        legend.extend(["        A - formed by Auto LAG", ""]);
    }
    legend.push("");
    let mut out: Vec<String> = legend.iter().map(|s| s.to_string()).collect();
    out.push(format!("Number of channel-groups in use: {}", groups.len()));
    out.push(format!("Number of aggregators:           {}", groups.len()));
    out.push(String::new());
    out.push("Group  Port-channel  Protocol    Ports".into());
    out.push(
        "------+-------------+-----------+-----------------------------------------------".into(),
    );
    for (n, (protocol, members)) in groups {
        let layer = if config_of(d, &format!("Port-channel{n}")).contains(&"no switchport") {
            'R'
        } else {
            'S'
        };
        let state = if bundle(d, &format!("Port-channel{n}")).is_some() {
            'U'
        } else {
            'D'
        };
        let po = format!("Po{n}({layer}{state})");
        // no protocol prints ` -` (the lab C9300, 17.6.4, and the lab C2960G)
        let protocol = if protocol == "-" { " -" } else { protocol };
        if classic && before_15_2(d) {
            // before 15.2 each member is padded to 12, its padding kept (the lab C2960G,
            // 15.0(2)SE11); wrapping as 15.2 does is inferred
            let mut line = format!("{n:<7}{po:<16}{protocol:<10}");
            for (i, m) in members.iter().enumerate() {
                if i > 0 && line.trim_end().len() + 1 + m.len() > 78 {
                    out.push(line);
                    line = format!("{:33}", "");
                }
                line.push_str(&format!("{m:<12}"));
            }
            out.push(line);
        } else if classic {
            // the members wrap under the Ports column before column 79 (c2960x 15.2(7)E capture 7's
            // four); a group with none keeps its padding (the lab C2960G's Po1)
            let mut line = format!("{n:<7}{po:<16}{protocol:<10}");
            for (i, m) in members.iter().enumerate() {
                if i > 0 && line.len() + 1 + m.len() > 78 {
                    out.push(line);
                    line = format!("{:33}", "");
                } else if i > 0 {
                    line.push(' ');
                }
                line.push_str(m);
            }
            out.push(if members.is_empty() {
                line
            } else {
                line.trim_end().to_string()
            });
        } else {
            // IOS XE: each member padded to 16, two a line, the rest under the Ports column, at
            // any terminal width (the lab C9300, 17.6.4, at widths 80, 100 and 512)
            let mut line = format!("{n:<7}{po:<16}{protocol:<12}");
            for (i, m) in members.iter().enumerate() {
                if i > 0 && i % 2 == 0 {
                    out.push(line);
                    line = format!("{:35}", "");
                }
                line.push_str(&format!("{m:<16}"));
            }
            out.push(line);
        }
    }
    out.push(String::new());
    out
}

/// `show vtp status` under VTP version 1 or 2, laid out as the cores and c2960x 15.2(7)E capture 8 print it: the
/// VTP lines vlan.dat keeps (default version 1, mode Server, pruning off), traps from `snmp-server
/// enable traps vtp`, the updater the lowest-numbered VLAN interface with an address, and the VLANs
/// counted with 1 and 1002-1005, Device ID the base MAC address. A virtual switch has no change
/// history or vlan.dat digest: the MD5 digest comes from the domain and VLANs, and the database reads
/// as never modified.
pub fn vtp_status(d: &Device) -> Vec<String> {
    let global = |prefix: &str| {
        d.global
            .iter()
            .find_map(|e| e.text.strip_prefix(prefix))
            .map(str::to_string)
    };
    let has = |line: &str| d.global.iter().any(|e| e.text == line);
    let version = global("vtp version ").unwrap_or_else(|| "1".into());
    // version 3 prints a layout of its own (a primary server, a block per feature) that no capture
    // shows: every captured switch runs version 1
    if version == "3" {
        return vec![
            "% vswitch has no output model for 'show vtp status' under VTP version 3 yet".into(),
        ];
    }
    let domain = global("vtp domain ").unwrap_or_default();
    let mode = match global("vtp mode ").as_deref() {
        Some("client") => "Client",
        Some("transparent") => "Transparent",
        Some("off") => "Off",
        _ => "Server",
    };
    let on = |b: bool| if b { "Enabled" } else { "Disabled" };
    // IOS XE from 17.6 adds the operational state (both C9200Ls; the cores at 17.5 do not); pruning
    // runs only on a server or client
    let pruning = has("vtp pruning");
    let pruning_line = match crate::release::Release::parse(&d.release) {
        Some(r) if !r.classic() && r >= crate::release::Release::parse("17.6").expect("17.6") => {
            let runs = pruning && matches!(mode, "Server" | "Client");
            format!("{} (Operationally {})", on(pruning), on(runs))
        }
        _ => on(pruning).to_string(),
    };
    let traps = has("snmp-server enable traps vtp") || has("snmp-server enable traps");
    // the lowest-numbered VLAN interface with an address, else the first layer 3 interface with
    // one (the lab C9300, 17.6.4: Lo0 while Vlan10 waited on DHCP)
    let address = |b: &Block| {
        b.lines
            .iter()
            .find_map(|e| e.text.strip_prefix("ip address "))?
            .split_whitespace()
            .next()
            .filter(|ip| ip.contains('.'))
            .map(str::to_string)
    };
    let updater = d
        .blocks
        .iter()
        .filter_map(|b| {
            let n: u32 = b.header.strip_prefix("interface Vlan")?.parse().ok()?;
            Some((n, address(b)?))
        })
        .min()
        .map(|(n, ip)| (format!("Vl{n}"), ip, "lowest numbered VLAN interface found"))
        .or_else(|| {
            let mut l3: Vec<&Block> = d
                .blocks
                .iter()
                .filter(|b| {
                    b.header.starts_with("interface ")
                        && !b.header.starts_with("interface Vlan")
                        && address(b).is_some()
                })
                .collect();
            l3.sort_by_key(|b| interface_order(&b.header, d));
            l3.first().and_then(|b| {
                let name = b.header.strip_prefix("interface ")?;
                Some((short(name), address(b)?, "first layer3 interface found"))
            })
        });
    let mut vlans = defined_vlans(d);
    vlans.extend([1, 1002, 1003, 1004, 1005]);
    // FNV-1a over a seed and a text, for the values a real switch holds and vswitch does not
    let hash = |seed: u64, text: &str| {
        text.bytes().fold(0xcbf2_9ce4_8422_2325 ^ seed, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
    };
    // the base MAC address, as `show version` names it (13 captures that print both agree)
    let device_id = d.mac.to_string();
    let database = format!("{domain} {vlans:?}");
    let digest: Vec<String> = [hash(1, &database), hash(2, &database)]
        .iter()
        .flat_map(|h| h.to_be_bytes())
        .map(|b| format!("0x{b:02X}"))
        .collect();
    // IOS ends both digest lines with a space
    let gap = " ";
    let field = |name: &str, v: &str| format!("{name:<32}: {v}");
    let feature = |name: &str, v: &str| format!("{name:<34}: {v}");
    let mut out = vec![
        field("VTP Version capable", "1 to 3"),
        field("VTP version running", &version),
        field("VTP Domain Name", &domain),
        field("VTP Pruning Mode", &pruning_line),
        field("VTP Traps Generation", on(traps)),
        field("Device ID", &device_id),
        "Configuration last modified by 0.0.0.0 at 0-0-00 00:00:00".into(),
        match &updater {
            Some((n, ip, why)) => format!("Local updater ID is {ip} on interface {n} ({why})"),
            None => "Local updater ID is 0.0.0.0 (no valid interface found)".into(),
        },
        String::new(),
        "Feature VLAN:".into(),
        "--------------".into(),
        feature("VTP Operating Mode", mode),
        // a 2960-Plus's LAN Base image keeps 255 (c2960 15.2(6)E capture 5)
        feature(
            "Maximum VLANs supported locally",
            match d.stack.family() {
                Some(f) if f.classic_two_part() => "255",
                // ie3300 17.6 capture 1, 17.6.4
                Some(crate::platform::Family::IE3300) => "259",
                _ => "1005",
            },
        ),
        feature("Number of existing VLANs", &vlans.len().to_string()),
        feature("Configuration Revision", "0"),
        feature("MD5 digest", &format!("{}{gap}", digest[..8].join(" "))),
        format!("{:36}{}{gap}", "", digest[8..].join(" ")),
    ];
    // a transparent switch updates no one (c9200 17.15 capture 27), nor one with VTP off (the lab
    // C9300, 17.6.4)
    if matches!(mode, "Transparent" | "Off") {
        out.remove(7);
    }
    out
}

/// `show ip interface brief`, laid out as the 9300 command reference's example: the VLAN interfaces,
/// the ports and the port-channels, then loopbacks and tunnels. Up: a loopback, an AppGigabitEthernet
/// port, a port with a link, a port-channel with a bundled member, a VLAN interface whose VLAN a
/// linked port carries.
pub fn ip_interface_brief(d: &Device) -> Vec<String> {
    // Protocol padded to its header's width (the IE-3300s, the C2960G; only terminal logs lost it);
    // a space ends each column, so a name as wide as its column stays apart from the address
    // IOS XE prints a name wider than its column short (the lab C9300, 17.6.4: Te1/1/1, Fo1/1/1,
    // Ap1/0/1; TwentyFiveGigE1/1/1 and Port-channel1 fit); classic IOS prints it whole
    let classic = d.stack.classic();
    let row = |i: &str, ip: &str, ok: &str, m: &str, st: &str, pr: &str| {
        let i = if i.len() > 22 && !classic {
            short(i)
        } else {
            i.to_string()
        };
        format!("{i:<22} {ip:<15} {ok:<3} {m:<6} {st:<21} {pr:<8}")
    };
    // no blank line before the header (c9200 17.6 capture 1 and c9200 17.15 capture 27, C9200L 17.6.4 and 17.15.2)
    let mut out = vec![row(
        "Interface",
        "IP-Address",
        "OK?",
        "Method",
        "Status",
        "Protocol",
    )];
    let named = |prefix: &str| {
        let mut v: Vec<(u64, String)> = d
            .blocks
            .iter()
            .filter_map(|b| b.header.strip_prefix("interface "))
            .filter_map(|n| {
                n.strip_prefix(prefix)
                    .and_then(|x| x.parse().ok())
                    .map(|x| (x, n.to_string()))
            })
            .collect();
        v.sort();
        v.into_iter().map(|(_, n)| n).collect::<Vec<_>>()
    };
    let defined = defined_vlans(d);
    let mut vlans = named("Vlan");
    if !vlans.iter().any(|v| v == "Vlan1") {
        vlans.insert(0, "Vlan1".into());
    }
    let names = vlans
        .into_iter()
        .chain(d.stack.physical())
        .chain(named("Port-channel"))
        .chain(named("Loopback"))
        .chain(named("Tunnel"));
    for n in names {
        let header = format!("interface {n}");
        let block = d.blocks.iter().find(|b| b.header == header);
        let c = config_of(d, &n);
        let ip = value(&c, "ip address ")
            .and_then(|v| v.split_whitespace().next())
            .filter(|v| v.contains('.'))
            .unwrap_or("unassigned");
        // NVRAM where the configuration gives an address or none; a switch port is unset however
        // configured (both C9200Ls), and so is a loopback with no address (c9200 17.15 capture 27 and the six
        // IE-3300s, each Loopback0 a tool wrote)
        let loopback_bare =
            n.starts_with("Loopback") && !c.iter().any(|l| l.starts_with("ip address "));
        // a front-panel port's `no ip address` is unset too (the lab C9300's routed Gi1/0/6, 17.6.4);
        // the management port's and a VLAN interface's read NVRAM
        let port_bare = d.stack.physical().contains(&n)
            && Some(n.as_str()) != d.stack.management()
            && !c.iter().any(|l| l.starts_with("ip address "));
        // an address typed at the console reads `manual`, DHCP's too while no server answers (the
        // lab C9300, 17.6.4); one in a configuration loaded, `DHCP`
        let method = if d.manual.contains(&header) && c.iter().any(|l| l.starts_with("ip address "))
        {
            "manual"
        } else if c.iter().any(|l| l.starts_with("ip address dhcp")) {
            "DHCP"
        } else if !loopback_bare
            && !port_bare
            && c.iter()
                .any(|l| l.starts_with("ip address ") || *l == "no ip address")
        {
            "NVRAM"
        } else {
            "unset"
        };
        // a VLAN interface is up while its VLAN exists and a running switch port carries it
        // (autostate, 9300 configuration guide)
        let vlan_up = n
            .strip_prefix("Vlan")
            .and_then(|v| v.parse::<u32>().ok())
            .is_some_and(|id| {
                (id == 1 || defined.contains(&id))
                    && d.links
                        .keys()
                        .filter_map(|p| up(d, p).map(|l| (p, l)))
                        .any(|(p, l)| carries(d, p, l, id))
            });
        let running = n.starts_with("Loopback")
            || n.starts_with("AppGigabitEthernet")
            || up(d, &n).is_some()
            || bundle(d, &n).is_some()
            || vlan_up;
        // Vlan1 is shut down until configured (the cores' running configurations)
        let shut = c.contains(&"shutdown") || (n == "Vlan1" && block.is_none());
        // a VLAN interface whose VLAN exists but no running port carries is up, its protocol down
        // (the C2960G's Vlan1)
        let vlan_exists = n
            .strip_prefix("Vlan")
            .and_then(|v| v.parse::<u32>().ok())
            .is_some_and(|id| id == 1 || defined.contains(&id));
        let (status, protocol) = if shut {
            ("administratively down", "down")
        } else if running {
            ("up", "up")
        } else if vlan_exists {
            ("up", "down")
        } else {
            ("down", "down")
        };
        out.push(row(&n, ip, "YES", method, status, protocol));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli, device, grammars, platform, release::Release, shell};

    fn engine(book: &str, release: &str) -> cli::Engine {
        cli::Engine::new(grammars::grammar(book, Release::parse(release).unwrap()).unwrap())
    }

    /// A switch with `lines` typed in configuration mode, back at the privileged prompt.
    fn typed<'e>(e: &'e cli::Engine, model: &str, lines: &[&str]) -> shell::Session<'e> {
        let stack = platform::parse_stack(model).unwrap();
        let mut s = shell::Session::new(e, device::Device::new(stack));
        for l in ["enable", "configure terminal"]
            .iter()
            .chain(lines)
            .chain(&["end"])
        {
            let (o, r) = s.type_line(l);
            assert_eq!(r, shell::Result::Ok, "{l}: {o:?}");
        }
        s
    }

    /// The lines of the running configuration's block `header`, the header first.
    fn block_of(run: &[String], header: &str) -> Vec<String> {
        let at = run.iter().position(|l| l == header).unwrap();
        run[at..]
            .iter()
            .enumerate()
            .take_while(|(i, l)| *i == 0 || l.starts_with(' '))
            .map(|(_, l)| l.clone())
            .collect()
    }

    /// The row of a table that starts with `first` and a space.
    fn row(table: &[String], first: &str) -> String {
        table
            .iter()
            .find(|r| r.starts_with(&format!("{first} ")))
            .cloned()
            .unwrap_or_else(|| panic!("{first}: {table:?}"))
    }

    /// The line starting `field` in the `show interfaces switchport` block of `port`.
    fn switchport_field(out: &[String], port: &str, field: &str) -> String {
        let at = out
            .iter()
            .position(|l| *l == format!("Name: {port}"))
            .unwrap();
        out[at..]
            .iter()
            .find(|l| l.starts_with(field))
            .cloned()
            .unwrap()
    }

    #[test]
    fn a_typed_version_line_prints_once_as_the_release_the_switch_runs() {
        let e = engine("c9300", "17.15");
        let mut s = typed(&e, "C9300-24T", &["version 17.15"]);
        let versions = |run: &[String]| -> Vec<String> {
            run.iter()
                .filter(|l| l.starts_with("version "))
                .cloned()
                .collect()
        };
        assert_eq!(versions(&configuration(&s.device)), ["version 17.15"]);
        assert_eq!(
            versions(&s.type_line("show running-config").0),
            ["version 17.15"]
        );
    }

    #[test]
    fn a_vlan_access_map_prints_whatever_the_vtp_mode_and_an_empty_vlan_configuration_never() {
        let e = engine("c9300", "17.15");
        let s = typed(
            &e,
            "C9300-24P",
            &[
                "vlan access-map DROPX 10",
                "action drop",
                "exit",
                "vlan filter DROPX vlan-list 10",
                "vlan configuration 10",
                "exit",
                "vlan 20",
                "exit",
            ],
        );
        // a VTP server, the default: VLAN 20 is in vlan.dat, the access map in the running
        // configuration; a VLAN configuration with nothing under it is not kept (the lab C9300)
        let run = configuration(&s.device);
        assert_eq!(
            block_of(&run, "vlan access-map DROPX 10"),
            ["vlan access-map DROPX 10", " action drop"]
        );
        assert!(
            !run.iter().any(|l| l.starts_with("vlan configuration")),
            "{run:?}"
        );
        assert!(!run.contains(&"vlan 20".to_string()), "{run:?}");
        assert_eq!(vlan_database(&s.device), ["vlan 20"]);
        assert!(!has_place("vlan access-map DROPX 10"));
        assert!(has_place("vlan 11,65-66"));
    }

    #[test]
    fn show_ip_interface_brief_prints_a_name_wider_than_its_column_short() {
        let e = engine("c9300", "17.15");
        let s = typed(&e, "C9300-24P", &[]);
        let brief = ip_interface_brief(&s.device);
        let tail = |r: &str| r[r.find("unassigned").unwrap()..].to_string();
        let gi = row(&brief, "GigabitEthernet1/0/1");
        for (long, short) in [
            ("TenGigabitEthernet1/1/1", "Te1/1/1"),
            ("FortyGigabitEthernet1/1/1", "Fo1/1/1"),
            ("AppGigabitEthernet1/0/1", "Ap1/0/1"),
        ] {
            assert!(!brief.iter().any(|r| r.starts_with(long)), "{long}");
            let r = row(&brief, short);
            assert_eq!(r.find("unassigned"), gi.find("unassigned"), "{r}");
            if !long.starts_with("App") {
                assert_eq!(tail(&r), tail(&gi));
            }
        }
        // the captures' columns: a name of 22 or fewer, then a space
        assert_eq!(
            row(&brief, "Vlan1"),
            "Vlan1                  unassigned      YES unset  administratively down down    "
        );
    }

    #[test]
    fn lines_typed_out_of_order_print_in_the_order_ios_prints_them() {
        let e = engine("c9300", "17.15");
        let mut s = typed(
            &e,
            "C9300-24T",
            &[
                "spanning-tree mode rapid-pvst",
                "hostname SW1",
                "service password-encryption",
                "interface GigabitEthernet1/0/2",
                "switchport mode access",
                "switchport access vlan 10",
                "description uplink",
                "interface range GigabitEthernet1/0/3 - 4",
                "channel-group 1 mode active",
                "interface Port-channel1",
                "switchport mode trunk",
                "exit",
                "logging host 10.0.0.9",
                "ip default-gateway 10.0.0.1",
                "snmp-server contact lab",
                "snmp-server community INVENTEDro RO",
                "policy-map PM1",
                "exit",
                "class-map match-any CM1",
                "exit",
            ],
        );
        let run = configuration(&s.device);
        let at = |l: &str| run.iter().position(|x| x == l).unwrap();
        assert!(at("service password-encryption") < at("hostname SW1"));
        assert!(at("hostname SW1") < at("spanning-tree mode rapid-pvst"));
        // the lines after the interfaces too, and every class map before every policy map
        assert!(at("ip default-gateway 10.0.0.1") < at("logging host 10.0.0.9"));
        assert!(at("snmp-server community INVENTEDro RO") < at("snmp-server contact lab"));
        assert!(at("class-map match-any CM1") < at("policy-map PM1"));
        let port = [
            "interface GigabitEthernet1/0/2",
            " description uplink",
            " switchport access vlan 10",
            " switchport mode access",
        ];
        assert_eq!(block_of(&run, port[0]), port);
        // a port-channel's line reaches its members above their `channel-group`
        assert_eq!(
            block_of(&run, "interface GigabitEthernet1/0/3"),
            [
                "interface GigabitEthernet1/0/3",
                " switchport mode trunk",
                " channel-group 1 mode active"
            ]
        );
        // the block alone prints the same
        let one = s
            .type_line("show running-config interface GigabitEthernet1/0/2")
            .0;
        assert_eq!(one[one.len() - 6..one.len() - 2], port, "{one:?}");
        assert_eq!(one[one.len() - 2..], ["end", ""]);
    }

    #[test]
    fn a_routing_process_prints_after_the_interfaces() {
        let e = engine("c9300", "17.15");
        let s = typed(
            &e,
            "C9300-24T",
            &[
                "hostname R",
                "ip routing",
                "router ospf 1",
                "router-id 1.1.1.1",
                "network 10.0.0.0 0.0.0.255 area 0",
                "exit",
                "interface Vlan10",
                "ip address 10.0.0.1 255.255.255.0",
                "exit",
                "ip route 0.0.0.0 0.0.0.0 10.0.0.254",
            ],
        );
        let run = configuration(&s.device);
        let at = |l: &str| run.iter().position(|x| x == l).unwrap();
        assert!(at("interface Vlan10") < at("router ospf 1"), "{run:?}");
        assert!(at("router ospf 1") < at("ip route 0.0.0.0 0.0.0.0 10.0.0.254"));
        assert_eq!(
            block_of(&run, "router ospf 1"),
            [
                "router ospf 1",
                " router-id 1.1.1.1",
                " network 10.0.0.0 0.0.0.255 area 0"
            ]
        );
    }

    #[test]
    fn class_and_policy_maps_no_capture_holds_print_after_the_vlan_blocks() {
        // the C9200Ls' order, whatever the order typed: the VLAN blocks, the `cdp` lines, every
        // class map, then every policy map; these maps no capture holds, so their place orders them
        let e = engine("c9300", "17.15");
        for map in ["class-map match-any CM-UNHELD", "policy-map PM-UNHELD"] {
            assert!(!e.held.has_exact("", map), "{map}");
        }
        let s = typed(
            &e,
            "C9300-24T",
            &[
                "vtp mode transparent",
                "policy-map PM-UNHELD",
                "exit",
                "class-map match-any CM-UNHELD",
                "exit",
                "vlan 10",
                "exit",
                "no cdp run",
            ],
        );
        let run = configuration(&s.device);
        let at = |l: &str| {
            run.iter()
                .position(|x| x == l)
                .unwrap_or_else(|| panic!("{l}: {run:?}"))
        };
        assert!(at("vlan 10 ") < at("no cdp run"), "{run:?}");
        assert!(
            at("no cdp run") < at("class-map match-any CM-UNHELD"),
            "{run:?}"
        );
        assert!(
            at("class-map match-any CM-UNHELD") < at("policy-map PM-UNHELD"),
            "{run:?}"
        );
    }

    #[test]
    fn line_blocks_print_con_then_each_vty_range_in_order() {
        let e = engine("c9300", "17.15");
        let s = typed(
            &e,
            "C9300-48P",
            &[
                "line vty 5 15",
                "login local",
                "line vty 0 4",
                "transport input ssh",
                "login local",
                "exec-timeout 10 0",
                "line con 0",
                "exec-timeout 5 0",
            ],
        );
        let run = configuration(&s.device);
        let headers: Vec<&String> = run.iter().filter(|l| l.starts_with("line ")).collect();
        assert_eq!(headers, ["line con 0", "line vty 0 4", "line vty 5 15"]);
        assert_eq!(
            block_of(&run, "line vty 0 4"),
            [
                "line vty 0 4",
                " exec-timeout 10 0",
                " login local",
                " transport input ssh"
            ]
        );
    }

    #[test]
    fn show_interfaces_switchport_reads_a_voice_keyword_and_a_pruning_list_as_ios_does() {
        let e = engine("c9300", "17.15");
        let s = typed(
            &e,
            "C9300-48P",
            &[
                "interface GigabitEthernet1/0/1",
                "switchport mode trunk",
                "switchport voice vlan dot1p",
                "switchport trunk pruning vlan none",
                "interface GigabitEthernet1/0/2",
                "switchport voice vlan untagged",
                "switchport trunk pruning vlan except 2-100",
                "interface GigabitEthernet1/0/3",
                "switchport trunk pruning vlan 10,20-30",
                "interface GigabitEthernet1/0/5",
                "switchport trunk pruning vlan 10-20",
                "switchport trunk pruning vlan remove 15",
            ],
        );
        let out = interfaces_switchport(&s.device);
        let field = |port: &str, f: &str| switchport_field(&out, port, f);
        assert_eq!(field("Gi1/0/1", "Voice VLAN: "), "Voice VLAN: dot1p");
        assert_eq!(
            field("Gi1/0/1", "Pruning VLANs Enabled: "),
            "Pruning VLANs Enabled: NONE"
        );
        assert_eq!(field("Gi1/0/2", "Voice VLAN: "), "Voice VLAN: untagged");
        assert_eq!(
            field("Gi1/0/2", "Pruning VLANs Enabled: "),
            "Pruning VLANs Enabled: 101-1001"
        );
        assert_eq!(
            field("Gi1/0/3", "Pruning VLANs Enabled: "),
            "Pruning VLANs Enabled: 10,20-30"
        );
        assert_eq!(field("Gi1/0/4", "Voice VLAN: "), "Voice VLAN: none");
        assert_eq!(
            field("Gi1/0/4", "Pruning VLANs Enabled: "),
            "Pruning VLANs Enabled: 2-1001"
        );
        // a `remove` after a list takes from that list
        assert_eq!(
            field("Gi1/0/5", "Pruning VLANs Enabled: "),
            "Pruning VLANs Enabled: 10-14,16-20"
        );
    }

    #[test]
    fn tunnel_private_vlan_and_tagged_native_ports_read_as_ios_prints_them() {
        let e = engine("c9300", "17.15");
        let mut s = typed(
            &e,
            "C9300-48P",
            &[
                "interface GigabitEthernet1/0/1",
                "switchport mode dot1q-tunnel",
                "interface GigabitEthernet1/0/2",
                "switchport mode private-vlan host",
                "interface GigabitEthernet1/0/3",
                "switchport mode trunk",
                "switchport trunk native vlan tag",
                "switchport trunk native vlan 99",
            ],
        );
        let out = interfaces_switchport(&s.device);
        let field = |port: &str, f: &str| switchport_field(&out, port, f);
        assert_eq!(
            field("Gi1/0/1", "Administrative Mode: "),
            "Administrative Mode: tunnel"
        );
        assert_eq!(
            field("Gi1/0/1", "Negotiation of Trunking: "),
            "Negotiation of Trunking: Off"
        );
        assert_eq!(
            field("Gi1/0/2", "Administrative Mode: "),
            "Administrative Mode: private-vlan host"
        );
        assert_eq!(
            field("Gi1/0/3", "Trunking Native Mode VLAN: "),
            "Trunking Native Mode VLAN: 99 (Inactive)"
        );
        // `native vlan tag` names no VLAN in the trunk table either
        s.device.links.insert(
            "GigabitEthernet1/0/3".into(),
            device::Link {
                peer: "far".into(),
                optic: None,
                speed: None,
                half: false,
                trunk: true,
                bundled: false,
            },
        );
        assert_eq!(
            row(&interfaces_trunk(&s.device), "Gi1/0/3")
                .split_whitespace()
                .last(),
            Some("99")
        );
    }

    #[test]
    fn a_vlan_range_kept_as_a_global_line_is_defined() {
        // ie3000 15.2(6)E capture 1, an IE 3000 at 15.2(6)E, keeps `vlan 65-66` at the top level
        let mut d = device::Device::new(platform::parse_stack("C9300-48P").unwrap());
        d.global.push(device::Entry {
            key: "vlan 65-66".into(),
            text: "vlan 65-66".into(),
        });
        let brief = vlan_brief(&d, 80);
        assert!(row(&brief, "65").contains("VLAN0065"), "{brief:?}");
        assert!(row(&brief, "66").contains("VLAN0066"), "{brief:?}");
        assert!(defined_vlans(&d).contains(&66));
    }

    #[test]
    fn a_dual_purpose_uplink_linked_by_copper_shows_its_rj_45_type() {
        let mut d = device::Device::new(platform::parse_stack("WS-C2960-24TC-L").unwrap());
        d.release = "15.2(6)E".into();
        let port = d
            .stack
            .physical()
            .into_iter()
            .find(|p| d.stack.dual_purpose(p))
            .expect("a dual-purpose uplink");
        d.links.insert(
            port.clone(),
            device::Link {
                peer: "far".into(),
                optic: None,
                speed: None,
                half: false,
                trunk: false,
                bundled: false,
            },
        );
        let status = interfaces_status(&d);
        assert!(
            row(&status, &short(&port)).ends_with("10/100/1000BaseTX"),
            "{status:?}"
        );
    }

    #[test]
    fn show_vtp_status_has_no_layout_under_vtp_version_3() {
        let e = engine("c9300", "17.15");
        let s = typed(&e, "C9300-24T", &["vtp domain LAB", "vtp version 3"]);
        assert_eq!(
            vtp_status(&s.device),
            ["% vswitch has no output model for 'show vtp status' under VTP version 3 yet"]
        );
        let s = typed(&e, "C9300-24T", &["vtp domain LAB", "vtp version 2"]);
        assert_eq!(
            vtp_status(&s.device)[1],
            "VTP version running             : 2"
        );
    }

    #[test]
    fn an_address_typed_reads_method_manual_and_one_loaded_nvram_or_dhcp() {
        let e = engine("c9300", "17.15");
        let lines = [
            "interface Vlan10",
            "ip address dhcp",
            "interface Vlan20",
            "ip address 10.1.1.1 255.255.255.0",
        ];
        // typed at the console (the lab C9300, 17.6.4: Vlan10 `manual` with no lease, Lo0 `manual`)
        let s = typed(&e, "C9300-24T", &lines);
        let brief = ip_interface_brief(&s.device);
        assert!(
            row(&brief, "Vlan10").starts_with("Vlan10                 unassigned      YES manual "),
            "{brief:?}"
        );
        assert!(
            row(&brief, "Vlan20").starts_with("Vlan20                 10.1.1.1        YES manual ")
        );
        // a configuration loaded
        let (loaded, _) = crate::check::type_config(
            &e,
            crate::platform::parse_stack("C9300-24T").unwrap(),
            &format!("{}\n", lines.join("\n")),
        );
        let brief = ip_interface_brief(&loaded.device);
        assert!(
            row(&brief, "Vlan10").starts_with("Vlan10                 unassigned      YES DHCP   "),
            "{brief:?}"
        );
        assert!(
            row(&brief, "Vlan20").starts_with("Vlan20                 10.1.1.1        YES NVRAM  ")
        );
    }

    #[test]
    fn an_access_port_whose_vlan_is_deleted_is_never_inactive() {
        let e = engine("c9300", "17.15");
        let s = typed(
            &e,
            "C9300-24T",
            &[
                "vtp mode transparent",
                "vlan 50",
                "exit",
                "interface GigabitEthernet1/0/1",
                "switchport mode access",
                "switchport access vlan 50",
                "exit",
                "no vlan 50",
            ],
        );
        // the lab C9300 (17.6.4) and the lab C2960G: `notconnect 50`, `connected 50` when linked
        let status = interfaces_status(&s.device);
        let words: Vec<String> = row(&status, "Gi1/0/1")
            .split_whitespace()
            .map(str::to_string)
            .collect();
        assert_eq!(words[1..3], ["notconnect", "50"]);
    }

    #[cfg(all(test, captures))]
    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/show-tests.rs"));

    /// A link to a host on `port`, bundled or not.
    fn host_link(bundled: bool) -> device::Link {
        device::Link {
            peer: "host".into(),
            optic: None,
            speed: None,
            half: false,
            trunk: false,
            bundled,
        }
    }

    #[test]
    fn ios_xe_suspends_an_lacp_member_it_cannot_bundle_and_its_port_channel_has_no_vlan() {
        let e = engine("c9300", "17.15");
        let mut s = typed(
            &e,
            "C9300-24T",
            &[
                "interface GigabitEthernet1/0/1",
                "channel-group 1 mode active",
                "interface GigabitEthernet1/0/2",
                "channel-group 2 mode active",
            ],
        );
        s.device
            .links
            .insert("GigabitEthernet1/0/1".into(), host_link(false));
        // the lab C9300 (17.6.4): `suspended 1 auto a-1000`, `(s)`, the port-channel `unassigned`
        let status = interfaces_status(&s.device);
        let words = |p: &str| {
            row(&status, p)
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(words("Gi1/0/1")[1..5], ["suspended", "1", "auto", "a-1000"]);
        assert_eq!(words("Gi1/0/2")[1..3], ["notconnect", "1"]);
        assert_eq!(words("Po1")[1..3], ["notconnect", "unassigned"]);
        let summary = etherchannel_summary(&s.device);
        assert!(
            summary.contains(&"1      Po1(SD)         LACP        Gi1/0/1(s)      ".to_string()),
            "{summary:?}"
        );
        let out = interfaces_switchport(&s.device);
        let field = |p: &str, f: &str| switchport_field(&out, p, f);
        assert_eq!(
            field("Po1", "Access Mode VLAN: "),
            "Access Mode VLAN: unassigned"
        );
        assert_eq!(
            field("Po1", "Trunking Native Mode VLAN: "),
            "Trunking Native Mode VLAN: 0 (Inactive)"
        );
        // unlinked too
        assert_eq!(
            field("Gi1/0/2", "Operational Mode: "),
            "Operational Mode: down (suspended member of bundle Po2)"
        );
        // a port-channel has no capture lines; a port has
        let at = out.iter().position(|l| l == "Name: Po1").unwrap();
        let po: Vec<&String> = out[at..].iter().take_while(|l| !l.is_empty()).collect();
        assert!(!po.iter().any(|l| l.starts_with("Capture")), "{po:?}");
        assert!(out.contains(&"Capture Mode Disabled".to_string()));
        // `no port-channel standalone-disable` prints, and lets the member stand alone
        let mut s = s;
        for l in [
            "configure terminal",
            "interface Port-channel1",
            "no port-channel standalone-disable",
            "end",
        ] {
            s.type_line(l);
        }
        assert_eq!(
            block_of(&configuration(&s.device), "interface Port-channel1"),
            [
                "interface Port-channel1",
                " no port-channel standalone-disable"
            ]
        );
        let status = interfaces_status(&s.device);
        assert_eq!(
            row(&status, "Gi1/0/1").split_whitespace().nth(1),
            Some("connected")
        );
        assert!(
            etherchannel_summary(&s.device)
                .contains(&"1      Po1(SD)         LACP        Gi1/0/1(I)      ".to_string())
        );
    }

    #[test]
    fn an_etherchannel_row_on_ios_xe_pads_each_member_and_wraps_two_a_line() {
        let e = engine("c9300", "17.15");
        let s = typed(
            &e,
            "C9300-24T",
            &[
                "interface range GigabitEthernet1/0/1 - 3",
                "channel-group 4 mode on",
                "interface Port-channel9",
            ],
        );
        let summary = etherchannel_summary(&s.device);
        let at = summary.iter().position(|l| l.starts_with("4 ")).unwrap();
        // the lab C9300 (17.6.4) at widths 80, 100 and 512
        assert_eq!(
            summary[at..at + 2],
            [
                "4      Po4(SD)          -          Gi1/0/1(D)      Gi1/0/2(D)      ",
                "                                   Gi1/0/3(D)      "
            ]
        );
        assert!(
            summary.contains(&"9      Po9(SD)          -          ".to_string()),
            "{summary:?}"
        );
    }

    #[test]
    fn a_classic_group_with_no_members_keeps_its_padding_and_a_down_port_channel_its_vlan_from_15_2()
     {
        let mut d = device::Device::new(platform::parse_stack("WS-C2960G-8TC-L").unwrap());
        d.blocks.push(device::Block {
            header: "interface Port-channel1".into(),
            lines: vec![],
        });
        // the lab C2960G (15.0(2)SE11)
        d.release = "15.0(2)SE".into();
        assert!(
            etherchannel_summary(&d).contains(&"1      Po1(SD)          -        ".to_string()),
            "{:?}",
            etherchannel_summary(&d)
        );
        let status = interfaces_status(&d);
        assert_eq!(
            row(&status, "Po1"),
            "Po1                          notconnect   unassigned   auto   auto "
        );
        // a 2960-X capture at 15.2: the access VLAN
        d.release = "15.2(7)E".into();
        assert_eq!(
            row(&interfaces_status(&d), "Po1").split_whitespace().nth(2),
            Some("1")
        );
    }

    #[test]
    fn an_application_hosting_port_runs_as_an_access_port() {
        let e = engine("c9300", "17.15");
        let s = typed(&e, "C9300-24T", &[]);
        // the lab C9300 (17.6.4)
        assert!(
            row(&interfaces_status(&s.device), "Ap1/0/1")
                .ends_with("a-full a-1000 App-hosting port")
        );
        let out = interfaces_switchport(&s.device);
        let field = |f: &str| switchport_field(&out, "Ap1/0/1", f);
        assert_eq!(
            field("Operational Mode: "),
            "Operational Mode: static access"
        );
        assert_eq!(
            field("Operational Trunking Encapsulation: "),
            "Operational Trunking Encapsulation: native"
        );
    }

    #[test]
    fn the_vtp_updater_is_the_first_vlan_interface_with_an_address_else_the_first_layer_3_one() {
        let e = engine("c9300", "17.15");
        let s = typed(
            &e,
            "C9300-24T",
            &[
                "interface Vlan10",
                "ip address dhcp",
                "interface Loopback0",
                "ip address 192.0.2.1 255.255.255.255",
            ],
        );
        // the lab C9300 (17.6.4): Vlan10 waits on DHCP, so Lo0
        assert!(
            vtp_status(&s.device).contains(
                &"Local updater ID is 192.0.2.1 on interface Lo0 (first layer3 interface found)"
                    .to_string()
            )
        );
        let s = typed(&e, "C9300-24T", &["vtp mode off"]);
        assert!(
            !vtp_status(&s.device)
                .iter()
                .any(|l| l.starts_with("Local updater"))
        );
    }

    #[test]
    fn one_port_s_switchport_block_prints_alone() {
        let e = engine("c9300", "17.15");
        let mut s = typed(
            &e,
            "C9300-24T",
            &[
                "interface GigabitEthernet1/0/1",
                "channel-group 1 mode active",
            ],
        );
        s.device
            .links
            .insert("GigabitEthernet1/0/1".into(), host_link(false));
        // the lab C9300 (17.6.4), Gi1/0/1 cabled to a port that runs no LACP
        assert_eq!(
            s.type_line("show interfaces Gi1/0/1 switchport").0,
            [
                "Name: Gi1/0/1",
                "Switchport: Enabled",
                "Administrative Mode: dynamic auto",
                "Operational Mode: down (suspended member of bundle Po1)",
                "Administrative Trunking Encapsulation: dot1q",
                "Negotiation of Trunking: On",
                "Access Mode VLAN: 1 (default)",
                "Trunking Native Mode VLAN: 1 (default)",
                "Administrative Native VLAN tagging: enabled",
                "Voice VLAN: none",
                "Administrative private-vlan host-association: none ",
                "Administrative private-vlan mapping: none ",
                "Administrative private-vlan trunk native VLAN: none",
                "Administrative private-vlan trunk Native VLAN tagging: enabled",
                "Administrative private-vlan trunk encapsulation: dot1q",
                "Administrative private-vlan trunk normal VLANs: none",
                "Administrative private-vlan trunk associations: none",
                "Administrative private-vlan trunk mappings: none",
                "Operational private-vlan: none",
                "Trunking VLANs Enabled: ALL",
                "Pruning VLANs Enabled: 2-1001",
                "Capture Mode Disabled",
                "Capture VLANs Allowed: ALL",
                "",
                "Protected: false",
                "Unknown unicast blocked: disabled",
                "Unknown multicast blocked: disabled",
                "Vepa Enabled: false",
                "App Interface: false",
                "Appliance trust: none",
            ]
        );
    }

    #[test]
    fn ios_xe_pads_the_trunk_table_header() {
        let e = engine("c9300", "17.15");
        let mut s = typed(
            &e,
            "C9300-24T",
            &["interface GigabitEthernet1/0/1", "switchport mode trunk"],
        );
        let mut link = host_link(false);
        link.trunk = true;
        s.device.links.insert("GigabitEthernet1/0/1".into(), link);
        // the lab C9300 (17.6.4)
        let trunk = interfaces_trunk(&s.device);
        assert!(trunk[1].ends_with("Native vlan  "), "{trunk:?}");
    }

    #[test]
    fn a_bundled_member_s_operational_mode_names_its_port_channel() {
        let e = engine("c9300", "17.15");
        let mut s = typed(
            &e,
            "C9300-24T",
            &[
                "interface range GigabitEthernet1/0/1 - 2",
                "channel-group 1 mode active",
                "interface Port-channel1",
                "switchport mode trunk",
            ],
        );
        let link = |bundled| device::Link {
            peer: "host".into(),
            optic: None,
            speed: None,
            half: false,
            trunk: true,
            bundled,
        };
        s.device
            .links
            .insert("GigabitEthernet1/0/1".into(), link(true));
        s.device
            .links
            .insert("GigabitEthernet1/0/2".into(), link(false));
        let out = interfaces_switchport(&s.device);
        let mode = |port: &str| switchport_field(&out, port, "Operational Mode: ");
        assert_eq!(
            mode("Gi1/0/1"),
            "Operational Mode: trunk (member of bundle Po1)"
        );
        // linked, not bundled: suspended, with no operational encapsulation (the lab C9300, 17.6.4)
        assert_eq!(
            mode("Gi1/0/2"),
            "Operational Mode: down (suspended member of bundle Po1)"
        );
        let at = out.iter().position(|l| l == "Name: Gi1/0/2").unwrap();
        assert_eq!(
            out[at + 4],
            "Administrative Trunking Encapsulation: dot1q",
            "{out:?}"
        );
        assert_eq!(out[at + 5], "Negotiation of Trunking: On", "{out:?}");
        assert_eq!(mode("Po1"), "Operational Mode: trunk");
        // standing alone under `no port-channel standalone-disable`
        let mut s = s;
        s.type_line("configure terminal");
        s.type_line("interface Port-channel1");
        s.type_line("no port-channel standalone-disable");
        s.type_line("end");
        let out = interfaces_switchport(&s.device);
        assert_eq!(
            switchport_field(&out, "Gi1/0/2", "Operational Mode: "),
            "Operational Mode: trunk"
        );
    }
}
