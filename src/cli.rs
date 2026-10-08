//! Matching a typed line against the grammar, as IOS does: keywords by any prefix that names one
//! keyword at that point, an exact keyword ahead of a longer one it prefixes, a keyword ahead of
//! a free word, `no` forms, and the error IOS prints when nothing matches.

use crate::compile::{ArgKind, Command, Grammar};
use crate::syntax::Node;
use std::collections::{BTreeMap, BTreeSet};

/// The keywords `words` name when they are a `no` form of the line (its own `no` left off): every
/// keyword of the command, each by an abbreviation, up to its first argument or choice of values
/// (`no ip address`, `no spanning-tree bpdufilter`). An optional group between them is passed over
/// (`no ip pim rp-address` for `ip pim [vrf <name>] rp-address <address>`), and of a choice of further
/// keywords the one typed is followed. Fewer words name no command: `no ip`, `no spanning-tree`.
fn no_form(node: &Node, words: &[Tok]) -> Option<Vec<String>> {
    let mut items: Vec<&Node> = match node {
        Node::Seq(v) => v.iter().collect(),
        n => vec![n],
    };
    if matches!(items.first(), Some(Node::Kw(k)) if k.eq_ignore_ascii_case("no")) {
        items.remove(0);
    }
    if words.is_empty() {
        return None;
    }
    no_form_in(&items, words)
}

/// `no_form` over what is left of a line, its groups opened in place.
fn no_form_in(items: &[&Node], words: &[Tok]) -> Option<Vec<String>> {
    let Some((first, after)) = items.split_first() else {
        return words.is_empty().then(Vec::new);
    };
    match first {
        Node::Kw(k) => {
            let w = words.first()?;
            if !k.to_lowercase().starts_with(&w.text.to_lowercase()) {
                return None;
            }
            let mut out = vec![k.clone()];
            out.extend(no_form_in(after, &words[1..])?);
            Some(out)
        }
        Node::Arg(_) => words.is_empty().then(Vec::new),
        Node::Opt(_) => no_form_in(after, words),
        Node::Seq(inner) => {
            let mut v: Vec<&Node> = inner.iter().collect();
            v.extend_from_slice(after);
            no_form_in(&v, words)
        }
        Node::Alt(branches) if words.is_empty() => branches.iter().all(value).then(Vec::new),
        Node::Alt(branches) => branches.iter().find_map(|b| {
            let mut v: Vec<&Node> = match b {
                Node::Seq(inner) => inner.iter().collect(),
                n => vec![n],
            };
            v.extend_from_slice(after);
            no_form_in(&v, words)
        }),
    }
}

/// Whether a branch of a choice is a value the command takes (`enable`, `auto [<speed>]`, `<vlan-id>`)
/// rather than a further command (`bpdufilter {enable | disable}`, `prefix-list <name>`).
fn value(n: &Node) -> bool {
    match n {
        Node::Kw(_) | Node::Arg(_) | Node::Opt(_) => true,
        Node::Alt(v) => v.iter().all(value),
        Node::Seq(v) => match v.split_first() {
            None => true,
            Some((Node::Arg(_), _)) => true,
            Some((Node::Kw(_), rest)) => rest.iter().all(|x| matches!(x, Node::Opt(_))),
            Some(_) => false,
        },
    }
}

/// A token of the typed line and where it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Tok {
    pub text: String,
    pub at: usize,
}

pub fn tokens(line: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in line.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                out.push(Tok {
                    text: line[s..i].to_string(),
                    at: s,
                });
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push(Tok {
            text: line[s..].to_string(),
            at: s,
        });
    }
    out
}

/// One matched element: a keyword (as the grammar spells it) or an argument and its value.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Kw {
        kw: String,
        tok: usize,
        exact: bool,
    },
    Arg {
        name: String,
        value: String,
        tok: usize,
        /// a stated type (interface, number, address), not a free word
        typed: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Parse {
    pub command: usize,
    pub line: usize,
    pub negated: bool,
    pub items: Vec<Item>,
}

impl Parse {
    /// the line with keywords written out in full, as the running configuration shows it
    pub fn canonical(&self) -> String {
        let words: Vec<String> = self
            .items
            .iter()
            .map(|i| match i {
                Item::Kw { kw, .. } => kw.clone(),
                Item::Arg { value, .. } => value.clone(),
            })
            .collect();
        let body = words.join(" ");
        if self.negated && !body.starts_with("no ") {
            format!("no {body}")
        } else {
            body
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Ok(Parse),
    /// `% Invalid input detected at '^' marker.` with the caret under this byte of the line
    Invalid(usize),
    /// `% Incomplete command.`
    Incomplete,
    /// `% Ambiguous command:  "<line>"`
    Ambiguous,
}

pub fn interface_types() -> &'static [(&'static str, &'static str)] {
    // full name, shortest abbreviation IOS shows
    &[
        ("AppGigabitEthernet", "Ap"),
        // a 2960-X's management port, FastEthernet0
        ("FastEthernet", "Fa"),
        ("FiveGigabitEthernet", "Fi"),
        ("FortyGigabitEthernet", "Fo"),
        ("GigabitEthernet", "Gi"),
        ("HundredGigE", "Hu"),
        ("Loopback", "Lo"),
        ("Port-channel", "Po"),
        ("TenGigabitEthernet", "Te"),
        ("Tunnel", "Tu"),
        ("TwentyFiveGigE", "Twe"),
        // 2.5G mGig ports (C9300-48UXM); IOS prints `Tw1/0/1` (9300 command reference)
        ("TwoGigabitEthernet", "Tw"),
        ("Vlan", "Vl"),
    ]
}

/// `TwentyFiveGigE1/0/1`, `twe1/0/1` or `Twe` + `1/0/1`: the full type name and the number.
pub fn interface_name(first: &str, second: Option<&str>) -> Option<(String, String, usize)> {
    let split = first
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(first.len());
    let (ty, num) = first.split_at(split);
    if ty.is_empty() {
        return None;
    }
    let matches: Vec<&str> = interface_types()
        .iter()
        .filter(|(full, _)| full.to_lowercase().starts_with(&ty.to_lowercase()))
        .map(|(full, _)| *full)
        .collect();
    // `T` fits Ten, Tunnel and TwentyFive; IOS needs enough letters for one
    let full = match matches.as_slice() {
        [one] => *one,
        many => many.iter().find(|f| f.eq_ignore_ascii_case(ty)).copied()?,
    };
    let numbered = |n: &str| {
        !n.is_empty()
            && n.chars()
                .all(|c| c.is_ascii_digit() || c == '/' || c == '.')
    };
    if !num.is_empty() {
        return numbered(num).then(|| (full.to_string(), num.to_string(), 1));
    }
    let n = second?;
    numbered(n).then(|| (full.to_string(), n.to_string(), 2))
}

/// The most numbers one segment of an interface range spans: a VLAN range's, the widest an interface
/// type IOS ranges over has (`interface range vlan 1 - 4094`).
const RANGE_SPAN: u32 = 4094;

/// `Twe1/0/1 - 8, Twe2/0/1 - 8`: comma-separated segments, each an interface and an optional
/// `- <last port>`. The value is written as IOS shows it: `TwentyFiveGigE1/0/1 - 8 , ...`.
pub fn interface_range(text: &str) -> Option<String> {
    let mut out = Vec::new();
    for seg in text.split(',') {
        let seg = seg.trim();
        // the range's hyphen follows the first port's number: `Port-channel` holds one of its own
        let number = seg.find(|c: char| c.is_ascii_digit()).unwrap_or(seg.len());
        let (head, last) = match seg[number..].find('-') {
            Some(i) => (seg[..number + i].trim(), Some(seg[number + i + 1..].trim())),
            None => (seg, None),
        };
        let words: Vec<&str> = head.split_whitespace().collect();
        let (full, num, used) = interface_name(words.first()?, words.get(1).copied())?;
        if used != words.len() {
            return None;
        }
        match last {
            Some(l) => {
                let first_port: u32 = num.rsplit('/').next()?.parse().ok()?;
                let last_port: u32 = digits(l)?.parse().ok()?;
                if last_port < first_port || last_port - first_port >= RANGE_SPAN {
                    return None;
                }
                out.push(format!("{full}{num} - {last_port}"));
            }
            None => out.push(format!("{full}{num}")),
        }
    }
    (!out.is_empty()).then(|| out.join(" , "))
}

/// `t` when it is a decimal number as IOS reads one: digits only, no sign.
fn digits(t: &str) -> Option<&str> {
    (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit())).then_some(t)
}

fn ipv4(t: &str) -> bool {
    let parts: Vec<&str> = t.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            p.len() <= 3 && digits(p).is_some_and(|d| d.parse::<u16>().is_ok_and(|v| v <= 255))
        })
}

