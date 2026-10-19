//! The show lines a sweep typed at a switch's privileged prompt, and how the switch took each on
//! Enter. The corpus holds each answered line as a form, its values typed by the `?` row they fit,
//! and each refused line as the words the switch read up to its caret, by licence level. The engine
//! reads a level's forms as commands (`cli::Engine::parse_measured`), and the shell refuses a
//! measured line before the books read it (`shell::Session::read_at`).
//!
//! A form's word is a keyword as the switch's `?` row prints it, or a value token: `<number:1-10>`,
//! `<hex:0-FFFFFFFF>`, `<interface:Vlan>`, `<ipv4>`, `<ipv4/len>`, `<ipv6>`, `<ipv6/len>`, `<mac>`,
//! `<word>` (a `WORD` row) or `<line>` (a `LINE` row, the rest of the line).

use crate::cli::{self, Outcome};
use crate::compile::{ArgKind, Command, Line};
use crate::syntax::Node;
use std::collections::{BTreeMap, BTreeSet};

/// How the switch took a swept line on Enter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Taken {
    /// output, nothing, or a state message
    Answered,
    /// `% Invalid input` with the caret `offset` characters into word `word` (from 0)
    Invalid {
        word: usize,
        offset: usize,
    },
    Incomplete,
    Ambiguous,
}

impl Taken {
    /// As the corpus writes it: empty, `invalid <word> <offset>`, `incomplete` or `ambiguous`.
    pub fn write(&self) -> String {
        match self {
            Taken::Answered => String::new(),
            Taken::Invalid { word, offset } => format!("invalid {word} {offset}"),
            Taken::Incomplete => "incomplete".into(),
            Taken::Ambiguous => "ambiguous".into(),
        }
    }

    pub fn read(s: &str) -> Option<Taken> {
        let words: Vec<&str> = s.split_whitespace().collect();
        match words[..] {
            [] => Some(Taken::Answered),
            ["invalid", w, o] => Some(Taken::Invalid {
                word: w.parse().ok()?,
                offset: o.parse().ok()?,
            }),
            ["incomplete"] => Some(Taken::Incomplete),
            ["ambiguous"] => Some(Taken::Ambiguous),
            _ => None,
        }
    }
}

/// What the sweeps measured at one licence level, as the shell reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Swept {
    /// each answered line's form, by word
    pub forms: Vec<Vec<String>>,
    /// each refused line's words up to its caret (all of them for an incomplete or ambiguous one)
    pub refusals: Vec<(Vec<String>, Taken)>,
}

impl Swept {
    pub fn add(&mut self, line: &str, taken: Taken) {
        let words = line.split_whitespace().map(str::to_string).collect();
        match taken {
            Taken::Answered => self.forms.push(words),
            t => self.refusals.push((words, t)),
        }
    }

    /// The refusal the switch gave `line` where it is a measured one: the line itself, its words
    /// before the caret written as any unique start of the keyword offered there (`keywords`, by
    /// the text before the word), a number or interface as IOS writes it. The caret word is the
    /// measured word whole, or under `<word>` any word starting no keyword offered there; the words
    /// after it are the measured ones, `<word>` any word.
    pub fn refused(
        &self,
        line: &str,
        keywords: &dyn Fn(&str) -> Option<Vec<String>>,
    ) -> Option<Outcome> {
        let toks = cli::tokens(line);
        // the keywords offered before each word, asked once
        let offered: Vec<std::cell::OnceCell<Option<Vec<String>>>> =
            toks.iter().map(|_| std::cell::OnceCell::new()).collect();
        for (words, taken) in &self.refusals {
            if toks.len() != words.len() {
                continue;
            }
            let fits = (0..words.len()).all(|i| {
                // the caret word and those after it as measured, and every word of an ambiguous line
                let exact = match taken {
                    Taken::Invalid { word, .. } => i >= *word,
                    Taken::Ambiguous => true,
                    _ => false,
                };
                let after = matches!(taken, Taken::Invalid { word, .. } if i > *word);
                if after && words[i] == "<word>" {
                    return true;
                }
                let known = || {
                    offered[i]
                        .get_or_init(|| keywords(&line[..toks[i].at]))
                        .as_deref()
                };
                same(&words[i], &toks[i].text, exact, &known)
            });
            if fits {
                return Some(match taken {
                    Taken::Invalid { word, offset } => Outcome::Invalid(toks[*word].at + offset),
                    Taken::Incomplete => Outcome::Incomplete,
                    _ => Outcome::Ambiguous,
                });
            }
        }
        None
    }
}

