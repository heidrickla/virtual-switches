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
/// capture printed the pair. A banner's text and indented lines stay as they are. Where the switch's
/// own configuration was replayed (`Layout`), two of its lines that meet again keep the `!` and
/// comment lines it printed between them, and lines added between two of them take the computed
/// lines, the rest of its own going before the first added line (the lab C9300, 17.6.4: `vtp
/// domain` typed before a replayed `vtp mode off`, a loopback before the first port).
fn separate(d: &Device, body: Vec<String>) -> Vec<String> {
    let o = orders();
    let book = crate::grammars::book(d.stack.family());
    let classic = crate::release::Release::parse(&d.release)
        .map_or_else(|| d.stack.classic(), |r| r.classic());
    let lab = lab_gap(d);
    let gap = |a: usize, b: usize| {
        o.release_gaps
            .get(&(book.to_string(), d.release.clone(), a, b))
            .or_else(|| o.family_gaps[usize::from(!classic)].get(&(a, b)))
            .copied()
    };
    // each top-level line with the `!` lines computed before it and the lines under it
    struct Unit {
        line: String,
        bangs: usize,
        under: Vec<String>,
    }
    let mut head: Vec<String> = vec![];
    let mut units: Vec<Unit> = vec![];
    let mut before: Option<Option<usize>> = None;
    let mut prev_text = String::new();
    let mut bangs = 0;
    let mut closes: Option<String> = None;
    for l in body {
        if let Some(d) = &closes {
            if l.contains(d.as_str()) {
                closes = None;
            }
            if let Some(u) = units.last_mut() {
                u.under.push(l);
            }
            continue;
        }
        if before.is_none() {
            before = l.starts_with("version ").then(|| o.number(&command(&l)));
            if before.is_some() {
                prev_text = l.clone();
            }
            head.push(l);
            continue;
        }
        if l == "!" {
            bangs += 1;
            continue;
        }
        if l.is_empty() || l.starts_with([' ', '\t']) {
            let u = units.last_mut();
            match u {
                Some(u) => {
                    u.under.extend(std::iter::repeat_n("!".to_string(), bangs));
                    u.under.push(l);
                }
                None => {
                    head.extend(std::iter::repeat_n("!".to_string(), bangs));
                    head.push(l);
                }
            }
            bangs = 0;
            continue;
        }
        let this = o.number(&command(&l));
        let n = lab(&prev_text, &l).unwrap_or_else(|| match (before.flatten(), this) {
            (Some(a), Some(b)) => gap(a, b).unwrap_or(bangs),
            _ => bangs,
        });
        bangs = 0;
        before = Some(this);
        closes = crate::live::banner_opens(&l);
        prev_text = l.clone();
        units.push(Unit {
            line: l,
            bangs: n,
            under: vec![],
        });
    }
    let tail = bangs;
    let bang = |n: usize| std::iter::repeat_n("!".to_string(), n);
    let Some(layout) = &d.layout else {
        let mut out = head;
        for u in units {
            out.extend(bang(u.bangs));
            out.push(u.line);
            out.extend(u.under);
        }
        out.extend(bang(tail));
        return out;
    };
    // a replayed configuration's own lines print in its order, a line added since after the line
    // computed before it (the lab C9300-24T, 17.6.4: 48 of 251 top-level lines placed otherwise)
    let key = |l: &str| crate::device::layout_key(l.trim_end());
    let mut groups: Vec<(Option<usize>, Vec<Unit>)> = vec![];
    for u in units {
        match layout.order.get(&key(&u.line)) {
            Some(&at) => groups.push((Some(at), vec![u])),
            None => match groups.last_mut() {
                Some(g) => g.1.push(u),
                None => groups.push((None, vec![u])),
            },
        }
    }
    groups.sort_by_key(|g| g.0);
    // and after every replayed line whose command IOS prints before its own (the lab
    // WS-C2960G-8TC-L, 15.0(2)SE11: a new `spanning-tree mst configuration` after `spanning-tree
    // extend system-id`)
    let top = top_order(d);
    let precedes = |a: &str, b: &str| match (o.number(&command(a)), o.number(&command(b))) {
        (Some(x), Some(y)) => x != y && top.iter().any(|p| p.before(x, y)),
        _ => false,
    };
    for i in 0..groups.len() {
        let at = groups[i].0;
        let mut j = usize::from(at.is_some());
        while j < groups[i].1.len() {
            let line = &groups[i].1[j].line;
            let later = (i + 1..groups.len())
                .rev()
                .find(|&k| precedes(&groups[k].1[0].line, line));
            match later {
                Some(k) => {
                    let u = groups[i].1.remove(j);
                    groups[k].1.push(u);
                }
                None => j += 1,
            }
        }
    }
    let units: Vec<Unit> = groups.into_iter().flat_map(|g| g.1).collect();
    let version = head
        .iter()
        .find(|l| l.starts_with("version "))
        .cloned()
        .unwrap_or_default();
    let tops: Vec<String> = units.iter().map(|u| key(&u.line)).collect();
    let prev_of = |i: usize| {
        if i == 0 {
            key(&version)
        } else {
            tops[i - 1].clone()
        }
    };
    // the separator lines before each unit
    let mut seps: Vec<Vec<String>> = units.iter().map(|u| bang(u.bangs).collect()).collect();
    let mut i = 0;
    while i < units.len() {
        if let Some(own) = layout.pairs.get(&(prev_of(i), tops[i].clone())) {
            seps[i] = own.clone();
            i += 1;
            continue;
        }
        // a run of added lines between two of the replayed configuration's own: the first takes
        // the replayed `!` lines (all of them less those computed after it, where those are fewer),
        // the rest their computed ones (the lab C9300, 17.6.4)
        let start = i;
        let mut end = i;
        while end < units.len() && !layout.tops.contains(&tops[end]) {
            end += 1;
        }
        let own = (end < units.len())
            .then(|| layout.pairs.get(&(prev_of(start), tops[end].clone())))
            .flatten();
        if let Some(own) = own.filter(|_| end > start) {
            let rest: usize = (start + 1..=end).map(|j| seps[j].len()).sum();
            let first = if rest + seps[start].len() < own.len() {
                own.len() - rest
            } else {
                seps[start].len().min(own.len())
            };
            seps[start] = own[..first].to_vec();
            i = end + 1;
        } else {
            i = start.max(end) + usize::from(end == start);
        }
    }
    let mut out: Vec<String> = head;
    // each line as the switch printed it: the form with the same words, else the only form
    fn words(s: &str) -> Vec<&str> {
        s.split_whitespace().collect()
    }
    let trailing = |l: String| {
        let Some(forms) = layout.raw.get(&key(&l)) else {
            return l;
        };
        forms
            .iter()
            .find(|f| words(f) == words(&l))
            .or_else(|| (forms.len() == 1).then(|| &forms[0]))
            .cloned()
            .unwrap_or(l)
    };
    for (u, s) in units.into_iter().zip(seps) {
        out.extend(s);
        let header = key(&u.line);
        out.push(trailing(u.line));
        for l in u.under {
            if let Some(inner) = layout.inner.get(&(header.clone(), key(&l)))
                && !out.ends_with(inner)
            {
                out.extend(inner.iter().cloned());
            }
            out.push(trailing(l));
        }
    }
    out.extend(bang(tail));
    out
}