/// A VLAN list (`10,20-30`), each VLAN 1 to 4094, written as IOS writes it (`010` is `10`).
fn vlan_list(t: &str) -> Option<String> {
    if t.is_empty() {
        return None;
    }
    let vlan = |x: &str| {
        digits(x)?
            .parse::<u16>()
            .ok()
            .filter(|v| (1..=4094).contains(v))
    };
    let mut parts = Vec::new();
    for part in t.split(',') {
        parts.push(match part.split_once('-') {
            None => vlan(part)?.to_string(),
            Some((a, b)) => format!("{}-{}", vlan(a)?, vlan(b)?),
        });
    }
    Some(parts.join(","))
}

/// How many tokens an argument takes at `i`, and its value; none when it does not fit. A replayed
/// configuration (`redacted`) takes the secret sweep's `<removed>` for any argument but an
/// interface: the value it stood for was there.
fn take_arg(kind: &ArgKind, toks: &[Tok], i: usize, redacted: bool) -> Option<(usize, String)> {
    let t = toks.get(i)?;
    if redacted
        && t.text == crate::sweep::REMOVED
        && !matches!(kind, ArgKind::Interface | ArgKind::InterfaceRange)
    {
        return Some((1, t.text.clone()));
    }
    match kind {
        ArgKind::Number { min, max } => {
            // IOS takes digits, a minus first where the range goes below zero, and writes the number
            // plain (`010` is `10`); Rust's parse would also take `+10`
            let unsigned = match t.text.strip_prefix('-') {
                Some(d) if *min < 0 => d,
                _ => &t.text,
            };
            digits(unsigned)?;
            let v: i64 = t.text.parse().ok()?;
            (v >= *min && v <= *max).then(|| (1, v.to_string()))
        }
        ArgKind::VlanList => vlan_list(&t.text).map(|v| (1, v)),
        ArgKind::Interface => {
            let (full, num, n) = interface_name(&t.text, toks.get(i + 1).map(|x| x.text.as_str()))?;
            Some((n, format!("{full}{num}")))
        }
        ArgKind::Ipv4 => ipv4(&t.text).then(|| (1, t.text.clone())),
        ArgKind::Ipv6 => t.text.contains(':').then(|| (1, t.text.clone())),
        ArgKind::Mac => {
            let parts: Vec<&str> = t.text.split('.').collect();
            (parts.len() == 3
                && parts
                    .iter()
                    .all(|p| p.len() == 4 && p.chars().all(|c| c.is_ascii_hexdigit())))
            .then(|| (1, t.text.clone()))
        }
        ArgKind::Text => {
            // the rest of the line with its spacing: IOS keeps a LINE argument as typed
            let mut text = t.text.clone();
            for pair in toks[i..].windows(2) {
                let gap = pair[1].at.saturating_sub(pair[0].at + pair[0].text.len());
                text.push_str(&" ".repeat(gap.max(1)));
                text.push_str(&pair[1].text);
            }
            Some((toks.len() - i, text))
        }
        ArgKind::InterfaceRange => {
            let rest: Vec<&str> = toks[i..].iter().map(|x| x.text.as_str()).collect();
            interface_range(&rest.join(" ")).map(|v| (toks.len() - i, v))
        }
        ArgKind::Word => Some((1, t.text.clone())),
        ArgKind::Dscp => dscp(&t.text).then(|| (1, t.text.clone())),
    }
}

/// A DSCP value as IOS takes it: 0 to 63, or one of its names.
fn dscp(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    digits(&t).is_some_and(|d| d.parse::<u8>().is_ok_and(|v| v <= 63))
        || matches!(t.as_str(), "ef" | "default")
        || t.strip_prefix("cs")
            .is_some_and(|d| matches!(d, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7"))
        || t.strip_prefix("af").is_some_and(|d| {
            let b = d.as_bytes();
            b.len() == 2 && (b'1'..=b'4').contains(&b[0]) && (b'1'..=b'3').contains(&b[1])
        })
}

/// Every way `node` can consume tokens from `i`: the end position and the items matched.
/// `best` records the furthest token any attempt reached, for the caret.
fn walk(
    node: &Node,
    cmd: &Command,
    toks: &[Tok],
    i: usize,
    best: &mut usize,
    redacted: bool,
) -> Vec<(usize, Vec<Item>)> {
    match node {
        Node::Kw(k) => {
            let Some(t) = toks.get(i) else { return vec![] };
            let typed = t.text.to_lowercase();
            let kw = k.to_lowercase();
            if kw.starts_with(&typed) {
                *best = (*best).max(i + 1);
                vec![(
                    i + 1,
                    vec![Item::Kw {
                        kw: k.clone(),
                        tok: i,
                        exact: kw == typed,
                    }],
                )]
            } else {
                vec![]
            }
        }
        Node::Arg(a) => {
            let kind = cmd.args.get(a).cloned().unwrap_or(ArgKind::Word);
            match take_arg(&kind, toks, i, redacted) {
                Some((n, value)) => {
                    *best = (*best).max(i + n);
                    vec![(
                        i + n,
                        vec![Item::Arg {
                            name: a.clone(),
                            value,
                            tok: i,
                            typed: kind != ArgKind::Word,
                        }],
                    )]
                }
                None => vec![],
            }
        }
        Node::Seq(items) => {
            let mut states = vec![(i, Vec::new())];
            for it in items {
                let mut next = Vec::new();
                for (pos, acc) in &states {
                    for (end, got) in walk(it, cmd, toks, *pos, best, redacted) {
                        let mut a = acc.clone();
                        a.extend(got);
                        next.push((end, a));
                    }
                }
                if next.is_empty() {
                    return vec![];
                }
                // keep the search small: one way to reach each position for each reading `choose`
                // tells apart, so two keywords of the line one abbreviation fits stay ambiguous
                // (`speed 1`) and a keyword is not lost to a word argument before it (`set dscp def`)
                next.sort_by_key(|(p, _)| *p);
                let mut kept: Vec<(usize, Vec<Item>)> = Vec::new();
                for (p, got) in next {
                    if !kept.iter().any(|(q, k)| *q == p && same_reading(k, &got)) {
                        kept.push((p, got));
                    }
                }
                states = kept;
            }
            states
        }
        Node::Alt(branches) => branches
            .iter()
            .flat_map(|b| walk(b, cmd, toks, i, best, redacted))
            .collect(),
        Node::Opt(inner) => {
            let mut out = vec![(i, Vec::new())];
            out.extend(walk(inner, cmd, toks, i, best, redacted));
            out
        }
    }
}

/// Whether two ways through a line read every token alike as far as `choose` looks: the same keyword,
/// or an argument of a stated type, or a free word.
fn same_reading(a: &[Item], b: &[Item]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|pair| match pair {
            (Item::Kw { kw: x, tok: s, .. }, Item::Kw { kw: y, tok: t, .. }) => {
                s == t && x.eq_ignore_ascii_case(y)
            }
            (
                Item::Arg {
                    tok: s, typed: x, ..
                },
                Item::Arg {
                    tok: t, typed: y, ..
                },
            ) => s == t && x == y,
            _ => false,
        })
}

/// Whether the command could go on after consuming every token (the input is a prefix of it).
fn could_continue(node: &Node, cmd: &Command, toks: &[Tok]) -> bool {
    // a line is incomplete when adding a placeholder token lets it reach further
    let mut best = 0;
    let _ = walk(node, cmd, toks, 0, &mut best, false);
    best >= toks.len() && !toks.is_empty()
}

/// What may follow a line: a keyword, or an argument and its type, with the reference's description.
#[derive(Clone, Debug, PartialEq)]
pub enum Next {
    Kw {
        kw: String,
        help: Option<String>,
    },
    Arg {
        name: String,
        kind: ArgKind,
        help: Option<String>,
    },
}

/// Like `walk`, but where a path has used every token, the keyword or argument it reaches next goes
/// to `out`. Returns the positions the node can end at.
fn expect(node: &Node, cmd: &Command, toks: &[Tok], i: usize, out: &mut Vec<Next>) -> Vec<usize> {
    match node {
        Node::Kw(k) if i == toks.len() => {
            out.push(Next::Kw {
                kw: k.clone(),
                help: cmd.help.get(&k.to_lowercase()).cloned(),
            });
            vec![]
        }
        Node::Arg(a) if i == toks.len() => {
            out.push(Next::Arg {
                name: a.clone(),
                kind: cmd.args.get(a).cloned().unwrap_or(ArgKind::Word),
                help: cmd.help.get(a).cloned(),
            });
            vec![]
        }
        Node::Kw(_) | Node::Arg(_) => {
            let mut best = 0;
            walk(node, cmd, toks, i, &mut best, false)
                .into_iter()
                .map(|(end, _)| end)
                .collect()
        }
        Node::Seq(items) => {
            let mut states = vec![i];
            for it in items {
                let mut next: Vec<usize> = states
                    .iter()
                    .flat_map(|&p| expect(it, cmd, toks, p, out))
                    .collect();
                next.sort_unstable();
                next.dedup();
                if next.is_empty() {
                    return vec![];
                }
                states = next;
            }
            states
        }
        Node::Alt(branches) => branches
            .iter()
            .flat_map(|b| expect(b, cmd, toks, i, out))
            .collect(),
        Node::Opt(inner) => {
            let mut v = vec![i];
            v.extend(expect(inner, cmd, toks, i, out));
            v
        }
    }
}

/// One way to read a line: a syntax line of a command the mode runs, with the typed `no` either its
/// own (offset 0) or put before its positive form (offset 1).
struct Try<'g> {
    command: usize,
    line: usize,
    node: &'g Node,
    offset: usize,
    negated: bool,
}