/// Whether `typed` stands where a measured refusal holds `stored` (`Swept::refused`), `known` the
/// keywords offered there.
fn same<'k>(
    stored: &str,
    typed: &str,
    exact: bool,
    known: &dyn Fn() -> Option<&'k [String]>,
) -> bool {
    let reads = |known: &[String]| {
        let known: Vec<&str> = known.iter().map(String::as_str).collect();
        match crate::context::reads(typed, &known) {
            crate::context::Reads::One(k) => Some(Some(k.to_lowercase())),
            crate::context::Reads::None => Some(None),
            _ => None,
        }
    };
    if stored == "<word>" {
        // a word no keyword offered there begins
        return known().and_then(reads) == Some(None);
    }
    if stored.starts_with('<') {
        return kind_of(stored).is_some_and(|k| takes(&k, typed));
    }
    if stored.eq_ignore_ascii_case(typed) {
        return true;
    }
    if exact {
        return false;
    }
    if digits(stored) && digits(typed) {
        return stored.parse::<u64>().ok() == typed.parse::<u64>().ok();
    }
    if let (Some(a), Some(b)) = (
        cli::interface_name(stored, None),
        cli::interface_name(typed, None),
    ) {
        return a.0 == b.0 && a.1 == b.1;
    }
    known().and_then(reads) == Some(Some(stored.to_lowercase()))
}

fn digits(w: &str) -> bool {
    !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit())
}

/// The argument a value token stands for.
pub fn kind_of(token: &str) -> Option<ArgKind> {
    let inner = token.strip_prefix('<')?.strip_suffix('>')?;
    let (name, arg) = inner.split_once(':').unwrap_or((inner, ""));
    Some(match name {
        "number" => {
            let (min, max) = decimal_range(arg)?;
            ArgKind::Number { min, max }
        }
        "hex" => {
            let (min, max) = hex_range(arg)?;
            ArgKind::Hex { min, max }
        }
        "interface" if !arg.is_empty() => ArgKind::InterfaceOf {
            of: arg.to_string(),
        },
        "ipv4" => ArgKind::Ipv4,
        "ipv4/len" => ArgKind::Ipv4Prefix,
        "ipv6" => ArgKind::Ipv6,
        "ipv6/len" => ArgKind::Ipv6Prefix,
        "mac" => ArgKind::Mac,
        "word" => ArgKind::Word,
        "line" => ArgKind::Text,
        _ => return None,
    })
}

/// `1-10` or `-10-10`: a decimal range as a `?` row prints it inside `<>`.
fn decimal_range(s: &str) -> Option<(i64, i64)> {
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (a, b) = rest.split_once('-')?;
    if !digits(a) || !(digits(b) || b.strip_prefix('-').is_some_and(digits)) {
        return None;
    }
    let lo: i64 = a.parse().ok()?;
    Some((if neg { -lo } else { lo }, b.parse().ok()?))
}

/// `0-FFFFFFFF` or `0x0-0xFFFFFFFF`: a range of hexadecimal numbers, one bound at least written in
/// hex.
fn hex_range(s: &str) -> Option<(u64, u64)> {
    if decimal_range(s).is_some() {
        return None;
    }
    let (a, b) = s.split_once('-')?;
    Some((hex(a)?, hex(b)?))
}

fn hex(w: &str) -> Option<u64> {
    let d = w
        .strip_prefix("0x")
        .or_else(|| w.strip_prefix("0X"))
        .unwrap_or(w);
    if d.is_empty() || !d.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(d, 16).ok()
}

