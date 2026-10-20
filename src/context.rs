//! What a held `?` answer depends on besides the mode: the lines that entered the submode stack.
//! `(config-router)#` answers differently under `router ospf`, `router bgp` and `router eigrp`, and
//! `(config-if)#` under a physical port, a VLAN interface and a loopback. The context is one chain of
//! those entering lines with their lab object names left out, derived here for both the corpus (from a
//! session's prompts) and the shell (from its frames), so the two cannot disagree.

use crate::live::{fits, help_key};

/// Keyword sequences after which the next word is an object's name, not a keyword. Each keyword may
/// be abbreviated as IOS reads a unique start among the ones a sequence continues with there.
const NAMED: &[&[&str]] = &[
    &["policy-map"],
    &["policy-map", "type", "control", "subscriber"],
    &["policy-map", "type", "queueing"],
    &["policy-map", "type", "inspect"],
    &["class-map"],
    &["class-map", "match-any"],
    &["class-map", "match-all"],
    &["class-map", "type", "control", "subscriber", "match-any"],
    &["class-map", "type", "control", "subscriber", "match-all"],
    &["class"],
    &["table-map"],
    &["key", "chain"],
    &["mka", "policy"],
    &["ip", "access-list", "standard"],
    &["ip", "access-list", "extended"],
    &["ip", "access-list", "role-based"],
    &["ip", "access-list", "fqdn"],
    &["ipv6", "access-list"],
    &["mac", "access-list", "extended"],
    &["object-group", "network"],
    &["object-group", "service"],
    &["route-map"],
    &["radius", "server"],
    &["tacacs", "server"],
    &["aaa", "group", "server", "radius"],
    &["aaa", "group", "server", "tacacs+"],
    &["ip", "dhcp", "pool"],
    &["ip", "dhcp", "class"],
    &["ipv6", "dhcp", "pool"],
    &["ipv6", "dhcp", "guard", "policy"],
    &["ipv6", "nd", "inspection", "policy"],
    &["ipv6", "nd", "raguard", "policy"],
    &["ipv6", "source-guard", "policy"],
    &["device-tracking", "policy"],
    &["event", "manager", "applet"],
    &["crypto", "pki", "trustpoint"],
    &["crypto", "pki", "certificate", "map"],
    &["crypto", "pki", "certificate", "chain"],
    &["vlan", "access-map"],
    &["template"],
    &["service-template"],
    &["bfd-template", "single-hop"],
    &["bfd-template", "multi-hop"],
    &["fallback", "profile"],
    &["profile"],
    &["username"],
    &["dot1x", "credentials"],
    &["macro", "name"],
    &["time-range"],
    &["sampler"],
    &["stack-power", "stack"],
    &["maintenance", "template"],
    &["app-hosting", "appid"],
    &["location", "geo-location", "identifier"],
    &["location", "civic-location", "identifier"],
    &["l2vpn", "xconnect", "context"],
    &["ip", "explicit-path", "name"],
    &["vrf", "definition"],
    &["ip", "vrf"],
    &["flow", "exporter"],
    &["flow", "record"],
    &["flow", "monitor"],
];

/// Lines that open a block of their own from global configuration, whichever block's submode they
/// are typed in: IOS leaves that block for global configuration first. The shell's frame stack and
/// the crawl's `Stack` both read this table (`reopens`). A sequence with a name count takes exactly
/// that many words after it, so `ip vrf forwarding RED` (an interface's line) opens nothing.
const GLOBAL_BLOCKS: &[(&[&str], Option<usize>)] = &[
    (&["interface"], None),
    (&["vlan"], None),
    (&["line"], None),
    (&["router"], None),
    (&["vrf", "definition"], Some(1)),
    (&["ip", "vrf"], Some(1)),
    (&["route-map"], None),
    (&["class-map"], None),
    (&["policy-map"], None),
    (&["ip", "access-list"], None),
];