/// The `!` lines the lab C9300 (17.6.4) printed between two top-level lines where its replayed
/// configuration had none between them: a static route after the `ip http` lines, an extended
/// access list after the route, and the control-plane block after the access list.
fn lab_gap(d: &Device) -> impl Fn(&str, &str) -> Option<usize> {
    let at = !d.stack.classic()
        && crate::release::Release::parse(&d.release).is_some_and(|r| r.config_version() == "17.6");
    move |a: &str, b: &str| {
        if !at {
            return None;
        }
        let (a, b) = (command(a), command(b));
        match (a.as_str(), b.as_str()) {
            ("ip http", "ip route") => Some(0),
            ("ip route", "ip access-list") => Some(1),
            ("ip access-list", "control-plane") => Some(3),
            _ => None,
        }
    }
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
fn block_lines(b: &Block, d: &Device) -> Vec<String> {
    let classic = d.stack.classic();
    if let Some((kind, _)) = crate::acl::of(&b.header) {
        let pad = !classic
            && crate::release::Release::parse(&d.release)
                .is_some_and(|r| r.config_version() == "17.6");
        return crate::acl::running(kind, &b.lines, classic, pad);
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
    let lines: Vec<String> = lines.iter().map(|e| format!(" {}", e.text)).collect();
    let named_eigrp = b
        .header
        .strip_prefix("router eigrp ")
        .is_some_and(|n| !n.bytes().all(|c| c.is_ascii_digit()));
    let essentials = crate::license::stated(d.global.iter().map(|e| e.text.as_str()))
        .is_some_and(|l| l == "network-essentials");
    if named_eigrp && essentials && !classic {
        return eigrp_stub(lines);
    }
    lines
}

/// A named EIGRP process's lines at network-essentials, which runs EIGRP as a stub: the lab
/// C9300-24T (17.6.4) printed `eigrp stub connected summary` last in each IPv4 address family and
/// `eigrp stub connected` first in each IPv4 service family that held no `eigrp stub` line.
fn eigrp_stub(lines: Vec<String>) -> Vec<String> {
    let opens = |l: &str, family: &str| {
        l.strip_prefix(family)
            .is_some_and(|r| r.starts_with("ipv4 ") && r.contains("autonomous-system "))
    };
    let mut out = Vec::with_capacity(lines.len() + 2);
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        let kind = [
            (" address-family ", " exit-address-family"),
            (" service-family ", " exit-service-family"),
        ]
        .into_iter()
        .find(|(family, _)| opens(l, family));
        let Some((family, closing)) = kind else {
            out.push(l.clone());
            i += 1;
            continue;
        };
        let end = lines[i..]
            .iter()
            .position(|x| x == closing)
            .map_or(lines.len(), |n| i + n);
        let stub = lines[i..end]
            .iter()
            .any(|x| x.trim_start().starts_with("eigrp stub"));
        out.push(l.clone());
        if !stub && family == " service-family " {
            out.push("  eigrp stub connected".into());
        }
        out.extend(lines[i + 1..end].iter().cloned());
        if !stub && family == " address-family " {
            out.push("  eigrp stub connected summary".into());
        }
        i = end;
    }
    out
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

fn interface_order(header: &str, d: &Device) -> (u8, usize, usize) {
    let name = header.trim_start_matches("interface ");
    let num = |prefix: &str| {
        name.strip_prefix(prefix)
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(0)
    };
    // Null0 first (the lab C9300, 17.6.4), then loopbacks (c9200 17.15 capture 27), port-channels,
    // tunnels, the ports with each subinterface after its port, Bluetooth and the VLAN interfaces
    // (the lab C9300-24T, 17.6.4)
    if name.starts_with("Null") {
        return (0, num("Null"), 0);
    }
    if name.starts_with("Loopback") {
        return (1, num("Loopback"), 0);
    }
    if name.starts_with("Port-channel") {
        return (2, num("Port-channel"), 0);
    }
    if name.starts_with("Tunnel") {
        return (3, num("Tunnel"), 0);
    }
    let (port, sub) = match name.split_once('.') {
        Some((port, sub)) => (port, sub.parse::<usize>().map_or(0, |s| s + 1)),
        None => (name, 0),
    };
    if let Some(i) = d.stack.physical().iter().position(|p| p == port) {
        return (4, i, sub);
    }
    if name.starts_with("Bluetooth") {
        return (5, 0, 0);
    }
    if name.starts_with("Vlan") {
        return (6, num("Vlan"), 0);
    }
    (7, 0, 0)
}

/// The size `Current configuration` names: each line from the first `!` through `end` with its
/// newline, plus one (the lab C9300, 17.6.4: the whole configuration and one interface's). An empty
/// line counts nothing (the same switch named 25775 bytes for a configuration with two, after `udld
/// enable` and `errdisable recovery interval 60`).
fn config_bytes(body: &[String]) -> usize {
    body.iter()
        .filter(|l| !l.is_empty())
        .map(|l| l.len() + 1)
        .sum::<usize>()
        + 1
}

/// The running configuration as `show running-config` prints it, a blank line after `end` (the lab
/// C9300, 17.6.4).
pub fn running_config(d: &Device) -> Vec<String> {
    let body = configuration(d);
    let bytes = config_bytes(&body);
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
    // IOS XE 17.6 names the last change between two `!` lines, counted in the size (the lab C9300,
    // 17.6.4); other releases print none here (inferred)
    if let Some(at) = d.last_change.filter(|_| {
        !d.stack.classic()
            && crate::release::Release::parse(&d.release)
                .is_some_and(|r| r.config_version() == "17.6")
    }) {
        let by = d
            .last_change_by
            .as_ref()
            .map_or_else(String::new, |u| format!(" by {u}"));
        body.push(format!(
            "! Last configuration change at {}{by}",
            crate::shell::config_stamp(at)
        ));
        body.push("!".to_string());
    }
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
    let off = d.global_value("vtp mode ") == Some("off");
    let kept: Vec<_> = d
        .global
        .iter()
        .filter(|e| !starts(&e.text, NOT_STORED) && !typed_version(&e.text))
        // `vtp version` prints whatever the mode (six C9200s at 17.9 and 17.12 print `vtp version
        // 1` with no `vtp mode` line)
        .filter(|e| in_config || !e.text.starts_with("vtp ") || e.text.starts_with("vtp version "))
        // VTP mode off prints no `vtp version` line (the lab C9300, 17.6.4, at versions 1 and 3)
        .filter(|e| !(off && e.text.starts_with("vtp version ")))
        .filter(|e| !d.held_top.contains(&e.text))
        .collect();
    // banners print after the global lines that follow the interfaces, before the line blocks
    let (banners, kept): (Vec<_>, Vec<_>) =
        kept.into_iter().partition(|e| e.key.starts_with("banner "));
    // IOS XE 17.6 writes `vlan filter` after the `vlan access-map` blocks, before the VLANs (the lab
    // C9300, 17.6.4); other releases keep it among the global lines before them (inferred)
    let late_filter = !d.stack.classic()
        && crate::release::Release::parse(&d.release).is_some_and(|r| r.config_version() == "17.6");
    let (filters, kept): (Vec<_>, Vec<_>) = kept
        .into_iter()
        .partition(|e| late_filter && e.text.starts_with("vlan filter "));
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
        if b.lines.is_empty() && vlan_ids_of(&b.header).is_some() {
            body.push(format!("{} ", b.header));
        } else {
            body.push(b.header.clone());
        }
        body.extend(block_lines(b, d));
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
    let vlans: Vec<&Block> = d
        .blocks
        .iter()
        .filter(|b| in_config && vlan_ids_of(&b.header).is_some())
        .collect();
    let vlans = vlan_units(&vlans);
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
    let mut acls: Vec<&Block> = d
        .blocks
        .iter()
        .filter(|b| starts(&b.header, BEFORE_SNMP) && !d.held_top.contains(&b.header))
        .collect();
    // IOS XE writes the standard lists, then the extended, each by name, whatever order they were
    // typed in (the lab C9300-24T, 17.6.4: `ZZ-FIRST` typed before `AA-SECOND`; 42 captures at
    // 17.6 and 17.15 holding two or more)
    if !d.stack.classic() {
        fn rank(b: &Block) -> Option<(bool, &str)> {
            crate::acl::of(&b.header).map(|(kind, name)| (kind != crate::acl::Kind::Standard, name))
        }
        acls.sort_by(|a, b| rank(a).cmp(&rank(b)));
    }
    let (mut post, pre): (Vec<&Block>, Vec<&Block>) = others
        .into_iter()
        .partition(|b| starts(&b.header, AFTER_VLAN_BLOCKS));
    // every class map, then every policy map (the 62 captures that hold them)
    post.sort_by_key(|b| {
        AFTER_VLAN_BLOCKS
            .iter()
            .position(|p| b.header.starts_with(p))
    });
    for b in &pre {
        block(&mut body, b);
    }
    for e in &filters {
        body.push(e.text.clone());
    }
    if !filters.is_empty() {
        body.push("!".into());
    }
    for b in &vlans {
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
        body.extend(block_lines(b, d));
    }
    body.push("!".into());
    for e in &last {
        body.push(e.text.clone());
    }
    if !last.is_empty() {
        body.push("!".into());
    }
    place_held(d, &mut body);
    vtp_together(&mut body);
    body.push("end".into());
    separate(d, body)
}

/// The VLAN blocks as the running configuration prints them: the VLANs with no line under them on
/// one line, their list compressed (`vlan 10,100-141 `, the lab C9300, 17.6.4; `vlan 65-66 ` on an IE
/// 3000), split at each VLAN with lines, which is its own block, in order of their first VLAN (the lab
/// C9300-24T, 17.6.4: `vlan 100-119 `, `vlan 120` named, `vlan 121-139 `).
fn vlan_units(vlans: &[&Block]) -> Vec<Block> {
    let mut units: Vec<Block> = vlans
        .iter()
        .filter(|b| !b.lines.is_empty())
        .map(|b| (*b).clone())
        .collect();
    let lined: Vec<u32> = units
        .iter()
        .filter_map(|b| vlan_ids_of(&b.header))
        .flat_map(crate::generate::vlan_ids)
        .collect();
    // a VLAN given lines after a range named it prints in its own block alone
    let bare: std::collections::BTreeSet<u32> = vlans
        .iter()
        .filter(|b| b.lines.is_empty())
        .filter_map(|b| vlan_ids_of(&b.header))
        .flat_map(crate::generate::vlan_ids)
        .filter(|id| !lined.contains(id))
        .collect();
    // the bare VLANs between two VLANs with lines
    let mut runs: std::collections::BTreeMap<usize, Vec<u32>> = Default::default();
    for id in bare {
        runs.entry(lined.iter().filter(|&&l| l < id).count())
            .or_default()
            .push(id);
    }
    for v in runs.into_values() {
        let mut parts = vec![];
        let mut i = 0;
        while i < v.len() {
            let mut j = i;
            while j + 1 < v.len() && v[j + 1] == v[j] + 1 {
                j += 1;
            }
            parts.push(if j > i {
                format!("{}-{}", v[i], v[j])
            } else {
                v[i].to_string()
            });
            i = j + 1;
        }
        units.push(Block {
            header: format!("vlan {}", parts.join(",")),
            lines: vec![],
        });
    }
    units.sort_by_key(|b| first_vlan(&b.header));
    units
}

/// The VTP lines together where the first of them prints, in the order IOS writes them whatever
/// order they were typed in: `vtp domain` before `vtp mode` (the lab C9300, 17.6.4, with `vtp mode
/// off` loaded and the domain typed), then `vtp version` and `vtp password` (inferred).
fn vtp_together(body: &mut Vec<String>) {
    const ORDER: [&str; 4] = ["vtp domain ", "vtp mode ", "vtp version ", "vtp password "];
    let rank = |t: &str| ORDER.iter().position(|p| t.starts_with(p));
    let Some(first) = body.iter().position(|l| rank(l).is_some()) else {
        return;
    };
    let mut lines: Vec<String> = body.iter().filter(|l| rank(l).is_some()).cloned().collect();
    lines.sort_by_key(|l| rank(l));
    // no VTP line stands before the first, so its place holds once they are taken out
    body.retain(|l| rank(l).is_none());
    body.splice(first..first, lines);
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
                    u.extend(block_lines(b, d));
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
    // a measured form the books lack has no model of its own: a running configuration's view the
    // books do not list is not the whole configuration
    let running =
        p.canonical().starts_with("show running-config") && !s.engine.is_measured(p.command);
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
                let name = words[3..].join("");
                let want = format!("interface {name}");
                let mut body: Vec<String> = vec![];
                if let Some(b) = s
                    .device
                    .blocks
                    .iter()
                    .find(|b| b.header.eq_ignore_ascii_case(&want))
                {
                    body.push("!".into());
                    body.push(b.header.clone());
                    body.extend(block_lines(b, &s.device));
                } else if s
                    .device
                    .stack
                    .physical()
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(&name))
                {
                    // a port never configured is an empty block
                    body.push("!".into());
                    body.push(want);
                }
                // an interface not made prints `end` alone (the lab C9300-24T, 17.6.4: Bluetooth0/4
                // before it was typed, `Current configuration : 5 bytes`)
                body.push("end".into());
                let bytes = config_bytes(&body);
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
            let eff = transceivers_checked(eff);
            let d: &Device = &eff;
            // every platform keeps IOS's trailing spaces (`act/unsup `): every family's show tech
            // has them; only a terminal log loses them
            match c.as_str() {
                // `show` alone (the lab C9300-24T, 17.6.4)
                "show" => vec!["% Type \"show ?\" for a list of subcommands".into()],
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
                "show vtp counters" => vtp_counters(d),
                "show vtp password" => vtp_password(d),
                "show license summary" => crate::version::license_summary(d),
                "show version" => crate::version::show_version(d, s.now(), s.booted),
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
                    all_access_lists(&s.device, c.split_whitespace().nth(2))
                }
                _ if c == "show ipv6 access-list" || c.starts_with("show ipv6 access-list ") => {
                    ipv6_access_lists(&s.device, c.split_whitespace().nth(3))
                }
                _ => vec![format!("% vswitch has no output model for '{c}' yet")],
            }
        }
        _ => vec![],
    }
}

/// The switch with each port a third-party transceiver err-disables (`Device::gbic_invalid`)
/// err-disabled with that transceiver and without its link.
fn transceivers_checked(d: std::borrow::Cow<'_, Device>) -> std::borrow::Cow<'_, Device> {
    let bad = d.gbic_invalid();
    if bad.is_empty() {
        return d;
    }
    let mut d = d.into_owned();
    for p in bad {
        let optic = d.transceivers.get(&p).map(|t| t.0);
        d.links.remove(&p);
        d.errdisabled.insert(p, optic);
    }
    std::borrow::Cow::Owned(d)
}

fn config_of<'d>(d: &'d Device, name: &str) -> Vec<&'d str> {
    d.config_of(name)
}

/// Whether the switch holds the access lists IOS XE makes itself (`acl::SYSTEM_V4`): a Catalyst 9300
/// at 17.6 (the lab C9300-24T, 17.6.4); other platforms and releases are inferred to list none.
fn system_lists(d: &Device) -> bool {
    d.stack.family() == Some(crate::platform::Family::C9300)
        && crate::release::Release::parse(&d.release).is_some_and(|r| r.config_version() == "17.6")
}

/// `show ip access-lists [<name>]`: each IPv4 access list, or the one named; nothing when there is
/// none (the lab 2960G with none configured). Classic IOS lists them as created; IOS XE the standard
/// lists, then the extended, each by name in byte order, with the lists it makes itself (the lab
/// C9300-24T, 17.6.4).
pub fn access_lists(d: &Device, name: Option<&str>) -> Vec<String> {
    let classic = d.stack.classic();
    let mut lists: Vec<(crate::acl::Kind, &str, Vec<String>)> = d
        .blocks
        .iter()
        .filter_map(|b| crate::acl::of(&b.header).map(|(kind, n)| (kind, n, b)))
        .map(|(kind, n, b)| (kind, n, crate::acl::show(kind, n, &b.lines, classic)))
        .collect();
    if system_lists(d) {
        for (n, entries) in crate::acl::SYSTEM_V4 {
            if lists.iter().any(|(_, have, _)| have == n) {
                continue;
            }
            // the HTTP servers' list holds an entry while each server runs (the lab C9300-24T,
            // 17.6.4: both gone after `no ip http server` and `no ip http secure-server`, the list
            // kept; one entry per server inferred)
            let off = |server: &str| {
                d.global
                    .iter()
                    .any(|e| e.text == format!("no ip http {server}"))
            };
            let served = |e: &&&str| {
                *n != "IP-Adm-V4-Int-ACL-global"
                    || !(e.ends_with(" eq www") && off("server")
                        || e.ends_with(" eq 443") && off("secure-server"))
            };
            let mut out = vec![format!("Extended IP access list {n}")];
            out.extend(entries.iter().filter(served).map(|e| format!("    {e}")));
            lists.push((crate::acl::Kind::Extended, n, out));
        }
    }
    if !classic {
        lists.sort_by(|a, b| {
            (a.0 != crate::acl::Kind::Standard, a.1).cmp(&(b.0 != crate::acl::Kind::Standard, b.1))
        });
    }
    lists
        .into_iter()
        .filter(|(_, n, _)| name.is_none_or(|want| want == *n))
        .flat_map(|(_, _, out)| out)
        .collect()
}