/// Whether one word is a value of `kind`, for the kinds a form or refusal holds.
pub fn takes(kind: &ArgKind, w: &str) -> bool {
    match kind {
        ArgKind::Number { min, max } => {
            let unsigned = match w.strip_prefix('-') {
                Some(d) if *min < 0 => d,
                _ => w,
            };
            digits(unsigned) && w.parse::<i64>().is_ok_and(|v| v >= *min && v <= *max)
        }
        ArgKind::Hex { min, max } => hex(w).is_some_and(|v| v >= *min && v <= *max),
        ArgKind::Ipv4 => cli::ipv4(w),
        ArgKind::Ipv4Prefix => w.split_once('/').is_some_and(|(a, n)| {
            cli::ipv4(a) && digits(n) && n.parse::<u8>().is_ok_and(|n| n <= 32)
        }),
        ArgKind::Ipv6 => ipv6(w),
        ArgKind::Ipv6Prefix => w
            .split_once('/')
            .is_some_and(|(a, n)| ipv6(a) && digits(n) && n.parse::<u8>().is_ok_and(|n| n <= 128)),
        ArgKind::Mac => {
            let parts: Vec<&str> = w.split('.').collect();
            parts.len() == 3
                && parts
                    .iter()
                    .all(|p| p.len() == 4 && p.bytes().all(|b| b.is_ascii_hexdigit()))
        }
        ArgKind::InterfaceOf { of } => interface_of(w, None, of).is_some(),
        ArgKind::Word | ArgKind::Text => !w.is_empty(),
        _ => false,
    }
}

fn ipv6(w: &str) -> bool {
    w.contains(':') && w.parse::<std::net::Ipv6Addr>().is_ok()
}

/// An interface of type `of` named at `first` (and its number at `second`): its full name and
/// number, and the words it takes. A type vswitch models is read as `cli::interface_name` reads it;
/// another is its own name, whole, then its number.
pub fn interface_of(first: &str, second: Option<&str>, of: &str) -> Option<(String, usize)> {
    if let Some((full, num, n)) = cli::interface_name(first, second) {
        return full
            .eq_ignore_ascii_case(of)
            .then(|| (format!("{full}{num}"), n));
    }
    let numbered = |n: &str| {
        n.starts_with(|c: char| c.is_ascii_digit())
            && n.chars()
                .all(|c| c.is_ascii_digit() || c == '/' || c == '.' || c == ':')
    };
    let rest = first.get(of.len()..)?;
    if !first[..of.len()].eq_ignore_ascii_case(of) {
        return None;
    }
    if !rest.is_empty() {
        return numbered(rest).then(|| (format!("{of}{rest}"), 1));
    }
    let n = second?;
    numbered(n).then(|| (format!("{of}{n}"), 2))
}

/// One command per first two words of a level's forms, each a tree of the forms beginning with
/// them, run at the privileged prompt.
pub fn commands(level: &str, forms: &[Vec<String>]) -> Vec<Command> {
    #[derive(Default)]
    struct Trie {
        end: bool,
        kids: BTreeMap<String, Trie>,
    }
    fn item(tok: &str) -> Node {
        match tok.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
            Some(name) => Node::Arg(name.to_string()),
            None => Node::Kw(tok.to_string()),
        }
    }
    fn node(tok: &str, t: &Trie) -> Node {
        if t.kids.is_empty() {
            return item(tok);
        }
        let mut kids: Vec<Node> = t.kids.iter().map(|(k, c)| node(k, c)).collect();
        let mut rest = if kids.len() == 1 {
            kids.remove(0)
        } else {
            Node::Alt(kids)
        };
        if t.end {
            rest = Node::Opt(Box::new(rest));
        }
        match rest {
            Node::Seq(mut v) => {
                v.insert(0, item(tok));
                Node::Seq(v)
            }
            r => Node::Seq(vec![item(tok), r]),
        }
    }
    let mut groups: BTreeMap<(String, String), Trie> = BTreeMap::new();
    for f in forms {
        let Some(first) = f.first() else { continue };
        let second = f.get(1).cloned().unwrap_or_default();
        let mut t = groups.entry((first.clone(), second)).or_default();
        for w in &f[1..] {
            t = t.kids.entry(w.clone()).or_default();
        }
        t.end = true;
    }
    let mut out = vec![];
    for ((first, second), t) in groups {
        let mut args = BTreeMap::new();
        let mut stack = vec![&t];
        while let Some(t) = stack.pop() {
            for (k, c) in &t.kids {
                if let Some(kind) = kind_of(k) {
                    args.insert(k[1..k.len() - 1].to_string(), kind);
                }
                stack.push(c);
            }
        }
        let tree = if second.is_empty() {
            item(&first)
        } else {
            node(&first, &t)
        };
        let name = format!("{first} {second}");
        out.push(Command {
            name: format!("{} (measured)", name.trim()),
            source: format!("measured at {level}"),
            modes: vec!["privileged".into()],
            lines: vec![Line {
                text: name.trim().to_string(),
                node: tree,
                negated: false,
                repairs: vec![],
            }],
            args,
            examples: vec![],
            enters: None,
            evidence: None,
            help: BTreeMap::new(),
        });
    }
    out
}