/// Whether a whole keyword is the books' short spelling of `long` at the same point, which IOS reads
/// as `long` because it has no such keyword of its own: the singular of a plural (`show interface`,
/// `show crypto pki certificate`) and `show env`. Any other longer keyword is another command
/// (`ip` and `ipv6`, `flow` and `flowcontrol`, `net` and `network`).
fn short_spelling(word: &str, long: &str) -> bool {
    long.strip_prefix(word)
        .is_some_and(|rest| rest == "s" || rest == "es")
        || (word == "env" && long == "environment")
}

/// IOS takes a token that is a whole keyword as that keyword, so a longer keyword it abbreviates at the
/// same point is out (`service x` is refused at `x`, not ambiguous among `service-policy` and
/// `service-template`; `ip mld` is refused at `mld`, not read as `ipv6 mld`). Returns, per try,
/// whether no whole keyword shadows it, over each line's leading keywords. A book's short spelling
/// (`short_spelling`) shadows nothing: IOS has only the longer keyword.
fn shadow(tries: &[Try<'_>], toks: &[Tok]) -> Vec<bool> {
    // each step of a try's leading keywords: the keyword, the token it took, and whether it is whole
    let paths: Vec<Vec<(String, String, bool)>> = tries
        .iter()
        .map(|t| {
            let mut path = Vec::new();
            if t.offset == 1 {
                path.push(("no".to_string(), "no".to_string(), true));
            }
            let rest = toks.get(t.offset..).unwrap_or(&[]);
            for (k, tok) in crate::syntax::leading_keywords(t.node).iter().zip(rest) {
                let (k, typed) = (k.to_lowercase(), tok.text.to_lowercase());
                if !k.starts_with(&typed) {
                    break;
                }
                let exact = k == typed;
                path.push((k, typed, exact));
            }
            path
        })
        .collect();
    let context = |p: &[(String, String, bool)]| {
        p.iter()
            .map(|(k, _, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut exact_after = std::collections::BTreeSet::new();
    for p in &paths {
        for t in 0..p.len() {
            if p[t].2 {
                exact_after.insert(context(&p[..t]));
            }
        }
    }
    paths
        .iter()
        .map(|p| {
            (0..p.len()).all(|t| {
                let (k, typed, exact) = &p[t];
                *exact || short_spelling(typed, k) || !exact_after.contains(&context(&p[..t]))
            })
        })
        .collect()
}

pub struct Engine {
    pub grammar: Grammar,
    by_mode: BTreeMap<String, Vec<usize>>,
    /// what real switches of this platform and release held (data/live)
    pub held: crate::live::Held,
    /// every keyword of the grammar, lowercase and sorted: what an abbreviation could stand for
    keywords: Vec<String>,
    /// the mode a form of a command enters where its examples show one other than the command's
    /// (`ip access-list standard` beside `ip access-list extended`), by command and keywords
    forms: BTreeMap<(usize, Vec<String>), String>,
}

/// A parsed line's keywords, lowercase: its form, which modes are entered by.
fn form(p: &Parse) -> Vec<String> {
    p.items
        .iter()
        .filter_map(|i| match i {
            Item::Kw { kw, .. } => Some(kw.to_lowercase()),
            Item::Arg { .. } => None,
        })
        .collect()
}

/// The keywords of a node, lowercase.
fn keywords_in(n: &Node, out: &mut Vec<String>) {
    match n {
        Node::Kw(k) => out.push(k.to_lowercase()),
        Node::Arg(_) => {}
        Node::Seq(v) | Node::Alt(v) => v.iter().for_each(|x| keywords_in(x, out)),
        Node::Opt(b) => keywords_in(b, out),
    }
}

/// Global configuration or one of its submodes (`is_submode`).
pub fn is_config(mode: &str) -> bool {
    mode == "config" || is_submode(mode)
}

/// A configuration submode: IOS names most `config-*`, some `conf-*` (`conf-ssh-pubkey-user`), and the
/// PKI, crypto and DHCP ones by their own families (`ca-trustpoint`, `cs-server`, `cfg-crypto-trans`,
/// `dhcp-config`).
pub fn is_submode(mode: &str) -> bool {
    ["config-", "conf-", "ca-", "cs-", "cfg-", "dhcp-"]
        .iter()
        .any(|p| mode.starts_with(p))
}

/// Whether a prompt change climbs the mode tree: a configuration submode to global configuration or
/// EXEC, or a submode to the one its name extends (`config-if-vrf` to `config-if`).
fn upward(from: &str, to: &str) -> bool {
    let config = is_config(from);
    config && (to == "privileged" || to == "exec")
        || is_submode(from) && to == "config"
        || from.starts_with(&format!("{to}-"))
}

impl Engine {
    pub fn new(grammar: Grammar) -> Self {
        let held = crate::live::Held::of(&grammar.platform, &grammar.release);
        Engine::with_held(grammar, held)
    }

    /// An engine on these held lines, as the corpus being built gives them.
    pub fn with_held(grammar: Grammar, held: crate::live::Held) -> Self {
        let mut by_mode: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, c) in grammar.commands.iter().enumerate() {
            for m in &c.modes {
                by_mode.entry(m.clone()).or_default().push(i);
            }
        }
        let mut keywords = Vec::new();
        for l in grammar.commands.iter().flat_map(|c| &c.lines) {
            keywords_in(&l.node, &mut keywords);
        }
        keywords.sort_unstable();
        keywords.dedup();
        let mut e = Engine {
            grammar,
            by_mode,
            held,
            keywords,
            forms: BTreeMap::new(),
        };
        // the mode each command enters, from the example lines that changed the prompt: seen at
        // least twice, into a mode some command runs in (one-off prompts like `(config-f)` are typos).
        // A prompt that climbs to an enclosing mode after an ordinary line is an example that left out
        // `exit`; only `exit...` commands climb. So is a submode's line landing in a mode global
        // configuration enters that is not deeper (the 2960 books' `set dscp default` then
        // `(config-if)#`).
        let top: BTreeSet<String> = e
            .grammar
            .transitions
            .iter()
            .filter(|t| t.from == "config")
            .map(|t| t.to.clone())
            .collect();
        let sideways = |from: &str, to: &str| {
            from != "config"
                && is_submode(from)
                && top.contains(to)
                && !to.starts_with(&format!("{from}-"))
        };
        let mut seen: BTreeMap<(usize, String), usize> = BTreeMap::new();
        let mut by_form: BTreeMap<(usize, Vec<String>), BTreeMap<String, usize>> = BTreeMap::new();
        for t in &e.grammar.transitions {
            if let Outcome::Ok(p) = e.parse(&t.from, &t.line)
                && !p.negated
                && if upward(&t.from, &t.to) {
                    e.grammar.commands[p.command].name.starts_with("exit")
                } else {
                    !sideways(&t.from, &t.to)
                }
            {
                *seen.entry((p.command, t.to.clone())).or_default() += 1;
                *by_form
                    .entry((p.command, form(&p)))
                    .or_default()
                    .entry(t.to.clone())
                    .or_default() += 1;
            }
        }
        let mut best: BTreeMap<usize, (usize, String)> = BTreeMap::new();
        for ((ci, to), n) in seen {
            if n >= 2 && e.has_mode(&to) && best.get(&ci).is_none_or(|(m, _)| n > *m) {
                best.insert(ci, (n, to));
            }
        }
        for (ci, (_, to)) in best {
            let c = &mut e.grammar.commands[ci];
            if c.enters.is_none() {
                c.enters = Some(to);
            }
        }
        // a form most of whose examples, two at least, enter another mode than its command enters
        // (the books' one `ip access-list {standard | extended}`, the examples a stray prompt apart)
        for ((ci, words), targets) in by_form {
            let total: usize = targets.values().sum();
            let Some((to, n)) = targets.into_iter().max_by_key(|(_, n)| *n) else {
                continue;
            };
            if n >= 2
                && n * 2 > total
                && e.has_mode(&to)
                && e.grammar.commands[ci].enters.as_deref() != Some(to.as_str())
            {
                e.forms.insert((ci, words), to);
            }
        }
        e
    }

    /// The mode a parsed line enters: its form's own where its examples show one (`forms`), else its
    /// command's.
    pub fn enters(&self, p: &Parse) -> Option<&str> {
        self.forms
            .get(&(p.command, form(p)))
            .or(self.grammar.commands[p.command].enters.as_ref())
            .map(String::as_str)
    }

    /// Whether some command runs in the mode (an interface range runs the interface commands).
    pub fn has_mode(&self, mode: &str) -> bool {
        self.by_mode.contains_key(mode)
            || (mode == "config-if-range" && self.by_mode.contains_key("config-if"))
    }

    fn candidates(&self, mode: &str) -> Vec<usize> {
        let mut out = self.by_mode.get(mode).cloned().unwrap_or_default();
        // privileged EXEC runs every user EXEC command
        if mode == "privileged" {
            out.extend(self.by_mode.get("exec").cloned().unwrap_or_default());
        }
        // `*config`: every configuration submode
        if is_submode(mode) {
            out.extend(self.by_mode.get("*config").cloned().unwrap_or_default());
        }
        // an interface range takes the interface commands, applied to each member
        if mode == "config-if-range" {
            out.extend(self.by_mode.get("config-if").cloned().unwrap_or_default());
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The syntax lines a line typed in `mode` is read against, but those a whole keyword shadows
    /// (`shadow`).
    fn tries(&self, mode: &str, toks: &[Tok]) -> Vec<Try<'_>> {
        let negated_input = toks
            .first()
            .is_some_and(|t| t.text.eq_ignore_ascii_case("no"));
        let mut out = Vec::new();
        for ci in self.candidates(mode) {
            let cmd = &self.grammar.commands[ci];
            for (li, l) in cmd.lines.iter().enumerate() {
                // a `no` input matches the stated `no` form, or the command's own form after `no`,
                // whose arguments IOS takes and ignores (`no switchport access vlan 10`)
                let (offset, negated) = match (negated_input, l.negated) {
                    (true, true) | (false, false) => (0, l.negated),
                    (true, false) => (1, true),
                    (false, true) => continue,
                };
                out.push(Try {
                    command: ci,
                    line: li,
                    node: &l.node,
                    offset,
                    negated,
                });
            }
        }
        let kept = shadow(&out, toks);
        out.into_iter()
            .zip(kept)
            .filter_map(|(t, k)| k.then_some(t))
            .collect()
    }

    /// What `?` lists after the whole tokens `toks` in `mode`: what may come next, and whether the
    /// line is complete as typed (`<cr>`).
    pub fn next(&self, mode: &str, toks: &[Tok]) -> (Vec<Next>, bool) {
        let mut out = Vec::new();
        let mut complete = false;
        for t in self.tries(mode, toks) {
            let cmd = &self.grammar.commands[t.command];
            let rest = &toks[t.offset.min(toks.len())..];
            let mut found = Vec::new();
            if expect(t.node, cmd, rest, 0, &mut found).contains(&rest.len()) && !toks.is_empty() {
                complete = true;
            }
            // a `no` form names the command's keywords and no more (`no_form`)
            if t.negated && no_form(t.node, &toks[1..]).is_some() {
                complete = true;
            }
            // the first word of a command whose name it is says what the command does
            for f in &mut found {
                if let Next::Kw { kw, help: h @ None } = f
                    && cmd.name.eq_ignore_ascii_case(kw)
                {
                    *h = cmd.help.get("").cloned();
                }
            }
            out.extend(found);
        }
        (out, complete)
    }

    /// Matches `line` typed in `mode`.
    pub fn parse(&self, mode: &str, line: &str) -> Outcome {
        self.parse_as(mode, line, false)
    }

    /// `parse` for a line of a configuration being replayed, which may hold the secret sweep's
    /// `<removed>` where a value was.
    pub fn parse_redacted(&self, mode: &str, line: &str) -> Outcome {
        self.parse_as(mode, line, true)
    }

    fn parse_as(&self, mode: &str, line: &str, redacted: bool) -> Outcome {
        let toks = tokens(line);
        if toks.is_empty() {
            return Outcome::Incomplete;
        }
        let mut full: Vec<Parse> = Vec::new();
        let mut furthest = 0usize;
        let mut prefix_of_some = false;
        for t in self.tries(mode, &toks) {
            let (ci, li, node, offset, negated) = (t.command, t.line, t.node, t.offset, t.negated);
            let cmd = &self.grammar.commands[ci];
            {
                let rest = &toks[offset..];
                let mut best = 0;
                for (end, items) in walk(node, cmd, rest, 0, &mut best, redacted) {
                    if end == rest.len() {
                        let items = items
                            .into_iter()
                            .map(|it| match it {
                                Item::Kw { kw, tok, exact } => Item::Kw {
                                    kw,
                                    tok: tok + offset,
                                    exact,
                                },
                                Item::Arg {
                                    name,
                                    value,
                                    tok,
                                    typed,
                                } => Item::Arg {
                                    name,
                                    value,
                                    tok: tok + offset,
                                    typed,
                                },
                            })
                            .collect();
                        full.push(Parse {
                            command: ci,
                            line: li,
                            negated,
                            items,
                        });
                    }
                }
                // a `no` form needs only the command's keywords, a stated `no` line's as well
                // (`no ip address`, where the 2960-X books state `no ip address <ip-address> <mask>`)
                if negated
                    && !full.iter().any(|p| p.command == ci)
                    && let Some(kws) = no_form(node, &toks[1..])
                {
                    let items = toks[1..]
                        .iter()
                        .zip(&kws)
                        .enumerate()
                        .map(|(j, (t, k))| Item::Kw {
                            kw: k.clone(),
                            tok: j + 1,
                            exact: k.eq_ignore_ascii_case(&t.text),
                        })
                        .collect();
                    full.push(Parse {
                        command: ci,
                        line: li,
                        negated: true,
                        items,
                    });
                    continue;
                }
                furthest = furthest.max(best + offset);
                if could_continue(node, cmd, rest) {
                    prefix_of_some = true;
                }
            }
        }
        if full.is_empty() {
            // IOS reads left to right: an abbreviation two keywords fit is ambiguous before the
            // line is found incomplete or wrong further on (`co`: configure, connect, copy)
            if (0..=furthest.min(toks.len() - 1)).any(|t| self.ambiguous_at(mode, &toks, t)) {
                return Outcome::Ambiguous;
            }
            if prefix_of_some && furthest >= toks.len() {
                return Outcome::Incomplete;
            }
            let at = toks.get(furthest.min(toks.len() - 1)).map_or(0, |t| t.at);
            return Outcome::Invalid(at);
        }
        // a `no` form documented as a command of its own (`no vlan`) names no more of the line, and
        // the command it undoes is the one that enters or removes a block
        let named = |ci: usize| {
            let name = &self.grammar.commands[ci].name;
            let words: Vec<&str> = name
                .split(" (")
                .next()
                .unwrap_or(name)
                .split_whitespace()
                .collect();
            let no = words.first().is_some_and(|w| w.eq_ignore_ascii_case("no"));
            (words.len() - usize::from(no), no)
        };
        match choose(full, &toks, named) {
            // an abbreviated keyword another keyword at its point also fits, though only one reading
            // completes (`show vlan i`: id, ifindex)
            Outcome::Ok(p)
                if p.items.iter().any(|i| {
                    matches!(i, Item::Kw { exact: false, tok, .. } if self.ambiguous_at(mode, &toks, *tok))
                }) =>
            {
                Outcome::Ambiguous
            }
            o => o,
        }
    }

    /// Whether token `t` abbreviates two or more keywords that may follow the tokens before it, none
    /// of them whole, and no argument there takes it.
    fn ambiguous_at(&self, mode: &str, toks: &[Tok], t: usize) -> bool {
        let typed = toks[t].text.to_lowercase();
        // two keywords anywhere in the grammar that the token abbreviates, before looking at its point
        let from = self.keywords.partition_point(|k| *k < typed);
        let longer = self.keywords[from..]
            .iter()
            .take_while(|k| k.starts_with(&typed))
            .filter(|k| **k != typed)
            .take(2)
            .count();
        if longer < 2 {
            return false;
        }
        let (next, _) = self.next(mode, &toks[..t]);
        let mut fits = BTreeSet::new();
        for n in &next {
            match n {
                Next::Kw { kw, .. } => {
                    let k = kw.to_lowercase();
                    if k == typed {
                        return false;
                    }
                    if k.starts_with(&typed) {
                        fits.insert(k);
                    }
                }
                Next::Arg { kind, .. } => {
                    if take_arg(kind, toks, t, false).is_some() {
                        return false;
                    }
                }
            }
        }
        fits.len() - short_spellings(&fits).len() > 1
    }
}

/// Picks among complete parses: an exact keyword over a longer one it abbreviates, a keyword over
/// an argument, a typed argument over a free word; different keywords both abbreviated by one
/// token are ambiguous. `named` gives the words of a command's name past a leading `no`, and whether
/// it has one.
fn choose(mut full: Vec<Parse>, toks: &[Tok], named: impl Fn(usize) -> (usize, bool)) -> Outcome {
    let key = |p: &Parse| -> Vec<String> {
        p.items
            .iter()
            .map(|i| match i {
                Item::Kw { kw, .. } => format!("k:{}", kw.to_lowercase()),
                Item::Arg { tok, .. } => format!("a:{tok}"),
            })
            .collect()
    };
    fn item_at(p: &Parse, t: usize) -> Option<&Item> {
        p.items.iter().find(|i| match i {
            Item::Kw { tok, .. } | Item::Arg { tok, .. } => *tok == t,
        })
    }
    for t in 0..toks.len() {
        if full
            .iter()
            .any(|p| matches!(item_at(p, t), Some(Item::Kw { exact: true, .. })))
        {
            full.retain(|p| matches!(item_at(p, t), Some(Item::Kw { exact: true, .. })));
        } else if full
            .iter()
            .any(|p| matches!(item_at(p, t), Some(Item::Kw { .. })))
            && full
                .iter()
                .any(|p| matches!(item_at(p, t), Some(Item::Arg { .. })))
        {
            full.retain(|p| matches!(item_at(p, t), Some(Item::Kw { .. })));
        } else if full
            .iter()
            .any(|p| matches!(item_at(p, t), Some(Item::Arg { typed: true, .. })))
        {
            // an argument of a stated type (interface, number, address) over a free word
            full.retain(|p| !matches!(item_at(p, t), Some(Item::Arg { typed: false, .. })));
        }
        let mut kws: BTreeSet<String> = full
            .iter()
            .filter_map(|p| match item_at(p, t) {
                Some(Item::Kw { kw, .. }) => Some(kw.to_lowercase()),
                _ => None,
            })
            .collect();
        // a book's short spelling and the keyword IOS has are one keyword, read as IOS's
        // (`sh int status` is `show interfaces status`)
        let short = short_spellings(&kws);
        if !short.is_empty() {
            full.retain(|p| {
                !matches!(item_at(p, t), Some(Item::Kw { kw, .. }) if short.contains(&kw.to_lowercase()))
            });
            kws.retain(|k| !short.contains(k));
        }
        if kws.len() > 1 {
            return Outcome::Ambiguous;
        }
    }
    // readings this far apart take the same keywords and arguments; of two commands that read a line
    // alike, the one named for more of its keywords documents that form (`ip access-list standard`
    // over `ip access-list`)
    full.sort_by_key(|p| {
        let (words, no) = named(p.command);
        (key(p).len(), std::cmp::Reverse(words), no)
    });
    Outcome::Ok(full.swap_remove(0))
}

/// The keywords of `kws` that are short spellings of another of them (`short_spelling`).
fn short_spellings(kws: &BTreeSet<String>) -> BTreeSet<String> {
    kws.iter()
        .filter(|k| kws.iter().any(|l| short_spelling(k, l)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{Command, Grammar, Line};
    use crate::syntax;

    fn cmd(name: &str, modes: &[&str], lines: &[&str], args: &[(&str, ArgKind)]) -> Command {
        Command {
            name: name.into(),
            source: "test".into(),
            modes: modes.iter().map(|m| m.to_string()).collect(),
            lines: lines
                .iter()
                .map(|l| Line {
                    text: l.to_string(),
                    node: syntax::parse(l).0,
                    negated: l.starts_with("no "),
                    repairs: vec![],
                })
                .collect(),
            args: args
                .iter()
                .map(|(n, k)| (n.to_string(), k.clone()))
                .collect(),
            examples: vec![],
            enters: None,
            evidence: None,
            help: Default::default(),
        }
    }

    fn engine() -> Engine {
        Engine::new(Grammar {
            release: "t".into(),
            platform: "t".into(),
            source: "t".into(),
            transitions: vec![],
            commands: vec![
                cmd(
                    "show vlan",
                    &["exec", "privileged"],
                    &["show vlan [brief | id <vlan-id>]"],
                    &[("vlan-id", ArgKind::Number { min: 1, max: 4094 })],
                ),
                cmd(
                    "show version",
                    &["exec", "privileged"],
                    &["show version"],
                    &[],
                ),
                cmd("show ip", &["privileged"], &["show ip route"], &[]),
                cmd("show ipv6", &["privileged"], &["show ipv6 route"], &[]),
                cmd(
                    "switchport access vlan",
                    &["config-if"],
                    &[
                        "switchport access vlan <vlan-id>",
                        "no switchport access vlan",
                    ],
                    &[("vlan-id", ArgKind::Number { min: 1, max: 4094 })],
                ),
                cmd(
                    "interface",
                    &["config"],
                    &["interface <interface-id>"],
                    &[("interface-id", ArgKind::Interface)],
                ),
                cmd(
                    "description",
                    &["config-if"],
                    &["description <string>"],
                    &[("string", ArgKind::Text)],
                ),
                cmd("hostname", &["config"], &["hostname <name>"], &[]),
            ],
        })
    }

    fn ok(o: Outcome) -> Parse {
        match o {
            Outcome::Ok(p) => p,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_replayed_placeholder_fills_any_argument_but_an_interface() {
        let toks = tokens("<removed>");
        let number = ArgKind::Number { min: 1, max: 15 };
        assert_eq!(take_arg(&number, &toks, 0, false), None);
        assert_eq!(
            take_arg(&number, &toks, 0, true),
            Some((1, "<removed>".to_string()))
        );
        assert!(take_arg(&ArgKind::Ipv4, &toks, 0, true).is_some());
        assert_eq!(take_arg(&ArgKind::Interface, &toks, 0, true), None);
        assert_eq!(take_arg(&ArgKind::InterfaceRange, &toks, 0, true), None);
    }

    #[test]
    fn a_dscp_value_is_a_number_to_63_or_a_name() {
        for ok in [
            "0", "63", "ef", "EF", "default", "cs0", "cs7", "af11", "af43",
        ] {
            assert!(dscp(ok), "{ok}");
        }
        for bad in ["64", "-1", "cs8", "af44", "af51", "af1", "efx", "banana"] {
            assert!(!dscp(bad), "{bad}");
        }
    }

    #[test]
    fn abbreviations_expand_to_the_full_keywords() {
        let e = engine();
        assert_eq!(
            ok(e.parse("privileged", "sh vl br")).canonical(),
            "show vlan brief"
        );
        assert_eq!(
            ok(e.parse("privileged", "show vlan id 20")).canonical(),
            "show vlan id 20"
        );
        assert_eq!(
            ok(e.parse("config-if", "sw acc vl 10")).canonical(),
            "switchport access vlan 10"
        );
        assert_eq!(
            ok(e.parse("config", "int twe1/0/1")).canonical(),
            "interface TwentyFiveGigE1/0/1"
        );
        assert_eq!(
            ok(e.parse("config", "interface Hu 1/1/1")).canonical(),
            "interface HundredGigE1/1/1"
        );
        assert_eq!(
            ok(e.parse("config-if", "description to node 1 bond0")).canonical(),
            "description to node 1 bond0"
        );
    }

    #[test]
    fn an_exact_keyword_wins_over_a_longer_one_it_prefixes() {
        let e = engine();
        assert_eq!(
            ok(e.parse("privileged", "sh ip route")).canonical(),
            "show ip route"
        );
        assert_eq!(e.parse("privileged", "sh v"), Outcome::Ambiguous);
    }

    #[test]
    fn errors_name_the_point_of_failure() {
        let e = engine();
        // caret under the first character of the token that fits nothing
        assert_eq!(
            e.parse("privileged", "show vlan bogus"),
            Outcome::Invalid(10)
        );
        assert_eq!(
            e.parse("privileged", "show vlan id 5000"),
            Outcome::Invalid(13)
        );
        assert_eq!(
            e.parse("config-if", "switchport access vlan"),
            Outcome::Incomplete
        );
        assert_eq!(
            e.parse("exec", "show ip route"),
            Outcome::Invalid(5),
            "show ip is privileged only"
        );
        assert_eq!(
            e.parse("config", "int t1/0/1"),
            Outcome::Invalid(4),
            "T is Ten, Tunnel or TwentyFive"
        );
    }

    #[test]
    fn no_forms() {
        let e = engine();
        let p = ok(e.parse("config-if", "no sw acc vl"));
        assert!(p.negated);
        assert_eq!(p.canonical(), "no switchport access vlan");
        let p = ok(e.parse("config", "no hostname"));
        assert_eq!(p.canonical(), "no hostname");
    }

    fn keywords() -> Engine {
        Engine::new(Grammar {
            release: "t".into(),
            platform: "t".into(),
            source: "t".into(),
            transitions: vec![],
            commands: vec![
                cmd("service dhcp", &["config"], &["service dhcp"], &[]),
                cmd(
                    "service-policy",
                    &["config"],
                    &["service-policy <name>"],
                    &[],
                ),
                cmd(
                    "service-template",
                    &["config"],
                    &["service-template <name>"],
                    &[],
                ),
                // a book's abbreviated spelling beside the command IOS has
                cmd(
                    "show interface",
                    &["privileged"],
                    &["show interface status"],
                    &[],
                ),
                cmd(
                    "show interfaces",
                    &["privileged"],
                    &["show interfaces <interface-id> switchport"],
                    &[("interface-id", ArgKind::Interface)],
                ),
            ],
        })
    }

    #[test]
    fn a_whole_keyword_is_not_read_as_a_longer_one() {
        let e = keywords();
        // `service` is the service command: `x` is refused, not ambiguous among service-*
        assert_eq!(e.parse("config", "service x"), Outcome::Invalid(8));
        assert_eq!(e.parse("config", "serv x"), Outcome::Ambiguous);
        assert_eq!(
            ok(e.parse("config", "service dhcp")).canonical(),
            "service dhcp"
        );
        // a book's `show interface` does not hide `show interfaces`, which IOS has
        let p = ok(e.parse("privileged", "show interface Twe1/0/1 switchport"));
        assert_eq!(
            p.canonical(),
            "show interfaces TwentyFiveGigE1/0/1 switchport"
        );
    }

    #[test]
    fn next_lists_what_may_follow_and_whether_the_line_is_complete() {
        let e = keywords();
        let kws = |n: &[Next]| {
            let mut v: Vec<String> = n
                .iter()
                .filter_map(|x| match x {
                    Next::Kw { kw, .. } => Some(kw.clone()),
                    Next::Arg { .. } => None,
                })
                .collect();
            v.sort();
            v
        };
        let (n, complete) = e.next("config", &tokens("service "));
        assert_eq!(kws(&n), vec!["dhcp"]);
        assert!(!complete);
        let (n, complete) = e.next("config", &tokens("service dhcp"));
        assert!(n.is_empty() && complete);
        let (n, _) = e.next("privileged", &tokens("show "));
        assert_eq!(kws(&n), vec!["interface", "interfaces"]);
        let (n, _) = e.next("privileged", &tokens("show interfaces "));
        assert!(matches!(
            n.as_slice(),
            [Next::Arg {
                kind: ArgKind::Interface,
                ..
            }]
        ));
    }

    fn matcher() -> Engine {
        let vlan = ArgKind::Number { min: 1, max: 4094 };
        Engine::new(Grammar {
            release: "t".into(),
            platform: "t".into(),
            source: "t".into(),
            transitions: vec![],
            commands: vec![
                cmd("ip routing", &["config"], &["ip routing"], &[]),
                cmd(
                    "ipv6 mld snooping",
                    &["config"],
                    &["ipv6 mld snooping"],
                    &[],
                ),
                cmd(
                    "ipv6 unicast-routing",
                    &["config"],
                    &["ipv6 unicast-routing"],
                    &[],
                ),
                cmd(
                    "ip pim rp-address",
                    &["config"],
                    &["ip pim [vrf <vrf-name>] rp-address <rp-address>"],
                    &[("rp-address", ArgKind::Ipv4)],
                ),
                // the shape a misread top-level bar leaves: `ip` alone before a choice of commands
                cmd(
                    "ip",
                    &["config"],
                    &["ip {admission <admission-name> | prefix-list <list-name> <seq>}"],
                    &[],
                ),
                cmd(
                    "do",
                    &["config"],
                    &["do <command>"],
                    &[("command", ArgKind::Text)],
                ),
                cmd(
                    "dot1x system-auth-control",
                    &["config"],
                    &["dot1x system-auth-control"],
                    &[],
                ),
                cmd("show env", &["privileged"], &["show env all"], &[]),
                cmd(
                    "show environment",
                    &["privileged"],
                    &["show environment {all | power}"],
                    &[],
                ),
                cmd(
                    "spanning-tree portfast",
                    &["config-if"],
                    &["spanning-tree portfast [trunk]"],
                    &[],
                ),
                cmd(
                    "spanning-tree bpdufilter",
                    &["config-if"],
                    &["spanning-tree bpdufilter {enable | disable}"],
                    &[],
                ),
                cmd(
                    "spanning-tree cost",
                    &["config-if"],
                    &["spanning-tree [vlan <vlan-id>] cost <cost>"],
                    &[("vlan-id", vlan.clone())],
                ),
                cmd(
                    "ip address",
                    &["config-if"],
                    &["ip address <ip-address> <subnet-mask>"],
                    &[
                        ("ip-address", ArgKind::Ipv4),
                        ("subnet-mask", ArgKind::Ipv4),
                    ],
                ),
                cmd(
                    "switchport access vlan",
                    &["config-if"],
                    &[
                        "switchport access vlan <vlan-id>",
                        "no switchport access vlan",
                    ],
                    &[("vlan-id", vlan.clone())],
                ),
                cmd(
                    "speed",
                    &["config-if"],
                    &["speed {10 | 100 | 1000 | auto}", "no speed"],
                    &[],
                ),
                cmd(
                    "description",
                    &["config-if"],
                    &["description <line>"],
                    &[("line", ArgKind::Text)],
                ),
                cmd(
                    "class-map",
                    &["config"],
                    &["class-map [match-all | match-any] <class-map-name>"],
                    &[],
                ),
                cmd(
                    "set dscp",
                    &["config-pmap-c"],
                    &["set dscp {<dscp-value> | default | ef}"],
                    &[],
                ),
                cmd(
                    "configure terminal",
                    &["privileged"],
                    &["configure terminal"],
                    &[],
                ),
                cmd("connect", &["privileged"], &["connect <host>"], &[]),
                cmd(
                    "copy",
                    &["privileged"],
                    &["copy <source-url> <destination-url>"],
                    &[],
                ),
                cmd(
                    "show vlan",
                    &["privileged"],
                    &["show vlan [brief | id <vlan-id> | ifindex]"],
                    &[("vlan-id", vlan.clone())],
                ),
                // the 2960G books' line, the bar and the ellipsis as they print them
                cmd(
                    "aaa authentication login",
                    &["config"],
                    &[
                        "aaa authentication login {default <| list-name>} <method1> [<method2> . . . ]",
                    ],
                    &[],
                ),
                // a book's spelling beside the keyword IOS has
                cmd(
                    "show interface",
                    &["privileged"],
                    &["show interface status"],
                    &[],
                ),
                cmd(
                    "show interfaces",
                    &["privileged"],
                    &["show interfaces [status]"],
                    &[],
                ),
            ],
        })
    }

    #[test]
    fn a_method_list_reads_as_the_book_s_choice_and_ellipsis_mean() {
        let e = matcher();
        for (line, shown) in [
            (
                "aaa authentication login default local",
                "aaa authentication login default local",
            ),
            (
                "aaa authentication login VTY local",
                "aaa authentication login VTY local",
            ),
            (
                "aaa auth login default group tacacs+ local none",
                "aaa authentication login default group tacacs+ local none",
            ),
        ] {
            assert_eq!(ok(e.parse("config", line)).canonical(), shown, "{line}");
        }
        assert_eq!(
            e.parse("config", "aaa authentication login default"),
            Outcome::Incomplete
        );
    }

    #[test]
    fn an_abbreviation_of_a_book_s_short_spelling_is_the_keyword_ios_has() {
        let e = matcher();
        assert_eq!(
            ok(e.parse("privileged", "sh int status")).canonical(),
            "show interfaces status"
        );
        assert_eq!(
            ok(e.parse("privileged", "sh en all")).canonical(),
            "show environment all"
        );
        assert_eq!(
            ok(e.parse("privileged", "show interface status")).canonical(),
            "show interface status"
        );
    }

    #[test]
    fn an_abbreviation_two_keywords_fit_is_ambiguous_whatever_follows() {
        let e = matcher();
        // no reading completes, and `co` begins configure, connect and copy
        assert_eq!(e.parse("privileged", "co"), Outcome::Ambiguous);
        // only `ifindex` completes, and `i` begins `id` as well
        assert_eq!(e.parse("privileged", "sh vlan i"), Outcome::Ambiguous);
        assert_eq!(
            ok(e.parse("privileged", "sh vlan if")).canonical(),
            "show vlan ifindex"
        );
        assert_eq!(
            ok(e.parse("privileged", "conf t")).canonical(),
            "configure terminal"
        );
        assert_eq!(
            e.parse("privileged", "show vlan id"),
            Outcome::Incomplete,
            "a whole keyword is not ambiguous"
        );
    }

    #[test]
    fn a_whole_keyword_is_never_read_as_another_longer_one() {
        let e = matcher();
        // IOS takes `ip` as `ip` and refuses at the word after it
        assert_eq!(e.parse("config", "ip mld snooping"), Outcome::Invalid(3));
        assert_eq!(e.parse("config", "ip unicast-routing"), Outcome::Invalid(3));
        assert_eq!(
            ok(e.parse("config", "ip routing")).canonical(),
            "ip routing"
        );
        // a book's `show env` is IOS's `show environment`
        assert_eq!(
            ok(e.parse("privileged", "show env power")).canonical(),
            "show environment power"
        );
        assert_eq!(
            ok(e.parse("privileged", "show env all")).canonical(),
            "show env all"
        );
    }

    #[test]
    fn question_mark_after_a_whole_keyword_lists_that_keyword_s_words() {
        let e = matcher();
        let kws = |n: &[Next]| {
            let mut v: Vec<String> = n
                .iter()
                .filter_map(|x| match x {
                    Next::Kw { kw, .. } => Some(kw.clone()),
                    Next::Arg { .. } => None,
                })
                .collect();
            v.sort();
            v
        };
        let (n, _) = e.next("config", &tokens("ip "));
        assert_eq!(kws(&n), ["admission", "pim", "prefix-list", "routing"]);
        let (n, _) = e.next("config", &tokens("do "));
        assert!(
            matches!(
                n.as_slice(),
                [Next::Arg {
                    kind: ArgKind::Text,
                    ..
                }]
            ),
            "{n:?}"
        );
        let (n, _) = e.next("privileged", &tokens("show env "));
        assert_eq!(kws(&n), ["all", "all", "power"]);
    }

    #[test]
    fn a_no_form_names_every_keyword_of_its_command() {
        let e = matcher();
        for line in [
            "no spanning",
            "no spanning-tree",
            "no switchport access",
            "no ip",
        ] {
            assert_eq!(e.parse("config-if", line), Outcome::Incomplete, "{line}");
        }
        for line in ["no ip", "no ip pim"] {
            assert_eq!(e.parse("config", line), Outcome::Incomplete, "{line}");
        }
        for (line, shown) in [
            ("no spanning-tree portfast", "no spanning-tree portfast"),
            ("no spanning-tree bpdufilter", "no spanning-tree bpdufilter"),
            ("no span cost", "no spanning-tree cost"),
            ("no ip address", "no ip address"),
        ] {
            assert_eq!(ok(e.parse("config-if", line)).canonical(), shown);
        }
        assert_eq!(
            ok(e.parse("config", "no ip pim rp-address")).canonical(),
            "no ip pim rp-address"
        );
        assert!(!e.next("config-if", &tokens("no spanning-tree")).1);
        assert!(e.next("config-if", &tokens("no spanning-tree portfast")).1);
    }

    #[test]
    fn a_no_form_takes_its_command_s_own_line_after_no() {
        let e = matcher();
        let p = ok(e.parse("config-if", "no switchport access vlan 10"));
        assert!(p.negated);
        assert_eq!(p.canonical(), "no switchport access vlan 10");
        assert_eq!(
            ok(e.parse("config-if", "no speed 100")).canonical(),
            "no speed 100"
        );
        assert_eq!(
            ok(e.parse("config-if", "no switchport access vlan")).canonical(),
            "no switchport access vlan"
        );
    }

    #[test]
    fn keywords_of_one_line_one_abbreviation_fits_are_ambiguous() {
        let e = matcher();
        assert_eq!(e.parse("config-if", "speed 1"), Outcome::Ambiguous);
        assert_eq!(ok(e.parse("config-if", "speed 10")).canonical(), "speed 10");
        assert_eq!(
            e.parse("config", "class-map match-a VOICE"),
            Outcome::Ambiguous
        );
        assert_eq!(
            ok(e.parse("config", "class-map match-all VOICE")).canonical(),
            "class-map match-all VOICE"
        );
        // a keyword of the line over the word argument before it
        assert_eq!(
            ok(e.parse("config-pmap-c", "set dscp def")).canonical(),
            "set dscp default"
        );
        assert_eq!(
            ok(e.parse("config-pmap-c", "set dscp 46")).canonical(),
            "set dscp 46"
        );
    }

    #[test]
    fn a_port_channel_range_reads_back_as_written() {
        for typed in [
            "Port-channel1 - 2",
            "port-channel 1 - 2",
            "Po1-2",
            "po 1 -2",
        ] {
            assert_eq!(
                interface_range(typed).as_deref(),
                Some("Port-channel1 - 2"),
                "{typed}"
            );
        }
        assert_eq!(
            interface_range("Port-channel1").as_deref(),
            Some("Port-channel1")
        );
        let shown = interface_range("Po1 - 2 , Gi1/0/1 - 4").unwrap();
        assert_eq!(interface_range(&shown).as_deref(), Some(shown.as_str()));
    }

    #[test]
    fn a_range_spans_no_more_than_a_vlan_range() {
        assert!(interface_range("Vlan1 - 4094").is_some());
        assert!(interface_range("Gi1/0/1 - 48").is_some());
        assert_eq!(interface_range("Gi1/0/1 - 4294967295"), None);
        assert_eq!(interface_range("Gi1/0/1 - 99999"), None);
        assert_eq!(interface_range("Gi1/0/1 - +4"), None);
    }

    #[test]
    fn a_number_is_digits_and_is_written_plain() {
        let toks = |t: &str| tokens(t);
        let vlan = ArgKind::Number { min: 1, max: 4094 };
        assert_eq!(take_arg(&vlan, &toks("+10"), 0, false), None);
        assert_eq!(
            take_arg(&vlan, &toks("010"), 0, false),
            Some((1, "10".to_string()))
        );
        let offset = ArgKind::Number { min: -12, max: 12 };
        assert_eq!(
            take_arg(&offset, &toks("-5"), 0, false),
            Some((1, "-5".to_string()))
        );
        assert_eq!(take_arg(&vlan, &toks("-5"), 0, false), None);
        assert!(!ipv4("+10.0.0.1") && !ipv4("10.0.0.+1") && ipv4("10.0.0.1"));
        assert_eq!(vlan_list("+10"), None);
        assert_eq!(vlan_list("010,020-030").as_deref(), Some("10,20-30"));
        assert!(!dscp("+46") && dscp("46"));
        let e = engine();
        assert_eq!(
            ok(e.parse("config-if", "switchport access vlan 010")).canonical(),
            "switchport access vlan 10"
        );
        assert_eq!(
            e.parse("config-if", "switchport access vlan +10"),
            Outcome::Invalid(23)
        );
    }

    #[test]
    fn a_line_argument_keeps_the_spacing_typed() {
        let e = matcher();
        assert_eq!(
            ok(e.parse("config-if", "description uplink  to   core")).canonical(),
            "description uplink  to   core"
        );
    }

    #[test]
    fn a_line_an_example_left_its_submode_after_enters_no_mode() {
        // the 2960-X guides' `set dscp default` then `(config-if)#`, an example that left out
        // `exit` and `interface`; a climb out of the section by `exit-address-family` still counts
        let e = Engine::new(
            crate::grammars::grammar(
                "c2960x",
                crate::release::Release::parse("15.2(7)E").unwrap(),
            )
            .unwrap(),
        );
        let Outcome::Ok(p) = e.parse("config-pmap-c", "set dscp ef") else {
            panic!("set dscp ef");
        };
        assert_eq!(e.grammar.commands[p.command].enters, None);
        let Outcome::Ok(p) = e.parse("config", "policy-map xx") else {
            panic!("policy-map");
        };
        assert_eq!(
            e.grammar.commands[p.command].enters.as_deref(),
            Some("config-pmap")
        );
        let x = Engine::new(
            crate::grammars::grammar("c9300", crate::release::Release::parse("17.15").unwrap())
                .unwrap(),
        );
        let Outcome::Ok(p) = x.parse("config-router-af", "exit-address-family") else {
            panic!("exit-address-family");
        };
        assert_eq!(
            x.grammar.commands[p.command].enters.as_deref(),
            Some("config-router")
        );
    }

    #[test]
    fn a_form_most_of_whose_examples_enter_another_mode_enters_it() {
        let t = |line: &str, to: &str| crate::compile::Transition {
            from: "config".into(),
            line: line.into(),
            to: to.into(),
        };
        let name = [("name", ArgKind::Word)];
        let e = Engine::new(Grammar {
            release: "t".into(),
            platform: "t".into(),
            source: "t".into(),
            transitions: vec![
                t("ip access-list extended a", "config-ext-nacl"),
                t("ip access-list extended b", "config-ext-nacl"),
                t("ip access-list extended c", "config-ext-nacl"),
                t("ip access-list standard d", "config-std-nacl"),
                t("ip access-list standard e", "config-std-nacl"),
                // a stray prompt among them
                t("ip access-list standard f", "config-ext-nacl"),
                // one example is too few
                t("ip access-list fqdn g", "config-fqdn-acl"),
                // and examples split evenly settle nothing
                t("ip access-list role-based h", "config-rb-acl"),
                t("ip access-list role-based i", "config-rb-acl"),
                t("ip access-list role-based j", "config-ext-nacl"),
                t("ip access-list role-based k", "config-ext-nacl"),
            ],
            commands: vec![
                cmd(
                    "ip access-list",
                    &["config"],
                    &["ip access-list {standard | extended | fqdn | role-based} <name>"],
                    &name,
                ),
                cmd(
                    "permit",
                    &[
                        "config-ext-nacl",
                        "config-std-nacl",
                        "config-fqdn-acl",
                        "config-rb-acl",
                    ],
                    &["permit <name>"],
                    &name,
                ),
            ],
        });
        let enters = |line: &str| match e.parse("config", line) {
            Outcome::Ok(p) => e.enters(&p).map(str::to_string),
            o => panic!("{line}: {o:?}"),
        };
        assert_eq!(
            enters("ip access-list extended x").as_deref(),
            Some("config-ext-nacl")
        );
        assert_eq!(
            enters("ip access-list standard x").as_deref(),
            Some("config-std-nacl")
        );
        // abbreviated, the keywords are the form
        assert_eq!(
            enters("ip access-l st x").as_deref(),
            Some("config-std-nacl")
        );
        assert_eq!(
            enters("ip access-list fqdn x").as_deref(),
            Some("config-ext-nacl")
        );
        assert_eq!(
            enters("ip access-list role-based x").as_deref(),
            Some("config-ext-nacl")
        );
    }

    #[test]
    fn of_two_commands_that_read_a_line_alike_the_one_named_for_more_of_it_is_taken() {
        let args = [
            ("name", ArgKind::Word),
            ("vlan-id", ArgKind::Number { min: 1, max: 4094 }),
        ];
        let mut combined = cmd(
            "ip access-list",
            &["config"],
            &["ip access-list {standard | extended} <name>"],
            &args,
        );
        combined.enters = Some("config-ext-nacl".into());
        let mut standard = cmd(
            "ip access-list standard",
            &["config"],
            &["ip access-list standard <name>"],
            &args,
        );
        standard.enters = Some("config-std-nacl".into());
        let mut vlan = cmd(
            "vlan",
            &["config"],
            &["vlan <vlan-id>", "no vlan <vlan-id>"],
            &args,
        );
        vlan.enters = Some("config-vlan".into());
        let e = Engine::new(Grammar {
            release: "t".into(),
            platform: "t".into(),
            source: "t".into(),
            transitions: vec![],
            commands: vec![
                combined,
                standard,
                // a `no` form the books document as a command of its own, listed first
                cmd("no vlan", &["config"], &["no vlan <vlan-id>"], &args),
                vlan,
            ],
        });
        let named = |line: &str| match e.parse("config", line) {
            Outcome::Ok(p) => e.grammar.commands[p.command].name.clone(),
            o => panic!("{line}: {o:?}"),
        };
        assert_eq!(
            named("ip access-list standard x"),
            "ip access-list standard"
        );
        assert_eq!(named("ip access-list extended x"), "ip access-list");
        assert_eq!(named("no vlan 5"), "vlan");
    }

    #[test]
    fn a_stated_no_line_is_named_by_its_keywords_alone() {
        // a command a book states only in its `no` form, with an argument (the 2960-X's
        // `no device-tracking policy <policy-name>`): its keywords name it, as `no ip address` does
        let e = Engine::new(Grammar {
            release: "t".into(),
            platform: "t".into(),
            source: "t".into(),
            transitions: vec![],
            commands: vec![cmd(
                "device-tracking policy",
                &["config"],
                &["no device-tracking policy <policy-name>"],
                &[("policy-name", ArgKind::Word)],
            )],
        });
        for line in ["no device-tracking policy", "no device-tracking policy P1"] {
            assert!(matches!(e.parse("config", line), Outcome::Ok(_)), "{line}");
        }
    }
}