/// `show ipv6 access-list [<name>]`: each IPv6 access list by name in byte order, with the lists IOS
/// XE makes itself (as `access_lists`).
pub fn ipv6_access_lists(d: &Device, name: Option<&str>) -> Vec<String> {
    let mut lists: Vec<(&str, Vec<String>)> = d
        .blocks
        .iter()
        .filter_map(|b| {
            let n = b.header.strip_prefix("ipv6 access-list ")?;
            (!n.contains(' ')).then(|| (n, crate::acl::show_v6(&b.lines)))
        })
        .collect();
    if system_lists(d) {
        for (n, entries) in crate::acl::SYSTEM_V6 {
            if !lists.iter().any(|(have, _)| have == n) {
                lists.push((n, entries.iter().map(|e| format!("    {e}")).collect()));
            }
        }
    }
    lists.sort_by(|a, b| a.0.cmp(b.0));
    lists
        .into_iter()
        .filter(|(n, _)| name.is_none_or(|want| want == *n))
        .flat_map(|(n, entries)| std::iter::once(format!("IPv6 access list {n}")).chain(entries))
        .collect()
}

/// `show access-lists [<name>]`: the IPv4 lists, the IPv6 lists, then the MAC lists, each MAC
/// list's title with a space after it (the lab C9300-24T, 17.6.4).
pub fn all_access_lists(d: &Device, name: Option<&str>) -> Vec<String> {
    let mut out = access_lists(d, name);
    if d.stack.classic() {
        return out;
    }
    out.extend(ipv6_access_lists(d, name));
    for b in &d.blocks {
        let Some(n) = b.header.strip_prefix("mac access-list extended ") else {
            continue;
        };
        if name.is_some_and(|want| want != n) {
            continue;
        }
        out.push(format!("Extended MAC access list {n} "));
        out.extend(b.lines.iter().map(|e| format!("    {}", e.text)));
    }
    out
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

/// Whether a `monitor session ... destination interface` line names the port (written short or whole,
/// singly, in a list or as a range).
pub(crate) fn span_destination(d: &Device, p: &str) -> bool {
    let want = short(p);
    d.global.iter().any(|e| {
        let Some(rest) = e.text.strip_prefix("monitor session ") else {
            return false;
        };
        let Some((_, list)) = rest.split_once(" destination interface ") else {
            return false;
        };
        let list = list.split(" encapsulation ").next().unwrap_or(list);
        list.split(',').any(|seg| {
            let seg = seg.trim();
            match seg.split_once(" - ") {
                Some((first, last)) => {
                    let first = short(first.trim());
                    let Some((stem, lo)) = first.rsplit_once('/') else {
                        return false;
                    };
                    let (Ok(lo), Ok(hi)) = (lo.parse::<u32>(), last.trim().parse::<u32>()) else {
                        return false;
                    };
                    (lo..=hi).any(|n| format!("{stem}/{n}") == want)
                }
                None => short(seg) == want,
            }
        })
    })
}

/// A SPAN destination with a link: `monitoring`, carrying no VLAN, out of trunks and spanning tree,
/// its protocol down (the lab C9300-24T, 17.6.4, Gi1/0/4 with a link); one with none is
/// `notconnect` as any port.
pub(crate) fn monitoring(d: &Device, p: &str) -> bool {
    up(d, p).is_some() && span_destination(d, p)
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
    if c.contains(&"no switchport") || suspended(d, name) || monitoring(d, name) {
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
        // a third-party transceiver's reason is its cause's name (the lab C9300's log, 17.6.4)
        let reason = if d.gbic_invalid().contains(&p) {
            "gbic-invalid"
        } else {
            "bpduguard"
        };
        out.push(format!(
            "{:<13}{name:<13}{:<13}{reason}",
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
        } else if monitoring(d, &p) {
            "monitoring"
        } else if app || link.is_some() {
            "connected"
        } else {
            "notconnect"
        };
        let vlan = if c.contains(&"no switchport") {
            "routed"
        } else if channel_down(d, &p) {
            "unassigned"
        } else if link.as_ref().is_some_and(|(_, l)| l.trunk) && !monitoring(d, &p) {
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
            // an err-disabled port still names its transceiver (the lab C9300, 17.6.4), and so does
            // one with a transceiver fitted and no link (inferred)
            None => {
                if let Some(Some(o)) = d.errdisabled.get(&p) {
                    ty = optic_type(d, o);
                } else if let Some((o, _)) = d.transceivers.get(&p) {
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

/// The ports that trunk, as `show interfaces trunk` lists them: a port-channel for its bundled
/// members, never a suspended LACP member.
pub(crate) fn trunking_ports(d: &Device) -> Vec<String> {
    ports(d)
        .into_iter()
        .filter(|p| {
            let link = up(d, p).or_else(|| bundle(d, p).map(|(_, l)| l));
            let member = !p.starts_with("Port-channel") && link.is_some_and(|l| l.bundled);
            // a suspended LACP member does not trunk (the lab C9300, 17.6.4)
            link.is_some_and(|l| l.trunk) && !member && !suspended(d, p) && !monitoring(d, p)
        })
        .collect()
}

/// `show interfaces trunk` with spanning tree's state: the last list keeps the VLANs a port forwards
/// in (`forwards`), `none` on a port blocked in all of them (the lab C9300, 17.6.4).
pub fn interfaces_trunk_with(d: &Device, forwards: &dyn Fn(&str, u32) -> bool) -> Vec<String> {
    let mut defined = defined_vlans(d);
    defined.insert(1);
    let rows: Vec<(String, Vec<&str>)> = trunking_ports(d)
        .into_iter()
        .map(|p| {
            let c = config_of(d, &p);
            (p, c)
        })
        .collect();
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
    for m in d.stack.members.iter().filter(|m| !m.absent) {
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
    // a VLAN's own lines win over a range that named it with none
    let mut blocks: Vec<&Block> = d.blocks.iter().collect();
    blocks.sort_by_key(|b| b.lines.is_empty());
    for b in blocks {
        let Some(ids) = vlan_ids_of(&b.header) else {
            continue;
        };
        let name = b.lines.iter().find_map(|e| e.text.strip_prefix("name "));
        // `state suspend` reads `suspended` (the lab C9300-24T, 17.6.4)
        let status = if b.lines.iter().any(|e| e.text == "shutdown") {
            "act/lshut"
        } else if b.lines.iter().any(|e| e.text == "state suspend") {
            "suspended"
        } else {
            "active"
        };
        // an extended VLAN is listed after 1005 (the lab C9300-24T, 17.6.4: VLAN 4000)
        for id in crate::generate::vlan_ids(ids) {
            if id == 1 || (1002..=1005).contains(&id) || vlans.iter().any(|v| v.0 == id) {
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
        if id != 1 && !(1002..=1005).contains(&id) && !vlans.iter().any(|v| v.0 == id) {
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
            // nor a SPAN destination, linked or not (the lab C9300-24T, 17.6.4)
            if c.contains(&"no switchport") || !d.stack.fitted(&p) || span_destination(d, &p) {
                return None;
            }
            // a trunking port and a bundle's member are not listed; the bundle is, unless it trunks,
            // and only once a member is bundled (c2960x 15.2(7)E capture 8's Po1, every member down, is
            // not); a suspended LACP member is, in its access VLAN (the lab C9300-24T, 17.6.4)
            let link = up(d, &p).or_else(|| bundle(d, &p).map(|(_, l)| l));
            if !suspended(d, &p)
                && link.is_some_and(|l| l.trunk || (l.bundled && !p.starts_with("Port-channel")))
            {
                return None;
            }
            if p.starts_with("Port-channel") && link.is_none() {
                return None;
            }
            let id = value(&c, "switchport access vlan ")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            // a voice VLAN lists the port too (the lab C9300-24T, 17.6.4: Gi1/0/14 under 10 and 40)
            let voice = value(&c, "switchport voice vlan ")
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|&v| v != id);
            Some(
                [(short(&p), id)]
                    .into_iter()
                    .chain(voice.map(|v| (short(&p), v))),
            )
        })
        .flatten()
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
            // a PAgP or `on` member with no link too (the lab C9300-24T, 17.6.4: Po2's and Po3's)
            Some((n, _)) if !d.stack.classic() && (link.is_none() || suspended(d, &p)) => {
                held = true;
                b.push(format!(
                    "Operational Mode: down (suspended member of bundle Po{n})"
                ));
            }
            _ if monitoring(d, &p) => {
                held = true;
                b.push("Operational Mode: down".into());
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

/// The VTP updater: the lowest-numbered VLAN interface with an address, else the first layer 3
/// interface with one (the lab C9300, 17.6.4: Lo0 while Vlan10 waited on DHCP), as its short name,
/// its address and why IOS chose it.
pub fn vtp_updater(d: &Device) -> Option<(String, String, &'static str)> {
    let address = |b: &Block| {
        b.lines
            .iter()
            .find_map(|e| e.text.strip_prefix("ip address "))?
            .split_whitespace()
            .next()
            .filter(|ip| ip.contains('.'))
            .map(str::to_string)
    };
    d.blocks
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
            // tunnels last, as before they took their place in the configuration's order
            l3.sort_by_key(|b| {
                let o = interface_order(&b.header, d);
                (o.0 == 3, o)
            });
            l3.first().and_then(|b| {
                let name = b.header.strip_prefix("interface ")?;
                Some((short(name), address(b)?, "first layer3 interface found"))
            })
        })
}

/// One VLAN's record as a VTP subset advertisement carries it: length, status, type, name length,
/// ID, MTU, 802.10 SAID (100000 plus the ID), the name padded to four bytes, then the type's
/// attributes (type, length in two-byte words, value): 1002 and 1003 a ring number and a parent of
/// 0, 1004 a bridge number of 0 and STP `ieee`, 1005 a bridge number of 0 and STP `ibm`.
pub fn vtp_vlan_record(id: u16, name: &str, status: u8, mtu: u16) -> Vec<u8> {
    let (kind, attrs): (u8, &[(u8, u16)]) = match id {
        1002 => (2, &[(1, 0), (4, 0)]),
        1003 => (3, &[(1, 0), (4, 0)]),
        1004 => (4, &[(2, 0), (3, 1)]),
        1005 => (5, &[(2, 0), (3, 2)]),
        _ => (1, &[]),
    };
    let name = name.as_bytes();
    let padded = name.len().div_ceil(4) * 4;
    let mut r = vec![0, status, kind, name.len() as u8];
    r.extend(id.to_be_bytes());
    r.extend(mtu.to_be_bytes());
    r.extend((100_000 + u32::from(id)).to_be_bytes());
    r.extend(name);
    r.resize(12 + padded, 0);
    for (t, v) in attrs {
        r.extend([*t, 1]);
        r.extend(v.to_be_bytes());
    }
    r[0] = r.len() as u8;
    r
}

/// The VTP version 1 and 2 MD5 digest: MD5 over the 16-byte secret, the summary advertisement
/// (version, code 1, followers and timestamp 0, the domain padded to 32 bytes, revision, updater,
/// a zero digest), each VLAN's record in VLAN order, and the secret again. With no password the
/// secret is 16 zero bytes. Measured on the lab C9300 (17.6.4) and the lab C2960G (15.0(2)SE11).
pub fn vtp_digest(
    version: u8,
    domain: &str,
    revision: u32,
    updater: std::net::Ipv4Addr,
    vlans: &[Vec<u8>],
    secret: [u8; 16],
) -> [u8; 16] {
    use md5::{Digest, Md5};
    let mut m = Md5::new();
    m.update(secret);
    let d = &domain.as_bytes()[..domain.len().min(32)];
    let mut dom = [0u8; 32];
    dom[..d.len()].copy_from_slice(d);
    m.update([version, 1, 0, d.len() as u8]);
    m.update(dom);
    m.update(revision.to_be_bytes());
    m.update(updater.octets());
    m.update([0u8; 12 + 16]);
    for v in vlans {
        m.update(v);
    }
    m.update(secret);
    m.finalize().into()
}

/// The VTP secret a password keys: MD5 over the password repeated through 1563 64-byte blocks
/// (100,032 bytes, the first block boundary past 100,000). Measured with six passwords on the lab
/// C2960G (15.0(2)SE11): repeats of one password give one digest.
pub fn vtp_secret(password: &str) -> [u8; 16] {
    use md5::{Digest, Md5};
    let p = password.as_bytes();
    if p.is_empty() {
        return [0; 16];
    }
    let mut m = Md5::new();
    let mut block = [0u8; 64];
    let mut at = 0;
    for _ in 0..1563 {
        for b in block.iter_mut() {
            *b = p[at % p.len()];
            at += 1;
        }
        m.update(block);
    }
    m.finalize().into()
}

/// The digest `show vtp status` prints for the switch's VLAN database, keyed by the VTP password's
/// secret when one is set.
fn vtp_device_digest(d: &Device, version: &str, domain: &str) -> [u8; 16] {
    let secret = d.global_value("vtp password ").map_or([0; 16], vtp_secret);
    // versions 1 and 2 carry the normal range only; a suspended VLAN's status 1 and a VLAN's own
    // MTU are inferred from the advertisement's fields
    let records: Vec<Vec<u8>> = vlan_list(d)
        .into_iter()
        .filter(|(id, _, _)| *id <= 1005)
        .map(|(id, name, _)| {
            let block = d.blocks.iter().find(|b| {
                vlan_ids_of(&b.header)
                    .is_some_and(|ids| crate::generate::vlan_ids(ids).contains(&id))
            });
            let has =
                |p: &str| block.and_then(|b| b.lines.iter().find_map(|e| e.text.strip_prefix(p)));
            let status = u8::from(has("state ") == Some("suspend"));
            let mtu = has("mtu ").and_then(|m| m.parse().ok()).unwrap_or(1500);
            vtp_vlan_record(id as u16, &name, status, mtu)
        })
        .collect();
    let updater = d
        .vtp
        .modified_by
        .parse()
        .unwrap_or(std::net::Ipv4Addr::UNSPECIFIED);
    let v = if version == "2" { 2 } else { 1 };
    vtp_digest(v, domain, d.vtp.revision, updater, &records, secret)
}

/// `show vtp status` as the lab C9300 (17.6.4), the lab C2960G (15.0(2)SE11) and the cores print it.
/// Version, domain, pruning and mode come from the VTP lines vlan.dat keeps (version 1, server and
/// pruning off by default), traps from `snmp-server enable traps vtp`; the VLANs are counted with 1
/// and 1002-1005, Device ID is the base MAC address, and the revision and the last change are the
/// VLAN database's own (`Device::vtp`).
pub fn vtp_status(d: &Device) -> Vec<String> {
    let has = |line: &str| d.global.iter().any(|e| e.text == line);
    let (version, mode, domain) = d.vtp_settings();
    let mode = match mode.as_str() {
        "client" => "Client",
        "transparent" => "Transparent",
        "off" => "Off",
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
    let mut vlans = defined_vlans(d);
    vlans.extend([1, 1002, 1003, 1004, 1005]);
    let field = |name: &str, v: &str| format!("{name:<32}: {v}");
    let feature = |name: &str, v: &str| format!("{name:<34}: {v}");
    // the base MAC address, as `show version` names it (13 captures that print both agree)
    let mut out = vec![
        field("VTP Version capable", "1 to 3"),
        field("VTP version running", &version),
        field("VTP Domain Name", &domain),
        field("VTP Pruning Mode", &pruning_line),
        field("VTP Traps Generation", on(traps)),
        field("Device ID", &d.mac.to_string()),
    ];
    let keeps = matches!(mode, "Server" | "Client");
    if version == "3" {
        // a block per feature; MST and the unknown feature stay transparent, and an off or
        // transparent VLAN feature prints no revision (the lab C9300, 17.6.4). A client prints a
        // server's lines (inferred): no capture holds one
        out.push(String::new());
        out.push("Feature VLAN:".into());
        out.push("--------------".into());
        out.push(feature("VTP Operating Mode", mode));
        out.push(feature(
            "Number of existing VLANs",
            &vlans.len().to_string(),
        ));
        let extended = vlans.iter().filter(|&&v| v > 1005).count();
        out.push(feature(
            "Number of existing extended VLANs",
            &extended.to_string(),
        ));
        out.push(feature("Maximum VLANs supported locally", "4096"));
        // no primary server, which vswitch does not model: the primary's ID is zero and the digest
        // empty
        if keeps {
            out.push(feature(
                "Configuration Revision",
                &d.vtp.revision.to_string(),
            ));
            out.push(feature("Primary ID", "0000.0000.0000"));
            out.push(feature("Primary Description", ""));
            out.push(feature("MD5 digest", ""));
        }
        for other in ["MST", "UNKNOWN"] {
            out.push(String::new());
            out.push(String::new());
            out.push(format!("Feature {other}:"));
            out.push("--------------".into());
            out.push(feature("VTP Operating Mode", "Transparent"));
        }
        out.push(String::new());
        return out;
    }
    out.push(format!(
        "Configuration last modified by {} at {}",
        d.vtp.modified_by, d.vtp.modified_at
    ));
    // a transparent switch updates no one (c9200 17.15 capture 27), nor one with VTP off (the lab
    // C9300, 17.6.4)
    if keeps {
        out.push(match vtp_updater(d) {
            Some((n, ip, why)) => format!("Local updater ID is {ip} on interface {n} ({why})"),
            None => "Local updater ID is 0.0.0.0 (no valid interface found)".into(),
        });
    }
    out.extend([
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
        // versions 1 and 2 count the normal range (the lab C9300-24T, 17.6.4: 95 with VLAN 4000)
        feature(
            "Number of existing VLANs",
            &vlans.iter().filter(|&&v| v <= 1005).count().to_string(),
        ),
        feature("Configuration Revision", &d.vtp.revision.to_string()),
    ]);
    // the digest's two rows, each ending with a space on every platform
    let md5 = vtp_device_digest(d, &version, &domain);
    let row = |b: &[u8]| b.iter().map(|x| format!("0x{x:02X} ")).collect::<String>();
    out.push(feature("MD5 digest", &row(&md5[..8])));
    out.push(format!("{:36}{}", "", row(&md5[8..])));
    out
}

/// `show vtp counters` as the lab C9300 (17.6.4) printed it: no advertisement sent or received, as
/// vswitch exchanges none (inferred for a lab of VTP servers), and a pruning row per trunking port.
pub fn vtp_counters(d: &Device) -> Vec<String> {
    let mut out: Vec<String> = [
        "Summary advertisements received    : 0",
        "Subset advertisements received     : 0",
        "Request advertisements received    : 0",
        "Summary advertisements transmitted : 0",
        "Subset advertisements transmitted  : 0",
        "Request advertisements transmitted : 0",
        "Number of config revision errors   : 0",
        "Number of config digest errors     : 0",
        "Number of V1 summary errors        : 0",
    ]
    .iter()
    .map(|l| l.to_string())
    .collect();
    out.insert(0, "VTP statistics:".into());
    out.extend([
        String::new(),
        String::new(),
        "VTP pruning statistics:".into(),
        String::new(),
        "Trunk            Join Transmitted Join Received    Summary advts received from".into(),
        format!("{:51}non-pruning-capable device", ""),
        "---------------- ---------------- ---------------- ---------------------------".into(),
    ]);
    for p in trunking_ports(d) {
        out.push(format!("{:<20}{:<17}{:<17}{:<9}", short(&p), 0, 0, 0));
    }
    out
}

/// `show vtp password`: the lab C9300's answer (17.6.4) with none set, the lab C2960G's
/// (15.0(2)SE11) with one set, its text as typed (on IOS XE inferred).
pub fn vtp_password(d: &Device) -> Vec<String> {
    match d.global_value("vtp password ") {
        Some(p) => vec![format!("VTP Password: {p}")],
        None => vec!["The VTP password is not configured.".into()],
    }
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
    // each subinterface after its port, then Bluetooth (the lab C9300-24T, 17.6.4)
    let subs = |port: &str| {
        let mut v: Vec<(u64, String)> = d
            .blocks
            .iter()
            .filter_map(|b| b.header.strip_prefix("interface "))
            .filter_map(|n| {
                let (p, s) = n.split_once('.')?;
                (p == port).then(|| s.parse().ok().map(|s| (s, n.to_string())))?
            })
            .collect();
        v.sort();
        v.into_iter().map(|(_, n)| n).collect::<Vec<_>>()
    };
    let ports: Vec<String> = d
        .stack
        .physical()
        .into_iter()
        .flat_map(|p| {
            let s = subs(&p);
            std::iter::once(p).chain(s)
        })
        .collect();
    let bluetooth: Vec<String> = d
        .blocks
        .iter()
        .filter_map(|b| b.header.strip_prefix("interface Bluetooth"))
        .map(|n| format!("Bluetooth{n}"))
        .collect();
    let names = vlans
        .into_iter()
        .chain(ports)
        .chain(bluetooth)
        .chain(named("Port-channel"))
        .chain(named("Loopback"))
        .chain(named("Tunnel"));
    for n in names {
        let header = format!("interface {n}");
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
        let port_bare = (d.stack.physical().contains(&n) || n.starts_with("Bluetooth"))
            && Some(n.as_str()) != d.stack.management()
            && !c.iter().any(|l| l.starts_with("ip address "));
        let (status, protocol) = brief_state(d, &n, &defined);
        // an address typed at the console reads `manual`, and so does `no ip address` typed after
        // it; DHCP's reads `manual` while the interface is down and `DHCP` once it is up, with or
        // without a lease (the lab C9300, 17.6.4: Vlan1 shut, then up; up with its protocol down,
        // inferred). One in a configuration loaded reads `DHCP` or NVRAM, and the factory default's
        // `no ip address` `unset`
        let typed = d.manual.contains(&header);
        let dhcp = c.iter().any(|l| l.starts_with("ip address dhcp"));
        let addressed = c
            .iter()
            .any(|l| l.starts_with("ip address ") || *l == "no ip address");
        let method = if typed && dhcp {
            if (status, protocol) == ("up", "up") {
                "DHCP"
            } else {
                "manual"
            }
        } else if typed && addressed {
            "manual"
        } else if dhcp {
            "DHCP"
        } else if !loopback_bare && !port_bare && !d.factory && addressed {
            "NVRAM"
        } else {
            "unset"
        };
        out.push(row(&n, ip, "YES", method, status, protocol));
    }
    // the tunnels PIM makes, after the configured ones: they print no configuration
    for (n, ip, status, protocol) in pim_tunnels(d, &defined) {
        out.push(row(&n, &ip, "YES", "unset", status, protocol));
    }
    out
}

/// An interface's status and protocol as `show ip interface brief` prints them.
fn brief_state(
    d: &Device,
    n: &str,
    defined: &std::collections::BTreeSet<u32>,
) -> (&'static str, &'static str) {
    let header = format!("interface {n}");
    let block = d.blocks.iter().find(|b| b.header == header);
    let c = config_of(d, n);
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
        || n.starts_with("AppGigabitEthernet") && !d.stack.absent(n)
        || up(d, n).is_some()
        || bundle(d, n).is_some()
        || vlan_up;
    // Vlan1 is shut down until configured (the cores' running configurations)
    let shut = c.contains(&"shutdown") || (n == "Vlan1" && block.is_none());
    // a VLAN interface whose VLAN exists but no running port carries is up, its protocol down
    // (the C2960G's Vlan1)
    let vlan_exists = n
        .strip_prefix("Vlan")
        .and_then(|v| v.parse::<u32>().ok())
        .is_some_and(|id| id == 1 || defined.contains(&id));
    if shut {
        ("administratively down", "down")
    } else if n.starts_with("Tunnel") {
        // a tunnel is up; its protocol runs while its source does and a connected subnet holds its
        // destination (the lab C9300-24T, 17.6.4: Loopback0 to 10.99.201.1 `up down`, to 10.99.1.2
        // in Vlan1's subnet `up up`, Loopback0 shut `up down`); a static route's next hop in one
        // counts too (inferred)
        let source =
            value(&c, "tunnel source ").is_some_and(|s| match s.parse::<std::net::Ipv4Addr>() {
                Ok(a) => addressed(d, defined).any(|(_, x, _)| x == a),
                Err(_) => !s.starts_with("Tunnel") && brief_state(d, s, defined) == ("up", "up"),
            });
        let reaches = value(&c, "tunnel destination ")
            .and_then(|v| v.parse().ok())
            .is_some_and(|a| reachable(d, defined, a));
        ("up", if source && reaches { "up" } else { "down" })
    } else if suspended(d, n) || monitoring(d, n) {
        // a suspended LACP member and a linked SPAN destination run no protocol (the lab
        // C9300-24T, 17.6.4)
        ("up", "down")
    } else if running {
        ("up", "up")
    } else if vlan_exists {
        ("up", "down")
    } else {
        ("down", "down")
    }
}

/// The global routing table's running interfaces with an address: name, address, mask. Tunnels
/// are left out.
fn addressed<'d>(
    d: &'d Device,
    defined: &'d std::collections::BTreeSet<u32>,
) -> impl Iterator<Item = (&'d str, std::net::Ipv4Addr, std::net::Ipv4Addr)> + 'd {
    d.blocks.iter().filter_map(move |b| {
        let n = b.header.strip_prefix("interface ")?;
        if n.starts_with("Tunnel")
            || b.lines
                .iter()
                .any(|e| e.text.starts_with("vrf forwarding "))
        {
            return None;
        }
        let mut words = b
            .lines
            .iter()
            .find_map(|e| e.text.strip_prefix("ip address "))?
            .split_whitespace();
        let a = words.next()?.parse().ok()?;
        let m = words.next()?.parse().ok()?;
        (brief_state(d, n, defined) == ("up", "up")).then_some((n, a, m))
    })
}

/// Whether the global routing table reaches an address: a running interface's subnet holds it, or
/// a static route's next hop in one.
fn reachable(d: &Device, defined: &std::collections::BTreeSet<u32>, a: std::net::Ipv4Addr) -> bool {
    let within = |a: std::net::Ipv4Addr, net: std::net::Ipv4Addr, mask: std::net::Ipv4Addr| {
        u32::from(a) & u32::from(mask) == u32::from(net) & u32::from(mask)
    };
    let connected = |a| addressed(d, defined).any(|(_, x, m)| within(a, x, m));
    connected(a)
        || d.global.iter().any(|e| {
            let words: Vec<&str> = e.text.split_whitespace().collect();
            let ["ip", "route", net, mask, hop, ..] = words[..] else {
                return false;
            };
            let (Ok(net), Ok(mask)) = (net.parse(), mask.parse()) else {
                return false;
            };
            within(a, net, mask) && hop.parse().is_ok_and(&connected)
        })
}

/// The tunnels PIM makes where `ip pim rp-address` names an RP and multicast routing runs, numbered
/// after the configured tunnels (the lab C9300-24T, 17.6.4, with Tunnel0 configured): the
/// encapsulation tunnel, addressed from the first sparse-mode interface (unassigned and its
/// protocol down with none), and on the RP the decapsulation tunnel, addressed with the RP's
/// address, down with its interface. Their numbering past the lowest free is inferred.
fn pim_tunnels(
    d: &Device,
    defined: &std::collections::BTreeSet<u32>,
) -> Vec<(String, String, &'static str, &'static str)> {
    let multicast = d.global.iter().any(|e| {
        matches!(
            e.text.split_whitespace().collect::<Vec<_>>()[..],
            ["ip", "multicast-routing"] | ["ip", "multicast-routing", "distributed"]
        )
    });
    let rp = d
        .global
        .iter()
        .find_map(|e| e.text.strip_prefix("ip pim rp-address "))
        .and_then(|v| {
            v.split_whitespace()
                .next()?
                .parse::<std::net::Ipv4Addr>()
                .ok()
        });
    let Some(rp) = rp.filter(|_| multicast) else {
        return vec![];
    };
    let mut free = (0u32..).filter(|n| {
        let h = format!("interface Tunnel{n}");
        !d.blocks.iter().any(|b| b.header == h)
    });
    let mut out = vec![];
    let mut sparse: Vec<&Block> = d
        .blocks
        .iter()
        .filter(|b| b.lines.iter().any(|e| e.text == "ip pim sparse-mode"))
        .collect();
    sparse.sort_by_key(|b| interface_order(&b.header, d));
    let source = sparse.iter().find_map(|b| {
        b.lines
            .iter()
            .find_map(|e| e.text.strip_prefix("ip address "))?
            .split_whitespace()
            .next()
            .filter(|a| a.contains('.'))
    });
    if let Some(n) = free.next() {
        out.push(match source {
            Some(a) => (format!("Tunnel{n}"), a.to_string(), "up", "up"),
            None => (format!("Tunnel{n}"), "unassigned".into(), "up", "down"),
        });
    }
    let own = d.blocks.iter().find_map(|b| {
        let name = b.header.strip_prefix("interface ")?;
        let a = b
            .lines
            .iter()
            .find_map(|e| e.text.strip_prefix("ip address "))?
            .split_whitespace()
            .next()?;
        (a.parse() == Ok(rp)).then_some(name)
    });
    if let (Some(name), Some(n)) = (own, free.next()) {
        let protocol = brief_state(d, name, defined).1;
        out.push((format!("Tunnel{n}"), rp.to_string(), "up", protocol));
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

    /// A C9300 at 17.6 typed `lines` at the console at `clock`, its base MAC 0011.2233.4401.
    fn vtp_switch<'e>(e: &'e cli::Engine, lines: &[&str]) -> shell::Session<'e> {
        let mut s = typed(e, "C9300-24T", &[]);
        s.device.mac = crate::stp::Mac::parse("0011.2233.4401").unwrap();
        s.clock = Some(1_792_175_417);
        for l in ["configure terminal"].iter().chain(lines).chain(&["end"]) {
            let (o, r) = s.type_line(l);
            assert_eq!(r, shell::Result::Ok, "{l}: {o:?}");
        }
        s
    }

    /// The digest rows for a version 1 database with no password, updater 0.0.0.0, `vlans` and
    /// 1002-1005, built byte by byte from the advertisement's layout.
    fn hand_vtp_digest(domain: &[u8], revision: u32, vlans: &[(u16, &str)]) -> [String; 2] {
        hand_keyed_vtp_digest(domain, revision, vlans, "")
    }

    /// The digest by hand with a password: its secret is MD5 over the password repeated to
    /// 100,032 bytes, at both ends.
    fn hand_keyed_vtp_digest(
        domain: &[u8],
        revision: u32,
        vlans: &[(u16, &str)],
        password: &str,
    ) -> [String; 2] {
        use md5::{Digest, Md5};
        let secret: Vec<u8> = if password.is_empty() {
            vec![0; 16]
        } else {
            let fill: Vec<u8> = password.bytes().cycle().take(100_032).collect();
            Md5::digest(&fill).to_vec()
        };
        let mut b = secret.clone();
        b.extend([1, 1, 0, domain.len() as u8]);
        b.extend(domain);
        b.resize(16 + 4 + 32, 0);
        b.extend(revision.to_be_bytes());
        b.resize(b.len() + 4 + 12 + 16, 0);
        let mut all: Vec<(u16, &str, u8, &[u8])> =
            vlans.iter().map(|(i, n)| (*i, *n, 1, &[][..])).collect();
        all.extend([
            (1002, "fddi-default", 2, &[1, 1, 0, 0, 4, 1, 0, 0][..]),
            (1003, "token-ring-default", 3, &[1, 1, 0, 0, 4, 1, 0, 0][..]),
            (1004, "fddinet-default", 4, &[2, 1, 0, 0, 3, 1, 0, 1][..]),
            (1005, "trnet-default", 5, &[2, 1, 0, 0, 3, 1, 0, 2][..]),
        ]);
        for (id, name, kind, tlv) in all {
            let pad = (4 - name.len() % 4) % 4;
            b.push((12 + name.len() + pad + tlv.len()) as u8);
            b.extend([0, kind, name.len() as u8]);
            b.extend(id.to_be_bytes());
            b.extend(1500u16.to_be_bytes());
            b.extend((100_000 + u32::from(id)).to_be_bytes());
            b.extend(name.as_bytes());
            b.extend(vec![0; pad]);
            b.extend(tlv);
        }
        b.extend(&secret);
        let d = Md5::digest(&b);
        let row = |x: &[u8]| x.iter().map(|y| format!("0x{y:02X} ")).collect::<String>();
        [
            format!("MD5 digest                        : {}", row(&d[..8])),
            format!("{:36}{}", "", row(&d[8..])),
        ]
    }

    #[test]
    fn a_vtp_vlan_record_follows_the_subset_advertisement_layout() {
        // VLAN 1: length 20, active, Ethernet, name length 7, ID, MTU 1500, SAID 100001, the name
        // padded to 8
        assert_eq!(
            vtp_vlan_record(1, "default", 0, 1500),
            [
                20, 0, 1, 7, 0, 1, 0x05, 0xDC, 0, 0x01, 0x86, 0xA1, b'd', b'e', b'f', b'a', b'u',
                b'l', b't', 0
            ]
        );
        // 1005: the TrBRF type, a bridge number of 0 and STP `ibm` after the padded name
        let r = vtp_vlan_record(1005, "trnet-default", 0, 1500);
        assert_eq!((r[0], r[2], r.len()), (36, 5, 36));
        assert_eq!(r[28..], [2, 1, 0, 0, 3, 1, 0, 2]);
        // the summary's fields each move the digest
        let v = [vtp_vlan_record(1, "default", 0, 1500)];
        let base = vtp_digest(1, "", 0, [0, 0, 0, 0].into(), &v, [0; 16]);
        for other in [
            vtp_digest(2, "", 0, [0, 0, 0, 0].into(), &v, [0; 16]),
            vtp_digest(1, "A", 0, [0, 0, 0, 0].into(), &v, [0; 16]),
            vtp_digest(1, "", 1, [0, 0, 0, 0].into(), &v, [0; 16]),
            vtp_digest(1, "", 0, [192, 0, 2, 1].into(), &v, [0; 16]),
            vtp_digest(1, "", 0, [0, 0, 0, 0].into(), &[], [0; 16]),
        ] {
            assert_ne!(other, base);
        }
    }

    #[test]
    fn a_factory_vlan_database_prints_its_digest() {
        let e = engine("c9300", "17.6");
        let s = vtp_switch(&e, &[]);
        let status = vtp_status(&s.device);
        assert_eq!(
            status[status.len() - 2..],
            hand_vtp_digest(b"", 0, &[(1, "default")])
        );
        assert_eq!(
            status[6],
            "Configuration last modified by 0.0.0.0 at 0-0-00 00:00:00"
        );
    }

    #[test]
    fn a_vtp_server_counts_a_revision_per_vlan_change_and_stamps_it() {
        let e = engine("c9300", "17.6");
        let s = vtp_switch(&e, &["vtp domain LAB", "vlan 140", "exit"]);
        let lines = vtp_status(&s.device);
        assert_eq!(
            lines[..9],
            [
                "VTP Version capable             : 1 to 3",
                "VTP version running             : 1",
                "VTP Domain Name                 : LAB",
                "VTP Pruning Mode                : Disabled (Operationally Disabled)",
                "VTP Traps Generation            : Disabled",
                "Device ID                       : 0011.2233.4401",
                "Configuration last modified by 0.0.0.0 at 10-16-26 18:30:17",
                "Local updater ID is 0.0.0.0 (no valid interface found)",
                "",
            ]
        );
        assert!(lines.contains(&"Configuration Revision            : 1".to_string()));
        assert!(lines.contains(&"Number of existing VLANs          : 6".to_string()));
        // the digest over the summary and the six VLANs' records, each row ending with a space
        let want = hand_vtp_digest(b"LAB", 1, &[(1, "default"), (140, "VLAN0140")]);
        assert_eq!(lines[lines.len() - 2..], want);
        // a password keys the digest; its repeats key the same one
        let keyed = |pw: &str| {
            let mut p = vtp_switch(&e, &[&format!("vtp password {pw}")]);
            p.device.vtp.revision = 0;
            vtp_status(&p.device)
        };
        let lines = keyed("SECRET");
        assert_eq!(
            lines[lines.len() - 2..],
            hand_keyed_vtp_digest(b"", 0, &[(1, "default")], "SECRET")
        );
        assert_ne!(lines, keyed("SECRET2"));
        assert_eq!(keyed("ab"), keyed("abab"));
        assert_eq!(vtp_secret(""), [0; 16]);
        // a VLAN named in its submode is one change; the server keeps its VLANs out of the running
        // configuration
        let mut s = s;
        for l in ["configure terminal", "vlan 141", "name X", "exit", "end"] {
            s.type_line(l);
        }
        assert_eq!(s.device.vtp.revision, 2);
        let run = running_config(&s.device);
        assert!(
            !run.iter()
                .any(|l| l.starts_with("vlan ") || l.starts_with("vtp "))
        );
        // a new domain starts again at 0, and so does transparent mode, where a change counts nothing
        for l in ["configure terminal", "vtp domain LAB2", "end"] {
            s.type_line(l);
        }
        assert_eq!(s.device.vtp.revision, 0);
        for l in [
            "configure terminal",
            "vtp mode transparent",
            "vlan 142",
            "exit",
            "end",
        ] {
            s.type_line(l);
        }
        assert_eq!(s.device.vtp.revision, 0);
        let lines = vtp_status(&s.device);
        assert!(!lines.iter().any(|l| l.starts_with("Local updater")));
    }

    #[test]
    fn vtp_version_3_prints_a_block_per_feature_and_mode_off_its_vlans_on_one_line() {
        let e = engine("c9300", "17.6");
        let s = vtp_switch(
            &e,
            &["vtp domain LAB", "vtp version 3", "vlan 10,20-22", "exit"],
        );
        let status = vtp_status(&s.device);
        assert_eq!(
            status[6..],
            [
                "",
                "Feature VLAN:",
                "--------------",
                "VTP Operating Mode                : Server",
                "Number of existing VLANs          : 9",
                "Number of existing extended VLANs : 0",
                "Maximum VLANs supported locally   : 4096",
                "Configuration Revision            : 0",
                "Primary ID                        : 0000.0000.0000",
                "Primary Description               : ",
                "MD5 digest                        : ",
                "",
                "",
                "Feature MST:",
                "--------------",
                "VTP Operating Mode                : Transparent",
                "",
                "",
                "Feature UNKNOWN:",
                "--------------",
                "VTP Operating Mode                : Transparent",
                "",
            ]
        );
        let mut s = s;
        for l in [
            "configure terminal",
            "vtp mode off",
            "vlan 23",
            "name NAMED",
            "end",
        ] {
            s.type_line(l);
        }
        let status = vtp_status(&s.device);
        assert_eq!(status[9], "VTP Operating Mode                : Off");
        assert_eq!(status[12], "Maximum VLANs supported locally   : 4096");
        assert_eq!(status[13], "");
        // mode off prints the domain, the mode and the VLANs, no `vtp version`; unnamed VLANs share
        // one line ending in a space
        let run = running_config(&s.device);
        let vtp: Vec<&String> = run
            .iter()
            .filter(|l| l.starts_with("vtp ") || l.starts_with("vlan "))
            .collect();
        assert_eq!(
            vtp,
            [
                "vtp domain LAB",
                "vtp mode off",
                "vlan 10,20-22 ",
                "vlan 23"
            ]
        );
    }

    #[test]
    fn vtp_counters_list_each_trunk_and_the_password_answers_as_measured() {
        let e = engine("c9300", "17.6");
        let s = vtp_switch(&e, &[]);
        let c = vtp_counters(&s.device);
        assert_eq!(c[0], "VTP statistics:");
        assert_eq!(c[9], "Number of V1 summary errors        : 0");
        assert_eq!(
            c[14],
            "Trunk            Join Transmitted Join Received    Summary advts received from"
        );
        assert_eq!(c.len(), 17);
        assert_eq!(
            vtp_password(&s.device),
            ["The VTP password is not configured."]
        );
        let s = vtp_switch(&e, &["vtp password secret1"]);
        assert_eq!(vtp_password(&s.device), ["VTP Password: secret1"]);
    }

    #[test]
    fn a_c9300_at_17_6_boots_its_factory_default_and_reads_its_addresses_as_measured() {
        let e = engine("c9300", "17.6");
        let mut s = shell::Session::new(
            &e,
            device::Device::new(platform::parse_stack("C9300-24T").unwrap()),
        );
        s.factory_default();
        assert_eq!(s.prompt(), "Switch>");
        s.type_line("enable");
        assert_eq!(
            s.type_line("show startup-config").0,
            ["startup-config is not present"]
        );
        let run = running_config(&s.device);
        for l in [
            "spanning-tree mode rapid-pvst",
            "spanning-tree extend system-id",
            "license boot level network-advantage addon dna-advantage",
        ] {
            assert!(run.contains(&l.to_string()), "{l}: {run:?}");
        }
        assert_eq!(
            block_of(&run, "interface Vlan1"),
            ["interface Vlan1", " no ip address", " shutdown"]
        );
        // the factory's `no ip address` reads unset
        let brief = ip_interface_brief(&s.device);
        assert!(row(&brief, "Vlan1").contains(" YES unset  administratively down down"));
        assert!(row(&brief, "GigabitEthernet0/0").contains(" YES unset "));
        let vtp = vtp_status(&s.device);
        assert!(vtp.contains(&"Number of existing VLANs          : 5".to_string()));
        // DHCP typed on a shut interface reads manual, DHCP once it is up; `no ip address` after it
        // stays manual
        for l in [
            "configure terminal",
            "interface Vlan1",
            "ip address dhcp",
            "end",
        ] {
            s.type_line(l);
        }
        assert_eq!(
            block_of(&running_config(&s.device), "interface Vlan1"),
            ["interface Vlan1", " ip address dhcp", " shutdown"]
        );
        assert!(row(&ip_interface_brief(&s.device), "Vlan1").contains(" YES manual "));
        s.device.links.insert(
            "GigabitEthernet1/0/1".into(),
            device::Link {
                peer: "host".into(),
                optic: None,
                speed: None,
                half: false,
                trunk: false,
                bundled: false,
            },
        );
        for l in [
            "configure terminal",
            "interface Vlan1",
            "no shutdown",
            "end",
        ] {
            s.type_line(l);
        }
        assert!(row(&ip_interface_brief(&s.device), "Vlan1").starts_with(
            "Vlan1                  unassigned      YES DHCP   up                    up"
        ));
        for l in [
            "configure terminal",
            "interface Vlan1",
            "no ip address",
            "end",
        ] {
            s.type_line(l);
        }
        assert!(row(&ip_interface_brief(&s.device), "Vlan1").contains(" YES manual "));
        // a boot level typed takes effect at the next boot: it is said, not stored
        s.type_line("configure terminal");
        assert_eq!(
            s.type_line("license boot level network-essentials addon dna-essentials")
                .0,
            ["% use 'write' command to make license boot config take effect on next boot"]
        );
        s.type_line("end");
        assert!(
            !running_config(&s.device)
                .iter()
                .any(|l| l.contains("network-essentials"))
        );
        // no capture holds another release's default: a 17.15 switch starts blank
        let e = engine("c9300", "17.15");
        let mut s = shell::Session::new(
            &e,
            device::Device::new(platform::parse_stack("C9300-24T").unwrap()),
        );
        s.factory_default();
        assert!(!s.device.factory && s.device.global.is_empty());
    }

    #[test]
    fn at_17_6_vlan_filter_follows_the_access_maps_and_the_vtp_domain_precedes_the_mode() {
        let lines = [
            "vtp mode transparent",
            "vtp domain LAB",
            "vlan access-map DROPX 10",
            "action drop",
            "exit",
            "vlan filter DROPX vlan-list 10",
            "vlan 10",
            "exit",
        ];
        let e = engine("c9300", "17.6");
        let run = configuration(&typed(&e, "C9300-24T", &lines).device);
        let at = |l: &str| {
            run.iter()
                .position(|x| x == l)
                .unwrap_or_else(|| panic!("{l}: {run:?}"))
        };
        assert_eq!(at("vtp mode transparent"), at("vtp domain LAB") + 1);
        assert_eq!(at("vlan filter DROPX vlan-list 10"), at(" action drop") + 2);
        assert!(at("vlan 10 ") > at("vlan filter DROPX vlan-list 10"));
        // other releases keep it among the global lines, before the access maps
        let e = engine("c9300", "17.15");
        let run = configuration(&typed(&e, "C9300-24T", &lines).device);
        let at = |l: &str| run.iter().position(|x| x == l).unwrap();
        assert!(at("vlan filter DROPX vlan-list 10") < at("vlan access-map DROPX 10"));
    }

    #[test]
    fn the_configuration_size_counts_one_past_its_lines() {
        let e = engine("c9300", "17.6");
        let mut s = typed(
            &e,
            "C9300-24T",
            &["interface Vlan1", "ip address dhcp", "shutdown"],
        );
        // `!`, the header, two lines and `end`, each with its newline, plus one
        assert_eq!(
            s.type_line("show running-config interface Vlan1").0[2],
            "Current configuration : 50 bytes"
        );
    }

    #[test]
    fn vtp_stamps_print_month_and_day_unpadded() {
        assert_eq!(shell::vtp_stamp(1_792_175_417), "10-16-26 18:30:17");
        assert_eq!(shell::vtp_stamp(731_735_372), "3-10-93 03:49:32");
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

    /// A C9300-24T with a network module at 17.6, a third-party SFP in its first module port.
    fn with_third_party_sfp<'e>(e: &'e cli::Engine, lines: &[&str]) -> shell::Session<'e> {
        let mut s = typed(e, "C9300-24T+C9300-NM-4G", lines);
        s.device.transceivers.insert(
            "GigabitEthernet1/1/1".into(),
            (platform::optic("GLC-T").unwrap(), true),
        );
        s
    }

    #[test]
    fn a_third_party_transceiver_is_err_disabled_until_the_switch_allows_it() {
        let e = engine("c9300", "17.6");
        let mut s = with_third_party_sfp(&e, &[]);
        s.type_line("enable");
        let status = s.type_line("show interfaces status").0;
        assert_eq!(
            row(&status, "Gi1/1/1"),
            "Gi1/1/1                         err-disabled 1            auto   auto 10/100/1000BaseTX SFP"
        );
        let errdisabled = s.type_line("show interfaces status err-disabled").0;
        assert!(row(&errdisabled, "Gi1/1/1").ends_with("err-disabled gbic-invalid"));
        for allow in [
            "service unsupported-transceiver",
            "no errdisable detect cause gbic-invalid",
        ] {
            let mut s = with_third_party_sfp(&e, &[allow]);
            s.type_line("enable");
            let status = s.type_line("show interfaces status").0;
            assert!(row(&status, "Gi1/1/1").contains(" notconnect "), "{allow}");
            assert!(
                configuration(&s.device).contains(&allow.to_string()),
                "{allow}"
            );
        }
        // a Cisco optic in the same port is never checked
        let mut s = with_third_party_sfp(&e, &[]);
        s.device
            .transceivers
            .get_mut("GigabitEthernet1/1/1")
            .unwrap()
            .1 = false;
        s.type_line("enable");
        let status = s.type_line("show interfaces status").0;
        assert!(row(&status, "Gi1/1/1").contains(" notconnect "));
    }

    /// A configuration as a switch printed it: comment and `!` lines, a `! ` with its space, an
    /// indented `!` and a line ending in a space.
    const PRINTED: &str = "!\nversion 17.6\nservice timestamps debug datetime msec\n! Call-home is enabled by Smart-Licensing.\nservice call-home\n!\nhostname Lab\n!\n!\n!\nvrf definition Mgmt-vrf\n !\n address-family ipv4\n exit-address-family\n!\n! \n!\nspanning-tree mode rapid-pvst\n!\nclass-map match-any system-cpp-default\n  description EWLC Data, Inter FED Traffic \n!\n!\nline con 0\n stopbits 1\n!\n!\n!\nend";

    #[test]
    fn a_replayed_configuration_prints_its_own_layout_and_keeps_it_around_added_lines() {
        let e = engine("c9300", "17.6");
        let mut s = shell::Session::new(
            &e,
            device::Device::new(platform::parse_stack("C9300-24T").unwrap()),
        );
        crate::check::type_into(&mut s, PRINTED);
        let run = configuration(&s.device);
        let at = |l: &str| run.iter().position(|x| x == l).unwrap();
        // the printed lines between `version` and `end` come back as printed
        let commands = |lines: Vec<&str>| -> Vec<String> {
            lines
                .into_iter()
                .filter(|l| !l.starts_with("interface ") && !l.starts_with('!'))
                .map(str::to_string)
                .collect()
        };
        assert_eq!(
            commands(
                run[at("version 17.6")..]
                    .iter()
                    .map(String::as_str)
                    .collect()
            ),
            commands(PRINTED.lines().skip(1).collect())
        );
        assert_eq!(
            run[at("service call-home") - 1],
            "! Call-home is enabled by Smart-Licensing."
        );
        assert_eq!(
            run[at("vrf definition Mgmt-vrf") - 3..at("vrf definition Mgmt-vrf")],
            ["!", "!", "!"]
        );
        assert_eq!(run[at(" address-family ipv4") - 1], " !");
        assert_eq!(run[at("class-map match-any system-cpp-default") - 1], "!");
        assert!(run.contains(&"  description EWLC Data, Inter FED Traffic ".to_string()));
        assert!(run.contains(&"! ".to_string()));
        assert_eq!(run[at("end") - 3..at("end")], ["!", "!", "!"]);
        // a line added between two printed ones takes the printed `!` lines before it
        s.type_line("configure terminal");
        s.type_line("ip domain name example.com");
        s.type_line("end");
        let run = configuration(&s.device);
        let at = |l: &str| run.iter().position(|x| x == l).unwrap();
        assert!(at("hostname Lab") < at("ip domain name example.com"));
    }

    #[test]
    fn a_line_added_to_a_replayed_configuration_follows_the_replayed_lines_ios_prints_before_it() {
        // the lab WS-C2960G-8TC-L (15.0(2)SE11) printed a new MST region after its spanning-tree
        // lines, where vswitch computes it after the MAC list before them
        let e = engine("c2960", "15.0(2)SE");
        let mut s = shell::Session::new(
            &e,
            device::Device::new(platform::parse_stack("WS-C2960G-8TC-L").unwrap()),
        );
        let body = [
            "!",
            "version 15.0",
            "!",
            "hostname Lab",
            "!",
            "mac access-list extended M1",
            "spanning-tree mode pvst",
            "spanning-tree extend system-id",
            "!",
            "vlan internal allocation policy ascending",
            "!",
            "end",
        ];
        crate::check::type_into(&mut s, &(body.join("\n") + "\n"));
        for l in ["spanning-tree mst configuration", "name LAB", "end"] {
            s.type_line(l);
        }
        let run = configuration(&s.device);
        let at = |l: &str| run.iter().position(|x| x == l).unwrap();
        assert!(at("mac access-list extended M1") < at("spanning-tree mode pvst"));
        assert!(at("spanning-tree extend system-id") < at("spanning-tree mst configuration"));
        assert!(
            at("spanning-tree mst configuration") < at("vlan internal allocation policy ascending")
        );
    }

    #[test]
    fn at_17_6_the_last_change_is_named_and_counted() {
        let e = engine("c9300", "17.6");
        let mut s = typed(&e, "C9300-24T", &["hostname Lab"]);
        // 2026-03-05 12:34:56 UTC
        s.device.last_change = Some(1_772_714_096);
        let run = running_config(&s.device);
        assert_eq!(
            run[3..7],
            [
                "!",
                "! Last configuration change at 12:34:56 UTC Thu Mar 5 2026",
                "!",
                "version 17.6"
            ]
        );
        let bytes: usize = run[3..].iter().map(|l| l.len() + 1).sum::<usize>();
        assert_eq!(run[2], format!("Current configuration : {bytes} bytes"));
        // a typed line moves it to the switch's clock
        s.clock = Some(1_772_714_160);
        s.type_line("configure terminal");
        s.type_line("hostname Lab2");
        s.type_line("end");
        assert_eq!(s.device.last_change, Some(1_772_714_160));
        // 17.15 prints no such line
        let e = engine("c9300", "17.15");
        let mut s = typed(&e, "C9300-24T", &["hostname Lab"]);
        s.device.last_change = Some(1_772_714_096);
        assert_eq!(configuration(&s.device)[1], "version 17.15");
    }

    #[test]
    fn a_replayed_last_change_keeps_its_user_until_a_line_is_typed() {
        let e = engine("c9300", "17.6");
        let stack = platform::parse_stack("C9300-24T").unwrap();
        let mut s = shell::Session::new(&e, device::Device::new(stack));
        let config = "!\n! Last configuration change at 12:34:56 UTC Thu Mar 5 2026 by admin\n!\nversion 17.6\nhostname Lab\n!\nend\n";
        crate::check::type_into(&mut s, config);
        s.type_line("end");
        assert_eq!(
            configuration(&s.device)[1],
            "! Last configuration change at 12:34:56 UTC Thu Mar 5 2026 by admin"
        );
        s.clock = Some(1_772_714_160);
        s.type_line("configure terminal");
        s.type_line("hostname Lab2");
        s.type_line("end");
        assert_eq!(
            configuration(&s.device)[1],
            "! Last configuration change at 12:36:00 UTC Thu Mar 5 2026"
        );
    }

    #[test]
    fn vlan_runs_split_at_each_vlan_with_lines_and_a_suspended_vlan_lists_suspended() {
        // the lab C9300-24T (17.6.4): `vlan 100-119 `, `vlan 120` with its name, `vlan 121-139 `;
        // `vlan 151` with ` state suspend`, listed `suspended`
        let e = engine("c9300", "17.6");
        let mut s = typed(
            &e,
            "C9300-24T",
            &[
                "vtp mode transparent",
                "vlan 100-106",
                "vlan 103",
                "name NAMED",
                "vlan 105",
                "state suspend",
                "vlan 106",
                "state suspend",
                "state active",
            ],
        );
        let run = configuration(&s.device);
        let at = run.iter().position(|l| l.starts_with("vlan 100")).unwrap();
        assert_eq!(
            run[at..at + 12],
            [
                "vlan 100-102 ",
                "!",
                "vlan 103",
                " name NAMED",
                "!",
                "vlan 104 ",
                "!",
                "vlan 105",
                " state suspend",
                "!",
                "vlan 106 ",
                "!"
            ]
        );
        let brief = s.type_line("show vlan brief").0;
        assert_eq!(
            row(&brief, "103"),
            "103  NAMED                            active    "
        );
        assert_eq!(
            row(&brief, "105"),
            "105  VLAN0105                         suspended "
        );
        assert_eq!(
            row(&brief, "106"),
            "106  VLAN0106                         active    "
        );
    }

    #[test]
    fn vlan_brief_lists_voice_ports_and_extended_vlans_and_leaves_out_a_span_destination() {
        // the lab C9300-24T (17.6.4): Gi1/0/14 under its access VLAN and its voice VLAN; VLAN 4000
        // after 1005 and not counted by VTP; the SPAN destination Gi1/0/21 under no VLAN; the source
        // printed by its short name
        let e = engine("c9300", "17.6");
        let mut s = typed(
            &e,
            "C9300-24T",
            &[
                "vtp mode transparent",
                "vlan 100",
                "vlan 103",
                "vlan 4000",
                "name EXT",
                "interface GigabitEthernet1/0/14",
                "switchport access vlan 100",
                "switchport mode access",
                "switchport voice vlan 103",
                "exit",
                "monitor session 1 source interface GigabitEthernet1/0/11",
                "monitor session 1 destination interface GigabitEthernet1/0/21",
            ],
        );
        let brief = s.type_line("show vlan brief").0;
        assert_eq!(
            row(&brief, "100"),
            "100  VLAN0100                         active    Gi1/0/14"
        );
        assert_eq!(
            row(&brief, "103"),
            "103  VLAN0103                         active    Gi1/0/14"
        );
        assert_eq!(
            brief[brief.len() - 2..],
            [
                "1005 trnet-default                    act/unsup ",
                "4000 EXT                              active    "
            ]
        );
        assert!(!brief.iter().any(|l| l.contains("Gi1/0/21")), "{brief:?}");
        let vtp = s.type_line("show vtp status").0;
        assert!(
            vtp.contains(&"Number of existing VLANs          : 7".to_string()),
            "{vtp:?}"
        );
        let run = configuration(&s.device);
        for l in [
            "monitor session 1 source interface Gi1/0/11",
            "monitor session 1 destination interface Gi1/0/21",
        ] {
            assert!(run.contains(&l.to_string()), "{l}: {run:?}");
        }
    }

    #[test]
    fn access_lists_print_as_the_lab_c9300_printed_them_at_17_6() {
        // the lab C9300-24T (17.6.4): the running configuration pads `deny` in both kinds of list,
        // `show ip access-lists` in a standard one only; an IPv6 entry typed bare takes the next
        // sequence; `show access-lists` adds the IPv6 lists, then the MAC lists
        let e = engine("c9300", "17.6");
        let mut s = typed(
            &e,
            "C9300-24T",
            &[
                "ip access-list standard S1",
                "permit 10.0.0.0 0.0.0.255",
                "deny any log",
                "ip access-list extended E1",
                "deny ip any any log",
                "ipv6 access-list V6",
                "permit ipv6 2001:DB8::/32 any",
                "deny ipv6 any any",
                "mac access-list extended M1",
                "permit host 0011.2233.4455 any",
            ],
        );
        let run = configuration(&s.device);
        assert_eq!(
            block_of(&run, "ip access-list standard S1"),
            [
                "ip access-list standard S1",
                " 10 permit 10.0.0.0 0.0.0.255",
                " 20 deny   any log"
            ]
        );
        assert_eq!(
            block_of(&run, "ip access-list extended E1"),
            ["ip access-list extended E1", " 10 deny   ip any any log"]
        );
        assert_eq!(
            block_of(&run, "ipv6 access-list V6"),
            [
                "ipv6 access-list V6",
                " sequence 10 permit ipv6 2001:DB8::/32 any",
                " sequence 20 deny ipv6 any any"
            ]
        );
        assert_eq!(
            access_lists(&s.device, Some("S1")),
            [
                "Standard IP access list S1",
                "    10 permit 10.0.0.0, wildcard bits 0.0.0.255",
                "    20 deny   any log"
            ]
        );
        assert_eq!(
            access_lists(&s.device, Some("E1")),
            ["Extended IP access list E1", "    10 deny ip any any log"]
        );
        let all = s.type_line("show access-lists").0;
        let at = |l: &str| all.iter().position(|x| x == l).unwrap();
        assert_eq!(
            all[at("IPv6 access list V6")..at("IPv6 access list V6") + 3],
            [
                "IPv6 access list V6",
                "    permit ipv6 2001:DB8::/32 any sequence 10",
                "    deny ipv6 any any sequence 20"
            ]
        );
        assert_eq!(
            all[all.len() - 2..],
            [
                "Extended MAC access list M1 ",
                "    permit host 0011.2233.4455 any"
            ]
        );
        // the lists IOS XE makes itself, standard lists first
        assert_eq!(all[0], "Standard IP access list S1");
        assert!(at("Extended IP access list E1") < at("Extended IP access list implicit_deny"));
        assert!(at("Extended IP access list preauth_v4") < at("IPv6 access list V6"));
        assert!(at("IPv6 access list V6") < at("IPv6 access list implicit_deny_v6"));
        assert_eq!(
            access_lists(&s.device, Some("IP-Adm-V4-Int-ACL-global")),
            [
                "Extended IP access list IP-Adm-V4-Int-ACL-global",
                "    10 permit tcp any any eq www",
                "    20 permit tcp any any eq 443"
            ]
        );
        // the lab C9300-24T (17.6.4) after `no ip http server` and `no ip http secure-server`; the
        // running configuration's lists, standard then extended, by name
        for l in [
            "configure terminal",
            "no ip http server",
            "no ip http secure-server",
            "ip access-list extended ZZ-FIRST",
            "permit ip any any",
            "ip access-list extended AA-SECOND",
            "permit ip any any",
            "end",
        ] {
            s.type_line(l);
        }
        assert_eq!(
            access_lists(&s.device, Some("IP-Adm-V4-Int-ACL-global")),
            ["Extended IP access list IP-Adm-V4-Int-ACL-global"]
        );
        let lists: Vec<String> = configuration(&s.device)
            .into_iter()
            .filter(|l| l.starts_with("ip access-list "))
            .collect();
        assert_eq!(
            lists,
            [
                "ip access-list standard S1",
                "ip access-list extended AA-SECOND",
                "ip access-list extended E1",
                "ip access-list extended ZZ-FIRST"
            ]
        );
    }

    #[test]
    fn tunnels_bluetooth_and_a_provisioned_member_print_as_the_lab_c9300_printed_them() {
        // the lab C9300-24T (17.6.4) with member 2 provisioned and absent: its ports print and take
        // lines, run nowhere; Tunnel0 up and down with its destination unreachable, after the
        // port-channels; Bluetooth0/4 made by typing it
        let e = engine("c9300", "17.6");
        let mut s = typed(&e, "C9300-24T", &[]);
        assert_eq!(
            s.type_line("show running-config interface Bluetooth0/4").0,
            [
                "Building configuration...",
                "",
                "Current configuration : 5 bytes",
                "end",
                ""
            ]
        );
        assert!(
            !ip_interface_brief(&s.device)
                .iter()
                .any(|l| l.starts_with("Bluetooth"))
        );
        for l in [
            "configure terminal",
            "switch 2 provision c9300-48p",
            "interface GigabitEthernet2/0/1",
            "description M2",
            "interface Port-channel5",
            "interface Loopback0",
            "ip address 10.9.9.9 255.255.255.255",
            "interface Tunnel0",
            "ip address 10.9.0.1 255.255.255.0",
            "tunnel source Loopback0",
            "tunnel destination 10.8.8.8",
            "interface Bluetooth0/4",
            "shutdown",
            "end",
        ] {
            let (o, r) = s.type_line(l);
            assert_eq!(r, shell::Result::Ok, "{l}: {o:?}");
        }
        let run = configuration(&s.device);
        let at = |l: &str| {
            run.iter()
                .position(|x| x == l)
                .unwrap_or_else(|| panic!("{l}: {run:?}"))
        };
        assert!(at("interface Loopback0") < at("interface Port-channel5"));
        assert!(at("interface Port-channel5") < at("interface Tunnel0"));
        assert!(at("interface Tunnel0") < at("interface GigabitEthernet0/0"));
        assert!(at("interface AppGigabitEthernet2/0/1") < at("interface Bluetooth0/4"));
        assert_eq!(
            block_of(&run, "interface GigabitEthernet2/0/1"),
            ["interface GigabitEthernet2/0/1", " description M2"]
        );
        assert!(run.contains(&"interface GigabitEthernet2/0/48".to_string()));
        let brief = ip_interface_brief(&s.device);
        for want in [
            "GigabitEthernet2/0/1   unassigned      YES unset  down                  down    ",
            "Bluetooth0/4           unassigned      YES unset  administratively down down    ",
            "Tunnel0                10.9.0.1        YES manual up                    down    ",
        ] {
            assert!(brief.contains(&want.to_string()), "{want}: {brief:?}");
        }
        assert!(
            !interfaces_status(&s.device)
                .iter()
                .any(|l| l.starts_with("Gi2/")),
        );
    }

    #[test]
    fn the_lines_the_lab_c9300_took_and_the_book_lines_refused_print_as_typed() {
        // each line the lab C9300-24T (17.6.4) took and printed, which the books' lines refused
        let e = engine("c9300", "17.6");
        let mut s = typed(
            &e,
            "C9300-24T",
            &[
                "aaa new-model",
                "aaa accounting network TESTACCT",
                "ip dhcp pool P1",
                "network 10.99.20.0 255.255.255.0",
                "default-router 10.99.20.1",
                "dns-server 10.99.20.5",
                "device-tracking policy DT1",
                "no protocol udp",
                "table-map TM1",
                "default copy",
                "l2vpn",
                "object-group network OG1",
                "10.99.0.0 255.255.0.0",
                "mac access-list extended M1",
                "permit host 0011.2233.4455 any",
                "service-list mdns-sd SL2 query",
                "mdns-sd service-list SLI IN",
                "mdns-sd service-list SLO OUT",
                "class-map match-any CM1",
                "match dscp cs4  cs5  ef",
                "match precedence 4  5",
                "policy-map PM1",
                "class CM1",
                "bandwidth remaining percent 30",
                "queue-limit dscp 10 percent 20",
                "queue-limit dscp 20 percent 30",
                "macro auto mac-address-group MAG1",
                "interface GigabitEthernet1/0/3",
                "ip dhcp snooping trust",
                "interface GigabitEthernet1/0/7.10",
                "encapsulation dot1Q 10",
                "interface GigabitEthernet1/0/15",
                "no switchport",
                "ip ospf 1 area 0",
                "exit",
                "ip rcmd rcp-enable",
                "ip rcmd rsh-enable",
                "route-map RM1 permit 10",
                "set metric 100",
                "performance-measurement",
                "audit",
                "app-hosting appid APP1",
                "app-vnic AppGigabitEthernet trunk",
                "vlan 10 guest-interface 1",
                "app-vnic gateway0 virtualportgroup 0 guest-interface 0",
                "app-vnic management guest-interface 0",
            ],
        );
        let run = configuration(&s.device);
        for (header, block) in [
            (
                "ip dhcp pool P1",
                &[
                    " network 10.99.20.0 255.255.255.0",
                    " default-router 10.99.20.1",
                    " dns-server 10.99.20.5",
                ][..],
            ),
            ("device-tracking policy DT1", &[" no protocol udp"][..]),
            ("table-map TM1", &[" default copy"][..]),
            ("object-group network OG1", &[" 10.99.0.0 255.255.0.0"][..]),
            (
                "mac access-list extended M1",
                &[" permit host 0011.2233.4455 any"][..],
            ),
            (
                "class-map match-any CM1",
                &[" match dscp cs4  cs5  ef ", " match precedence 4  5 "][..],
            ),
            (
                "policy-map PM1",
                &[
                    " class CM1",
                    "  bandwidth remaining percent 30",
                    "  queue-limit dscp 10 percent 20",
                    "  queue-limit dscp 20 percent 30",
                ][..],
            ),
            (
                "interface GigabitEthernet1/0/7.10",
                &[" encapsulation dot1Q 10"][..],
            ),
            (
                "app-hosting appid APP1",
                &[
                    " app-vnic AppGigabitEthernet trunk",
                    "  vlan 10 guest-interface 1",
                    " app-vnic gateway0 virtualportgroup 0 guest-interface 0",
                    " app-vnic management guest-interface 0",
                ][..],
            ),
        ] {
            let mut want = vec![header.to_string()];
            want.extend(block.iter().map(|l| l.to_string()));
            assert_eq!(block_of(&run, header), want, "{header}");
        }
        for l in [
            "aaa accounting network TESTACCT",
            "l2vpn",
            "service-list mdns-sd SL2 query",
            "mdns-sd service-list SLI IN",
            "mdns-sd service-list SLO OUT",
            "macro auto mac-address-group MAG1",
            "ip rcmd rcp-enable",
            "ip rcmd rsh-enable",
            "performance-measurement",
            "audit",
        ] {
            assert!(run.contains(&l.to_string()), "{l}: {run:?}");
        }
        assert!(
            block_of(&run, "interface GigabitEthernet1/0/3")
                .contains(&" ip dhcp snooping trust".to_string())
        );
        assert!(
            block_of(&run, "interface GigabitEthernet1/0/15")
                .contains(&" ip ospf 1 area 0".to_string())
        );
        // the mode each enters, as the lab C9300 prompted
        for (line, mode) in [
            ("ip dhcp pool P2", "dhcp-config"),
            ("mdns-sd service-list SLI IN", "config-mdns-sl-in"),
            ("mdns-sd service-list SLO OUT", "config-mdns-sl-out"),
            ("table-map TM2", "config-tablemap"),
        ] {
            s.type_line("configure terminal");
            s.type_line(line);
            assert_eq!(s.mode(), mode, "{line}");
            s.type_line("end");
        }
        // gleaning from a protocol again is the default, which prints nothing
        for l in [
            "configure terminal",
            "device-tracking policy DT1",
            "protocol udp",
            "end",
        ] {
            s.type_line(l);
        }
        assert_eq!(
            block_of(&configuration(&s.device), "device-tracking policy DT1"),
            ["device-tracking policy DT1"]
        );
    }

    #[test]
    fn routing_sections_nest_and_close_as_the_lab_c9300_printed_them() {
        // the lab C9300-24T (17.6.4): EIGRP named mode's families and topologies, a VRRP group's
        // address two spaces in, `bmp server` first in `router bgp`, ` no neighbor <x> activate`
        // kept under the IPv4 family, a route map after `ipv6 router ospf` at the top level
        let e = engine("c9300", "17.6");
        let s = typed(
            &e,
            "C9300-24T",
            &[
                "fhrp version vrrp v3",
                "interface Vlan20",
                "ip address 10.20.0.1 255.255.255.0",
                "vrrp 20 address-family ipv4",
                "address 10.20.0.254 primary",
                "exit-vrrp",
                "exit",
                "router eigrp NAMED",
                "address-family ipv4 unicast autonomous-system 100",
                "topology base",
                "exit-af-topology",
                "exit-address-family",
                "service-family ipv4 autonomous-system 200",
                "topology base",
                "exit-sf-topology",
                "exit-service-family",
                "exit",
                "router bgp 65000",
                "bmp server 1",
                "exit-bmp-server-mode",
                "template peer-policy PP1",
                "exit-peer-policy",
                "template peer-session PS1",
                "exit-peer-session",
                "neighbor 10.0.0.2 remote-as 65001",
                "address-family ipv4",
                "no neighbor 10.0.0.2 activate",
                "exit-address-family",
                "exit",
                "ipv6 router ospf 1",
                "route-map RM1 permit 10",
                "set metric 100",
            ],
        );
        let run = configuration(&s.device);
        // the section separators IOS prints between them aside
        let of = |header: &str| -> Vec<String> {
            block_of(&run, header)
                .into_iter()
                .filter(|l| l.trim() != "!")
                .collect()
        };
        assert_eq!(
            of("interface Vlan20"),
            [
                "interface Vlan20",
                " ip address 10.20.0.1 255.255.255.0",
                " vrrp 20 address-family ipv4",
                "  address 10.20.0.254 primary",
                "  exit-vrrp"
            ]
        );
        assert_eq!(
            of("router eigrp NAMED"),
            [
                "router eigrp NAMED",
                " address-family ipv4 unicast autonomous-system 100",
                "  topology base",
                "  exit-af-topology",
                " exit-address-family",
                " service-family ipv4 autonomous-system 200",
                "  topology base",
                "  exit-sf-topology",
                " exit-service-family"
            ]
        );
        assert_eq!(
            of("router bgp 65000"),
            [
                "router bgp 65000",
                " bmp server 1",
                " exit-bmp-server-mode",
                " template peer-policy PP1",
                " exit-peer-policy",
                " template peer-session PS1",
                " exit-peer-session",
                " neighbor 10.0.0.2 remote-as 65001",
                " address-family ipv4",
                "  no neighbor 10.0.0.2 activate",
                " exit-address-family"
            ]
        );
        assert_eq!(of("ipv6 router ospf 1"), ["ipv6 router ospf 1"]);
        let map = run
            .iter()
            .position(|l| l.starts_with("route-map RM1 permit 10"))
            .unwrap();
        assert_eq!(run[map + 1], " set metric 100");
    }

    #[test]
    fn a_replayed_configuration_keeps_its_comments_blank_lines_spacing_and_order() {
        // the lab C9300-24T (17.6.4) printed IOS comments in and out of blocks, blank lines after
        // `udld enable`, `match cos  5 ` and its own order of top-level items
        let e = engine("c9300", "17.6");
        let mut s = shell::Session::new(
            &e,
            device::Device::new(platform::parse_stack("C9300-24T").unwrap()),
        );
        let body = [
            "!",
            "version 17.6",
            "!",
            "hostname Lab",
            "!",
            "udld enable",
            "",
            "",
            "ip routing",
            "!",
            "crypto ikev2 policy POL1",
            " ! Policy Incomplete(no proposal)",
            "!",
            "! Incomplete EVPN instance config, no vlan configured",
            "class-map match-any CM1",
            " match cos  5 ",
            "!",
            "interface GigabitEthernet1/0/1",
            " description Uplink",
            "!",
            "interface GigabitEthernet1/0/2",
            " description UPLINK",
            "!",
            "end",
        ];
        crate::check::type_into(&mut s, &(body.join("\n") + "\n"));
        s.type_line("end");
        let run = configuration(&s.device);
        let at = |l: &str| run.iter().position(|x| x == l).unwrap();
        let printed: Vec<&str> = run[at("udld enable")..at("udld enable") + 3]
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(printed, ["udld enable", "", ""]);
        assert_eq!(
            run[at("crypto ikev2 policy POL1") + 1],
            " ! Policy Incomplete(no proposal)"
        );
        assert_eq!(
            run[at("class-map match-any CM1") - 1],
            "! Incomplete EVPN instance config, no vlan configured"
        );
        assert_eq!(run[at("class-map match-any CM1") + 1], " match cos  5 ");
        assert!(at("ip routing") < at("crypto ikev2 policy POL1"));
        // two lines that differ only in case each print as printed
        for (port, text) in [("1", " description Uplink"), ("2", " description UPLINK")] {
            assert_eq!(
                run[at(&format!("interface GigabitEthernet1/0/{port}")) + 1],
                text
            );
        }
    }

    #[test]
    fn a_named_eigrp_process_at_network_essentials_prints_as_a_stub() {
        let lines = |l: &[&str]| -> Vec<String> { l.iter().map(|x| x.to_string()).collect() };
        let process = lines(&[
            "router eigrp NAMED",
            " !",
            " address-family ipv4 unicast autonomous-system 100",
            "  network 10.0.0.0",
            " exit-address-family",
            " !",
            " service-family ipv4 autonomous-system 200",
            " exit-service-family",
            " !",
            " address-family ipv6 unicast autonomous-system 300",
            " exit-address-family",
        ]);
        let stub = eigrp_stub(process);
        assert_eq!(
            stub,
            lines(&[
                "router eigrp NAMED",
                " !",
                " address-family ipv4 unicast autonomous-system 100",
                "  network 10.0.0.0",
                "  eigrp stub connected summary",
                " exit-address-family",
                " !",
                " service-family ipv4 autonomous-system 200",
                "  eigrp stub connected",
                " exit-service-family",
                " !",
                " address-family ipv6 unicast autonomous-system 300",
                " exit-address-family",
            ])
        );
        // a family that states its stub keeps it
        let stated = lines(&[
            "router eigrp NAMED",
            " address-family ipv4 unicast autonomous-system 100",
            "  eigrp stub receive-only",
            " exit-address-family",
        ]);
        assert_eq!(eigrp_stub(stated.clone()), stated);
    }
}