/// Keywords of an `address-family` line; any other word is a value (a VRF's name, an AS number).
const FAMILY_WORDS: &[&str] = &[
    "ipv4",
    "ipv6",
    "l2vpn",
    "vpnv4",
    "vpnv6",
    "nsap",
    "rtfilter",
    "link-state",
    "mvpn",
    "unicast",
    "multicast",
    "labeled-unicast",
    "mdt",
    "flowspec",
    "evpn",
    "vpls",
    "vrf",
    "autonomous-system",
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
/// line whose presence ends them. The crawl's switch had AAA off, so its `aaa ?`, `no aaa ?` and
/// `default aaa ?` offer `new-model` alone.
pub const STATE_PREDICATES: &[(&str, &str)] = &[
    ("aaa ", "aaa new-model"),
    ("no aaa ", "aaa new-model"),
    ("default aaa ", "aaa new-model"),
];

/// Lines a crawl's `?` answers do not judge: the lab C9300-24T (17.6.4) took them where its crawl
/// offered none of their words. Each is a mode, a line's start (past `no` or `default`) and the
/// configured line under which the switch took them, `""` where the port's own state or hardware
/// decides or `?` hides the line in every state. The crawl ran without `aaa new-model`, `fhrp version vrrp v3` and `ptp profile
/// dot1as`, on ports with no dual-purpose media (the captured 2960s set `media-type` on theirs) and
/// on a sub-interface with no encapsulation.
pub const UNJUDGED: &[(&str, &str, &str)] = &[
    ("config", "radius", "aaa new-model"),
    ("config-line", "login authentication", "aaa new-model"),
    ("config-subif", "xconnect", ""),
    ("config", "tacacs", "aaa new-model"),
    ("config-if", "authentication", "aaa new-model"),
    ("config-if", "dot1x", "aaa new-model"),
    ("config-if", "mab", "aaa new-model"),
    ("config-if", "access-session", "aaa new-model"),
    ("config-if", "service-policy type control", "aaa new-model"),
    ("config-if", "vrrp", "fhrp version vrrp v3"),
    ("config", "ptp", "ptp profile dot1as"),
    ("config-if", "switchport port-security mac-address", ""),
    ("config-if", "media-type", ""),
    ("config-if", "ptp sync", ""),
    ("config", "vtp file", ""),
];

/// Whether `line` in `mode` is one a crawl's answers do not judge (`UNJUDGED`) in the state
/// `configured` describes.
pub fn unjudged(mode: &str, line: &str, configured: impl Fn(&str) -> bool) -> bool {
    let line = line.trim_start().to_lowercase();
    let line = ["no ", "default "]
        .iter()
        .find_map(|n| line.strip_prefix(n))
        .unwrap_or(&line);
    UNJUDGED.iter().any(|(m, start, state)| {
        *m == mode && line.starts_with(start) && (state.is_empty() || configured(state))
    })
}

/// Where a refused line's caret stands: under a word, or past its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caret {
    At(usize),
    After(usize),
}

/// A line a switch refused that its crawl's answers offer (`REFUSED`).
pub struct Refusal {
    /// the image it was measured on, by book and release, and the licence levels
    pub book: &'static str,
    pub release: &'static str,
    pub levels: &'static [&'static str],
    pub mode: &'static str,
    /// the line's words, `<word>` any one word
    pub words: &'static [&'static str],
    /// the configured line under which it was refused, `""` in any state
    pub state: &'static str,
    pub caret: Caret,
}

pub(crate) const C9300_17_6: (&str, &str) = ("c9300", "17.6");
pub(crate) const C9200_17_6: (&str, &str) = ("c9200", "17.6");
const BOTH: &[&str] = &["network-advantage", "network-essentials"];
const ADVANTAGE: &[&str] = &["network-advantage"];
const ESSENTIALS: &[&str] = &["network-essentials"];

/// A `REFUSED` entry under `ptp profile dot1as`, refused at its second word.
const fn dot1as(words: &'static [&'static str]) -> Refusal {
    Refusal {
        book: C9300_17_6.0,
        release: C9300_17_6.1,
        levels: ADVANTAGE,
        mode: "config",
        words,
        state: "ptp profile dot1as",
        caret: Caret::At(1),
    }
}