/// Why a swept line has no row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Left {
    /// the crawl at its level held no answer before one of its words
    NoAnswer,
    /// the crawl's `?` before one of its words was refused
    Refused,
    /// a word fits no row of the answer before it
    NoFit,
    /// a word names an object the switch's configuration made (a `User defined` row)
    Configured,
    /// the word under a caret holds an address or a name, which no row reads as a pattern
    Caret,
}

/// The rows a sweep's Enter log gives at one level, as (line, `Taken::write`), and the lines left
/// out by why. `tiers` are the image's crawls, the sweep's own level first (`Reader`).
pub fn derive(
    text: &str,
    tiers: &[&Answers<'_>],
) -> (BTreeSet<(String, String)>, BTreeMap<Left, usize>) {
    let mut rows = BTreeSet::new();
    let mut left: BTreeMap<Left, usize> = BTreeMap::new();
    let Some((host, exchanges)) = crate::capture::exchanges(text) else {
        return (rows, left);
    };
    let prompt = host.chars().count() + 1;
    let reader = Reader {
        tiers: tiers.to_vec(),
    };
    for e in exchanges
        .iter()
        .filter(|e| e.mode == "privileged" && !e.command.trim().is_empty())
    {
        let words: Vec<&str> = e.command.split_whitespace().collect();
        let taken = taken(e, prompt);
        let row = match &taken {
            Taken::Answered => reader.read(&words, words.len(), false).map(|(t, _)| t),
            Taken::Incomplete | Taken::Ambiguous => {
                reader.read(&words, words.len(), true).map(|(t, _)| t)
            }
            Taken::Invalid { word, .. } => reader.refused(&words, *word),
        };
        match row {
            Ok(tokens) => {
                rows.insert((tokens.join(" "), taken.write()));
            }
            Err(why) => *left.entry(why).or_default() += 1,
        }
    }
    (rows, left)
}

/// How the switch took an exchange: its first answer lines, the caret's column counted from the
/// line's start (`prompt` characters in).
fn taken(e: &crate::capture::Exchange, prompt: usize) -> Taken {
    let body: Vec<&String> = e.output.iter().filter(|l| !l.trim().is_empty()).collect();
    match body[..] {
        [caret, said, ..] if caret.trim() == "^" && said.starts_with("% Invalid input") => {
            let col = caret.find('^').unwrap_or(0).saturating_sub(prompt);
            let toks = cli::tokens(&e.command);
            // a caret past a word's end (on the space after it) is still that word's
            let word = toks.iter().rposition(|t| t.at <= col).unwrap_or(0);
            Taken::Invalid {
                word,
                offset: col - toks.get(word).map_or(0, |t| t.at),
            }
        }
        [first, ..] if first.starts_with("% Incomplete command") => Taken::Incomplete,
        [first, ..] if first.starts_with("% Ambiguous command") => Taken::Ambiguous,
        _ => Taken::Answered,
    }
}

/// A crawl's privileged EXEC answers at one level, by `context::key` of the line before `?`.
pub type Answers<'a> = BTreeMap<String, &'a Vec<String>>;

/// The crawls a sweep's words are read against: the sweep's own level first, then the image's other
/// levels, whose rows type a value where the level's own `?` leaves its word out (the lab
/// C9300-24T, 17.6.4, answered `show mpls traffic-eng ...` lines at network-essentials, whose
/// `show ?` lists no `mpls`).
struct Reader<'a> {
    tiers: Vec<&'a Answers<'a>>,
}

