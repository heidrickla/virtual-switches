//! Cisco command-reference syntax notation: keywords, `<argument>`, `[optional]`, `{a | b}`.
//!
//! The reference has typing errors (unbalanced brackets, `<| vlan-ID>`, `name<acl-name>`); the
//! parser repairs them the same way every time and lists each repair, so a compiled command says
//! which of its syntax lines were read literally.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v")]
pub enum Node {
    /// a keyword, matched by any unambiguous prefix
    Kw(String),
    /// an argument; the name is the reference's (`vlan-id`, `interface-id`)
    Arg(String),
    Seq(Vec<Node>),
    /// one of
    Alt(Vec<Node>),
    /// zero or one
    Opt(Box<Node>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Word(String),
    Arg(String),
    Open(char),
    Close(char),
    Bar,
    /// an ellipsis: the item before it may repeat (`[<method2> . . . ]`, `<file-url> ...`)
    Repeat,
}

/// Whether an argument whose name holds spaces is several arguments the reference set in one italic
/// run (`<ip-address subnet-mask>`, `<weight1 weight2>`, `<source source-wildcard>`) rather than one
/// named in words (`<vrf instance name>`, `<profile number>`): each word is an argument a Syntax
/// Description row names on its own (`known`), or every word is a hyphenated or numbered name, or
/// the run is an address and its mask or wildcard.
fn several(words: &[&str], known: &std::collections::BTreeSet<String>) -> bool {
    if words.len() < 2 {
        return false;
    }
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    // an interface's type and number are one interface argument (`gigabitethernet 1/0/2`), and a
    // file system and path one URL (`bootflash:cat9k.bin`)
    if lower.len() == 2 && interface_pair(&lower[0], &lower[1])
        || lower.iter().any(|w| w.ends_with(':') || w.starts_with('/'))
    {
        return false;
    }
    if lower.iter().all(|w| known.contains(w)) {
        return true;
    }
    // a number or a range (`10`, `-1000000`, `1-8`) names nothing: `<server-address 1>` is one
    let named =
        |w: &str| !numeric(w) && (w.contains('-') || w.ends_with(|c: char| c.is_ascii_digit()));
    if lower.iter().all(|w| named(w)) {
        return true;
    }
    // an address and what masks it: `address mask`, `source source-wildcard`
    lower.len() == 2
        && (matches!(lower[1].as_str(), "mask" | "wildcard")
            || lower[1].starts_with(&format!("{}-", lower[0])))
}

/// Whether a word is a number or a piece of a range: digits and `-` only.
fn numeric(w: &str) -> bool {
    !w.is_empty() && w.chars().all(|c| c.is_ascii_digit() || c == '-')
}

/// Whether two words name an interface's type and number.
fn interface_pair(first: &str, second: &str) -> bool {
    (first.contains("type") || first == "interface")
        && ["number", "num", "slot", "port"]
            .iter()
            .any(|n| second.contains(n))
}

/// The arguments a run of several reads as, an interface's type and number kept as one. A bare
/// `mask` or `wildcard` is named with the argument it masks (`<src-MAC-addr mask>` gives
/// `src-MAC-addr` and `src-MAC-addr mask`), so masks of different things are different arguments.
fn each_argument(words: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if i + 1 < words.len()
            && interface_pair(&words[i].to_lowercase(), &words[i + 1].to_lowercase())
        {
            out.push(format!("{} {}", words[i], words[i + 1]));
            i += 2;
        } else {
            let bare = matches!(words[i].to_lowercase().as_str(), "mask" | "wildcard");
            out.push(match out.last() {
                Some(owner) if bare => format!("{owner} {}", words[i]),
                _ => words[i].to_string(),
            });
            i += 1;
        }
    }
    out
}

/// Where the argument opened at `chars[open]` closes: the `>` that matches its `<`, nested ones
/// counted (`<<1-255>>`, `<switch number <1-2>>`).
fn closing(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (j, &c) in chars.iter().enumerate().skip(open) {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
    }
    None
}