/// Lines the lab C9300-24T (17.6.4) refused that its crawl's answers offer. It refused
/// `class <name>` at global configuration past `class` at both levels and took `class-map <name>`;
/// at network-advantage under `ptp profile dot1as` it refused `ptp mode` and `ptp transport` at
/// their second word and took `ptp priority1`. The lab C9200L-48T-4G (17.6.4) refused `class
/// <name>` alike at network-essentials.
pub const REFUSED: &[Refusal] = &[
    Refusal {
        book: C9300_17_6.0,
        release: C9300_17_6.1,
        levels: BOTH,
        mode: "config",
        words: &["class", "<word>"],
        state: "",
        caret: Caret::After(0),
    },
    Refusal {
        book: C9200_17_6.0,
        release: C9200_17_6.1,
        levels: ESSENTIALS,
        mode: "config",
        words: &["class", "<word>"],
        state: "",
        caret: Caret::After(0),
    },
    dot1as(&["ptp", "mode", "boundary", "delay-req"]),
    dot1as(&["ptp", "mode", "boundary", "pdelay-req"]),
    dot1as(&["ptp", "mode", "e2etransparent"]),
    dot1as(&["ptp", "mode", "p2ptransparent"]),
    dot1as(&["ptp", "transport", "ipv4", "udp"]),
];

/// How a `REFUSED` line is answered: refused with the caret under this byte, or ambiguous.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    At(usize),
    Ambiguous,
}

/// The answer `REFUSED` holds for `line` typed in `mode` on `image` (book, release) at `level`, in
/// the state `configured` describes. Each word is the entry's or a unique start of it among the
/// keywords offered before it (`keywords`, by the text before the word) and the entry's own word,
/// which the measured caret shows the switch read there; a start of the entry's word and another
/// keyword, up to the caret's word, answers ambiguous. The caret stands under the measured word.
pub fn refused(
    image: (&str, &str),
    level: &str,
    mode: &str,
    line: &str,
    configured: impl Fn(&str) -> bool,
    keywords: impl Fn(&str) -> Vec<String>,
) -> Option<Refused> {
    let toks = crate::cli::tokens(line);
    REFUSED.iter().find_map(|e| {
        if (e.book, e.release) != image
            || !e.levels.contains(&level)
            || e.mode != mode
            || toks.len() != e.words.len()
            || !(e.state.is_empty() || configured(e.state))
        {
            return None;
        }
        let (Caret::At(caret) | Caret::After(caret)) = e.caret;
        let mut ambiguous = false;
        for (i, (t, w)) in toks.iter().zip(e.words).enumerate() {
            if *w == "<word>" || t.text.eq_ignore_ascii_case(w) {
                continue;
            }
            match stands_for(&t.text, w, &keywords(&line[..t.at])) {
                Some(false) => {}
                // the switch read no word past the caret's: there only a unique start stands
                Some(true) if i <= caret => ambiguous = true,
                _ => return None,
            }
        }
        Some(if ambiguous {
            Refused::Ambiguous
        } else {
            match e.caret {
                Caret::At(k) => Refused::At(toks[k].at),
                Caret::After(k) => Refused::At(toks[k].at + toks[k].text.len()),
            }
        })
    })
}

/// Whether a crawl answer for `key` no longer holds: a predicate names it and `configured` says the
/// session has the line that changes the answer.
pub fn stale(key: &str, configured: impl Fn(&str) -> bool) -> bool {
    STATE_PREDICATES
        .iter()
        .any(|(start, line)| key.starts_with(start) && configured(line))
}