impl Reader<'_> {
    /// A crawl's answer after the words it typed.
    fn answer<'t>(tier: &Answers<'t>, typed: &[String]) -> Option<&'t Vec<String>> {
        let text = match typed {
            [] => String::new(),
            t => format!("{} ", t.join(" ")),
        };
        tier.get(&crate::context::key(&text)).copied()
    }

    /// The first tier's answer after `typed`.
    fn first(&self, typed: &[String]) -> Option<&Vec<String>> {
        self.tiers.iter().find_map(|t| Self::answer(t, typed))
    }

    /// The tokens of the first `upto` words as the crawls' answers read them (`fit`), the first tier
    /// whose answer a word fits; a number or an interface written as typed where `literal`; and the
    /// words the crawls typed for them. The answer after a value a crawl typed otherwise is the one
    /// after its value of the same kind.
    fn read(
        &self,
        words: &[&str],
        upto: usize,
        literal: bool,
    ) -> Result<(Vec<String>, Vec<String>), Left> {
        let mut tokens: Vec<String> = vec![];
        let mut typed: Vec<String> = vec![];
        for (k, w) in words.iter().enumerate().take(upto) {
            let mut why = Left::NoAnswer;
            let mut read = None;
            for t in &self.tiers {
                match Self::answer(t, &typed) {
                    None => {}
                    Some(a) if crate::live::is_refusal(a) => why = why.max(Left::Refused),
                    Some(a) => match fit(w, a) {
                        Ok(tok) => {
                            read = Some((tok, *t, a));
                            break;
                        }
                        Err(Left::Configured) => {
                            why = Left::Configured;
                            break;
                        }
                        Err(e) => why = why.max(e),
                    },
                }
            }
            let (tok, at) = match read {
                Some((tok, t, a)) => (tok, Some((t, a))),
                // the first word, where the crawl asked nothing before it
                None if k == 0 => (w.to_string(), None),
                // a word the switch read before its caret, which the `?` rows leave out
                None if literal => (shaped(w).ok_or(why)?, None),
                None => return Err(why),
            };
            if tok == "<line>" {
                tokens.push(tok);
                return Ok((tokens, typed));
            }
            let mut next = typed.clone();
            next.push(w.to_string());
            if k + 1 < upto
                && self.first(&next).is_none()
                && let Some((t, a)) = at
                && let Some(s) = Self::sibling(t, &typed, &tok, a)
            {
                next = typed.clone();
                next.push(s);
            }
            typed = next;
            let keep = literal
                && (tok.starts_with("<number:")
                    || tok.starts_with("<hex:")
                    || tok.starts_with("<interface:"));
            tokens.push(if keep { w.to_string() } else { tok });
        }
        Ok((tokens, typed))
    }

    /// A word a crawl typed after `typed` that fits `answer` as `tok` does, as its key holds it.
    fn sibling(
        tier: &Answers<'_>,
        typed: &[String],
        tok: &str,
        answer: &[String],
    ) -> Option<String> {
        let prefix = crate::context::key(&match typed {
            [] => String::new(),
            t => format!("{} ", t.join(" ")),
        });
        tier.range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .filter_map(|(k, _)| {
                let w = k[prefix.len()..].strip_suffix(' ')?;
                (!w.is_empty() && !w.contains(' ')).then_some(w)
            })
            .find(|w| fit(w, answer).is_ok_and(|t| t == tok))
            .map(str::to_string)
    }

    /// A refused line's tokens, its caret in word `caret`: the words before it as `read` reads them,
    /// numbers and interfaces as typed; the caret word as typed where it is a keyword's spelling, a
    /// number or an interface, else `<word>` where the answer before it offered no value (any other
    /// word is refused there alike); the words after it by their shape, any other as `<word>`.
    fn refused(&self, words: &[&str], caret: usize) -> Result<Vec<String>, Left> {
        let (mut tokens, typed) = self.read(words, caret, true)?;
        let w = words.get(caret).ok_or(Left::NoFit)?;
        let plain = |w: &str| {
            w.starts_with(|c: char| c.is_ascii_lowercase())
                && w.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        };
        let tok = if digits(w) || cli::interface_name(w, None).is_some() || plain(w) {
            w.to_string()
        } else {
            let answer = self.first(&typed).ok_or(Left::Caret)?;
            let rows = crate::live::answer_rows(answer);
            let valueless = rows
                .iter()
                .all(|(r, _)| crate::live::keyword(r) || *r == "<cr>" || *r == "|");
            let starts = rows
                .iter()
                .any(|(r, _)| r.to_lowercase().starts_with(&w.to_lowercase()));
            if valueless && !starts && !crate::live::is_refusal(answer) {
                "<word>".to_string()
            } else if w.chars().all(|c| c.is_ascii_alphabetic() || c == '-') && !shape_of(w) {
                // a keyword spelling in capitals (`Hostname`, `BDI`)
                w.to_string()
            } else {
                return Err(Left::Caret);
            }
        };
        tokens.push(tok);
        // the words after the caret as typed, which the switch's answer to the whole line may turn on
        // (the lab C9300-24T, 17.6.4, at network-essentials refused `show m|pls traffic-eng tunnels
        // attributes 10 auto-tunnel ...` and answered `show mpls traffic-eng link-management ...`)
        for w in &words[caret + 1..] {
            tokens.push(shaped(w).unwrap_or_else(|| "<word>".into()));
        }
        Ok(tokens)
    }
}