fn lex(s: &str, repairs: &mut Vec<String>, known: &std::collections::BTreeSet<String>) -> Vec<Tok> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => i += 1,
            '{' | '[' => {
                out.push(Tok::Open(c));
                i += 1;
            }
            '}' | ']' => {
                out.push(Tok::Close(c));
                i += 1;
            }
            '|' => {
                out.push(Tok::Bar);
                i += 1;
            }
            '<' => {
                // nested brackets that never balance (`<ipv6-prefix</prefix-length>`) close at the
                // first `>`
                let first = || chars[i..].iter().position(|&x| x == '>').map(|p| i + p);
                let Some(end) = closing(&chars, i).or_else(first) else {
                    repairs.push("an argument without its closing '>'".into());
                    i += 1;
                    continue;
                };
                let inner: String = chars[i + 1..end].iter().collect();
                argument(&inner, &mut out, repairs, known);
                i = end + 1;
            }
            '>' => {
                repairs.push("stray '>' dropped".into());
                i += 1;
            }
            _ => {
                let start = i;
                while i < chars.len()
                    && !matches!(
                        chars[i],
                        ' ' | '\t' | '{' | '}' | '[' | ']' | '|' | '<' | '>'
                    )
                {
                    i += 1;
                }
                out.push(Tok::Word(chars[start..i].iter().collect()));
            }
        }
    }
    interfaces(ellipses(out), repairs)
}

/// The tokens with an interface's type and number set as two arguments (`<interface-type>
/// <interface-number>`) made one, as one italic run of them reads (`interface_pair`).
fn interfaces(toks: Vec<Tok>, repairs: &mut Vec<String>) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::new();
    for t in toks {
        if let (Some(Tok::Arg(a)), Tok::Arg(b)) = (out.last(), &t) {
            let (la, lb) = (a.to_lowercase(), b.to_lowercase());
            if la.contains("interface") && lb.contains("interface") && interface_pair(&la, &lb) {
                repairs.push(format!("arguments <{a}> <{b}> read as <{a} {b}>"));
                let one = format!("{a} {b}");
                out.pop();
                out.push(Tok::Arg(one));
                continue;
            }
        }
        out.push(t);
    }
    out
}

/// The tokens with each ellipsis, a run of words of three or more dots in all (`...`, `. . .`,
/// `. ..`), made one `Repeat`; fewer dots are a keyword (`separator { - | : | . }`).
fn ellipses(toks: Vec<Tok>) -> Vec<Tok> {
    let dots = |t: &Tok| match t {
        Tok::Word(w) if w.chars().all(|c| c == '.') => w.len(),
        _ => 0,
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let run = toks[i..].iter().take_while(|t| dots(t) > 0).count();
        if run > 0 && toks[i..i + run].iter().map(dots).sum::<usize>() >= 3 {
            out.push(Tok::Repeat);
            i += run;
        } else {
            out.extend(toks[i..i + run.max(1)].iter().cloned());
            i += run.max(1);
        }
    }
    out
}

/// An item and up to seven more of it, as an ellipsis after it reads.
fn repeated(n: Node) -> Node {
    let mut more = Node::Opt(Box::new(n.clone()));
    for _ in 1..7 {
        more = Node::Opt(Box::new(Node::Seq(vec![n.clone(), more])));
    }
    Node::Seq(vec![n, more])
}