/// A crawl's help key: the line typed before `?` as `help_key` has it, each address in it
/// characterized (`values`), so a site address typed at the console finds the lab's answer and no
/// address is stored.
pub fn key(text: &str) -> String {
    help_key(text)
        .split(' ')
        .map(values)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The placeholder for a whole address: `«ipv4»` (a mask too), `«ipv6»` (full or compressed) or
/// `«mac»` (dotted, colon or hyphen form).
fn address(run: &str) -> Option<&'static str> {
    let mac = |sep: char| {
        let parts: Vec<&str> = run.split(sep).collect();
        parts.len() == 6
            && parts
                .iter()
                .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
    };
    if fits("«ipv4»", run) {
        Some("«ipv4»")
    } else if fits("«mac»", run) || mac(':') || mac('-') {
        Some("«mac»")
    } else if run.contains(':') && run.parse::<std::net::Ipv6Addr>().is_ok() {
        Some("«ipv6»")
    } else {
        None
    }
}

/// The runs of a word an address could be made of: hex digits, `.` and `:`.
fn runs(word: &str) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut start = None;
    for (i, c) in word
        .char_indices()
        .chain(std::iter::once((word.len(), ' ')))
    {
        let part = c.is_ascii_hexdigit() || c == '.' || c == ':';
        match (part, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// A word with each address in it characterized, whole or inside a longer word (`tftp://10.1.1.1/x`),
/// a prefix length after one as `«number»` (`2001:db8::/32` is `«ipv6»/«number»`). One normalization
/// for help keys and entering lines.
pub fn values(word: &str) -> String {
    if let Some(p) = address(word) {
        return p.to_string();
    }
    let mut out = String::with_capacity(word.len());
    let mut at = 0;
    for (s, e) in runs(word) {
        let Some(p) = address(&word[s..e]) else {
            continue;
        };
        out.push_str(&word[at..s]);
        out.push_str(p);
        at = e;
        let rest = &word[e..];
        let digits = rest
            .strip_prefix('/')
            .map_or(0, |r| r.chars().take_while(char::is_ascii_digit).count());
        if digits > 0 {
            out.push_str("/«number»");
            at = e + 1 + digits;
        }
    }
    out.push_str(&word[at..]);
    out
}

/// Whether `text` still holds an address of any kind: a dotted quad (a mask or not), an IPv6
/// address or a MAC address, whole or inside a word. Compiling a corpus drops a crawl's key or
/// context that does.
pub fn holds_address(text: &str) -> bool {
    text.split_whitespace().any(|w| {
        runs(w).into_iter().any(|(s, e)| {
            let r = &w[s..e];
            let quad = r.split('.').count() == 4
                && r.split('.')
                    .all(|o| (1..=3).contains(&o.len()) && o.chars().all(|c| c.is_ascii_digit()));
            quad || address(r).is_some()
        }) || address(w).is_some()
    })
}

/// Whether `line` opens a block from global configuration whatever submode it is typed in
/// (`GLOBAL_BLOCKS`), its keywords abbreviable.
pub fn global_block(line: &str) -> bool {
    let toks: Vec<&str> = line.split_whitespace().collect();
    GLOBAL_BLOCKS.iter().any(|(seq, names)| {
        toks.len() > seq.len()
            && names.is_none_or(|n| toks.len() == seq.len() + n)
            && seq.iter().enumerate().all(|(i, k)| {
                // a first word among every entering line's (`class` is not `class-map`), a later
                // one among the table's
                let here: Vec<&str> = if i == 0 {
                    FIRST.to_vec()
                } else {
                    GLOBAL_BLOCKS
                        .iter()
                        .filter(|(s, _)| s.len() > i && s[..i] == seq[..i])
                        .map(|(s, _)| s[i])
                        .collect()
                };
                expand(toks[i], &here) == *k
            })
    })
}

/// Whether `line`, typed in a submode entered by `outer` from global configuration, leaves that
/// submode for global configuration and opens its own block: both open global blocks. The shell's
/// frame stack and the crawl's `Stack` decide it here alike.
pub fn reopens(outer: &str, line: &str) -> bool {
    global_block(outer) && global_block(line)
}

/// How `typed` stands for the measured keyword `word` where `offered` are the keywords offered:
/// `Some(false)` where it is the word or a unique start of it among them and the word, which the
/// measurement shows the switch read there; `Some(true)` where it begins the word and another, an
/// ambiguous abbreviation; else `None`.
pub fn stands_for(typed: &str, word: &str, offered: &[String]) -> Option<bool> {
    if typed.eq_ignore_ascii_case(word) {
        return Some(false);
    }
    let mut known: Vec<&str> = offered.iter().map(String::as_str).collect();
    known.push(word);
    match reads(typed, &known) {
        Reads::One(k) if k.eq_ignore_ascii_case(word) => Some(false),
        Reads::Several if word.to_lowercase().starts_with(&typed.to_lowercase()) => Some(true),
        _ => None,
    }
}

/// How a typed word reads against the keywords offered where it is typed, case ignored.
#[derive(Debug, PartialEq, Eq)]
pub enum Reads<'k> {
    /// one of them, whole
    Exact,
    /// the start of just this one
    One(&'k str),
    /// the start of two or more
    Several,
    /// the start of none
    None,
}

/// How `word` reads against `known` (`Reads`).
pub fn reads<'k>(word: &str, known: &[&'k str]) -> Reads<'k> {
    let w = word.to_lowercase();
    if known.iter().any(|k| k.eq_ignore_ascii_case(&w)) {
        return Reads::Exact;
    }
    let mut starts: Vec<&'k str> = known
        .iter()
        .copied()
        .filter(|k| k.to_lowercase().starts_with(w.as_str()))
        .collect();
    starts.sort_by_key(|k| k.to_lowercase());
    starts.dedup_by_key(|k| k.to_lowercase());
    match starts[..] {
        [] => Reads::None,
        [k] => Reads::One(k),
        _ => Reads::Several,
    }
}