/// Whether a word has an address's shape.
fn shape_of(w: &str) -> bool {
    shaped(w).is_some_and(|t| t.starts_with('<'))
}

/// A word read with no answer before it, by its shape alone: an address as its token, a number,
/// an interface or a lowercase keyword spelling as typed.
fn shaped(w: &str) -> Option<String> {
    for (kind, tok) in [
        (ArgKind::Ipv4, "<ipv4>"),
        (ArgKind::Ipv4Prefix, "<ipv4/len>"),
        (ArgKind::Ipv6, "<ipv6>"),
        (ArgKind::Ipv6Prefix, "<ipv6/len>"),
        (ArgKind::Mac, "<mac>"),
    ] {
        if takes(&kind, w) {
            return Some(tok.to_string());
        }
    }
    let plain = w.starts_with(|c: char| c.is_ascii_lowercase())
        && w.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    (digits(w) || cli::interface_name(w, None).is_some() || plain).then(|| w.to_string())
}

/// The token a word typed after a `?` answer is: a keyword row it spells whole, as the row prints
/// it; else the first value row it fits, in this order: an interface type with its number, a
/// decimal range, a hexadecimal range, `A.B.C.D/nn`, an IPv4 address (`A.B.C.D`, `Hostname`),
/// `X:X:X:X::X/<0-128>`, an IPv6 address (bare where only the prefix row stands: the lab C9300-24T,
/// 17.6.4, answered `show ipv6 route 2001:DB8:97::`), `H.H.H`, `WORD`, `LINE`. A key's characterized
/// address (`«ipv4»`) reads as an address.
fn fit(word: &str, answer: &[String]) -> Result<String, Left> {
    let w = match word {
        "«ipv4»" => "192.0.2.1",
        "«ipv6»" => "2001:db8::1",
        "«mac»" => "0000.0000.0001",
        "«ipv4»/«number»" => "192.0.2.0/24",
        "«ipv6»/«number»" => "2001:db8::/32",
        w => w,
    };
    let rows = crate::live::answer_rows(answer);
    let lw = w.to_lowercase();
    // a name the switch's configuration made (`MON1  User defined`, a VRF's `VRF Name`), in
    // capitals or not; the management port's `Mgmt-vrf` is every IOS XE switch's
    let made = |r: &str, what: &str| {
        ["user defined", "defined by user"].contains(&what.to_lowercase().as_str())
            || what == "VRF Name" && r != "Mgmt-vrf"
    };
    if rows
        .iter()
        .any(|(r, what)| r.to_lowercase() == lw && made(r, what))
    {
        return Err(Left::Configured);
    }
    if let Some((r, _)) = rows
        .iter()
        .find(|(r, _)| crate::live::keyword(r) && r.to_lowercase() == lw)
    {
        return Ok(r.to_string());
    }
    let interface = |r: &str| {
        r == "nve"
            || r.starts_with(|c: char| c.is_ascii_uppercase())
                && r.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
                && !matches!(r, "WORD" | "LINE")
    };
    for (r, _) in &rows {
        if interface(r) && lw.starts_with(&r.to_lowercase()) && interface_of(w, None, r).is_some() {
            return Ok(format!("<interface:{r}>"));
        }
    }
    fn inner(r: &str) -> Option<&str> {
        r.strip_prefix('<').and_then(|r| r.strip_suffix('>'))
    }
    for (r, _) in &rows {
        if let Some((min, max)) = inner(r).and_then(decimal_range)
            && takes(&ArgKind::Number { min, max }, w)
        {
            return Ok(format!("<number:{min}-{max}>"));
        }
    }
    for (r, _) in &rows {
        if let Some(i) = inner(r)
            && let Some((min, max)) = hex_range(i)
            && takes(&ArgKind::Hex { min, max }, w)
        {
            return Ok(format!("<hex:{i}>"));
        }
    }
    let has = |name: &str| rows.iter().any(|(r, _)| *r == name);
    let checks = [
        (has("A.B.C.D/nn"), ArgKind::Ipv4Prefix, "<ipv4/len>"),
        (has("A.B.C.D") || has("Hostname"), ArgKind::Ipv4, "<ipv4>"),
        (has("X:X:X:X::X/<0-128>"), ArgKind::Ipv6Prefix, "<ipv6/len>"),
        (
            has("X:X:X:X::X") || has("X:X:X:X::X/<0-128>"),
            ArgKind::Ipv6,
            "<ipv6>",
        ),
        (has("H.H.H"), ArgKind::Mac, "<mac>"),
    ];
    for (offered, kind, tok) in checks {
        if offered && takes(&kind, w) {
            return Ok(tok.to_string());
        }
    }
    if has("WORD") {
        return Ok("<word>".into());
    }
    if has("LINE") {
        return Ok("<line>".into());
    }
    Err(Left::NoFit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(rows: &[&str]) -> Vec<String> {
        rows.iter().map(|r| format!("  {r}")).collect()
    }

    #[test]
    fn a_word_is_the_keyword_it_spells_or_the_value_row_it_fits() {
        let a = answer(&[
            "<1-10>          Cache number",
            "<0-FFFFFFFF>    Hex value",
            "A.B.C.D         Address",
            "GigabitEthernet GigabitEthernet IEEE 802.3z",
            "brief           Brief",
            "WORD            Name",
        ]);
        assert_eq!(fit("brief", &a), Ok("brief".into()));
        assert_eq!(fit("BRIEF", &a), Ok("brief".into()));
        assert_eq!(fit("7", &a), Ok("<number:1-10>".into()));
        assert_eq!(fit("11", &a), Ok("<hex:0-FFFFFFFF>".into()));
        assert_eq!(fit("192.0.2.7", &a), Ok("<ipv4>".into()));
        assert_eq!(fit("«ipv4»", &a), Ok("<ipv4>".into()));
        assert_eq!(
            fit("GigabitEthernet1/0/1", &a),
            Ok("<interface:GigabitEthernet>".into())
        );
        assert_eq!(fit("anything", &a), Ok("<word>".into()));
        let b = answer(&["detail  Detail", "<cr>    <cr>"]);
        assert_eq!(fit("anything", &b), Err(Left::NoFit));
    }

    #[test]
    fn a_name_the_configuration_made_is_no_form() {
        let a = answer(&[
            "MONITOR  User defined",
            "BLUE  VRF Name",
            "Mgmt-vrf  VRF Name",
            "brief  Brief",
        ]);
        assert_eq!(fit("MONITOR", &a), Err(Left::Configured));
        assert_eq!(fit("BLUE", &a), Err(Left::Configured));
        assert_eq!(fit("Mgmt-vrf", &a), Ok("Mgmt-vrf".into()));
    }

    #[test]
    fn a_hostname_row_takes_an_address_and_no_name() {
        let a = answer(&["Hostname  IP name", "detail  Detail"]);
        assert_eq!(fit("192.0.2.1", &a), Ok("<ipv4>".into()));
        assert_eq!(fit("router", &a), Err(Left::NoFit));
    }

    #[test]
    fn a_taken_line_reads_back_as_written() {
        for t in [
            Taken::Answered,
            Taken::Invalid { word: 3, offset: 2 },
            Taken::Incomplete,
            Taken::Ambiguous,
        ] {
            assert_eq!(Taken::read(&t.write()), Some(t));
        }
    }

    #[test]
    fn value_tokens_name_their_arguments() {
        assert_eq!(
            kind_of("<number:1-10>"),
            Some(ArgKind::Number { min: 1, max: 10 })
        );
        assert_eq!(
            kind_of("<number:-5-5>"),
            Some(ArgKind::Number { min: -5, max: 5 })
        );
        assert_eq!(
            kind_of("<hex:0x0-0xFFFF>"),
            Some(ArgKind::Hex {
                min: 0,
                max: 0xffff
            })
        );
        assert_eq!(
            kind_of("<interface:Vlan>"),
            Some(ArgKind::InterfaceOf { of: "Vlan".into() })
        );
        assert_eq!(kind_of("<ipv6/len>"), Some(ArgKind::Ipv6Prefix));
        assert_eq!(kind_of("brief"), None);
        assert!(takes(&ArgKind::Hex { min: 0, max: 0xff }, "0x1f"));
        assert!(!takes(&ArgKind::Hex { min: 0, max: 0xff }, "0x100"));
        assert!(takes(&ArgKind::Ipv4Prefix, "192.0.2.0/24"));
        assert!(!takes(&ArgKind::Ipv4Prefix, "192.0.2.0/33"));
        assert!(takes(&ArgKind::Ipv6Prefix, "2001:db8::/32"));
        assert!(takes(
            &ArgKind::InterfaceOf {
                of: "Lspvif".into()
            },
            "Lspvif0"
        ));
        assert!(!takes(
            &ArgKind::InterfaceOf { of: "Vlan".into() },
            "Loopback0"
        ));
    }

    #[test]
    fn a_refusal_matches_its_line_and_unique_starts_of_its_keywords_before_the_caret() {
        let mut s = Swept::default();
        s.add(
            "show widget table Hostname",
            Taken::Invalid { word: 3, offset: 0 },
        );
        s.add("show widget gadget", Taken::Invalid { word: 2, offset: 3 });
        let offered = |text: &str| -> Option<Vec<String>> {
            let words: &[&str] = match text.split_whitespace().count() {
                0 => &["show", "ssh"],
                1 => &["widget", "wireless"],
                2 => &["table", "gadget", "gauge"],
                3 => &["Hostname"],
                _ => return None,
            };
            Some(words.iter().map(|w| w.to_string()).collect())
        };
        let at = |l: &str| s.refused(l, &offered);
        assert_eq!(at("show widget table Hostname"), Some(Outcome::Invalid(18)));
        assert_eq!(at("sh widg t Hostname"), Some(Outcome::Invalid(10)));
        // the line is the measured one, word for word
        assert_eq!(at("sh widg t Hostname x"), None);
        // the caret word is the measured one whole
        assert_eq!(at("show widget table Host"), None);
        assert_eq!(at("show widget table 192.0.2.1"), None);
        // `wi` starts two keywords
        assert_eq!(at("sh wi t Hostname"), None);
        assert_eq!(at("show widget gadget"), Some(Outcome::Invalid(15)));
        assert_eq!(at("show widget gadg"), None);
    }

    #[test]
    fn a_word_under_a_caret_where_no_value_stands_is_any_word_starting_no_keyword() {
        let mut s = Swept::default();
        s.add("show group <word>", Taken::Invalid { word: 2, offset: 0 });
        let offered = |text: &str| -> Option<Vec<String>> {
            (text.split_whitespace().count() == 2).then(|| vec!["name".to_string()])
        };
        assert_eq!(
            s.refused("show group SOMETHING", &offered),
            Some(Outcome::Invalid(11))
        );
        assert_eq!(s.refused("show group na", &offered), None);
    }

    #[test]
    fn forms_are_commands_by_their_first_two_words() {
        let forms = vec![
            vec!["show".to_string()],
            vec!["show".into(), "widget".into(), "<number:1-10>".into()],
            vec!["show".into(), "widget".into(), "brief".into()],
            vec!["show".into(), "gadget".into()],
        ];
        let c = commands("level-a", &forms);
        let names: Vec<&str> = c.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "show (measured)",
                "show gadget (measured)",
                "show widget (measured)"
            ]
        );
        assert_eq!(
            c[2].args.get("number:1-10"),
            Some(&ArgKind::Number { min: 1, max: 10 })
        );
    }
}