/// The tokens of one argument, `inner` being what its `<` and `>` hold.
fn argument(
    inner: &str,
    out: &mut Vec<Tok>,
    repairs: &mut Vec<String>,
    known: &std::collections::BTreeSet<String>,
) {
    let trimmed = inner.trim();
    // braces inside the italics make a template (`http:<//[[username:password]@]{hostname | host-ip}
    // [/directory]/image-name>`): one argument, read as written
    if trimmed.contains(['{', '}']) {
        out.push(Tok::Arg(trimmed.to_string()));
        return;
    }
    // `<[options]>` is an optional argument
    if let Some(within) = trimmed
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .filter(|t| !t.contains(['[', ']']))
    {
        out.push(Tok::Open('['));
        argument(within, out, repairs, known);
        out.push(Tok::Close(']'));
        return;
    }
    // the italics took in the bar between two choices: `{default <| list-name>}`,
    // `{<file-url><| filename>}`, `{<bytes |>jumbo <bytes>}`; a bar between a keyword and the
    // argument named after it is a stray (`[vlan <| vlan-ID>]`, `port <| port_number>`)
    if let Some(rest) = trimmed.strip_prefix('|')
        && let Some(Tok::Word(w)) = out.last()
        && rest
            .trim_start()
            .to_lowercase()
            .starts_with(&w.to_lowercase())
    {
        repairs.push(format!("argument <{inner}> read as <{}>", rest.trim()));
        argument(rest, out, repairs, known);
        return;
    }
    if trimmed.contains('|') {
        let pieces: Vec<&str> = trimmed.split('|').map(str::trim).collect();
        repairs.push(format!(
            "argument <{inner}> read as {}",
            pieces
                .iter()
                .map(|p| if p.is_empty() {
                    String::new()
                } else {
                    format!("<{p}>")
                })
                .collect::<Vec<_>>()
                .join(" | ")
                .trim()
        ));
        for (n, p) in pieces.iter().enumerate() {
            if n > 0 {
                out.push(Tok::Bar);
            }
            if !p.is_empty() {
                argument(p, out, repairs, known);
            }
        }
        return;
    }
    // an argument named with its range in brackets of its own: `<<1-255>>`, `<slot-number <1-8>>`
    if trimmed.contains(['<', '>']) {
        let name: String = trimmed
            .chars()
            .filter(|c| !matches!(c, '<' | '>'))
            .collect();
        repairs.push(format!("argument <{inner}> read as <{}>", name.trim()));
        argument(&name, out, repairs, known);
        return;
    }
    // an optional part inside the italics: `<value [event-number]>` is `<value> [<event-number>]`
    if trimmed.contains(['[', ']']) && balanced(trimmed) {
        let syntax = italics(trimmed);
        repairs.push(format!("argument <{inner}> read as {syntax}"));
        out.extend(lex(&syntax, repairs, known));
        return;
    }
    let name = trimmed.trim_matches(|c: char| c == '[' || c == ']' || c.is_whitespace());
    if name != trimmed {
        repairs.push(format!("argument <{inner}> read as <{name}>"));
    }
    let words: Vec<&str> = name.split_whitespace().collect();
    if name.is_empty() {
        repairs.push(format!("empty argument <{inner}> dropped"));
    } else if let [addr, wild] = words[..]
        && wild.eq_ignore_ascii_case(&format!("{addr}-wildcard"))
    {
        // an access list's address and wildcard: `any` and `host <address>` stand for them, as
        // each such command's Syntax Description says
        repairs.push(format!(
            "argument <{name}> read as {{ <{addr}> <{wild}> | host <{addr}> | any }}"
        ));
        out.extend([
            Tok::Open('{'),
            Tok::Arg(addr.to_string()),
            Tok::Arg(wild.to_string()),
            Tok::Bar,
            Tok::Word("host".into()),
            Tok::Arg(addr.to_string()),
            Tok::Bar,
            Tok::Word("any".into()),
            Tok::Close('}'),
        ]);
    } else if words.len() > 1 && words.iter().all(|w| numeric(w)) {
        // a range set with a space: `<10 -1000000>`
        let range = words.concat();
        repairs.push(format!("argument <{name}> read as <{range}>"));
        out.push(Tok::Arg(range));
    } else if several(&words, known) {
        let each = each_argument(&words);
        repairs.push(format!(
            "argument <{name}> read as {}",
            each.iter()
                .map(|w| format!("<{w}>"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        out.extend(each.into_iter().map(Tok::Arg));
    } else {
        out.push(Tok::Arg(name.to_string()));
    }
}

/// Whether every `[` in `s` closes, in order.
fn balanced(s: &str) -> bool {
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return false;
        }
    }
    depth == 0
}

/// Italic text holding brackets as syntax: each run of words between them an argument.
fn italics(s: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if !run.trim().is_empty() {
            out.push_str(&format!("<{}>", run.trim()));
        }
        run.clear();
    };
    for c in s.chars() {
        if matches!(c, '[' | ']') {
            flush(&mut run, &mut out);
            out.push_str(&format!(" {c} "));
        } else {
            run.push(c);
        }
    }
    flush(&mut run, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    /// alternatives separated by `|` until the closer `until` (or the end)
    fn alts(&mut self, until: Option<char>, repairs: &mut Vec<String>) -> Node {
        let mut branches = vec![self.seq(until, repairs)];
        while self.pos < self.toks.len() && self.toks[self.pos] == Tok::Bar {
            self.pos += 1;
            branches.push(self.seq(until, repairs));
        }
        if branches.len() == 1 {
            branches.pop().unwrap_or(Node::Seq(vec![]))
        } else {
            Node::Alt(branches)
        }
    }

    fn seq(&mut self, until: Option<char>, repairs: &mut Vec<String>) -> Node {
        let mut items = Vec::new();
        while self.pos < self.toks.len() {
            match &self.toks[self.pos] {
                Tok::Bar => break,
                Tok::Close(c) => {
                    let want = until.map(|u| if u == '{' { '}' } else { ']' });
                    if Some(*c) == want || until.is_some() {
                        if Some(*c) != want {
                            repairs.push(format!(
                                "'{c}' closes a group opened with '{}'",
                                until.unwrap_or(' ')
                            ));
                        }
                        break;
                    }
                    repairs.push(format!("stray '{c}' dropped"));
                    self.pos += 1;
                }
                Tok::Open(c) => {
                    let c = *c;
                    self.pos += 1;
                    let inner = self.alts(Some(c), repairs);
                    if self.pos < self.toks.len() && matches!(self.toks[self.pos], Tok::Close(_)) {
                        self.pos += 1;
                    } else {
                        repairs.push(format!("'{c}' closed at the end of the line"));
                    }
                    items.push(if c == '[' {
                        Node::Opt(Box::new(inner))
                    } else {
                        inner
                    });
                }
                Tok::Word(w) => {
                    // `name<acl-name>` runs a keyword into its argument; the lexer already split them
                    items.push(Node::Kw(w.clone()));
                    self.pos += 1;
                }
                Tok::Arg(a) => {
                    items.push(Node::Arg(a.clone()));
                    self.pos += 1;
                }
                Tok::Repeat => {
                    match items.pop() {
                        Some(last) => {
                            repairs.push("ellipsis read as the item before it repeated".into());
                            items.push(repeated(last));
                        }
                        None => repairs.push("ellipsis with nothing before it dropped".into()),
                    }
                    self.pos += 1;
                }
            }
        }
        if items.len() == 1 {
            items.pop().unwrap_or(Node::Seq(vec![]))
        } else {
            Node::Seq(items)
        }
    }
}

/// The leading keywords of a node: the command's name as typed.
pub fn leading_keywords(n: &Node) -> Vec<String> {
    match n {
        Node::Kw(k) => vec![k.clone()],
        Node::Seq(items) => {
            let mut out = Vec::new();
            for it in items {
                match it {
                    Node::Kw(k) => out.push(k.clone()),
                    _ => break,
                }
            }
            out
        }
        _ => vec![],
    }
}

/// The notation a node is written in, for a repair's note.
fn written(n: &Node) -> String {
    match n {
        Node::Kw(k) => k.clone(),
        Node::Arg(a) => format!("<{a}>"),
        Node::Seq(v) => v.iter().map(written).collect::<Vec<_>>().join(" "),
        Node::Alt(v) => format!(
            "{{{}}}",
            v.iter().map(written).collect::<Vec<_>>().join(" | ")
        ),
        Node::Opt(b) => format!("[{}]", written(b)),
    }
}

/// A top-level `a <x> | b <y>` whose later branches do not repeat the command's keywords: each later
/// branch stands in for the end of the first. After a group it is more choices of that group
/// (`channel-group <n> mode {auto | on} | {active | passive}`); otherwise it stands in for as many of
/// the first branch's last items as it has, where their kinds match and hold an argument
/// (`dtls connectiontimeout <s> | idletimeout <s>`), else for all after the command's `keywords`
/// (`carrier-delay <seconds> | msec <milliseconds>`), the first branch keeping at least its first item.
fn continued(first: &Node, later: &[Node], keywords: usize, repairs: &mut Vec<String>) -> Node {
    let mut items: Vec<Node> = match first {
        Node::Seq(v) => v.clone(),
        n => vec![n.clone()],
    };
    let group = |n: &Node| matches!(n, Node::Alt(_) | Node::Opt(_) | Node::Seq(_));
    if items.len() > 1
        && let Some(last) = items.pop_if(|n| group(n))
    {
        let mut choice = match last {
            Node::Alt(v) => v,
            n => vec![n],
        };
        for b in later {
            match b {
                Node::Alt(v) => choice.extend(v.iter().cloned()),
                n => choice.push(n.clone()),
            }
        }
        repairs.push("top-level alternatives read as more choices of the group before them".into());
        items.push(Node::Alt(choice));
        return Node::Seq(items);
    }
    let kind = |n: &Node| match n {
        Node::Kw(_) => 0,
        Node::Arg(_) => 1,
        _ => 2,
    };
    let shape: Vec<u8> = match &later[0] {
        Node::Seq(v) => v.iter().map(kind).collect(),
        n => vec![kind(n)],
    };
    // a run of keywords alone matches any other and says nothing
    let mirrored = shape.contains(&1)
        && shape.len() < items.len()
        && items[items.len() - shape.len()..]
            .iter()
            .map(kind)
            .eq(shape.iter().copied());
    let cut = if mirrored {
        items.len() - shape.len()
    } else {
        keywords.min(items.len() - 1).max(1)
    };
    let mut choice = vec![Node::Seq(items.split_off(cut))];
    choice.extend(later.iter().cloned());
    let prefix: Vec<String> = items.iter().map(written).collect();
    repairs.push(format!(
        "top-level alternatives read as '{} {{...}}'",
        prefix.join(" ")
    ));
    items.push(Node::Alt(choice));
    Node::Seq(items)
}

/// Parses one syntax line. A top-level `a <x> | b <y>` whose later branches do not repeat the
/// command's keywords is read as the command followed by one of the branches (`continued`).
pub fn parse(line: &str) -> (Node, Vec<String>) {
    parse_with(line, &std::collections::BTreeSet::new())
}

/// `parse`, with the arguments the command's Syntax Description rows name one by one (lowercase),
/// the evidence that a run of italic words is several arguments.
pub fn parse_with(line: &str, known: &std::collections::BTreeSet<String>) -> (Node, Vec<String>) {
    let mut repairs = Vec::new();
    let toks = lex(line, &mut repairs, known);
    let mut p = Parser { toks, pos: 0 };
    let mut node = p.alts(None, &mut repairs);
    while p.pos < p.toks.len() {
        // a closer with nothing open: drop it and read on
        repairs.push("stray closer at the top level dropped".into());
        p.pos += 1;
        let more = p.alts(None, &mut repairs);
        node = Node::Seq(vec![node, more]);
    }
    if let Node::Alt(branches) = &node {
        let head = leading_keywords(&branches[0]);
        if let Some(first) = head.first()
            && branches[1..]
                .iter()
                .all(|b| leading_keywords(b).first() != Some(first))
        {
            node = continued(&branches[0], &branches[1..], head.len(), &mut repairs);
        }
    }
    (node, repairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kw(s: &str) -> Node {
        Node::Kw(s.into())
    }

    #[test]
    fn keywords_arguments_options_and_alternatives() {
        let (n, r) = parse("switchport trunk {allowed vlan <vlan-list> | native vlan <vlan-id>}");
        assert!(r.is_empty(), "{r:?}");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("switchport"),
                kw("trunk"),
                Node::Alt(vec![
                    Node::Seq(vec![
                        kw("allowed"),
                        kw("vlan"),
                        Node::Arg("vlan-list".into())
                    ]),
                    Node::Seq(vec![kw("native"), kw("vlan"), Node::Arg("vlan-id".into())]),
                ]),
            ])
        );
        let (n, _) = parse("show vlan [brief | id <vlan-id>]");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("show"),
                kw("vlan"),
                Node::Opt(Box::new(Node::Alt(vec![
                    kw("brief"),
                    Node::Seq(vec![kw("id"), Node::Arg("vlan-id".into())])
                ]))),
            ])
        );
    }

    #[test]
    fn the_reference_s_typing_errors_are_repaired_and_listed() {
        let (n, r) = parse("show running-config <[options]>");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("show"),
                kw("running-config"),
                Node::Opt(Box::new(Node::Arg("options".into())))
            ])
        );
        assert!(
            r.is_empty(),
            "a bracketed argument is the documented optional form: {r:?}"
        );
        let (n, r) = parse("show x [vlan <| vlan-ID>]");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("show"),
                kw("x"),
                Node::Opt(Box::new(Node::Seq(vec![
                    kw("vlan"),
                    Node::Arg("vlan-ID".into())
                ])))
            ])
        );
        assert_eq!(r.len(), 1);
        let (_, r) = parse("show a [b [c]");
        assert!(r.iter().any(|x| x.contains("closed at the end")), "{r:?}");
        let (_, r) = parse("show a b] c");
        assert!(r.iter().any(|x| x.contains("stray")), "{r:?}");
    }

    #[test]
    fn a_top_level_bar_continues_the_command() {
        let (n, r) = parse("carrier-delay <seconds> | msec <milliseconds>");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("carrier-delay"),
                Node::Alt(vec![
                    Node::Seq(vec![Node::Arg("seconds".into())]),
                    Node::Seq(vec![kw("msec"), Node::Arg("milliseconds".into())])
                ]),
            ])
        );
        assert_eq!(r.len(), 1);
    }

    /// The argument names of a line, in order.
    fn args(line: &str, known: &[&str]) -> Vec<String> {
        let known = known.iter().map(|k| k.to_string()).collect();
        let mut names = vec![];
        fn walk(n: &Node, out: &mut Vec<String>) {
            match n {
                Node::Arg(a) => out.push(a.clone()),
                Node::Seq(v) | Node::Alt(v) => v.iter().for_each(|x| walk(x, out)),
                Node::Opt(b) => walk(b, out),
                Node::Kw(_) => {}
            }
        }
        walk(&parse_with(line, &known).0, &mut names);
        names
    }

    #[test]
    fn an_italic_run_of_several_arguments_is_read_as_each() {
        // hyphenated or numbered names, and an address with its mask
        assert_eq!(
            args("ip address <ip-address subnet-mask>", &[]),
            ["ip-address", "subnet-mask"]
        );
        assert_eq!(args("x <weight1 weight2>", &[]), ["weight1", "weight2"]);
        assert_eq!(
            args("ip address <address mask>", &[]),
            ["address", "address mask"]
        );
        // words a Syntax Description row names one by one
        assert_eq!(
            args("clock timezone <zone hours-offset>", &[]),
            ["zone hours-offset"]
        );
        assert_eq!(
            args(
                "clock timezone <zone hours-offset>",
                &["zone", "hours-offset"]
            ),
            ["zone", "hours-offset"]
        );
        // one argument named in words, and an interface's type and number
        assert_eq!(args("vrf <vrf instance name>", &[]), ["vrf instance name"]);
        assert_eq!(
            args(
                "ip ftp source-interface <interface-type interface-number>",
                &[]
            ),
            ["interface-type interface-number"]
        );
        assert_eq!(
            args(
                "ip route static bfd <interface-type interface-number ip-address>",
                &[]
            ),
            ["interface-type interface-number", "ip-address"]
        );
        // the same pair set as two arguments
        assert_eq!(
            args(
                "show ipv6 mld groups [<group-name><| group-address>] [<interface-type> <interface-number>]",
                &[]
            ),
            [
                "group-name",
                "group-address",
                "interface-type interface-number"
            ]
        );
        assert_eq!(args("show x <type> <number>", &[]), ["type", "number"]);
        // a file system and its path are one URL, whatever rows name them
        assert_eq!(
            args(
                "boot system <filesystem: /file-url>",
                &["filesystem:", "/file-url"]
            ),
            ["filesystem: /file-url"]
        );
        // an access list's address and wildcard, with `host` and `any` standing for them
        let (node, _) = parse("permit ip <source source-wildcard>");
        assert_eq!(
            node,
            Node::Seq(vec![
                kw("permit"),
                kw("ip"),
                Node::Alt(vec![
                    Node::Seq(vec![
                        Node::Arg("source".into()),
                        Node::Arg("source-wildcard".into())
                    ]),
                    Node::Seq(vec![kw("host"), Node::Arg("source".into())]),
                    kw("any"),
                ]),
            ])
        );
    }

    #[test]
    fn a_number_or_a_range_in_an_italic_run_names_no_argument_of_its_own() {
        // `mac address-table aging-time { 0 | <10 -1000000>}` takes one value
        let (n, r) = parse("mac address-table aging-time { 0 | <10 -1000000>}");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("mac"),
                kw("address-table"),
                kw("aging-time"),
                Node::Alt(vec![kw("0"), Node::Arg("10-1000000".into())]),
            ])
        );
        assert_eq!(r, ["argument <10 -1000000> read as <10-1000000>"]);
        // the C9200's `ip name-server vrf Mgmt-vrf <server-address 1> [<server-address 2>] ...`
        assert_eq!(
            args(
                "ip name-server vrf Mgmt-vrf <server-address 1> [<server-address 2>]",
                &[]
            ),
            ["server-address 1", "server-address 2"]
        );
        assert_eq!(args("x <slot-number <1-8>>", &[]), ["slot-number 1-8"]);
    }

    #[test]
    fn a_mask_is_named_after_what_it_masks() {
        assert_eq!(
            args(
                "permit {any | host <src-MAC-addr> | <src-MAC-addr mask>} {any | host <dst-MAC-addr> | <dst-MAC-addr mask>} [<type mask> | lsap <lsap mask>]",
                &[]
            ),
            [
                "src-MAC-addr",
                "src-MAC-addr",
                "src-MAC-addr mask",
                "dst-MAC-addr",
                "dst-MAC-addr",
                "dst-MAC-addr mask",
                "type",
                "type mask",
                "lsap",
                "lsap mask"
            ]
        );
    }

    #[test]
    fn a_bar_the_italics_took_in_separates_the_choices_either_side() {
        let (n, r) = parse("aaa authentication login {default <| list-name>} <method1>");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("aaa"),
                kw("authentication"),
                kw("login"),
                Node::Alt(vec![kw("default"), Node::Arg("list-name".into())]),
                Node::Arg("method1".into()),
            ])
        );
        assert_eq!(r.len(), 1, "{r:?}");
        let (n, _) = parse("boot system {<file-url><| filename>}");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("boot"),
                kw("system"),
                Node::Alt(vec![
                    Node::Arg("file-url".into()),
                    Node::Arg("filename".into())
                ]),
            ])
        );
        let (n, _) = parse("system mtu {<bytes |>jumbo <bytes>}");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("system"),
                kw("mtu"),
                Node::Alt(vec![
                    Node::Arg("bytes".into()),
                    Node::Seq(vec![kw("jumbo"), Node::Arg("bytes".into())]),
                ]),
            ])
        );
        assert_eq!(
            parse("traceroute mac ip {<source-ip-address | source-hostname>}").0,
            Node::Seq(vec![
                kw("traceroute"),
                kw("mac"),
                kw("ip"),
                Node::Alt(vec![
                    Node::Arg("source-ip-address".into()),
                    Node::Arg("source-hostname".into())
                ]),
            ])
        );
        // a bar inside braces inside the italics belongs to a template, read as one argument
        assert_eq!(
            args(
                "x http:<//[[username:password]@]{hostname | host-ip}/image-name>",
                &[]
            ),
            ["//[[username:password]@]{hostname | host-ip}/image-name"]
        );
        assert_eq!(
            parse("logging file <url> [<max-file-size><|>nomax]").0,
            Node::Seq(vec![
                kw("logging"),
                kw("file"),
                Node::Arg("url".into()),
                Node::Opt(Box::new(Node::Alt(vec![
                    Node::Arg("max-file-size".into()),
                    kw("nomax")
                ]))),
            ])
        );
    }

    #[test]
    fn an_ellipsis_repeats_the_item_before_it() {
        for line in [
            "aaa authentication login {default <| list-name>} <method1> [<method2> . . . ]",
            "aaa authentication login {default <| list-name>} <method1> [<method2>...]",
        ] {
            let (n, r) = parse(line);
            assert_eq!(
                n,
                Node::Seq(vec![
                    kw("aaa"),
                    kw("authentication"),
                    kw("login"),
                    Node::Alt(vec![kw("default"), Node::Arg("list-name".into())]),
                    Node::Arg("method1".into()),
                    Node::Opt(Box::new(repeated(Node::Arg("method2".into())))),
                ]),
                "{line}"
            );
            assert!(r.iter().any(|x| x.contains("ellipsis")), "{r:?}");
        }
        assert_eq!(
            parse("boot system <file-url> ...").0,
            Node::Seq(vec![
                kw("boot"),
                kw("system"),
                repeated(Node::Arg("file-url".into()))
            ])
        );
        // a lone dot is a keyword
        assert_eq!(
            parse("separator { - | : | . }").0,
            Node::Seq(vec![
                kw("separator"),
                Node::Alt(vec![kw("-"), kw(":"), kw(".")])
            ])
        );
    }

    #[test]
    fn an_argument_closes_at_the_bracket_that_matches_its_own() {
        let (n, r) = parse("ptp ttl <<1-255>>");
        assert_eq!(
            n,
            Node::Seq(vec![kw("ptp"), kw("ttl"), Node::Arg("1-255".into())])
        );
        assert_eq!(r, ["argument <<1-255>> read as <1-255>"]);
        assert_eq!(
            parse("show stackwise-virtual switch <switch number <1-2>> link").0,
            Node::Seq(vec![
                kw("show"),
                kw("stackwise-virtual"),
                kw("switch"),
                Node::Arg("switch number 1-2".into()),
                kw("link"),
            ])
        );
        let (n, r) = parse("show a <b>> c");
        assert_eq!(
            n,
            Node::Seq(vec![kw("show"), kw("a"), Node::Arg("b".into()), kw("c")])
        );
        assert_eq!(r, ["stray '>' dropped"]);
    }

    #[test]
    fn an_optional_part_inside_an_argument_is_optional() {
        let (n, r) = parse(
            "rmon alarm <number> <variable> <interval> delta rising-threshold <value [event-number]>",
        );
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("rmon"),
                kw("alarm"),
                Node::Arg("number".into()),
                Node::Arg("variable".into()),
                Node::Arg("interval".into()),
                kw("delta"),
                kw("rising-threshold"),
                Node::Arg("value".into()),
                Node::Opt(Box::new(Node::Arg("event-number".into()))),
            ])
        );
        assert_eq!(
            r,
            ["argument <value [event-number]> read as <value> [ <event-number> ]"]
        );
    }

    #[test]
    fn a_later_branch_after_a_group_offers_more_choices_of_that_group() {
        let (n, _) = parse(
            "channel-group <channel-group-number> mode {auto | desirable | on} | {active | passive}",
        );
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("channel-group"),
                Node::Arg("channel-group-number".into()),
                kw("mode"),
                Node::Alt(vec![
                    kw("auto"),
                    kw("desirable"),
                    kw("on"),
                    kw("active"),
                    kw("passive")
                ]),
            ])
        );
        let (n, _) = parse(
            "monitor session <session_number> destination {interface <interface-id>} | {remote vlan <vlan-id>}",
        );
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("monitor"),
                kw("session"),
                Node::Arg("session_number".into()),
                kw("destination"),
                Node::Alt(vec![
                    Node::Seq(vec![kw("interface"), Node::Arg("interface-id".into())]),
                    Node::Seq(vec![kw("remote"), kw("vlan"), Node::Arg("vlan-id".into())]),
                ]),
            ])
        );
    }

    #[test]
    fn a_later_branch_stands_in_for_the_items_of_its_own_kinds() {
        let (n, _) = parse("dtls connectiontimeout <seconds> | idletimeout <seconds>");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("dtls"),
                Node::Alt(vec![
                    Node::Seq(vec![kw("connectiontimeout"), Node::Arg("seconds".into())]),
                    Node::Seq(vec![kw("idletimeout"), Node::Arg("seconds".into())]),
                ]),
            ])
        );
        let (n, _) = parse("ip nat pool <name> netmask <netmask> | prefix-length <prefix-length>");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("ip"),
                kw("nat"),
                kw("pool"),
                Node::Arg("name".into()),
                Node::Alt(vec![
                    Node::Seq(vec![kw("netmask"), Node::Arg("netmask".into())]),
                    Node::Seq(vec![kw("prefix-length"), Node::Arg("prefix-length".into())]),
                ]),
            ])
        );
        // all keywords: the last stands in for the later branches
        let (n, _) = parse("configure revert now | timer <minutes> | idle <minutes>");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("configure"),
                kw("revert"),
                Node::Alt(vec![
                    Node::Seq(vec![kw("now")]),
                    Node::Seq(vec![kw("timer"), Node::Arg("minutes".into())]),
                    Node::Seq(vec![kw("idle"), Node::Arg("minutes".into())]),
                ]),
            ])
        );
        // keywords alone match any run of keywords: the command keeps its own
        let (n, _) = parse("no device-sensor filter-list cdp | lldp list list-name");
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("no"),
                kw("device-sensor"),
                kw("filter-list"),
                Node::Alt(vec![
                    Node::Seq(vec![kw("cdp")]),
                    Node::Seq(vec![kw("lldp"), kw("list"), kw("list-name")]),
                ]),
            ])
        );
        // the command's keywords kept whole before an argument the later branches stand in for
        let (n, _) = parse(
            "ipv6 access-list <access-list-name> | match-local-traffic | role-based <list-name>",
        );
        assert_eq!(
            n,
            Node::Seq(vec![
                kw("ipv6"),
                kw("access-list"),
                Node::Alt(vec![
                    Node::Seq(vec![Node::Arg("access-list-name".into())]),
                    kw("match-local-traffic"),
                    Node::Seq(vec![kw("role-based"), Node::Arg("list-name".into())]),
                ]),
            ])
        );
    }
}