/// A word as IOS reads an abbreviation of one of `known`: itself when it is one, the one it begins
/// when it is two letters or more and begins just one, else as typed.
fn expand(word: &str, known: &[&str]) -> String {
    let w = word.to_lowercase();
    match reads(&w, known) {
        Reads::One(k) if w.len() >= 2 => k.to_string(),
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

/// `address-family` and its keywords, each expanded from its abbreviation (`uni`, `vr`) before the
/// values are told apart: a VRF's name, an AS number and any word no keyword begins are left out.
fn family_segment(rest: &[&str]) -> String {
    let mut out = vec!["address-family".to_string()];
    let mut skip = false;
    for w in rest {
        if std::mem::take(&mut skip) {
            continue;
        }
        let w = expand(w, FAMILY_WORDS);
        if !FAMILY_WORDS.contains(&w.as_str()) {
            // a value, or an abbreviation IOS would refuse as ambiguous: neither names a context
            continue;
        }
        skip = w == "vrf";
        out.push(w);
    }
    out.join(" ")
}

/// Any other entering line, its object names and numbers characterized (`policy-map «word»`,
/// `track «number» interface «word» line-protocol`). A keyword of a `NAMED` sequence is expanded from
/// its abbreviation before the word after the sequence is taken as a name.
fn generic_segment(toks: &[&str]) -> String {
    let mut out: Vec<String> = vec![];
    for (i, t) in toks.iter().enumerate() {
        let lower = t.to_lowercase();
        let before: Vec<&str> = out.iter().map(String::as_str).collect();
        let here: Vec<&str> = NAMED
            .iter()
            .filter(|e| e.len() > i && e[..i] == before[..])
            .map(|e| e[i])
            .collect();
        let word = expand(t, &here);
        let continues = here.contains(&word.as_str());
        let named = !continues && NAMED.iter().any(|e| e.len() == i && e[..] == before[..]);
        out.push(if continues {
            word
        } else if named {
            "«word»".into()
        } else if lower.chars().all(|c| c.is_ascii_digit()) {
            "«number»".into()
        } else if values(&lower) != lower {
            // an address or prefix as its placeholder; a word holding one is a name
            let v = values(&lower);
            if v.split('/').all(|p| p.starts_with('«') && p.ends_with('»')) {
                v
            } else {
                "«word»".into()
            }
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
        } else if self.frames.len() > 1 && reopens(&self.frames[1].1, command) {
            // a block of its own opens from global configuration, whichever submode it is typed in;
            // a global line that enters nothing leaves the prompt in global configuration
            self.frames.truncate(1);
            if next != "config" {
                self.frames.push((next.to_string(), command.to_string()));
            }
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
    fn one_table_says_which_lines_open_a_block_from_global_configuration() {
        for line in [
            "interface Vlan10",
            "int gi1/0/1",
            "vlan 10",
            "line vty 0 4",
            "router ospf 1",
            "vrf definition RED",
            "ip vrf RED",
            "route-map B deny 20",
            "class-map match-all B",
            "policy-map B",
            "ip access-list standard B",
            "ip access-l ext B",
        ] {
            assert!(global_block(line), "{line}");
        }
        for line in [
            "ip vrf forwarding RED",
            "vrf forwarding RED",
            "class class-default",
            "ip address 10.1.1.1 255.255.255.0",
            "key chain TEST",
            "interface",
            "service-policy input B",
        ] {
            assert!(!global_block(line), "{line}");
        }
        assert!(reopens("policy-map A", "policy-map B"));
        assert!(!reopens("key chain A", "policy-map B"));
        assert!(!reopens("interface Vlan10", "ip vrf forwarding RED"));
    }

    #[test]
    fn blocks_of_one_kind_typed_back_to_back_each_take_their_own_context() {
        let got = walk(&[
            ("privileged", "configure terminal", Some("config")),
            ("config", "policy-map A", Some("config-pmap")),
            ("config-pmap", "class class-default", Some("config-pmap-c")),
            ("config-pmap-c", "policy-map B", Some("config-pmap")),
            ("config-pmap", " ?", Some("config-pmap")),
            (
                "config-pmap",
                "route-map A permit 10",
                Some("config-route-map"),
            ),
            (
                "config-route-map",
                "route-map B deny 20",
                Some("config-route-map"),
            ),
            ("config-route-map", " ?", Some("config-route-map")),
            (
                "config-route-map",
                "ip access-list extended A",
                Some("config-ext-nacl"),
            ),
            (
                "config-ext-nacl",
                "ip access-list standard B",
                Some("config-std-nacl"),
            ),
            ("config-std-nacl", " ?", Some("config-std-nacl")),
            (
                "config-std-nacl",
                "class-map match-any A",
                Some("config-cmap"),
            ),
            ("config-cmap", "class-map match-all B", Some("config-cmap")),
            ("config-cmap", " ?", Some("config-cmap")),
            // a global line that enters nothing leaves the block for global configuration
            (
                "config-cmap",
                "ip access-list logging interval 10",
                Some("config"),
            ),
            ("config", " ?", Some("config")),
            ("config", "interface Vlan10", Some("config-if")),
            ("config-if", "ip vrf forwarding RED", Some("config-if")),
            ("config-if", " ?", Some("config-if")),
        ]);
        let at = |i: usize| got[i].as_deref();
        assert_eq!(at(4), Some("policy-map «word»"));
        assert_eq!(at(7), Some("route-map «word» deny «number»"));
        assert_eq!(at(10), Some("ip access-list standard «word»"));
        assert_eq!(at(13), Some("class-map match-all «word»"));
        assert_eq!(at(15), Some(""));
        assert_eq!(at(18), Some("interface vlan"));
    }

    #[test]
    fn an_address_is_characterized_whole_inside_a_word_and_with_its_prefix_length() {
        for (word, want) in [
            ("10.1.1.1", "«ipv4»"),
            ("255.255.255.0", "«ipv4»"),
            ("10.0.0.0/8", "«ipv4»/«number»"),
            ("2001:0db8:0000:0000:0000:0000:0000:0001", "«ipv6»"),
            ("2001:db8::1", "«ipv6»"),
            ("fe80::1", "«ipv6»"),
            ("::", "«ipv6»"),
            ("2001:db8:1::/48", "«ipv6»/«number»"),
            ("::ffff:10.1.1.1", "«ipv6»"),
            ("0011.2233.4455", "«mac»"),
            ("00:11:22:33:44:55", "«mac»"),
            ("00-11-22-33-44-55", "«mac»"),
            ("tftp://10.1.1.1/file", "tftp://«ipv4»/file"),
            ("[2001:db8::1]:80", "[«ipv6»]:80"),
            // not addresses: a keyword, an AS number pair, a release, hex words
            ("access-list", "access-list"),
            ("65000:1", "65000:1"),
            ("17.6.4", "17.6.4"),
            ("deadbeef", "deadbeef"),
        ] {
            assert_eq!(values(word), want, "{word}");
        }
        assert_eq!(
            key("ipv6 route 2001:DB8::/32 "),
            "ipv6 route «ipv6»/«number» "
        );
        assert_eq!(key("ipv6 address FE80::1 "), "ipv6 address «ipv6» ");
        for held in [
            "ipv6 route 2001:db8::/32",
            "copy tftp://10.1.1.1/x",
            "arp 10.1.1.1",
        ] {
            assert!(holds_address(held), "{held}");
            assert!(!holds_address(&key(held)), "{held}");
        }
        assert!(holds_address("ip route 999.1.1.1"));
        assert!(!holds_address(
            "router bgp «number» > address-family ipv4 vrf"
        ));
        assert!(!holds_address("route-target 65000:1"));
    }

    #[test]
    fn an_entering_line_s_keywords_are_expanded_before_its_values_are_left_out() {
        for (a, b) in [
            ("address-family ipv4 vr PROD", "address-family ipv4 vrf RED"),
            ("address-family ipv4 uni", "address-family ipv4 unicast"),
            (
                "addr ipv4 uni vr prod",
                "address-family ipv4 unicast vrf RED",
            ),
            (
                "address-family ipv4 vrf PROD autonomous-system 10",
                "address-family ipv4 vrf RED auto 1",
            ),
            ("ip access-list ext voice", "ip access-list extended TEST"),
            ("ip acc stan voice", "ip access-list standard TEST-STD"),
            ("class-map match-any voice", "class-map match-any TEST"),
            ("ip dhcp pool lab", "ip dhcp pool TEST"),
            ("key ch corp", "key chain TEST"),
            ("username test", "username admin"),
        ] {
            assert_eq!(segment(a), segment(b), "{a} / {b}");
        }
        assert_eq!(
            segment("address-family ipv4 vr PROD"),
            "address-family ipv4 vrf"
        );
        assert_eq!(
            segment("addr ipv4 uni vr prod"),
            "address-family ipv4 unicast vrf"
        );
        assert_eq!(
            segment("ip access-list ext voice"),
            "ip access-list extended «word»"
        );
        assert_eq!(segment("icmp-echo 2001:db8::1"), "icmp-echo «ipv6»");
        assert_eq!(segment("username test"), "username «word»");
    }

    /// Every entering line of the two crawls' plans (`crawl/plan.json`, C9300 17.6 and C2960G 15.0)
    /// gives a context without a lab name, number or address.
    #[test]
    fn the_crawl_plans_entering_lines_leave_no_lab_value_in_a_context() {
        let lines = [
            "interface GigabitEthernet1/0/5",
            "interface Vlan10",
            "interface Loopback0",
            "interface Port-channel1",
            "interface Tunnel0",
            "interface GigabitEthernet0/0",
            "interface AppGigabitEthernet1/0/1",
            "interface GigabitEthernet1/0/6.201",
            "interface range GigabitEthernet1/0/7 - 8",
            "line con 0",
            "line vty 0 4",
            "ip access-list extended TEST",
            "ip access-list standard TEST-STD",
            "ipv6 access-list TEST6",
            "mac access-list extended TESTMAC",
            "ip access-list role-based TESTRB",
            "ip access-list fqdn TESTFQ",
            "router ospf 1",
            "router bgp 65000",
            "address-family ipv4",
            "address-family l2vpn evpn",
            "router eigrp 1",
            "router eigrp NAMED",
            "address-family ipv4 autonomous-system 1",
            "router rip",
            "router isis",
            "router ospfv3 1",
            "ipv6 router ospf 1",
            "router lisp",
            "vlan 10",
            "vlan configuration 10",
            "spanning-tree mst configuration",
            "vlan access-map TESTMAP 10",
            "route-map TEST permit 10",
            "class-map match-any TEST",
            "policy-map TEST",
            "class class-default",
            "police 1000000",
            "table-map TESTTM",
            "key chain TEST",
            "key 1",
            "key chain TESTMS macsec",
            "mka policy TESTMKA",
            "ip dhcp pool TEST",
            "ip dhcp class TESTCLS",
            "ipv6 dhcp pool TEST6",
            "crypto pki trustpoint TEST",
            "crypto pki certificate map TESTMAP 10",
            "ip ssh pubkey-chain",
            "username test",
            "vrf definition TESTVRF",
            "ip vrf TESTVRF2",
            "track 1 interface GigabitEthernet1/0/5 line-protocol",
            "ip sla 10",
            "icmp-echo 10.1.1.1",
            "flow record TESTREC",
            "flow exporter TESTEXP",
            "flow monitor TESTMON",
            "sampler TESTSAMP",
            "monitor session 1 type erspan-source",
            "destination",
            "monitor session 2 type erspan-destination",
            "source",
            "event manager applet TEST",
            "ip igmp profile 10",
            "object-group network TESTNG",
            "object-group service TESTSG",
            "device-tracking policy TESTDT",
            "ipv6 nd inspection policy TESTND",
            "ipv6 dhcp guard policy TESTDG",
            "ipv6 source-guard policy TESTSRC",
            "radius server TESTRAD",
            "tacacs server TESTTAC",
            "call-home",
            "profile TESTPROF",
            "template TESTTPL",
            "service-template TESTST",
            "bfd-template single-hop TESTBFD",
            "redundancy",
            "main-cpu",
            "stack-power stack TESTSP",
            "stack-power switch 1",
            "transceiver type all",
            "maintenance template TESTMT",
            "app-hosting appid TESTAPP",
            "network-policy profile 60",
            "fallback profile TESTFB",
            "class-map type control subscriber match-all TESTCC",
            "policy-map type control subscriber TESTPC",
            "location civic-location identifier 1",
            "location geo-location identifier TESTGEO",
            "l2vpn xconnect context TESTXC",
            "et-analytics",
            "ip explicit-path name TESTEP enable",
            "archive",
            "telemetry ietf subscription 1",
            "interface GigabitEthernet0/3",
            "interface range GigabitEthernet0/5 - 6",
        ];
        for line in lines {
            let s = segment(line);
            let lab = s.split_whitespace().any(|w| {
                !w.starts_with('«')
                    && (w.contains("test")
                        || w.chars()
                            .any(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                            && !matches!(w, "ipv4" | "ipv6" | "l2vpn" | "ospfv3"))
            });
            assert!(!lab && !holds_address(&s), "{line}: {s}");
        }
    }

    #[test]
    fn a_state_predicate_ends_a_crawl_answer_only_when_its_line_is_configured() {
        assert!(!stale("aaa ", |_| false));
        assert!(stale("aaa ", |l| l == "aaa new-model"));
        assert!(stale("aaa authentication ", |l| l == "aaa new-model"));
        assert!(!stale("ip ", |_| true));
        for key in ["no aaa ", "default aaa ", "no aaa accounting "] {
            assert!(!stale(key, |_| false), "{key}");
            assert!(stale(key, |l| l == "aaa new-model"), "{key}");
        }
        assert!(!stale("no aaa ", |l| l == "no aaa new-model"));
        assert!(!stale("no ip ", |_| true));
    }
}
