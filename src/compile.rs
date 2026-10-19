//! Compiles an extracted command reference (tools/extract-releases.js) into the grammar the switch runs:
//! per command its syntax lines, the modes it is entered in, the type of each argument, and the
//! example lines Cisco shows at their prompts; from the examples, which commands enter a submode.
//! Of the prose, only the first sentence of each Syntax Description row and of the introduction is
//! carried over, as the help `?` prints.

use crate::release::Release;
use crate::syntax::{self, Node};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Line {
    pub text: String,
    pub node: Node,
    /// the `no` form
    pub negated: bool,
    /// how the parser read a line the reference mistyped
    pub repairs: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum ArgKind {
    Number {
        min: i64,
        max: i64,
    },
    VlanList,
    Interface,
    /// `Twe1/0/1 - 8 , Twe2/0/1 - 8`, to the end of the line
    InterfaceRange,
    Ipv4,
    Ipv6,
    Mac,
    /// the rest of the line
    Text,
    Word,
    /// a DSCP value, 0-63 or a name (`ef`, `af41`, `cs4`, `default`)
    Dscp,
    /// a hexadecimal number, `0x` before it or not (`<0-FFFFFFFF>`)
    Hex {
        min: u64,
        max: u64,
    },
    /// `A.B.C.D/nn`
    Ipv4Prefix,
    /// `X:X:X:X::X/<0-128>`
    Ipv6Prefix,
    /// an interface of this one type, a type vswitch models or not (`Lspvif0`)
    InterfaceOf {
        of: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Example {
    pub mode: String,
    pub line: String,
    /// output printed after the line, up to the next prompt
    pub output: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Command {
    pub name: String,
    pub source: String,
    pub modes: Vec<String>,
    pub lines: Vec<Line>,
    pub args: BTreeMap<String, ArgKind>,
    pub examples: Vec<Example>,
    /// the mode the command enters: a mode id, or `parent`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enters: Option<String>,
    /// why the syntax differs from the reference's line (data/overrides.json)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// what `?` says of each keyword (lowercase) and argument, from the reference's Syntax
    /// Description, and under "" what the command does, from its introduction
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub help: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Override {
    name: String,
    /// drop the command: its reference entry is not a usable syntax
    #[serde(default)]
    remove: bool,
    #[serde(default)]
    modes: Vec<String>,
    /// a patch takes these modes from the command: a guide set it in a submode another command owns
    #[serde(default)]
    drop_modes: Vec<String>,
    #[serde(default)]
    lines: Vec<String>,
    /// the lines a classic IOS release (`15.2(7)E`) takes instead: the IOS XE forms its books never show
    /// (`ntp server vrf`) left out
    #[serde(default)]
    classic_lines: Vec<String>,
    /// a patch adds these lines beside the command's own: a `no` form its books state only in full
    #[serde(default)]
    adds: Vec<String>,
    #[serde(default)]
    args: BTreeMap<String, ArgKind>,
    /// a patch renames these arguments in the command's lines outside the `no` form, so `args` can
    /// bound what a switch bounds there and leave the `no` form as the book has it (`privilege`'s level)
    #[serde(default)]
    renames: BTreeMap<String, String>,
    #[serde(default)]
    enters: Option<String>,
    evidence: String,
    /// a removal applies only where the book documents the command with exactly these lines
    #[serde(default)]
    corrects: Vec<String>,
    /// a hardware document's command, which the release books do not cover: taken at every release of
    /// this platform (`Catalyst 9400`)
    #[serde(default)]
    platform: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Overrides {
    commands: Vec<Override>,
}

/// The overrides a release's own sources call for, written against the 9300 17.15 books: a removal where
/// the book documents the command with the lines it `corrects`, a patch where the command exists, and a
/// replacement or addition where the book documents the lines it `corrects`, the release's example lines
/// type the command at one of its modes, or its books show the command as a configuration line (`docs`,
/// from `documented_lines`); an addition of a submode's own text (`conf-ssh-pubkey-data`'s key lines)
/// where a command enters that submode. A patch reaches every command of its name. Each keeps its
/// examples from the reference. Returns the names applied and those the release's sources do not call
/// for.
pub fn apply_overrides(
    g: &mut Grammar,
    text: &str,
    docs: &BTreeSet<String>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let o: Overrides = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let classic = Release::parse(&g.release).is_some_and(Release::classic);
    let (mut applied, mut skipped) = (Vec::new(), Vec::new());
    for ov in o.commands {
        let texts = if classic && !ov.classic_lines.is_empty() {
            &ov.classic_lines
        } else {
            &ov.lines
        };
        let lines: Vec<Line> = texts.iter().map(|s| syntax_line(s, None)).collect();
        let documented = |g: &Grammar| {
            g.commands.iter().any(|c| {
                c.name == ov.name
                    && c.lines
                        .iter()
                        .map(|l| l.text.as_str())
                        .eq(ov.corrects.iter().map(String::as_str))
            })
        };
        let called_for = if let Some(platform) = &ov.platform {
            // a patch only where the release has the command (12.2's `line` is borrowed later)
            *platform == g.platform
                && (!lines.is_empty() || ov.remove || g.commands.iter().any(|c| c.name == ov.name))
        } else if ov.remove {
            documented(g)
        } else if lines.is_empty() {
            g.commands.iter().any(|c| c.name == ov.name)
        } else {
            (!ov.corrects.is_empty() && documented(g))
                || typed_in_examples(g, &ov.name, &ov.modes)
                || shown_in(docs, &ov.name)
                // a submode's own text has no name to type: wanted where a command enters the submode
                || (!ov.modes.is_empty()
                    && lines.iter().all(|l| !starts_with_keyword(&l.node))
                    && ov.modes.iter().all(|m| {
                        g.commands
                            .iter()
                            .any(|c| c.enters.as_deref() == Some(m.as_str()))
                    }))
        };
        if !called_for {
            skipped.push(ov.name);
            continue;
        }
        applied.push(ov.name.clone());
        // an entry with no lines patches each command of its name: argument types, modes, the mode it
        // enters, lines added beside its own
        if lines.is_empty() && !ov.remove {
            if !g.commands.iter().any(|c| c.name == ov.name) {
                return Err(format!("{}: no such command to patch", ov.name));
            }
            let adds: Vec<Line> = ov.adds.iter().map(|s| syntax_line(s, None)).collect();
            for c in g.commands.iter_mut().filter(|c| c.name == ov.name) {
                c.args.extend(ov.args.clone());
                for l in &adds {
                    let mut names = BTreeSet::new();
                    args_of(&l.node, &mut names);
                    for a in names {
                        c.args.entry(a.clone()).or_insert_with(|| arg_kind(&a, ""));
                    }
                    if !c.lines.iter().any(|x| x.text == l.text) {
                        c.lines.push(l.clone());
                    }
                }
                // after the adds, so a line the patch adds is renamed as the command's own are
                for l in c.lines.iter_mut().filter(|l| !l.negated) {
                    for (from, to) in &ov.renames {
                        rename_arg(&mut l.node, from, to);
                        l.text = l.text.replace(&format!("<{from}>"), &format!("<{to}>"));
                    }
                }
                for m in &ov.modes {
                    if !c.modes.contains(m) {
                        c.modes.push(m.clone());
                    }
                }
                c.modes.retain(|m| !ov.drop_modes.contains(m));
                if ov.enters.is_some() {
                    c.enters.clone_from(&ov.enters);
                }
                c.evidence = Some(ov.evidence.clone());
            }
            continue;
        }
        let examples = g
            .commands
            .iter()
            .filter(|c| c.name == ov.name)
            .flat_map(|c| c.examples.clone())
            .collect();
        let help = g
            .commands
            .iter()
            .filter(|c| c.name == ov.name)
            .flat_map(|c| c.help.clone())
            .collect();
        g.commands.retain(|c| c.name != ov.name);
        if ov.remove {
            continue;
        }
        g.commands.push(Command {
            name: ov.name,
            source: "data/overrides.json".into(),
            modes: ov.modes,
            lines,
            args: ov.args,
            examples,
            enters: ov.enters,
            evidence: Some(ov.evidence),
            help,
        });
    }
    Ok((applied, skipped))
}

/// Every line of a book's preformatted blocks (configuration listings, show output), example lines and
/// syntax lines, without its prompt,
/// lowercased: where a book shows a command it does not list.
pub fn documented_lines(book: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    fn walk(v: &Value, key: &str, out: &mut BTreeSet<String>) {
        match v {
            Value::Object(m) => m.iter().for_each(|(k, x)| walk(x, k, out)),
            Value::Array(a) => a.iter().for_each(|x| walk(x, key, out)),
            Value::String(s) if matches!(key, "pre" | "pres" | "examples" | "syntax") => {
                for l in s.lines() {
                    // `Device(config-line)# transport input ssh` -> `transport input ssh`
                    let l = match l.find(['#', '>']) {
                        Some(i) if !l[..i].contains(' ') && i > 0 => &l[i + 1..],
                        _ => l,
                    };
                    let l = l
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .to_lowercase();
                    if !l.is_empty() {
                        out.insert(l);
                    }
                }
            }
            _ => {}
        }
    }
    walk(book, "", &mut out);
    out
}

/// Whether the books show the command (its words, then a space or the end) as a line.
fn shown_in(docs: &BTreeSet<String>, name: &str) -> bool {
    let words = name.split(" (").next().unwrap_or(name).to_lowercase();
    docs.range(words.clone()..)
        .take_while(|l| l.starts_with(&words))
        .any(|l| l.len() == words.len() || l[words.len()..].starts_with(' '))
}

/// Whether an example line types the command at one of the modes (`*config`: any configuration mode).
fn typed_in_examples(g: &Grammar, name: &str, modes: &[String]) -> bool {
    let words: Vec<String> = name
        .split(" (")
        .next()
        .unwrap_or(name)
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let at = |mode: &str| {
        modes.is_empty()
            || modes
                .iter()
                .any(|m| m == mode || (m == "*config" && crate::cli::is_config(mode)))
    };
    g.commands.iter().flat_map(|c| &c.examples).any(|e| {
        at(&e.mode)
            && e.line
                .split_whitespace()
                .map(str::to_lowercase)
                .take(words.len())
                .eq(words.iter().cloned())
    })
}

/// Drops syntax lines that start with an argument (the reference's `ip route static bfd` has one,
/// `<multihop-destination-address> <multihop-source-address>`, which would match any two words) and
/// commands left with none. A command line starts with a keyword or a choice of keywords (`{cdp | dhcp
/// | lldp} tlv-type`); an override's line is as written (a submode's free text). Returns the lines
/// dropped.
pub fn prune(g: &mut Grammar) -> usize {
    let mut dropped = 0;
    for c in &mut g.commands {
        if c.source == "data/overrides.json" {
            continue;
        }
        // an access list entry's leading `[sequence-number]` (`permit (IP)`, `deny (IP)`) is left
        // off: the shell reads a number before an entry itself (`acl::numbered`)
        for l in &mut c.lines {
            if let Node::Seq(items) = &mut l.node
                && matches!(items.first(), Some(Node::Opt(inner)) if matches!(inner.as_ref(), Node::Arg(a) if a.contains("sequence")))
                && starts_with_keyword(&Node::Seq(items[1..].to_vec()))
            {
                items.remove(0);
            }
        }
        let before = c.lines.len();
        c.lines.retain(|l| starts_with_keyword(&l.node));
        dropped += before - c.lines.len();
    }
    g.commands.retain(|c| !c.lines.is_empty());
    dropped
}

/// Whether every way into a syntax line starts with a keyword: a keyword, a choice whose every branch
/// does, or an optional part that does before a rest that does.
fn starts_with_keyword(n: &Node) -> bool {
    match n {
        Node::Kw(_) => true,
        Node::Arg(_) | Node::Opt(_) => false,
        Node::Alt(v) => !v.is_empty() && v.iter().all(starts_with_keyword),
        Node::Seq(v) => match v.split_first() {
            None => false,
            Some((Node::Opt(inner), rest)) => {
                starts_with_keyword(inner) && starts_with_keyword(&Node::Seq(rest.to_vec()))
            }
            Some((first, _)) => starts_with_keyword(first),
        },
    }
}

/// Adds the syntax lines of Cisco's general IOS XE command references to commands the platform's own sources
/// already name, with their modes. A command only the general books list is left out: they cover
/// every IOS XE platform, and the platform would refuse what it does not run. Returns lines added.
pub fn add_references(g: &mut Grammar, refs: &Value) -> usize {
    let release = Release::parse(&g.release);
    let mut added = 0;
    for book in refs["books"].as_array().into_iter().flatten() {
        for ch in book["chapters"].as_array().into_iter().flatten() {
            for c in ch["commands"].as_array().into_iter().flatten() {
                let name = c["name"].as_str().unwrap_or("");
                let Some(cmd) = g
                    .commands
                    .iter_mut()
                    .find(|x| x.name.eq_ignore_ascii_case(name))
                else {
                    continue;
                };
                let ms = modes(&texts(&c["sections"]["Command Modes"]));
                let later = release.map(|r| later_keywords(c, r)).unwrap_or_default();
                let own = own_word(name);
                // the same command in another context (`name` in a redundancy group) is not this one
                if !ms.is_empty()
                    && !cmd.modes.is_empty()
                    && !ms.iter().any(|m| cmd.modes.contains(m))
                {
                    continue;
                }
                for s in c["syntax"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    if cmd.lines.iter().any(|l| l.text == s) || names_any(s, &later) {
                        continue;
                    }
                    let mut l = syntax_line(s, Some(&known_args(c)));
                    // a line beginning with another word (a fragment, the book's prose) is not this
                    // command's: `place_lines`
                    if !begins_own(&l.node, &own) {
                        continue;
                    }
                    qualify(&mut l.node, &cmd.args);
                    let mut names = BTreeSet::new();
                    args_of(&l.node, &mut names);
                    for a in names {
                        cmd.args
                            .entry(a.clone())
                            .or_insert_with(|| arg_kind(&a, ""));
                    }
                    cmd.lines.push(l);
                    added += 1;
                }
                if cmd.modes.is_empty() {
                    cmd.modes = ms;
                }
            }
        }
    }
    added
}

/// Keywords a general reference's Command History says an IOS XE release after `release` added
/// (`15.1(1)S | This command was modified. The acr keyword was added.` names a train before IOS XE 16 and
/// counts for none). The general books cover every release, so a syntax line naming one is left out.
fn later_keywords(c: &Value, release: Release) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for row in c["sections"]["Command History"]["rows"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let cells: Vec<&str> = row
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let [when, what, ..] = cells[..] else {
            continue;
        };
        if xe_train(when).is_some_and(|r| !release.includes(r)) {
            out.extend(keywords_added(what));
        }
    }
    out
}

/// The IOS XE 16, 17 or 26 train a Command History release names (`Cisco IOS XE Fuji 16.9.1`); None
/// for IOS 12 and 15 and IOS XE 2 and 3.
fn xe_train(text: &str) -> Option<Release> {
    text.split_whitespace()
        .filter_map(Release::parse)
        .find(|r| matches!(r.major(), 16 | 17 | 26))
}

/// The keywords a Command History sentence adds: `The vrf and ipv6 keywords were added.`, and a
/// list whose keywords carry their own options (`The bgp, ospf [lsa | errors], and pim keywords`).
fn keywords_added(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for sentence in text.split('.') {
        let lower = sentence.to_lowercase();
        if !(lower.contains("added") || lower.contains("introduced")) {
            continue;
        }
        // a keyword's own options are not the keywords added
        let mut bare = String::new();
        let mut depth = 0;
        for ch in sentence.chars() {
            match ch {
                '[' | '{' => depth += 1,
                ']' | '}' => depth = (depth - 1).max(0),
                c if depth == 0 => bare.push(c),
                _ => {}
            }
        }
        let words: Vec<&str> = bare
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|w| !w.is_empty())
            .collect();
        for (i, w) in words.iter().enumerate() {
            if !w.to_lowercase().starts_with("keyword") {
                continue;
            }
            for k in words[..i].iter().rev() {
                let k = k.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
                match k.to_lowercase().as_str() {
                    "and" | "or" => continue,
                    "the" | "a" | "an" | "new" | "following" | "was" | "were" | "" => break,
                    _ => out.push(k.to_lowercase()),
                }
            }
        }
    }
    out
}

/// Whether a syntax line names one of the keywords (its words outside `<arguments>`).
fn names_any(line: &str, keywords: &BTreeSet<String>) -> bool {
    if keywords.is_empty() {
        return false;
    }
    let mut depth = 0;
    let mut bare = String::new();
    for ch in line.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = (depth - 1).max(0),
            c if depth == 0 => bare.push(c),
            _ => {}
        }
    }
    bare.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .any(|w| keywords.contains(&w.to_lowercase()))
}

/// The release a classic Command History row names: `12.2(53)EZ`, `15.0(2) SE`, or `15.0(2)` read as
/// its train's major and minor.
fn history_release(text: &str) -> Option<Release> {
    let joined: String = text.split_whitespace().collect();
    text.split_whitespace()
        .find_map(Release::parse)
        .or_else(|| Release::parse(&joined))
        .or_else(|| Release::parse(joined.split('(').next()?))
}

/// A release whose own command reference is lost, read from a later one of the same platform (the IE
/// 3010's 12.2(53)EZ book from its 15.0(2)SE book): a command whose Command History introduces it only
/// after `release` is left out, and so is each alternative naming a keyword a later row adds (`{ in |
/// out }` keeps `in` when `out` came later). Returns the commands left out and those whose lines lost
/// keywords.
pub fn as_of(reference: &mut Value, release: Release) -> (Vec<String>, Vec<String>) {
    let (mut dropped, mut trimmed) = (Vec::new(), Vec::new());
    for ch in reference["chapters"].as_array_mut().into_iter().flatten() {
        let Some(cmds) = ch["commands"].as_array_mut() else {
            continue;
        };
        cmds.retain_mut(|c| {
            let name = c["name"].as_str().unwrap_or("").to_string();
            let rows: Vec<(Release, String)> = c["sections"]["Command History"]["rows"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| {
                    let when = history_release(row.get(0)?.as_str()?)?;
                    Some((when, row.get(1)?.as_str()?.to_string()))
                })
                .collect();
            let introduced: Vec<Release> = rows
                .iter()
                .filter(|(_, what)| what.to_lowercase().contains("introduced"))
                .map(|(when, _)| *when)
                .collect();
            if !introduced.is_empty() && !introduced.iter().any(|w| release.includes(*w)) {
                dropped.push(name);
                return false;
            }
            let later: BTreeSet<String> = rows
                .iter()
                .filter(|(when, _)| !release.includes(*when))
                .flat_map(|(_, what)| keywords_added(what))
                .collect();
            if later.is_empty() {
                return true;
            }
            // the command's own words are never an alternative to drop
            let own = name
                .split('(')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .count();
            let lines: Vec<Value> = c["syntax"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|s| without(s, own, &later))
                .map(Value::from)
                .collect();
            if c["syntax"].as_array().is_some_and(|old| *old != lines) {
                trimmed.push(name);
                c["syntax"] = Value::from(lines);
            }
            true
        });
    }
    (dropped, trimmed)
}

/// A syntax line's words and groups: `{ in | out }` is a group of two alternatives.
#[derive(Clone, Debug, PartialEq)]
enum Part {
    Word(String),
    Group(char, Vec<Vec<Part>>),
}

/// The alternatives up to `close`, each a sequence of parts. A `|` outside any group is a word.
fn parts(chars: &[char], i: &mut usize, close: Option<char>) -> Vec<Vec<Part>> {
    let mut alts = vec![Vec::new()];
    let mut word = String::new();
    let flush = |word: &mut String, alts: &mut Vec<Vec<Part>>| {
        if !word.is_empty() {
            alts.last_mut()
                .unwrap()
                .push(Part::Word(std::mem::take(word)));
        }
    };
    while *i < chars.len() {
        let c = chars[*i];
        *i += 1;
        match c {
            '{' | '[' => {
                flush(&mut word, &mut alts);
                let inner = parts(chars, i, Some(if c == '{' { '}' } else { ']' }));
                alts.last_mut().unwrap().push(Part::Group(c, inner));
            }
            '}' | ']' if close.is_some() => {
                flush(&mut word, &mut alts);
                return alts;
            }
            '|' if close.is_some() => {
                flush(&mut word, &mut alts);
                alts.push(Vec::new());
            }
            c if c.is_whitespace() => flush(&mut word, &mut alts),
            c => word.push(c),
        }
    }
    flush(&mut word, &mut alts);
    alts
}

fn render(seq: &[Part]) -> String {
    seq.iter()
        .map(|p| match p {
            Part::Word(w) => w.clone(),
            Part::Group(open, alts) => {
                let close = if *open == '{' { '}' } else { ']' };
                let inner: Vec<String> = alts.iter().map(Vec::as_slice).map(render).collect();
                format!("{open} {} {close}", inner.join(" | "))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Drops each alternative a keyword in `later` begins; false when a required group is left empty.
fn drop_alternatives(seq: &mut Vec<Part>, later: &BTreeSet<String>) -> bool {
    let mut ok = true;
    seq.retain_mut(|p| {
        let Part::Group(open, alts) = p else {
            return true;
        };
        alts.retain_mut(|alt| {
            let named =
                matches!(alt.first(), Some(Part::Word(w)) if later.contains(&w.to_lowercase()));
            !named && drop_alternatives(alt, later)
        });
        if alts.is_empty() {
            ok &= *open == '[';
            return false;
        }
        true
    });
    ok
}

/// The syntax line without the alternatives a later keyword begins; None when the keyword is not an
/// alternative (`ip access-group <name> out`) or a required group loses every one. The first `own`
/// words are the command's name. A line naming no such keyword comes back as it was.
fn without(line: &str, own: usize, later: &BTreeSet<String>) -> Option<String> {
    if !names_any(line, later) {
        return Some(line.to_string());
    }
    let chars: Vec<char> = line.chars().collect();
    let mut seq = parts(&chars, &mut 0, None).swap_remove(0);
    let start = if seq.first() == Some(&Part::Word("no".into())) {
        own + 1
    } else {
        own
    };
    let bare_named = seq.iter().skip(start).any(
        |p| matches!(p, Part::Word(w) if !w.starts_with('<') && later.contains(&w.to_lowercase())),
    );
    if bare_named || !drop_alternatives(&mut seq, later) {
        return None;
    }
    Some(render(&seq))
}

/// A book with no command reference of its own (the IE3x00's): the base book's grammar at the same
/// release, keeping the commands its guides attest (a step's syntax, or a line at a prompt, begins with
/// the command's words, its qualifier left off: `license smart (global config)` is `license smart`) or
/// its captures at this release do (a configuration line, a command typed in a session), and those
/// every grammar takes (`data/overrides.json`), under the guides' platform name. Its guides' own steps
/// are added after (`add_guides`). Returns the grammar and how many base commands it kept and dropped.
pub fn project(
    base: &Grammar,
    guides: &Value,
    configs: &[String],
    sessions: &[String],
) -> (Grammar, usize, usize) {
    let mut said: Vec<String> = vec![];
    let mut push = |line: &str| {
        let l = line.trim().to_lowercase();
        let l = l.strip_prefix("no ").unwrap_or(&l).to_string();
        if !l.is_empty() {
            said.push(l);
        }
    };
    for book in guides["books"].as_array().into_iter().flatten() {
        for ch in book["chapters"].as_array().into_iter().flatten() {
            for step in ch["steps"].as_array().into_iter().flatten() {
                push(step["syntax"].as_str().unwrap_or(""));
                for ex in step["examples"].as_array().into_iter().flatten() {
                    if let Some((_, c)) = ex.as_str().and_then(prompt) {
                        push(&c);
                    }
                }
            }
            for pre in ch["pres"].as_array().into_iter().flatten() {
                for line in pre.as_str().unwrap_or("").lines() {
                    if let Some((_, c)) = prompt(line) {
                        push(&c);
                    }
                }
            }
        }
    }
    for line in configs.iter().flat_map(|c| c.lines()) {
        push(line);
    }
    // a session's output is not a command; the line at its prompt is
    for line in sessions.iter().flat_map(|s| s.lines()) {
        if let Some((_, c)) = prompt(line) {
            push(&c);
        }
    }
    let attested = |name: &str| {
        let n = bare_name(name);
        let word = format!("{n} ");
        said.iter().any(|l| *l == n || l.starts_with(&word))
    };
    let mut g = base.clone();
    let before = g.commands.len();
    g.commands
        .retain(|c| c.source == "data/overrides.json" || attested(&c.name));
    let kept = g.commands.len();
    if let Some(p) = guides["platform"].as_str() {
        g.platform = p.to_string();
    }
    (g, kept, before - kept)
}

/// A step's syntax whose first word (`first`, lowercase) is two or more of its example's words run
/// together, with those words spaced as the example has them: `configureterminal` typed as
/// `configure terminal`. The example's first word must begin a command the books already have
/// (`known`): an example with a stray space (`maximum- prefix`) does not split `maximum-prefix`.
fn run_together(
    syntax: &str,
    first: &str,
    words: &[&str],
    known: impl Fn(&str) -> bool,
) -> Option<String> {
    let rest = syntax.get(first.len()..)?;
    if !syntax[..first.len()].eq_ignore_ascii_case(first)
        || !words.first().is_some_and(|w| known(&w.to_lowercase()))
    {
        return None;
    }
    let mut joined = String::new();
    for (k, w) in words.iter().enumerate() {
        joined.push_str(&w.to_lowercase());
        if joined.len() >= first.len() {
            return (k >= 1 && joined == first).then(|| format!("{}{rest}", words[..=k].join(" ")));
        }
    }
    None
}

/// Whether a guide step's word is a value rather than a keyword: a number, address or hex value
/// (`98`, `192.168.20.1`, `876F`), a port, path or IPv6 address (`0/1`, `fe80::1`), or an interface
/// (`vlan10`, `po1`).
fn value_word(w: &str) -> bool {
    w.starts_with(|c: char| c.is_ascii_digit())
        || w.contains(['/', ':'])
        || crate::cli::interface_name(w, None).is_some()
}

/// Adds the procedure steps of the configuration guides (tools/extract-releases.js): a step counts when
/// one of its example lines starts with the step's first keyword, and its prompt gives the mode.
/// A step whose keywords name a reference command adds its line to that command (and the mode,
/// when the reference gave none); otherwise it becomes a command of its own. Guide examples and
/// the prompt lines of guide output blocks join the examples. Returns (lines added, commands added).
pub fn add_guides(g: &mut Grammar, guides: &Value) -> (usize, usize) {
    let (mut added_lines, mut added_cmds) = (0, 0);
    let mut seen_modes: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut block_lines: Vec<(String, String)> = Vec::new();
    for book in guides["books"].as_array().into_iter().flatten() {
        for ch in book["chapters"].as_array().into_iter().flatten() {
            let url = ch["url"].as_str().unwrap_or("");
            let page = url.rsplit('/').next().unwrap_or("");
            for step in ch["steps"].as_array().into_iter().flatten() {
                let syntax = step["syntax"].as_str().unwrap_or("").trim().to_string();
                let first = syntax
                    .split(|c: char| c.is_whitespace() || c == '<' || c == '[' || c == '{')
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                if first.is_empty()
                    || !first
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic())
                {
                    continue;
                }
                // every prompt line of the step's example counts as evidence of its command's mode
                for (m, c) in step["examples"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter_map(prompt)
                {
                    block_lines.push((m, c.to_lowercase()));
                }
                let examples: Vec<(String, String)> = step["examples"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter_map(prompt)
                    .filter(|(_, cmd)| {
                        cmd.split_whitespace().next().is_some_and(|w| {
                            first.starts_with(&w.to_lowercase())
                                || w.to_lowercase().starts_with(&first)
                        })
                    })
                    .collect();
                let Some((mode, typed)) = examples.first().cloned() else {
                    continue;
                };
                // a step whose syntax lost the spaces its example has (the 9200 MACsec guide's
                // `configureterminal`) is the example's words
                let words: Vec<&str> = typed.split_whitespace().collect();
                let begins = |w: &str| {
                    g.commands.iter().any(|c| {
                        c.name
                            .split_whitespace()
                            .next()
                            .is_some_and(|f| f.eq_ignore_ascii_case(w))
                    })
                };
                let syntax = run_together(&syntax, &first, &words, begins).unwrap_or(syntax);
                let mut line = syntax_line(&syntax, None);
                let kws = syntax::leading_keywords(&line.node);
                let name = kws.join(" ");
                if name.is_empty() {
                    continue;
                }
                let ex: Vec<Example> = examples
                    .iter()
                    .map(|(m, l)| Example {
                        mode: m.clone(),
                        line: l.clone(),
                        output: vec![],
                    })
                    .collect();
                // a step naming one instance (`interface gigabitethernet0/0`, `line vty 0 98`) is an
                // example of the command it instantiates, not a command; a keyword with a digit in it
                // (`cdp advertise-v2`, `ipv6`) is a keyword
                if kws.iter().any(|k| value_word(k)) {
                    let lower = name.to_lowercase();
                    if let Some(c) = g
                        .commands
                        .iter_mut()
                        .filter(|c| lower.starts_with(&c.name.to_lowercase()))
                        .max_by_key(|c| c.name.len())
                    {
                        c.examples.extend(ex);
                    }
                    continue;
                }
                match g
                    .commands
                    .iter_mut()
                    .find(|c| c.name.eq_ignore_ascii_case(&name))
                {
                    Some(c) => {
                        qualify(&mut line.node, &c.args);
                        let mut names = BTreeSet::new();
                        args_of(&line.node, &mut names);
                        // the guide shows the command in another mode too (`name` in config-vlan as
                        // well as config-mst); taken once two example lines show it there
                        if c.modes.is_empty() {
                            // the reference gave no mode: the guide's is the only evidence
                            c.modes.push(mode.clone());
                        } else if !c.modes.contains(&mode) {
                            *seen_modes
                                .entry((c.name.clone(), mode.clone()))
                                .or_default() +=
                                examples.iter().filter(|(m, _)| *m == mode).count();
                        }
                        if !c.lines.iter().any(|l| l.text == line.text) {
                            c.lines.push(line);
                            added_lines += 1;
                        }
                        for a in names {
                            c.args.entry(a.clone()).or_insert_with(|| arg_kind(&a, ""));
                        }
                        c.examples.extend(ex);
                    }
                    None => {
                        let mut names = BTreeSet::new();
                        args_of(&line.node, &mut names);
                        g.commands.push(Command {
                            name,
                            source: format!("guide:{page}"),
                            modes: vec![mode],
                            lines: vec![line],
                            args: names.iter().map(|a| (a.clone(), arg_kind(a, ""))).collect(),
                            examples: ex,
                            enters: None,
                            evidence: None,
                            help: BTreeMap::new(),
                        });
                        added_cmds += 1;
                        added_lines += 1;
                    }
                }
            }
            // prompt lines of configuration examples, as more example lines (and mode changes)
            for pre in ch["pres"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let mut last: Option<(String, String)> = None;
                for raw in pre.lines() {
                    let Some((mode, cmd)) = prompt(raw) else {
                        continue;
                    };
                    block_lines.push((mode.clone(), cmd.to_lowercase()));
                    let first = last
                        .as_ref()
                        .and_then(|(_, c)| c.split_whitespace().next())
                        .unwrap_or("")
                        .to_lowercase();
                    if let Some((m0, c0)) = &last
                        && *m0 != mode
                        && !c0.is_empty()
                        && !["exit", "end", "do", "no", "enable", "disable"]
                            .contains(&first.as_str())
                        && !first.starts_with("conf")
                    {
                        g.transitions.push(Transition {
                            from: m0.clone(),
                            line: c0.clone(),
                            to: mode.clone(),
                        });
                    }
                    last = Some((mode, cmd));
                }
            }
        }
    }
    for ((name, mode), n) in seen_modes {
        // configuration-example blocks count too: `(config-vlan)# name BLUE-VRF`
        let stem = name.to_lowercase();
        let n = n + block_lines
            .iter()
            .filter(|(m, c)| *m == mode && (*c == stem || c.starts_with(&format!("{stem} "))))
            .count();
        if n >= 2
            && let Some(c) = g.commands.iter_mut().find(|c| c.name == name)
            && !c.modes.contains(&mode)
        {
            c.modes.push(mode);
        }
    }
    (added_lines, added_cmds)
}

/// The commands a capture of the book's switches uses that its own books refuse, taken whole from a
/// donor book of the same IOS with the transitions their examples showed: a donor command is taken
/// when a refused capture line, or a configuration line a session typed, is its name or starts with
/// it (a `no` form too). Rounds repeat while they take something, so a borrowed block command's lines
/// are read in its mode. Returns the names taken.
pub fn borrow_commands(
    g: &mut Grammar,
    donor: &Grammar,
    configs: &[String],
    sessions: &[String],
) -> Vec<String> {
    let names = |l: &str, n: &str| l == n || l.starts_with(&format!("{n} "));
    let mut taken: Vec<String> = Vec::new();
    for _ in 0..3 {
        let (typed, commands): (Vec<_>, Vec<_>) = refused_commands(g, sessions)
            .into_iter()
            .partition(|(m, _)| crate::cli::is_config(m));
        let mut refused = refused_lines(g, configs);
        refused.extend(typed.into_iter().map(|(_, l)| l));
        let mut take: BTreeSet<String> = donor
            .commands
            .iter()
            .filter(|c| !taken.contains(&c.name))
            .filter(|c| refused.iter().any(|l| names(l, &c.name.to_lowercase())))
            .map(|c| c.name.clone())
            .collect();
        // a session's command takes the one donor command that names it most closely, never a
        // bare `show` that would take every show variant
        for (_, l) in commands {
            if let Some(c) = donor
                .commands
                .iter()
                .filter(|c| {
                    // a guide step's lone `show` (`show running-config | include define`) is no
                    // command of its own
                    c.name != "show"
                        && !taken.contains(&c.name)
                        && names(&l, &c.name.to_lowercase())
                })
                .max_by_key(|c| c.name.len())
            {
                take.insert(c.name.clone());
            }
        }
        if take.is_empty() {
            break;
        }
        for c in donor.commands.iter().filter(|c| take.contains(&c.name)) {
            g.commands.push(Command {
                source: format!("{} {}: {}", donor.platform, donor.release, c.source),
                ..c.clone()
            });
        }
        for t in &donor.transitions {
            let l = t.line.to_lowercase();
            if take.iter().any(|n| names(&l, &n.to_lowercase())) {
                g.transitions.push(t.clone());
            }
        }
        taken.extend(take);
    }
    taken
}

/// Whether a Command History row's release is IOS XE (`Cisco IOS XE Release 3.9S`, `XE3.2SP`,
/// `16.9.1`). A classic train whose code holds the letters (`12.2(18)SXE`, `12.0(5)XE2`) is not.
fn xe_row(text: &str) -> bool {
    text.split_whitespace().any(|w| {
        w == "XE"
            || w == "IOS-XE"
            || w.strip_prefix("XE")
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
    }) || xe_train(text).is_some()
}

/// Whether an IOS XE Command History row names a release before IOS XE 16 (`Cisco IOS XE Release
/// 3.10.0S`, `XE 2.4.2`), which every 16, 17 and 26 release follows.
fn xe_before_16(text: &str) -> bool {
    text.split_whitespace()
        .filter_map(|w| Release::parse(w.strip_prefix("XE").unwrap_or(w)))
        .any(|r| !r.classic() && r.major() < 16)
}

/// A classic Catalyst LAN switching train a Command History row names (`15.2(2)E`, `12.2(55)SE`):
/// the rows a 2960 or IE switch's release is dated against. Other trains (T, S, SR, SX) are no
/// switch's, and say nothing of when a switch train took a command.
fn lan_row(text: &str) -> Option<Release> {
    let r = history_release(text)?;
    let t = r.to_string();
    let code: String = t
        .rsplit(')')
        .next()
        .unwrap_or("")
        .chars()
        .take_while(char::is_ascii_uppercase)
        .collect();
    (r.classic()
        && matches!(
            code.as_str(),
            "E" | "SE" | "EX" | "EY" | "EZ" | "EA" | "EB" | "EC" | "ED"
        ))
    .then_some(r)
}

/// The general references as a release has them, by their Command History: for IOS XE, a command
/// IOS XE introduced after the release is left out (one IOS XE 2 or 3 introduced comes before every
/// 16, 17 and 26 release) and so is each alternative naming a keyword an IOS XE row after it adds; for
/// classic IOS, a command only IOS XE introduced is left out, and so is a keyword an IOS XE row or a
/// later LAN switching train adds. Rows of other trains date nothing. Returns the commands left out
/// and those whose lines lost keywords.
pub fn gate_general(reference: &mut Value, release: Release) -> (Vec<String>, Vec<String>) {
    let classic = release.classic();
    let (mut dropped, mut trimmed) = (Vec::new(), Vec::new());
    for ch in reference["chapters"].as_array_mut().into_iter().flatten() {
        let Some(cmds) = ch["commands"].as_array_mut() else {
            continue;
        };
        cmds.retain_mut(|c| {
            let name = c["name"].as_str().unwrap_or("").to_string();
            let rows: Vec<(String, String)> = c["sections"]["Command History"]["rows"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| {
                    Some((
                        row.get(0)?.as_str()?.to_string(),
                        row.get(1)?.as_str()?.to_string(),
                    ))
                })
                .filter(|(when, _)| !when.eq_ignore_ascii_case("release"))
                .collect();
            let introduced: Vec<&(String, String)> = rows
                .iter()
                .filter(|(_, what)| what.to_lowercase().contains("introduced"))
                .collect();
            let only_xe = !introduced.is_empty() && introduced.iter().all(|(w, _)| xe_row(w));
            let keep = if classic {
                !only_xe
            } else {
                !only_xe
                    || introduced.iter().any(|(w, _)| {
                        xe_before_16(w) || xe_train(w).is_some_and(|w| release.includes(w))
                    })
            };
            if !keep {
                dropped.push(name);
                return false;
            }
            let later: BTreeSet<String> = rows
                .iter()
                .filter(|(when, _)| {
                    if classic {
                        xe_row(when) || lan_row(when).is_some_and(|w| !release.includes(w))
                    } else {
                        xe_train(when).is_some_and(|w| !release.includes(w))
                    }
                })
                .flat_map(|(_, what)| keywords_added(what))
                .collect();
            if later.is_empty() {
                return true;
            }
            let own = name
                .split('(')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .count();
            let lines: Vec<Value> = c["syntax"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|s| without(s, own, &later))
                .map(Value::from)
                .collect();
            if c["syntax"].as_array().is_some_and(|old| *old != lines) {
                trimmed.push(name);
                c["syntax"] = Value::from(lines);
            }
            true
        });
    }
    (dropped, trimmed)
}

/// Cisco's general command references (`reference/ios-xe-cr.json`, one entry per book) as one
/// grammar for a release, gated by their Command History (`gate_general`), with the overrides their
/// own lines and examples call for (`certificate` under a chain takes `self-signed` and `ca`, as
/// every capture prints it).
pub fn general_for(
    refs: &Value,
    release: Release,
    overrides: Option<&str>,
) -> Result<Grammar, String> {
    let chapters: Vec<Value> = refs["books"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|b| b["chapters"].as_array().cloned().unwrap_or_default())
        .collect();
    let mut reference = serde_json::json!({
        "release": release.to_string(),
        "platform": "Cisco IOS general references",
        "source": "",
        "chapters": chapters,
    });
    gate_general(&mut reference, release);
    let mut g = compile(&reference);
    prune(&mut g);
    if let Some(text) = overrides {
        apply_overrides(&mut g, text, &BTreeSet::new())?;
    }
    for c in &mut g.commands {
        for e in &mut c.examples {
            e.output.clear();
        }
    }
    Ok(g)
}

/// The configuration and EXEC lines a book's own examples type at a prompt, by mode, lowercase.
pub fn prompted_lines(book: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    fn walk(v: &Value, key: &str, out: &mut Vec<(String, String)>) {
        match v {
            Value::Object(m) => m.iter().for_each(|(k, x)| walk(x, k, out)),
            Value::Array(a) => a.iter().for_each(|x| walk(x, key, out)),
            Value::String(s) if matches!(key, "pre" | "pres" | "examples") => {
                for l in s.lines() {
                    if let Some((mode, cmd)) = prompt(l)
                        && !cmd.is_empty()
                    {
                        out.push((mode, cmd.to_lowercase()));
                    }
                }
            }
            _ => {}
        }
    }
    walk(book, "", &mut out);
    out
}

/// The capture configuration lines an engine refuses, each with the mode it was read in, lowercase
/// and without `no` (`refused_lines`, which drops the mode). Each line is parsed once per mode.
fn refused_in_modes(e: &crate::cli::Engine, configs: &[String]) -> BTreeSet<(String, String)> {
    let mut seen: BTreeMap<(String, String), Option<Option<String>>> = BTreeMap::new();
    let mut out = BTreeSet::new();
    for cfg in configs {
        let mut open: Vec<(usize, Option<String>)> = Vec::new();
        for raw in crate::live::config_lines(cfg) {
            let indent = raw.len() - raw.trim_start().len();
            while open.last().is_some_and(|(i, _)| *i >= indent) {
                open.pop();
            }
            let mode = match open.last() {
                Some((_, Some(m))) => m.clone(),
                Some((_, None)) => {
                    open.push((indent, None));
                    continue;
                }
                None => "config".to_string(),
            };
            // IOS XE prints an access list entry's number, which names no command
            let Some(line) = unnumbered(&mode, raw.trim()) else {
                open.push((indent, None));
                continue;
            };
            // Some(enters) when the line is taken, None when refused
            let read = seen
                .entry((mode.clone(), line.to_string()))
                .or_insert_with(|| match e.parse(&mode, line) {
                    crate::cli::Outcome::Ok(p) => Some(e.enters(&p).map(str::to_string)),
                    _ => None,
                })
                .clone();
            let entered = match read {
                Some(enters) => enters,
                None => {
                    let l = line.to_lowercase();
                    out.insert((mode, l.strip_prefix("no ").map(str::to_string).unwrap_or(l)));
                    None
                }
            };
            open.push((indent, entered));
        }
    }
    out
}

/// Each top-level line of the captures with the lines under it, every capture's alike joined.
fn blocks(configs: &[String]) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for cfg in configs {
        let mut header = String::new();
        for l in crate::live::config_lines(cfg) {
            if l.starts_with(char::is_whitespace) {
                if !header.is_empty() {
                    out.entry(header.clone())
                        .or_default()
                        .insert(l.trim().to_string());
                }
            } else {
                header = l.trim().to_string();
            }
        }
    }
    out
}

/// Headers the books leave without a mode: each enters the mode the general references' examples
/// show their command of the same name entering, where the captures hold a line under one of its
/// headers that the platform's books or the general references read in that mode (`crypto pki
/// trustpoint` and its `enrollment selfsigned`). Returns how many opened.
fn open_headers(g: &mut Grammar, general: &mut General, configs: &[String]) -> usize {
    let entered = general.entered().clone();
    let ge = &general.engine;
    let by_name: BTreeMap<&str, usize> = ge
        .grammar
        .commands
        .iter()
        .enumerate()
        .map(|(i, c)| (c.name.as_str(), i))
        .collect();
    let wanted: BTreeMap<usize, &BTreeSet<String>> = g
        .commands
        .iter()
        .enumerate()
        .filter(|(_, c)| c.enters.is_none())
        .filter_map(|(ci, c)| Some((ci, entered.get(by_name.get(c.name.as_str())?)?)))
        .collect();
    if wanted.is_empty() {
        return 0;
    }
    let e = crate::cli::Engine::new(g.clone());
    let reads = |mode: &str, line: &str| {
        matches!(e.parse(mode, line), crate::cli::Outcome::Ok(_))
            || matches!(ge.parse(mode, line), crate::cli::Outcome::Ok(_))
    };
    let mut opened = Vec::new();
    for (header, under) in blocks(configs) {
        let crate::cli::Outcome::Ok(p) = e.parse("config", &header) else {
            continue;
        };
        let Some(modes) = wanted.get(&p.command) else {
            continue;
        };
        if opened.iter().any(|(ci, _)| *ci == p.command) {
            continue;
        }
        if let Some(m) = modes.iter().find(|m| under.iter().any(|l| reads(m, l))) {
            opened.push((p.command, m.clone()));
        }
    }
    for (ci, m) in &opened {
        let c = &mut g.commands[*ci];
        let why = format!(
            "enters {m} as the general references' examples show and the captures' lines under it read"
        );
        c.evidence = Some(match c.evidence.take() {
            Some(e) => format!("{e}; {why}"),
            None => why,
        });
        c.enters = Some(m.clone());
    }
    opened.len()
}

/// The general references at one release (`general_for`), and the command each line they were
/// asked about reads as, so books of the same release ask once.
pub struct General {
    pub engine: crate::cli::Engine,
    read: BTreeMap<(String, String), Option<usize>>,
    /// the submodes each command's examples show it entering, read once
    entered: Option<BTreeMap<usize, BTreeSet<String>>>,
}

impl General {
    pub fn new(grammar: Grammar) -> Self {
        General {
            engine: crate::cli::Engine::new(grammar),
            read: BTreeMap::new(),
            entered: None,
        }
    }

    /// The submodes the references' examples show each command entering, by command; a mode with no
    /// command of its own is left out.
    fn entered(&mut self) -> &BTreeMap<usize, BTreeSet<String>> {
        let e = &self.engine;
        self.entered.get_or_insert_with(|| {
            let mut out: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
            for t in &e.grammar.transitions {
                if let crate::cli::Outcome::Ok(p) = e.parse(&t.from, &t.line)
                    && !p.negated
                    && crate::cli::is_submode(&t.to)
                    && e.has_mode(&t.to)
                {
                    out.entry(p.command).or_default().insert(t.to.clone());
                }
            }
            out
        })
    }

    /// The general command that reads a line in a mode.
    fn command(&mut self, mode: &str, line: &str) -> Option<usize> {
        let e = &self.engine;
        *self
            .read
            .entry((mode.to_string(), line.to_string()))
            .or_insert_with(|| match e.parse(mode, line) {
                crate::cli::Outcome::Ok(p) if e.grammar.commands[p.command].name != "show" => {
                    Some(p.command)
                }
                _ => None,
            })
    }
}

/// General IOS commands a platform's own switches and books attest: a line its captures hold, a
/// line its sessions typed and the switch answered, or a line its guides' examples type, that its
/// grammar refuses, takes the general command that reads the line in that mode, with the transitions
/// the general references' examples show, unless the platform's books have a command of that name
/// there. It runs only in its modes where the books have none of that name. Rounds repeat while they
/// take something, so a borrowed block's lines are read in its mode. Returns the names taken.
pub fn borrow_general(
    g: &mut Grammar,
    general: &mut General,
    configs: &[String],
    sessions: &[String],
    guided: &[(String, String)],
) -> Vec<String> {
    let mut taken: BTreeSet<usize> = BTreeSet::new();
    // the guide lines still refused; all of them before the first round
    let mut others: BTreeSet<(String, String)> =
        guided.iter().map(|(m, l)| (m.clone(), l.clone())).collect();
    for _ in 0..4 {
        let opened = open_headers(g, general, configs);
        let e = crate::cli::Engine::new(g.clone());
        let refused_others: BTreeSet<(String, String)> = others
            .iter()
            .filter(|(m, l)| !matches!(e.parse(m, l), crate::cli::Outcome::Ok(_)))
            .cloned()
            .collect();
        let mut refused = refused_in_modes(&e, configs);
        refused.extend(refused_commands_by(&e, sessions));
        refused.extend(
            refused_others
                .iter()
                .map(|(m, l)| (m.clone(), l.strip_prefix("no ").unwrap_or(l).to_string())),
        );
        others = refused_others;
        // a command the platform's own books give in a mode is theirs to read there, and in the modes
        // that run that mode's commands: the general one (`interface` for every IOS platform's
        // interface types) never takes its place
        let own: BTreeSet<(String, String)> = g
            .commands
            .iter()
            .flat_map(|c| c.modes.iter().map(|m| (m.clone(), bare_name(&c.name))))
            .collect();
        let owned = |mode: &str, name: &str| {
            runs_in(mode)
                .iter()
                .any(|m| own.contains(&(m.to_string(), name.to_string())))
        };
        // each general command taken, with the modes it runs in here
        let mut take: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for (mode, line) in &refused {
            let Some(i) = general.command(mode, line) else {
                continue;
            };
            let c = &general.engine.grammar.commands[i];
            let name = bare_name(&c.name);
            if taken.contains(&i) || owned(mode, &name) {
                continue;
            }
            let lands: Vec<String> = c
                .modes
                .iter()
                .filter(|m| !owned(m, &name))
                .cloned()
                .collect();
            // the mode it read the line through must be one it may run in
            if runs_in(mode).iter().any(|m| lands.iter().any(|l| l == m)) {
                take.insert(i, lands);
            }
        }
        if take.is_empty() {
            // a header opened this round reads its lines in its mode next round
            if opened == 0 {
                break;
            }
            continue;
        }
        let ge = &general.engine;
        for (&i, lands) in &take {
            let c = &ge.grammar.commands[i];
            g.commands.push(Command {
                source: format!("{}: {}", ge.grammar.platform, c.source),
                modes: lands.clone(),
                ..c.clone()
            });
        }
        for t in &ge.grammar.transitions {
            if let crate::cli::Outcome::Ok(p) = ge.parse(&t.from, &t.line)
                && take.contains_key(&p.command)
            {
                g.transitions.push(t.clone());
            }
        }
        taken.extend(take.into_keys());
    }
    taken
        .iter()
        .map(|&i| general.engine.grammar.commands[i].name.clone())
        .collect()
}

/// The lines a session typed that the switch answered without an error (`%`) and the grammar refuses,
/// lowercase, each with the mode it is borrowed for: a command at `>` or `#` as a privileged one (the
/// 15.0(2)SE book lacks `show ip interface brief`, which the lab 2960G answers), and `do <command>`
/// as well; a configuration line in its own mode, without `no`. In an IPv4 access list's submode an
/// entry's sequence number is left off (`15 permit ip any any`), and `no 15` names no command.
fn refused_commands(g: &Grammar, sessions: &[String]) -> Vec<(String, String)> {
    refused_commands_by(&crate::cli::Engine::new(g.clone()), sessions)
}

/// `refused_commands` with an engine already built.
fn refused_commands_by(e: &crate::cli::Engine, sessions: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for log in sessions {
        for x in crate::capture::exchanges(log)
            .map(|(_, x)| x)
            .unwrap_or_default()
        {
            let typed = x.command.trim();
            if typed.is_empty()
                || typed.contains('?')
                || x.output.iter().any(|l| l.trim_start().starts_with('%'))
            {
                continue;
            }
            let (read_in, mode, line) = if matches!(x.mode.as_str(), "exec" | "privileged") {
                (x.mode.as_str(), "privileged", typed)
            } else if !crate::cli::is_config(&x.mode) {
                continue;
            } else if let Some(command) = typed.strip_prefix("do ") {
                ("privileged", "privileged", command.trim())
            } else {
                let line = typed.strip_prefix("no ").unwrap_or(typed).trim();
                let Some(line) = unnumbered(&x.mode, line) else {
                    continue;
                };
                (x.mode.as_str(), x.mode.as_str(), line)
            };
            if !matches!(e.parse(read_in, line), crate::cli::Outcome::Ok(_)) {
                out.push((mode.to_string(), line.to_lowercase()));
            }
        }
    }
    out
}

/// An IPv4 access list entry without the sequence number IOS XE prints and a session may type before
/// it (`15 permit ip any any`), which names no command; `None` for a number alone (`no 15`'s).
fn unnumbered<'a>(mode: &str, line: &'a str) -> Option<&'a str> {
    if !matches!(mode, "config-std-nacl" | "config-ext-nacl") {
        return Some(line);
    }
    match line.split_once(' ') {
        Some((n, entry)) if n.bytes().all(|b| b.is_ascii_digit()) => Some(entry.trim()),
        None if line.bytes().all(|b| b.is_ascii_digit()) => None,
        _ => Some(line),
    }
}

/// The capture configuration lines the grammar refuses, lowercase and without `no`: each line read in
/// the mode its block's header enters, by indentation; a line under a refused header is not read.
fn refused_lines(g: &Grammar, configs: &[String]) -> Vec<String> {
    let e = crate::cli::Engine::new(g.clone());
    let mut out = Vec::new();
    for cfg in configs {
        // (indentation, the mode the line at it entered; None under a refused or plain line)
        let mut open: Vec<(usize, Option<String>)> = Vec::new();
        for raw in crate::live::config_lines(cfg) {
            let indent = raw.len() - raw.trim_start().len();
            while open.last().is_some_and(|(i, _)| *i >= indent) {
                open.pop();
            }
            let mode = match open.last() {
                Some((_, Some(m))) => m.clone(),
                Some((_, None)) => {
                    open.push((indent, None));
                    continue;
                }
                None => "config".to_string(),
            };
            let Some(line) = unnumbered(&mode, raw.trim()) else {
                open.push((indent, None));
                continue;
            };
            let entered = match e.parse(&mode, line) {
                crate::cli::Outcome::Ok(p) => e.enters(&p).map(str::to_string),
                _ => {
                    let l = line.to_lowercase();
                    out.push(l.strip_prefix("no ").map(str::to_string).unwrap_or(l));
                    None
                }
            };
            open.push((indent, entered));
        }
    }
    out
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Transition {
    /// the mode the line was typed in
    pub from: String,
    /// an example line after which the prompt showed another mode
    pub line: String,
    pub to: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Grammar {
    pub release: String,
    pub platform: String,
    pub source: String,
    pub commands: Vec<Command>,
    pub transitions: Vec<Transition>,
}

/// A configuration mode's id as IOS prompts it, from a book's spelling: lowercase, hyphens for the
/// underscores, no space (`config- vrf-af`) and no trailing `#` some books print (`config_router-af`,
/// `config#`), and `configure` read as `config` (`configure-cmap-p`). `None` for a name that is no
/// configuration mode.
fn mode_id(raw: &str) -> Option<String> {
    let mut m = raw
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .trim_end_matches('#')
        .to_lowercase()
        .replace('_', "-");
    if let Some(rest) = m.strip_prefix("configure") {
        m = format!("config{rest}");
    }
    crate::cli::is_config(&m).then_some(m)
}

/// Mode ids from a "Command Modes" text: `exec`, `privileged`, `config`, `config-if`,
/// `ca-trustpoint`, ...
pub fn modes(text: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    let lower = text.to_lowercase();
    // every parenthesised mode id: `(config-if)`, `(ca-trustpoint)`
    let mut rest = text;
    while let Some(i) = rest.find('(') {
        let tail = &rest[i + 1..];
        if let Some(j) = tail.find(')')
            && let Some(m) = mode_id(&tail[..j])
        {
            out.insert(m);
        }
        rest = tail;
    }
    if lower.contains("user exec") || text.contains("(>)") {
        out.insert("exec".into());
    }
    if lower.contains("privileged exec") || text.contains("(#)") {
        out.insert("privileged".into());
    }
    for (n, id) in NAMED_MODES {
        if lower.contains(n) && !out.iter().any(|m| m.starts_with(id)) {
            out.insert(id.into());
        }
    }
    out.into_iter().collect()
}

/// The modes a Command Modes section may name in words alone.
const NAMED_MODES: [(&str, &str); 5] = [
    ("global configuration", "config"),
    ("interface configuration", "config-if"),
    ("vlan configuration", "config-vlan"),
    ("line configuration", "config-line"),
    ("router configuration", "config-router"),
];

/// Whether a Command Modes section names a configuration mode in words beyond `NAMED_MODES`
/// (`Policy-map class configuration` in the classic books): the prompts its examples type the command
/// at then say which, as an id beside the words can be a misprint (`power-priority`'s `(config-stack)`,
/// where its examples show `(config-switch-stackpower)`).
fn unnamed_modes(text: &str) -> bool {
    let mut bare = String::new();
    let mut depth = 0;
    for ch in text.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth = (depth - 1).max(0),
            c if depth == 0 => bare.push(c),
            _ => {}
        }
    }
    let mut lower = bare.to_lowercase();
    for (n, _) in NAMED_MODES {
        lower = lower.replace(n, " ");
    }
    lower.contains("configuration")
}

/// A prompt line from an example: `Device(config-if)# switchport mode access`.
pub fn prompt(line: &str) -> Option<(String, String)> {
    let t = line.trim_start();
    let host_end =
        t.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))?;
    if host_end == 0 {
        return None;
    }
    let rest = &t[host_end..];
    let (mode, after) = if let Some(r) = rest.strip_prefix('(') {
        let close = r.find(')')?;
        // a configuration mode: (config-if), (conf-ssh-pubkey-user), (ca-trustpoint)
        (Some(mode_id(&r[..close])?), &r[close + 1..])
    } else {
        (None, rest)
    };
    let (sym, cmd) = match after.strip_prefix('#') {
        Some(c) => ('#', c),
        None => ('>', after.strip_prefix('>')?),
    };
    let mode = match (mode, sym) {
        (Some(m), _) => m,
        (None, '#') => "privileged".into(),
        (None, _) => "exec".into(),
    };
    Some((mode, cmd.trim().to_string()))
}

fn range_in(text: &str) -> Option<(i64, i64)> {
    // "range is 1 to 4094", "from 1 to 4094", "1 to 4094", "1-4094"
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')' || c == ';')
        .filter(|w| !w.is_empty())
        .collect();
    // "from 1 to 32 characters" bounds a string's length: the lab C2960G took `vtp password a`
    let length = |end: usize| {
        words[end + 1..words.len().min(end + 4)]
            .iter()
            .any(|w| w.to_ascii_lowercase().starts_with("character"))
    };
    for i in 0..words.len() {
        let a = words[i].trim_end_matches('.');
        if let Some((x, y)) = a.split_once('-')
            && let (Ok(x), Ok(y)) = (x.parse::<i64>(), y.parse::<i64>())
            && x <= y
            && words[..i].iter().any(|w| {
                w.eq_ignore_ascii_case("range")
                    || w.eq_ignore_ascii_case("from")
                    || w.eq_ignore_ascii_case("between")
            })
            && !length(i)
        {
            return Some((x, y));
        }
        if i + 2 < words.len()
            && (words[i + 1] == "to" || words[i + 1] == "and" || words[i + 1] == "through")
            && let (Ok(x), Ok(y)) = (
                a.parse::<i64>(),
                words[i + 2].trim_end_matches('.').parse::<i64>(),
            )
            && x <= y
            && !length(i + 2)
        {
            return Some((x, y));
        }
    }
    None
}

/// An argument's type, from the whole words of its name and the range its Syntax Description row
/// states. A name is what its last words make it: `hash-mask-length` is a length, not a mask,
/// `ipv6-access-list-name` a name, not an address, `context-name` a name, not text, and
/// `interface-priority` a number, not an interface. An underscore reads as a hyphen (`ip_address`), and
/// the guides' soft hyphens (`interface-\u{ad}name`) as nothing.
pub fn arg_kind(name: &str, description: &str) -> ArgKind {
    let n = name.to_lowercase().replace('_', "-").replace('\u{ad}', "");
    if n.split_once(' ').is_some_and(|(a, b)| interface_pair(a, b)) {
        return ArgKind::Interface;
    }
    // a split mask is its owner's kind: a MAC address's mask is one, an EtherType's a number
    if let Some(owner) = mask_owner(&n) {
        return arg_kind(owner, description);
    }
    // one value IOS reads as one word: `<network>/<length>`, `<hh>:<mm>`, `<user>@<host>`
    if let Some(i) = n.find(['/', ':', '@']).filter(|&i| i > 0) {
        return match arg_kind(&n[..i], "") {
            ArgKind::Ipv6 if n[i..].starts_with('/') => ArgKind::Ipv6,
            _ => ArgKind::Word,
        };
    }
    let words: Vec<&str> = n.split(['-', ' ']).filter(|w| !w.is_empty()).collect();
    let has = |w: &str| words.contains(&w);
    let last = words.last().copied().unwrap_or("");
    // the name of a thing, not the thing (`ipv4-access-list`, `ipv6-hostname`, `mask-length`)
    let named = matches!(last, "name" | "list" | "acl" | "hostname" | "length");
    if has("vlan") && (has("list") || has("range")) || n == "list" && numbered_list(description) {
        return ArgKind::VlanList;
    }
    if interface_words(&words) {
        return ArgKind::Interface;
    }
    if n.contains("ipv6") && !named {
        return ArgKind::Ipv6;
    }
    if last.ends_with("mask") {
        if has("mac") {
            return ArgKind::Mac;
        }
        if !["ethertype", "lsap", "type", "bit"].iter().any(|w| has(w)) {
            return ArgKind::Ipv4;
        }
    } else if !named
        && (n.contains("ip-address")
            || n.contains("ipv4")
            || n == "address"
            || n.contains("ip-addr"))
    {
        return ArgKind::Ipv4;
    }
    if n.contains("mac") && n.contains("addr") {
        return ArgKind::Mac;
    }
    // the 2960 books range `new-dscp` 0 to 63 while their own examples type `set dscp af41`
    if n == "new-dscp" {
        return ArgKind::Dscp;
    }
    // a description's or a text's length limit is no range of numbers
    if !named && (has("description") || has("text")) {
        return ArgKind::Text;
    }
    if let Some((min, max)) = range_in(description) {
        return ArgKind::Number { min, max };
    }
    if n.contains("vlan-id") || n == "vlan" || n == "vlan-id" {
        return ArgKind::Number { min: 1, max: 4094 };
    }
    // a description's `character-string` runs to the end of the line, as IOS takes it
    if !named && (n == "string" || n == "character-string" || has("message") || n == "line") {
        return ArgKind::Text;
    }
    ArgKind::Word
}

/// Whether a bare `<list>` lists numbers, as a VLAN list does: VLANs, spanning-tree instances, SPAN
/// sessions or modules, by its Syntax Description.
fn numbered_list(description: &str) -> bool {
    let d = description.to_lowercase();
    ["vlan", "instance", "session", "module"]
        .iter()
        .any(|w| d.contains(w))
}

/// Whether an argument's words name an interface: `interface` with only its id, name, type or number
/// after it (`interface-id`, `source-interface`, `interface_type_no`).
fn interface_words(words: &[&str]) -> bool {
    words
        .iter()
        .rposition(|w| *w == "interface")
        .is_some_and(|i| {
            words[i + 1..].iter().all(|w| {
                matches!(
                    *w,
                    "id" | "name" | "type" | "number" | "num" | "no" | "list" | "range"
                )
            })
        })
}

/// Whether two arguments side by side are an interface's type and its number (`<interface-type>
/// <interface-number>`, `<type> <slot/port>`), which IOS reads as one interface name.
fn interface_pair(first: &str, second: &str) -> bool {
    let (first, second) = (first.to_lowercase(), second.to_lowercase());
    let a_type = first == "type"
        || first == "interface"
        || first.contains("interface") && first.contains("type");
    a_type
        && !second.contains("type")
        && ["number", "num", "slot", "port", "id", "name"]
            .iter()
            .any(|n| second.contains(n))
}

/// A command's words, lowercase, without the qualifier the books put after them (`description
/// (class-map)` is `description`).
fn bare_name(name: &str) -> String {
    name.split('(').next().unwrap_or("").trim().to_lowercase()
}

/// The modes whose commands a line typed in `mode` is read against, as the engine reads it: the mode,
/// user EXEC's from privileged EXEC, the interface's from an interface range, and `*config`'s from a
/// configuration submode.
fn runs_in(mode: &str) -> Vec<&str> {
    let mut out = vec![mode];
    if mode == "privileged" {
        out.push("exec");
    }
    if mode == "config-if-range" {
        out.push("config-if");
    }
    if crate::cli::is_submode(mode) {
        out.push("*config");
    }
    out
}

/// A syntax line as the compiler reads it (`syntax::parse_with`, with the arguments its command's
/// Syntax Description names where it has one), each pair of arguments IOS reads as one value joined:
/// an interface's type and number (`<interface-type> <interface-number>`), and two parts written
/// with `/`, `:` or `@` between them (`<network>/<length>`, `<hh>:<mm>`).
fn syntax_line(text: &str, known: Option<&BTreeSet<String>>) -> Line {
    let (node, mut repairs) = match known {
        Some(k) => syntax::parse_with(text, k),
        None => syntax::parse(text),
    };
    Line {
        text: text.to_string(),
        negated: text.starts_with("no "),
        node: joined(node, &mut repairs),
        repairs,
    }
}

/// `syntax_line`'s joins, in every sequence of the node, each listed as a repair.
fn joined(node: Node, repairs: &mut Vec<String>) -> Node {
    match node {
        Node::Seq(items) => {
            let mut out: Vec<Node> = Vec::with_capacity(items.len());
            for n in items {
                out.push(joined(n, repairs));
                // a join can make another: `<type> <slot>/<port>`
                loop {
                    let k = out.len();
                    let one = match &out[k.saturating_sub(3)..] {
                        [Node::Arg(a), Node::Kw(sep), Node::Arg(b)]
                            if sep.chars().all(|c| matches!(c, '/' | ':' | '@')) =>
                        {
                            Some((3, format!("<{a}>{sep}<{b}>"), format!("{a}{sep}{b}")))
                        }
                        [.., Node::Arg(a), Node::Arg(b)] if interface_pair(a, b) => {
                            Some((2, format!("<{a}> <{b}>"), format!("{a} {b}")))
                        }
                        _ => None,
                    };
                    let Some((parts, written, name)) = one else {
                        break;
                    };
                    repairs.push(format!("arguments {written} read as <{name}>"));
                    out.truncate(k - parts);
                    out.push(Node::Arg(name));
                }
            }
            if out.len() == 1 {
                out.pop().unwrap_or(Node::Seq(vec![]))
            } else {
                Node::Seq(out)
            }
        }
        Node::Alt(v) => Node::Alt(v.into_iter().map(|n| joined(n, repairs)).collect()),
        Node::Opt(b) => Node::Opt(Box::new(joined(*b, repairs))),
        other => other,
    }
}

/// The node with each argument a keyword stands before renamed `<keyword> <argument>` where the
/// command types it there apart from the same argument after other keywords (`hello-time <seconds>`
/// is 1 to 10, `max-age <seconds>` 6 to 40).
fn qualify(node: &mut Node, args: &BTreeMap<String, ArgKind>) {
    match node {
        Node::Seq(items) => {
            for i in 1..items.len() {
                if let (Node::Kw(k), Node::Arg(a)) = (&items[i - 1], &items[i]) {
                    let key = format!("{} <{a}>", k.to_lowercase());
                    if args.contains_key(&key) {
                        items[i] = Node::Arg(key);
                    }
                }
            }
            items.iter_mut().for_each(|n| qualify(n, args));
        }
        Node::Alt(v) => v.iter_mut().for_each(|n| qualify(n, args)),
        Node::Opt(b) => qualify(b, args),
        Node::Kw(_) | Node::Arg(_) => {}
    }
}

fn texts(sec: &Value) -> String {
    sec["text"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn keywords_of(n: &Node, out: &mut BTreeSet<String>) {
    match n {
        Node::Kw(k) => {
            out.insert(k.to_lowercase());
        }
        Node::Seq(v) | Node::Alt(v) => v.iter().for_each(|x| keywords_of(x, out)),
        Node::Opt(b) => keywords_of(b, out),
        Node::Arg(_) => {}
    }
}

/// The first sentence of a description, whitespace collapsed, without its full stop.
fn first_sentence(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = flat.find(". ").map_or(flat.len(), |i| i + 1);
    flat[..end].trim_end_matches('.').to_string()
}

/// A description without its parenthesised asides: "Sets the port to access mode (either ...)" is
/// "Sets the port to access mode".
fn without_asides(text: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What the command does, from "To enable PortFast ..., use the ... command": "Enable PortFast ...".
fn intro_help(intro: &str) -> Option<String> {
    let s = first_sentence(intro);
    let rest = s.strip_prefix("To ")?;
    let what = &rest[..rest.find(", use the ")?];
    let mut c = what.chars();
    let first = c.next()?;
    Some(first.to_uppercase().chain(c).collect())
}

fn rename_arg(n: &mut Node, from: &str, to: &str) {
    match n {
        Node::Arg(a) if a == from => *a = to.into(),
        Node::Seq(v) | Node::Alt(v) => v.iter_mut().for_each(|x| rename_arg(x, from, to)),
        Node::Opt(b) => rename_arg(b, from, to),
        Node::Arg(_) | Node::Kw(_) => {}
    }
}

fn args_of(n: &Node, out: &mut BTreeSet<String>) {
    match n {
        Node::Arg(a) => {
            out.insert(a.clone());
        }
        Node::Seq(v) | Node::Alt(v) => v.iter().for_each(|x| args_of(x, out)),
        Node::Opt(b) => args_of(b, out),
        Node::Kw(_) => {}
    }
}

/// A reference command's syntax lines split by where they belong. A line that begins with another
/// word than the command's own (`radius server`'s `key <string>`, a fragment of its first line, the
/// book's prose) stays where the command's examples type it in one of its modes (`vrrp
/// description`'s `description <text>`), or in theirs where the command has none; becomes a command
/// of its own where they type it in other modes (`key (radius server)` in config-radius-server); and
/// is dropped where they never type it (`device-tracking`'s `vlan { ... }`, which read `vlan 30` in
/// config-if). Returns the lines kept, the modes the command takes from them, and the other
/// commands, by name.
fn place_lines(
    name: &str,
    modes: &[String],
    lines: Vec<Line>,
    examples: &[Example],
) -> (Vec<Line>, Vec<String>, BTreeMap<String, Placed>) {
    let words = bare_name(name);
    let own = own_word(name);
    // an example's first word, past `no` and `default`
    let first = |line: &str| {
        line.split_whitespace()
            .map(str::to_lowercase)
            .find(|w| w != "no" && w != "default")
    };
    let mut kept = Vec::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut moved: BTreeMap<String, Placed> = BTreeMap::new();
    for l in lines {
        if begins_own(&l.node, &own) {
            kept.push(l);
            continue;
        }
        let starts = first_words(&l.node);
        let typed: Vec<&Example> = examples
            .iter()
            .filter(|e| first(&e.line).is_some_and(|w| starts.contains(&w)))
            .collect();
        let at: BTreeSet<String> = typed.iter().map(|e| e.mode.clone()).collect();
        if at.is_empty() {
            continue;
        }
        if modes.is_empty() || at.iter().any(|m| modes.contains(m)) {
            if modes.is_empty() {
                taken.extend(at);
            }
            kept.push(l);
            continue;
        }
        let lead: Vec<String> = syntax::leading_keywords(&l.node)
            .into_iter()
            .skip_while(|k| k.eq_ignore_ascii_case("no") || k.eq_ignore_ascii_case("default"))
            .collect();
        let lead = if lead.is_empty() {
            words.clone()
        } else {
            lead.join(" ")
        };
        let p = moved.entry(format!("{lead} ({words})")).or_default();
        p.modes.extend(at);
        p.lines.push(l);
        p.examples.extend(typed.into_iter().cloned());
    }
    (kept, taken.into_iter().collect(), moved)
}

/// A command of its own for lines a book files under another (`key (radius server)`): the modes its
/// examples type them in, the lines, and those examples.
#[derive(Default)]
struct Placed {
    modes: BTreeSet<String>,
    lines: Vec<Line>,
    examples: Vec<Example>,
}

/// A command's own first word, lowercase: `switchport` for `switchport mode`, past a `no`.
fn own_word(name: &str) -> String {
    bare_name(name)
        .split_whitespace()
        .find(|w| *w != "no")
        .unwrap_or("")
        .to_string()
}

/// Whether a syntax line begins with its command's own word (`own_word`) past `no` and `default`, or
/// with `no-` before it (`no-match`), or with an argument, which `prune` judges.
fn begins_own(n: &Node, own: &str) -> bool {
    let starts = first_words(n);
    starts.is_empty() || starts.contains(own) || starts.contains(&format!("no-{own}"))
}

/// The words a syntax line can begin with, lowercase, past a leading `no` or `default`.
fn first_words(n: &Node) -> BTreeSet<String> {
    fn starts(items: &[Node], out: &mut BTreeSet<String>) {
        match items.split_first() {
            None | Some((Node::Arg(_), _)) => {}
            Some((Node::Kw(k), rest))
                if !rest.is_empty()
                    && (k.eq_ignore_ascii_case("no") || k.eq_ignore_ascii_case("default")) =>
            {
                starts(rest, out);
            }
            Some((Node::Kw(k), _)) => {
                out.insert(k.to_lowercase());
            }
            Some((Node::Alt(branches), _)) => {
                for b in branches {
                    starts(std::slice::from_ref(b), out);
                }
            }
            Some((Node::Opt(inner), rest)) => {
                starts(std::slice::from_ref(inner), out);
                starts(rest, out);
            }
            Some((Node::Seq(v), rest)) => {
                let joined: Vec<Node> = v.iter().chain(rest).cloned().collect();
                starts(&joined, out);
            }
        }
    }
    let mut out = BTreeSet::new();
    starts(std::slice::from_ref(n), &mut out);
    out
}

/// Each argument's type, from its name and the Syntax Description rows naming it (`rows`, each its
/// cells). Where rows after different keywords type it apart (`hello-time <seconds>` 1 to 10,
/// `max-age <seconds>` 6 to 40), each keyword's own goes under `<keyword> <argument>` and the widest
/// under the name. A split mask takes its owner's type (`<src-MAC-addr mask>` is a MAC address's).
fn kinds_of(
    names: &BTreeSet<String>,
    rows: &[Vec<&str>],
    kws: &BTreeSet<String>,
) -> BTreeMap<String, ArgKind> {
    let mut typed: BTreeMap<&str, Vec<(Option<String>, ArgKind)>> = BTreeMap::new();
    for cells in rows {
        for a in names.iter().filter(|a| row_names(cells[0], a)) {
            typed.entry(a).or_default().push((
                keyword_before(cells[0], a, kws),
                arg_kind(a, &cells[1..].join(" ")),
            ));
        }
    }
    let mut args: BTreeMap<String, ArgKind> = BTreeMap::new();
    for a in names {
        let rows = typed.get(a.as_str()).map(Vec::as_slice).unwrap_or_default();
        let kind = match rows.last() {
            None => arg_kind(a, ""),
            Some((_, k)) if rows.iter().all(|(_, x)| x == k) => k.clone(),
            Some(_) => {
                for (kw, k) in rows {
                    if let Some(kw) = kw {
                        args.insert(format!("{kw} <{a}>"), k.clone());
                    }
                }
                widest(rows.iter().map(|(_, k)| k))
            }
        };
        args.insert(a.clone(), kind);
    }
    for a in names {
        if let Some(k) = mask_owner(a).and_then(|o| args.get(o)).cloned() {
            args.insert(a.clone(), k);
        }
    }
    args
}

/// What `?` says of each keyword and argument: the first sentence of the first Syntax Description row
/// naming it. A row describes its arguments and its last keyword (`dynamic auto` is `auto`), an
/// argument a keyword types apart its own (`hello-time <seconds>`), and a split mask the mask's row.
fn help_of(
    names: &BTreeSet<String>,
    rows: &[Vec<&str>],
    kws: &BTreeSet<String>,
    args: &BTreeMap<String, ArgKind>,
) -> BTreeMap<String, String> {
    let mut help: BTreeMap<String, String> = BTreeMap::new();
    for cells in rows {
        let text = without_asides(&first_sentence(&cells[1..].join(" ")));
        if text.is_empty() {
            continue;
        }
        let words: Vec<&str> = cells[0]
            .split(|c: char| c.is_whitespace() || "{}[]|<>".contains(c))
            .collect();
        let last_kw = words.iter().rev().find_map(|w| kws.get(&w.to_lowercase()));
        let named = names.iter().filter(|a| {
            words.iter().any(|w| a.eq_ignore_ascii_case(w))
                || row_names(cells[0], a)
                || mask_owner(a).is_some_and(|o| {
                    let suffix = &a[o.len() + 1..];
                    row_names(cells[0], suffix)
                })
        });
        let qualified = names.iter().filter_map(|a| {
            let kw = keyword_before(cells[0], a, kws)?;
            let key = format!("{kw} <{a}>");
            (row_names(cells[0], a) && args.contains_key(&key)).then_some(key)
        });
        for key in named.chain(last_kw).cloned().chain(qualified) {
            help.entry(key).or_insert_with(|| text.clone());
        }
    }
    help
}

/// The argument a split mask or wildcard belongs to: `src-MAC-addr` for `src-MAC-addr mask`.
fn mask_owner(name: &str) -> Option<&str> {
    [" mask", " wildcard"].into_iter().find_map(|s| {
        let cut = name.len().checked_sub(s.len()).filter(|&c| c > 0)?;
        (name.is_char_boundary(cut) && name[cut..].eq_ignore_ascii_case(s)).then(|| &name[..cut])
    })
}

/// Whether a Syntax Description row's first cell names an argument: `<seconds>`, the bare name, or the
/// parts of a joined one (`<network>/<length>` names `network/length`).
fn row_names(cell: &str, arg: &str) -> bool {
    let compact: String = cell
        .chars()
        .filter(|c| !c.is_whitespace() && !"<>".contains(*c))
        .collect();
    cell.contains(&format!("<{arg}>")) || cell.trim() == arg || compact.eq_ignore_ascii_case(arg)
}

/// The command's keyword a row writes right before an argument (`hello-time` in `hello-time
/// <seconds>`), lowercase.
fn keyword_before(cell: &str, arg: &str, kws: &BTreeSet<String>) -> Option<String> {
    let words: Vec<String> = cell
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| "{}[]|".contains(c)).to_lowercase())
        .filter(|w| !w.is_empty() && w != "<>")
        .collect();
    let arg = arg.to_lowercase();
    let at = words
        .iter()
        .position(|w| *w == format!("<{arg}>") || *w == arg)?;
    let before = words.get(at.checked_sub(1)?)?;
    kws.contains(before).then(|| before.clone())
}

/// The kind that takes what any of several takes: the span of their ranges, else any word.
fn widest<'a>(kinds: impl Iterator<Item = &'a ArgKind>) -> ArgKind {
    let mut span: Option<(i64, i64)> = None;
    for k in kinds {
        let ArgKind::Number { min, max } = k else {
            return ArgKind::Word;
        };
        span = Some(span.map_or((*min, *max), |(a, b)| (a.min(*min), b.max(*max))));
    }
    span.map_or(ArgKind::Word, |(min, max)| ArgKind::Number { min, max })
}

/// The arguments a command's Syntax Description rows name one to a row (`<ip-address>`), lowercase:
/// the evidence that a run of italic words in its syntax is several arguments.
fn known_args(c: &Value) -> BTreeSet<String> {
    c["sections"]["Syntax Description"]["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r.get(0)?.as_str())
        .filter_map(|h| {
            let bare: String = h.chars().filter(|ch| !"<>[]{}|".contains(*ch)).collect();
            let words: Vec<&str> = bare.split_whitespace().collect();
            (words.len() == 1).then(|| words[0].to_lowercase())
        })
        .collect()
}

pub fn compile(reference: &Value) -> Grammar {
    let mut commands = Vec::new();
    let mut transitions: BTreeSet<(String, String, String)> = BTreeSet::new();
    for ch in reference["chapters"].as_array().into_iter().flatten() {
        let chapter = ch["chapter"].as_str().unwrap_or("");
        for c in ch["commands"].as_array().into_iter().flatten() {
            let name = c["name"].as_str().unwrap_or("").to_string();
            let secs = &c["sections"];
            let known = known_args(c);
            let lines: Vec<Line> = c["syntax"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|s| syntax_line(s, Some(&known)))
                .collect();
            let rows: Vec<Vec<&str>> = secs["Syntax Description"]["rows"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|row| {
                    row.as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<&str>>()
                })
                .filter(|cells| cells.len() >= 2)
                .collect();
            // example lines at their prompts, and the output that follows each
            let mut examples: Vec<Example> = Vec::new();
            for pre in secs["Examples"]["pre"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let mut last: Option<(String, String)> = None;
                for raw in pre.lines() {
                    if let Some((mode, cmd)) = prompt(raw) {
                        if let Some((m0, c0)) = &last
                            && *m0 != mode
                            && !c0.is_empty()
                        {
                            let first = c0.split_whitespace().next().unwrap_or("").to_lowercase();
                            if !["exit", "end", "do", "no", "configure", "enable", "disable"]
                                .contains(&first.as_str())
                                && !first.starts_with("conf")
                            {
                                transitions.insert((m0.clone(), c0.clone(), mode.clone()));
                            }
                        }
                        if !cmd.is_empty() {
                            examples.push(Example {
                                mode: mode.clone(),
                                line: cmd.clone(),
                                output: vec![],
                            });
                        }
                        last = Some((mode, cmd));
                    } else if let Some(e) = examples.last_mut() {
                        e.output.push(raw.trim_end().to_string());
                    }
                }
            }
            for e in &mut examples {
                while e.output.last().is_some_and(|l| l.trim().is_empty()) {
                    e.output.pop();
                }
            }
            let text = texts(&secs["Command Modes"]);
            let mut ms = modes(&text);
            // a submode named in words alone takes the id of the prompt its own examples type the
            // command at: `Switch(config-pmap)# class class1`; never the parent of a mode the words'
            // ids name, where no line of that mode is typed (`accept-lifetime`'s `(config-keychain)`
            // beside `(config-keychain-key)`)
            if unnamed_modes(&text) {
                let lead = name.split('(').next().unwrap_or("").trim().to_lowercase();
                let named = ms.clone();
                for e in &examples {
                    let line = e.line.to_lowercase();
                    let typed = line == lead || line.starts_with(&format!("{lead} "));
                    let sub = e.mode.starts_with("config-") || e.mode.starts_with("conf-");
                    let parent = named.iter().any(|m| m.starts_with(&format!("{}-", e.mode)));
                    if typed && sub && !parent && !ms.contains(&e.mode) {
                        ms.push(e.mode.clone());
                    }
                }
                ms.sort();
            }
            let source = format!("{chapter}.html#{}", c["id"].as_str().unwrap_or(""));
            let (lines, taken, moved) = place_lines(&name, &ms, lines, &examples);
            if ms.is_empty() {
                ms = taken;
            }
            let about = c["intro"].as_str().and_then(intro_help);
            let moved = moved
                .into_iter()
                .map(|(n, p)| (n, p.modes.into_iter().collect(), p.lines, p.examples, None));
            for (name, modes, lines, examples, about) in
                std::iter::once((name, ms, lines, examples, about)).chain(moved)
            {
                let (mut names, mut kws) = (BTreeSet::new(), BTreeSet::new());
                for l in &lines {
                    args_of(&l.node, &mut names);
                    keywords_of(&l.node, &mut kws);
                }
                let args = kinds_of(&names, &rows, &kws);
                let mut lines = lines;
                for l in &mut lines {
                    qualify(&mut l.node, &args);
                }
                let mut help = help_of(&names, &rows, &kws, &args);
                if let Some(about) = about {
                    help.insert(String::new(), about);
                }
                commands.push(Command {
                    name,
                    source: source.clone(),
                    modes,
                    lines,
                    args,
                    examples,
                    enters: None,
                    evidence: None,
                    help,
                });
            }
        }
    }
    Grammar {
        release: reference["release"].as_str().unwrap_or("").to_string(),
        platform: reference["platform"].as_str().unwrap_or("").to_string(),
        source: reference["source"].as_str().unwrap_or("").to_string(),
        commands,
        transitions: transitions
            .into_iter()
            .map(|(from, line, to)| Transition { from, line, to })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_from_the_reference_s_wording() {
        assert_eq!(
            modes("Interface configuration (config-if)"),
            vec!["config-if"]
        );
        assert_eq!(
            modes("User EXEC (>) Privileged EXEC (#)"),
            vec!["exec", "privileged"]
        );
        assert_eq!(modes("Global configuration"), vec!["config"]);
        assert_eq!(
            modes("Interface configuration (config-if) Virtual network interface (config-if-vnet)"),
            vec!["config-if", "config-if-vnet"]
        );
        // the PKI submodes' own family, and the books' misspellings of a mode id
        assert_eq!(
            modes("CA-trustpoint configuration (ca-trustpoint)"),
            vec!["ca-trustpoint"]
        );
        assert_eq!(
            modes("Address family configuration (config_router-af)"),
            vec!["config-router-af"]
        );
        assert_eq!(modes("Global configuration (config#)"), vec!["config"]);
        assert_eq!(
            modes("VRF address family configuration (config- vrf-af)"),
            vec!["config-vrf-af"]
        );
        assert_eq!(
            modes("Policy map class police (configure-cmap-p)"),
            vec!["config-cmap-p"]
        );
        assert!(modes("Catalyst 9300 (C9300-48P)").is_empty());
    }

    #[test]
    fn prompts_name_their_mode() {
        assert_eq!(
            prompt("Device(config-if)# switchport mode access"),
            Some(("config-if".into(), "switchport mode access".into()))
        );
        assert_eq!(
            prompt("Device# show vlan brief"),
            Some(("privileged".into(), "show vlan brief".into()))
        );
        assert_eq!(
            prompt("Switch> enable"),
            Some(("exec".into(), "enable".into()))
        );
        assert_eq!(
            prompt("host1(conf-ssh-pubkey-user)# key-string"),
            Some(("conf-ssh-pubkey-user".into(), "key-string".into()))
        );
        assert_eq!(
            prompt("Device(ca-trustpoint)# enrollment selfsigned"),
            Some(("ca-trustpoint".into(), "enrollment selfsigned".into()))
        );
        // a misprint in Cisco's REP example, not a mode
        assert_eq!(
            prompt("Device(confif-if)# rep block port 2 vlan 1-50"),
            None
        );
        // the 17.18 guides' route-target example spaces its mode id
        assert_eq!(
            prompt("Device(config- vrf-af)# route-target 65101:101"),
            Some(("config-vrf-af".into(), "route-target 65101:101".into()))
        );
        assert_eq!(
            prompt("VLAN Name                             Status    Ports"),
            None
        );
        assert_eq!(prompt("Gi1/0/1  connected  1"), None);
    }

    #[test]
    fn argument_types_come_from_names_and_stated_ranges() {
        assert_eq!(
            arg_kind(
                "vlan-id",
                "VLAN ID of the access VLAN. The range is 1 to 4094."
            ),
            ArgKind::Number { min: 1, max: 4094 }
        );
        assert_eq!(
            arg_kind(
                "channel-group-number",
                "Channel group number. The range is 1 to 128."
            ),
            ArgKind::Number { min: 1, max: 128 }
        );
        assert_eq!(arg_kind("vlan-list", ""), ArgKind::VlanList);
        assert_eq!(
            arg_kind("new-dscp", "New DSCP value. The range is 0 to 63."),
            ArgKind::Dscp
        );
        assert_eq!(arg_kind("interface-id", ""), ArgKind::Interface);
        assert_eq!(arg_kind("ip-address", ""), ArgKind::Ipv4);
        assert_eq!(
            arg_kind("priority", "Priority. The range is 1-15 (from 1 to 15)."),
            ArgKind::Number { min: 1, max: 15 }
        );
    }

    /// A one-command reference at a release, as tools/extract-releases.js writes it.
    fn reference(release: &str, syntax: &[&str]) -> Value {
        serde_json::json!({
            "release": release, "platform": "Catalyst 9300", "source": "x/",
            "chapters": [{"chapter": "vlan_commands", "commands": [{
                "name": "interface (VLAN)", "id": "a", "intro": "", "syntax": syntax,
                "sections": {"Command Modes": {"text": ["Global configuration (config)"], "rows": [], "pre": []}}
            }]}]
        })
    }

    const OVERRIDES: &str = r#"{"commands": [
        {"name": "interface (VLAN)", "remove": true, "evidence": "e",
         "corrects": ["interface {<type switch>|<slot>}"]},
        {"name": "transport input", "modes": ["config-line"], "lines": ["transport input {all | none | ssh | telnet}"],
         "evidence": "e"}
    ]}"#;

    #[test]
    fn an_override_applies_only_where_the_release_s_books_call_for_it() {
        // the defect it corrects is documented, and a configuration listing shows the command
        let mut g = compile(&reference("16.12", &["interface {<type switch>|<slot>}"]));
        let docs: BTreeSet<String> = ["line vty 0 4".into(), "transport input ssh".into()].into();
        let (applied, skipped) = apply_overrides(&mut g, OVERRIDES, &docs).unwrap();
        assert_eq!(applied, ["interface (VLAN)", "transport input"]);
        assert!(skipped.is_empty());
        assert!(!g.commands.iter().any(|c| c.name == "interface (VLAN)"));
        // another release documents the command otherwise and never shows transport input
        let mut g = compile(&reference("16.5", &["interface Vlan <vlan-id>"]));
        let docs: BTreeSet<String> = ["transport inputs".into()].into();
        let (applied, skipped) = apply_overrides(&mut g, OVERRIDES, &docs).unwrap();
        assert!(applied.is_empty(), "{applied:?}");
        assert_eq!(skipped, ["interface (VLAN)", "transport input"]);
        assert!(g.commands.iter().any(|c| c.name == "interface (VLAN)"));
        assert!(!g.commands.iter().any(|c| c.name == "transport input"));
    }

    #[test]
    fn a_patch_takes_modes_from_a_command_as_well_as_giving_them() {
        let mut g = compile(&reference("17.15", &["interface Vlan <vlan-id>"]));
        let patch = r#"{"commands": [{"name": "interface (VLAN)", "modes": ["conf-x"],
            "drop_modes": ["config"], "evidence": "e"}]}"#;
        let (applied, _) = apply_overrides(&mut g, patch, &BTreeSet::new()).unwrap();
        assert_eq!(applied, ["interface (VLAN)"]);
        let c = g
            .commands
            .iter()
            .find(|c| c.name == "interface (VLAN)")
            .unwrap();
        assert_eq!(c.modes, ["conf-x"]);
    }

    #[test]
    fn a_hardware_document_s_command_applies_to_every_release_of_its_platform() {
        let hw = r#"{"commands": [{"name": "enable (uplink)", "platform": "Catalyst 9400",
            "modes": ["config-if"], "lines": ["enable", "no enable"], "evidence": "e"}]}"#;
        let docs = BTreeSet::new();
        let mut g = compile(&reference("16.6", &[]));
        g.platform = "Catalyst 9400".into();
        let (applied, _) = apply_overrides(&mut g, hw, &docs).unwrap();
        assert_eq!(applied, ["enable (uplink)"]);
        assert!(g.commands.iter().any(|c| c.name == "enable (uplink)"));
        // the 9300 books never take it
        let mut g = compile(&reference("17.15", &[]));
        let (applied, skipped) = apply_overrides(&mut g, hw, &docs).unwrap();
        assert!(applied.is_empty() && skipped == ["enable (uplink)"]);
    }

    #[test]
    fn a_keyword_a_later_release_added_is_left_out_of_an_earlier_one() {
        let c = serde_json::json!({"sections": {"Command History": {"rows": [
            ["Release", "Modification"],
            ["12.2(15)T", "The esp keyword was added."],
            ["XE 2.4.2", "The inside and<> outside keywords were added."],
            ["Cisco IOS XE Everest 16.5.1", "This command was modified. The output was updated."],
            ["Cisco IOS XE Gibraltar 16.12.1", "The vrf and ipv6 keywords were added."],
            ["26.1.1", "The interface keyword was added."]
        ]}}});
        let r = |t| Release::parse(t).unwrap();
        assert_eq!(
            later_keywords(&c, r("16.9")),
            ["interface", "ipv6", "vrf"].map(String::from).into()
        );
        assert_eq!(
            later_keywords(&c, r("17.15")),
            ["interface".to_string()].into()
        );
        assert!(later_keywords(&c, r("26.x")).is_empty());
        let later: BTreeSet<String> = ["interface".to_string()].into();
        assert!(names_any(
            "show ip nat translations [ vrf <vrf-name> interface <interface-name> ]",
            &later
        ));
        assert!(!names_any(
            "clear ip nat translation <interface-name>",
            &later
        ));
    }

    #[test]
    fn help_is_each_row_s_first_sentence_for_its_last_keyword_and_its_arguments() {
        let r = serde_json::json!({"chapters": [{"chapter": "c", "commands": [{
            "name": "switchport mode", "id": "a",
            "intro": "To configure the VLAN membership mode of a port, use the switchport mode command in interface configuration mode. More.",
            "syntax": ["switchport mode {access | dynamic {auto | desirable} | trunk} [vlan <vlan-id>]"],
            "sections": {
                "Command Modes": {"text": ["Interface configuration (config-if)"], "rows": [], "pre": []},
                "Syntax Description": {"text": [], "pre": [], "rows": [
                    ["access", "Sets the port to access mode (either static-access or dynamic-access). Second sentence."],
                    ["dynamic auto", "Sets the port trunking mode dynamic parameter to auto."],
                    ["vlan vlan-id", "The VLAN."]
                ]}
            }
        }]}]});
        let g = compile(&r);
        let h = &g.commands[0].help;
        assert_eq!(h["access"], "Sets the port to access mode");
        assert_eq!(
            h["auto"],
            "Sets the port trunking mode dynamic parameter to auto"
        );
        assert!(!h.contains_key("dynamic"), "the row is auto's: {h:?}");
        assert_eq!(h["vlan-id"], "The VLAN");
        assert_eq!(h["vlan"], "The VLAN");
        assert_eq!(h[""], "Configure the VLAN membership mode of a port");
    }

    fn command(name: &str, mode: &str, line: &str, enters: Option<&str>) -> Command {
        let (node, repairs) = syntax::parse(line);
        let mut names = BTreeSet::new();
        args_of(&node, &mut names);
        Command {
            name: name.into(),
            source: "t".into(),
            modes: vec![mode.into()],
            lines: vec![Line {
                text: line.into(),
                negated: false,
                node,
                repairs,
            }],
            args: names.iter().map(|a| (a.clone(), arg_kind(a, ""))).collect(),
            examples: vec![],
            enters: enters.map(Into::into),
            evidence: None,
            help: BTreeMap::new(),
        }
    }

    fn book(commands: Vec<Command>) -> Grammar {
        Grammar {
            release: "12.2(55)SE".into(),
            platform: "Catalyst 2960".into(),
            source: "t".into(),
            commands,
            transitions: vec![],
        }
    }

    #[test]
    fn a_projection_keeps_the_base_commands_its_guides_attest() {
        let mut base = book(vec![
            command("hostname", "config", "hostname <name>", None),
            command(
                "switch provision",
                "config",
                "switch <n> provision <type>",
                None,
            ),
            command("stack-power", "config", "stack-power stack <name>", None),
            command("rep segment", "config-if", "rep segment <id>", None),
        ]);
        base.commands[1].name = "switch".into();
        let guides = serde_json::json!({"platform": "Catalyst IE3300", "books": [{"chapters": [
            {"steps": [{"syntax": "hostname <name>", "examples": []}], "pres": []},
            {"steps": [], "pres": ["Device(config)# interface gigabitethernet1/1\nDevice(config-if)# no rep segment 1 edge"]}
        ]}]});
        let (g, kept, dropped) = project(&base, &guides, &[], &[]);
        let names: Vec<&str> = g.commands.iter().map(|c| c.name.as_str()).collect();
        // `switch` and `stack-power` are the 9300's stack; a `no` line attests its command
        assert_eq!(names, ["hostname", "rep segment"]);
        assert_eq!((kept, dropped), (2, 2));
        assert_eq!(g.platform, "Catalyst IE3300");
        // a capture at the release attests too: a configuration line, a command at a session's
        // prompt, never a session's output
        let (g, _, _) = project(
            &base,
            &guides,
            &["stack-power stack one\n".to_string()],
            &["sw1#switch 1 renumber 2\nswitch output\n".to_string()],
        );
        let names: Vec<&str> = g.commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["hostname", "switch", "stack-power", "rep segment"]);
        let (g, _, _) = project(&base, &guides, &[], &["switch output\n".to_string()]);
        assert_eq!(g.commands.len(), 2);
    }

    #[test]
    fn a_command_a_capture_uses_and_the_book_refuses_comes_from_the_donor() {
        let mut own = book(vec![
            command(
                "interface",
                "config",
                "interface <interface-id>",
                Some("config-if"),
            ),
            command(
                "switchport access",
                "config-if",
                "switchport access vlan <vlan-id>",
                None,
            ),
        ]);
        let donor = book(vec![
            command("hostname", "config", "hostname <name>", None),
            command(
                "switchport access vlan",
                "config-if",
                "switchport access vlan <vlan-id>",
                None,
            ),
            command("line", "config", "line <type> <first>", Some("config-line")),
            command("password", "config-line", "password <level> <secret>", None),
            command("ntp server", "config", "ntp server <address>", None),
        ]);
        let config = "version 12.2\nhostname SW1\ninterface FastEthernet0/1\n switchport access vlan 5\nline con 0\n password 7 x\nend\n";
        // `password` is read once the borrowed `line` opens its block; the book's own
        // `switchport access` takes its line; nothing uses `ntp server`
        let taken = borrow_commands(&mut own, &donor, &[config.to_string()], &[]);
        assert_eq!(taken, ["hostname", "line", "password"]);
        assert!(
            own.commands
                .iter()
                .any(|c| c.name == "line" && c.source.starts_with("Catalyst 2960 12.2(55)SE: "))
        );
        assert!(
            refused_lines(&own, &[config.to_string()])
                .iter()
                .all(|l| l == "version 12.2")
        );
    }

    #[test]
    fn a_patch_scoped_to_a_platform_skips_a_release_without_the_command() {
        let mut g = book(vec![command("hostname", "config", "hostname <name>", None)]);
        let text = r#"{"about": "", "commands": [{"name": "line", "platform": "Catalyst 2960", "enters": "config-line", "lines": [], "evidence": "e"}]}"#;
        let (applied, skipped) = apply_overrides(&mut g, text, &BTreeSet::new()).unwrap();
        assert!(
            applied.is_empty() && skipped == ["line"],
            "{applied:?} {skipped:?}"
        );
    }

    #[test]
    fn a_lost_release_reads_a_later_book_as_its_command_history_dates_it() {
        let history =
            |rows: serde_json::Value| serde_json::json!({"Command History": {"rows": rows}});
        let reference = serde_json::json!({"chapters": [{"commands": [
            {"name": "ip access-group", "syntax": ["ip access-group { <access-list-number> | <name> } { in | out }"],
             "sections": history(serde_json::json!([["Release", "Modification"], ["12.2(53)EZ", "This command was introduced."], ["15.0(2)SE", "The out keyword was added for switches running the IP Services image."]]))},
            {"name": "clear ipv6 dhcp conflict", "syntax": ["clear ipv6 dhcp conflict { * | <address> }"],
             "sections": history(serde_json::json!([["15.0(2) SE", "This command was introduced."]]))},
            {"name": "dot1x supplicant force-multicast", "syntax": ["dot1x supplicant force-multicast"],
             "sections": history(serde_json::json!([["12.2(52)SE", "This command was introduced."], ["12.2(53)EZ", "This command was introduced."]]))},
            {"name": "snmp-server enable traps", "syntax": ["snmp-server enable traps [ alarms | bgp | ospf [ errors | lsa ] | pim [ neighbor-change ] | vtp ]"],
             "sections": history(serde_json::json!([["12.2(53)EZ", "This command was introduced."], ["15.0(2)SE", "The bgp, ospf [errors | lsa], and pim [neighbor-change] keywords were added."]]))},
            {"name": "sdm prefer", "syntax": ["sdm prefer { default | lanbase-routing | dual-ipv4-and-ipv6 {default| routing } | routing }", "no sdm prefer"],
             "sections": history(serde_json::json!([["12.2(53)EZ", "This command was introduced."], ["15.0(2)SE", "The lanbase-routing keyword was added. The routing and dual-ipv4-and-ipv6 routing keywords were added."]]))},
            {"name": "show ipv6 dhcp conflict", "syntax": ["show ipv6 dhcp conflict"],
             "sections": history(serde_json::json!([["15.0(2)", "This command was introduced."]]))},
            // the same major and minor: only the whole train, its space closed, dates it later
            {"name": "show rep topology", "syntax": ["show rep topology"],
             "sections": history(serde_json::json!([["12.2(55) SE", "This command was introduced."]]))},
            {"name": "rep admin vlan", "syntax": ["rep admin vlan <vlan-id>"], "sections": {}}
        ]}]});
        let mut v = reference.clone();
        let (dropped, trimmed) = as_of(&mut v, Release::parse("12.2(53)EZ").unwrap());
        assert_eq!(
            dropped,
            [
                "clear ipv6 dhcp conflict",
                "show ipv6 dhcp conflict",
                "show rep topology"
            ]
        );
        assert_eq!(
            trimmed,
            ["ip access-group", "snmp-server enable traps", "sdm prefer"]
        );
        let syntax = |name: &str| {
            v["chapters"][0]["commands"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == name)
                .map(|c| c["syntax"].clone())
        };
        assert_eq!(
            syntax("ip access-group"),
            Some(serde_json::json!([
                "ip access-group { <access-list-number> | <name> } { in }"
            ]))
        );
        assert_eq!(
            syntax("snmp-server enable traps"),
            Some(serde_json::json!([
                "snmp-server enable traps [ alarms | vtp ]"
            ]))
        );
        assert_eq!(
            syntax("sdm prefer"),
            Some(serde_json::json!([
                "sdm prefer { default }",
                "no sdm prefer"
            ]))
        );
        // a command with no history, and one another platform introduced earlier, stay
        assert!(
            syntax("rep admin vlan").is_some()
                && syntax("dot1x supplicant force-multicast").is_some()
        );
        // the book's own release loses nothing
        let mut own = reference.clone();
        assert_eq!(
            as_of(&mut own, Release::parse("15.0(2)SE").unwrap()),
            (vec![], vec![])
        );
        assert_eq!(own, reference);
    }

    #[test]
    fn a_guide_step_whose_words_ran_together_reads_as_its_example() {
        let mut g = book(vec![command(
            "configure terminal",
            "privileged",
            "configure terminal",
            Some("config"),
        )]);
        let guides = serde_json::json!({"books": [{"chapters": [{"url": "x/macsec.html", "pres": [],
            "steps": [{"syntax": "configureterminal", "examples": ["Device# configure terminal"]}]}]}]});
        add_guides(&mut g, &guides);
        let names: Vec<&str> = g.commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["configure terminal"]);
        let known = |w: &str| ["macsec", "configure"].contains(&w);
        assert_eq!(
            run_together(
                "macsecnetwork-link <x>",
                "macsecnetwork-link",
                &["macsec", "network-link"],
                known
            ),
            Some("macsec network-link <x>".into())
        );
        // a word that is one of the example's is no run
        assert_eq!(
            run_together("configure", "configure", &["configure", "terminal"], known),
            None
        );
        // an example with a stray space does not split a hyphenated keyword its books lack
        assert_eq!(
            run_together(
                "maximum-prefix <n>",
                "maximum-prefix",
                &["maximum-", "prefix", "100"],
                known
            ),
            None
        );
    }

    #[test]
    fn a_submode_named_in_words_takes_the_prompt_its_examples_show() {
        let reference = serde_json::json!({"release": "15.2(2)E", "chapters": [{"chapter": "cli1", "commands": [{
            "name": "class", "id": "a", "syntax": ["class <class-map-name>"],
            "sections": {
                "Command Modes": {"text": ["Policy-map configuration"], "rows": [], "pre": []},
                "Examples": {"text": [], "rows": [], "pre": ["Switch(config)# policy-map policy1\nSwitch(config-pmap)# class class1\nSwitch(config-pmap-c)# set dscp 10"]}
            }
        }, {
            "name": "policy-map", "id": "b", "syntax": ["policy-map <policy-map-name>"],
            "sections": {
                "Command Modes": {"text": ["Global configuration"], "rows": [], "pre": []},
                "Examples": {"text": [], "rows": [], "pre": ["Switch(config)# policy-map policy1\nSwitch(config-pmap)# class class1"]}
            }
        }]}]});
        let g = compile(&reference);
        let modes_of = |n: &str| {
            g.commands
                .iter()
                .find(|c| c.name == n)
                .unwrap()
                .modes
                .clone()
        };
        assert_eq!(modes_of("class"), ["config-pmap"]);
        // a mode the wording resolves takes nothing from the examples
        assert_eq!(modes_of("policy-map"), ["config"]);
        assert!(unnamed_modes("Policy-map class configuration"));
        assert!(unnamed_modes(
            "Global configuration Policy-map configuration"
        ));
        assert!(!unnamed_modes("Interface configuration (config-if)"));
        assert!(!unnamed_modes("Privileged EXEC"));
    }

    #[test]
    fn a_later_keyword_that_is_no_alternative_takes_its_line() {
        let later: BTreeSet<String> = ["out".to_string(), "bar".to_string()].into();
        assert_eq!(without("ip access-group <name> out", 2, &later), None);
        assert_eq!(without("foo { bar }", 1, &later), None);
        assert_eq!(
            without("foo [ bar ] baz", 1, &later),
            Some("foo baz".to_string())
        );
        // the command's own words are no alternative
        assert_eq!(
            without("bar { x | y }", 1, &later),
            Some("bar { x | y }".to_string())
        );
        // the IE 3010 15.0(2)SE book's `snmp-server enable traps` row: seven words back to `The`
        assert_eq!(
            keywords_added(
                "The bgp, hsrp, ipmulticast, msdp, ospf [cisco-specific | errors | lsa | rate-limit | retransmit | state-change], and pim [invalid-pim-message | neighbor-change | rp-mapping-change] keywords were added."
            ),
            ["pim", "ospf", "msdp", "ipmulticast", "hsrp", "bgp"]
        );
    }

    #[test]
    fn a_general_command_its_switches_attest_is_taken_and_never_one_its_books_name() {
        let mut own = book(vec![
            command(
                "interface",
                "config",
                "interface <interface-id>",
                Some("config-if"),
            ),
            command("hostname", "config", "hostname <name>", None),
        ]);
        let mut general = book(vec![
            // a general `interface` reads any word as a name
            command("interface", "config", "interface <name>", Some("config-if")),
            command(
                "service timestamps",
                "config",
                "service timestamps <what>",
                None,
            ),
            command("description", "config-if", "description <text>", None),
            command("ntp master", "config", "ntp master", None),
            command("terminal monitor", "privileged", "terminal monitor", None),
        ]);
        general.platform = "Cisco IOS general references".into();
        let mut general = General::new(general);
        let config = "version 15.2\nhostname SW1\nservice timestamps log\ninterface Bogus7\n description up\ninterface GigabitEthernet0/1\n description uplink\nend\n";
        let session = "SW1#terminal monitor\nSW1#\n".to_string();
        let taken = borrow_general(
            &mut own,
            &mut general,
            &[config.to_string()],
            &[session],
            &[],
        );
        // the book's own `interface` keeps `Bogus7` refused; a description under a port the book takes
        // is read in config-if; nothing uses `ntp master`
        assert_eq!(
            taken,
            ["service timestamps", "description", "terminal monitor"]
        );
        let names: Vec<&str> = own.commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "interface",
                "hostname",
                "service timestamps",
                "description",
                "terminal monitor"
            ]
        );
        assert_eq!(own.commands[2].source, "Cisco IOS general references: t");
    }

    #[test]
    fn a_header_its_books_leave_without_a_mode_enters_the_one_its_lines_read_in() {
        // the platform's guides give `crypto pki trustpoint` with no mode it enters; the general
        // references' one example shows `(ca-trustpoint)#` after it, too few for the engine alone
        let mut own = book(vec![
            command(
                "crypto pki trustpoint",
                "config",
                "crypto pki trustpoint <name>",
                None,
            ),
            command("hostname", "config", "hostname <name>", None),
        ]);
        let mut general = book(vec![
            command(
                "crypto pki trustpoint",
                "config",
                "crypto pki trustpoint <name>",
                None,
            ),
            command(
                "enrollment selfsigned",
                "ca-trustpoint",
                "enrollment selfsigned",
                None,
            ),
            command("ntp master", "config", "ntp master", None),
        ]);
        general.transitions.push(Transition {
            from: "config".into(),
            line: "crypto pki trustpoint tp1".into(),
            to: "ca-trustpoint".into(),
        });
        let mut general = General::new(general);
        let held = "version 17.15\nhostname a\ncrypto pki trustpoint TP-self-signed-1\n enrollment selfsigned\nend\n";
        let taken = borrow_general(&mut own, &mut general, &[held.to_string()], &[], &[]);
        assert_eq!(taken, ["enrollment selfsigned"]);
        assert_eq!(own.commands[0].enters.as_deref(), Some("ca-trustpoint"));
        // with no line under it that reads there, it stays as its books give it
        let mut bare = book(vec![command(
            "crypto pki trustpoint",
            "config",
            "crypto pki trustpoint <name>",
            None,
        )]);
        let held = "version 17.15\ncrypto pki trustpoint TP-self-signed-1\n frobnicate\nend\n";
        borrow_general(&mut bare, &mut general, &[held.to_string()], &[], &[]);
        assert_eq!(bare.commands[0].enters, None);
    }

    #[test]
    fn the_general_references_are_read_as_each_release_has_them() {
        let reference = serde_json::json!({"chapters": [{"commands": [
            {"name": "xe only", "syntax": ["xe only"], "sections": {"Command History": {"rows": [
                ["Release", "Modification"],
                ["Cisco IOS XE Amsterdam 17.1.1", "This command was introduced."]
            ]}}},
            {"name": "ntp peer",
             "syntax": ["ntp peer <ip-address> [ vrf <vrf-name> ] [ burst ] [ key <key-id> ]"],
             "sections": {"Command History": {"rows": [
                ["Release", "Modification"],
                ["12.2(55)SE", "This command was introduced."],
                ["15.2(2)E", "The vrf keyword was added."],
                ["Cisco IOS XE Gibraltar 16.12.1", "The burst keyword was added."],
                ["15.1(1)SY", "The key keyword was added."]
            ]}}}
        ]}]});
        let read = |r: &str| {
            let mut v = reference.clone();
            let out = gate_general(&mut v, Release::parse(r).unwrap());
            (out, v["chapters"][0]["commands"].clone())
        };
        // classic IOS: no IOS XE command, and no keyword an IOS XE row or a later E train adds; an SY
        // row dates nothing
        let ((dropped, trimmed), cmds) = read("15.0(2)SE");
        assert_eq!(dropped, ["xe only"]);
        assert_eq!(trimmed, ["ntp peer"]);
        assert_eq!(
            cmds[0]["syntax"][0],
            "ntp peer <ip-address> [ key <key-id> ]"
        );
        let (_, cmds) = read("15.2(7)E");
        assert_eq!(
            cmds[0]["syntax"][0],
            "ntp peer <ip-address> [ vrf <vrf-name> ] [ key <key-id> ]"
        );
        // IOS XE: a command introduced after the release is left out, and so is a keyword a later
        // IOS XE row adds; a classic row dates nothing
        let ((dropped, _), cmds) = read("16.9");
        assert_eq!(dropped, ["xe only"]);
        assert_eq!(
            cmds[0]["syntax"][0],
            "ntp peer <ip-address> [ vrf <vrf-name> ] [ key <key-id> ]"
        );
        let ((dropped, trimmed), cmds) = read("17.15");
        assert!(dropped.is_empty() && trimmed.is_empty());
        assert_eq!(cmds.as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_classic_train_coded_xe_dates_nothing_and_ios_xe_3_comes_before_16() {
        let introduced = |when: &str| serde_json::json!({"Command History": {"rows": [["Release", "Modification"], [when, "This command was introduced."]]}});
        let reference = serde_json::json!({"chapters": [{"commands": [
            {"name": "transceiver type all", "syntax": ["transceiver type all"], "sections": introduced("12.2(18)SXE")},
            {"name": "show platform condition", "syntax": ["show platform condition"],
             "sections": introduced("Cisco IOS XE 3.10.0S")},
            {"name": "ip nbar protocol-discovery", "syntax": ["ip nbar protocol-discovery [ ipv4 | ipv6 ]"],
             "sections": {"Command History": {"rows": [["Release", "Modification"],
                ["12.0(5)XE2", "This command was introduced."],
                ["12.2(18)SXE", "The ipv4 and ipv6 keywords were added."]]}}}
        ]}]});
        let read = |r: &str| gate_general(&mut reference.clone(), Release::parse(r).unwrap());
        // classic IOS: a train coded SXE or XE2 is classic and dates nothing; IOS XE 3 is IOS XE
        assert_eq!(
            read("15.2(7)E"),
            (vec!["show platform condition".to_string()], vec![])
        );
        // every IOS XE 16, 17 and 26 release follows IOS XE 3
        for r in ["16.9", "17.15"] {
            assert_eq!(read(r), (vec![], vec![]), "{r}");
        }
        assert!(xe_row("XE3.2SP") && xe_row("Cisco IOS XE Release 3.10.0S"));
        assert!(!xe_row("12.2(18)SXE") && !xe_row("12.0(5)XE2"));
    }

    #[test]
    fn a_qualified_base_command_is_attested_by_its_words() {
        let base = book(vec![
            command(
                "license smart (global config)",
                "config",
                "license smart transport {callhome | off | smart}",
                None,
            ),
            command(
                "vtp (global configuration)",
                "config",
                "vtp mode {client | server | transparent}",
                None,
            ),
        ]);
        let guides = serde_json::json!({"platform": "Catalyst IE3300", "books": []});
        let held = "version 17.6\nlicense smart transport off\nend\n".to_string();
        let (g, kept, dropped) = project(&base, &guides, &[held], &[]);
        let names: Vec<&str> = g.commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["license smart (global config)"]);
        assert_eq!((kept, dropped), (1, 1));
    }

    #[test]
    fn a_patch_renames_an_argument_outside_the_no_form() {
        let mut c = command(
            "privilege",
            "config",
            "privilege <mode> level <level> <command>",
            None,
        );
        c.lines.push(syntax_line(
            "no privilege <mode> level <level> <command>",
            None,
        ));
        let mut g = book(vec![c]);
        let patch = r#"{"commands": [{"name": "privilege",
            "args": {"command": {"kind": "Text"}, "privilege-level": {"kind": "Number", "min": 0, "max": 15}},
            "adds": ["privilege <mode> all level <level> <command>"],
            "renames": {"level": "privilege-level"}, "evidence": "e"}]}"#;
        apply_overrides(&mut g, patch, &BTreeSet::new()).unwrap();
        let texts: Vec<&str> = g.commands[0]
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(
            texts,
            [
                "privilege <mode> level <privilege-level> <command>",
                "no privilege <mode> level <level> <command>",
                "privilege <mode> all level <privilege-level> <command>"
            ]
        );
        let e = crate::cli::Engine::new(g);
        let takes = |l: &str| matches!(e.parse("config", l), crate::cli::Outcome::Ok(_));
        assert!(takes("privilege exec level 14 show version"));
        assert!(!takes("privilege exec level 99 show version"));
        assert!(
            !takes("privilege exec all level 99 show"),
            "a line the patch adds"
        );
        assert!(
            takes("no privilege exec level 99 show"),
            "the no form keeps the book's level"
        );
    }

    #[test]
    fn a_patch_reaches_every_command_of_its_name() {
        // the 9300 16.6 grammar holds a guide's `spanning-tree vlan` at config-if and the global one
        let mut g = book(vec![
            command(
                "spanning-tree vlan",
                "config-if",
                "spanning-tree vlan <vlan-id> cost <cost>",
                None,
            ),
            command(
                "spanning-tree vlan",
                "config",
                "spanning-tree vlan <vlan-id> root primary",
                None,
            ),
        ]);
        let patch = r#"{"commands": [{"name": "spanning-tree vlan",
            "args": {"vlan-id": {"kind": "VlanList"}}, "evidence": "e"}]}"#;
        apply_overrides(&mut g, patch, &BTreeSet::new()).unwrap();
        for c in &g.commands {
            assert_eq!(c.args["vlan-id"], ArgKind::VlanList, "{:?}", c.modes);
            assert_eq!(c.evidence.as_deref(), Some("e"));
        }
    }

    #[test]
    fn an_argument_rows_type_apart_after_different_keywords_is_typed_after_each() {
        let r = serde_json::json!({"release": "17.15", "platform": "Catalyst 9300", "chapters": [{"chapter": "layer_2_3_commands", "commands": [{
            "name": "spanning-tree vlan", "id": "a",
            "syntax": ["spanning-tree vlan <vlan-id> [ forward-time <seconds> | hello-time <seconds> | max-age <seconds> | priority <priority> | root [ primary | secondary ] ]"],
            "sections": {
                "Command Modes": {"text": ["Global configuration (config)"]},
                "Syntax Description": {"rows": [
                    ["forward-time <seconds>", "(Optional) Sets the STP forward delay time. The range is from 4 to 30 seconds."],
                    ["hello-time <seconds>", "(Optional) Specifies the duration, in seconds, between the generation of configuration messages by the root switch. The range is from 1 to 10 seconds."],
                    ["max-age <seconds>", "(Optional) Sets the maximum number of seconds the information in a bridge packet data unit (BPDU) is valid. the range is from 6 to 40 seconds."]
                ]}
            }
        }]}]});
        let e = crate::cli::Engine::new(compile(&r));
        let takes = |l: &str| matches!(e.parse("config", l), crate::cli::Outcome::Ok(_));
        assert!(
            takes("spanning-tree vlan 10 hello-time 2"),
            "the IOS default"
        );
        assert!(takes("spanning-tree vlan 10 forward-time 4"));
        assert!(takes("spanning-tree vlan 10 max-age 40"));
        assert!(!takes("spanning-tree vlan 10 hello-time 40"));
        assert!(!takes("spanning-tree vlan 10 forward-time 3"));
        // `?` after the keyword describes its own argument
        let (next, _) = e.next(
            "config",
            &crate::cli::tokens("spanning-tree vlan 10 hello-time "),
        );
        let offered: Vec<(ArgKind, Option<String>)> = next
            .into_iter()
            .filter_map(|n| match n {
                crate::cli::Next::Arg { kind, help, .. } => Some((kind, help)),
                _ => None,
            })
            .collect();
        assert_eq!(
            offered,
            [(
                ArgKind::Number { min: 1, max: 10 },
                Some("Specifies the duration, in seconds, between the generation of configuration messages by the root switch".to_string())
            )]
        );
    }

    #[test]
    fn an_interface_s_type_and_number_are_one_argument() {
        let r = serde_json::json!({"release": "17.15", "platform": "Catalyst 9300", "chapters": [{"chapter": "c", "commands": [{
            "name": "show spanning-tree", "id": "a",
            "syntax": ["show spanning-tree [ active | detail | interface <interface-type> <interface-number> [portfast [edge]] | vlan <vlan-id> ]"],
            "sections": {"Command Modes": {"text": ["Privileged EXEC (#)"]}}
        }, {
            "name": "ip pim send-rp-announce", "id": "b",
            "syntax": ["ip pim send-rp-announce {<interface-type> <interface-number> | <ip-address>} scope <ttl-value>"],
            "sections": {"Command Modes": {"text": ["Global configuration (config)"]}}
        }]}]});
        let e = crate::cli::Engine::new(compile(&r));
        let takes = |m: &str, l: &str| matches!(e.parse(m, l), crate::cli::Outcome::Ok(_));
        assert!(takes(
            "privileged",
            "show spanning-tree interface GigabitEthernet1/0/1"
        ));
        assert!(takes(
            "privileged",
            "show spanning-tree interface GigabitEthernet 1/0/1 portfast"
        ));
        assert!(takes(
            "config",
            "ip pim send-rp-announce Loopback0 scope 16"
        ));
        // a name that only begins with `interface` names something else
        assert_eq!(arg_kind("interface-priority", ""), ArgKind::Word);
        assert_eq!(
            arg_kind("interface-bandwidth", "The range is from 1 to 10000000."),
            ArgKind::Number {
                min: 1,
                max: 10000000
            }
        );
        assert_eq!(arg_kind("source-interface", ""), ArgKind::Interface);
    }

    #[test]
    fn an_argument_is_what_its_last_words_name() {
        let k = arg_kind;
        assert_eq!(k("list", ""), ArgKind::Word);
        assert_eq!(
            k(
                "list",
                "(Optional) Specifies a multiple spanning-tree instance list."
            ),
            ArgKind::VlanList
        );
        assert_eq!(k("context-name", "Name of the context."), ArgKind::Word);
        assert_eq!(
            k(
                "hash-mask-length",
                "(Optional) Length of a mask (32 bits maximum)."
            ),
            ArgKind::Word
        );
        assert_eq!(
            k(
                "ipv6-access-list-name",
                "The IPv6 access list to be matched."
            ),
            ArgKind::Word
        );
        assert_eq!(k("ipv6-address", ""), ArgKind::Ipv6);
        assert_eq!(k("subnet-mask", ""), ArgKind::Ipv4);
        assert_eq!(
            k(
                "description",
                "(Optional) Descriptive name of the prefix list, from 1 to 80 characters in length."
            ),
            ArgKind::Text
        );
        // a string's length limit is no range of numbers; a value range after one still counts
        assert_eq!(
            k(
                "password",
                "Password for the VTP domain, an ASCII string from 1 to 32 characters."
            ),
            ArgKind::Word
        );
        assert_eq!(
            k("domain-name", "A name from 1-32 alphanumeric characters."),
            ArgKind::Word
        );
        assert_eq!(
            k("priority", "Up to 8 characters; the priority is 0 to 255."),
            ArgKind::Number { min: 0, max: 255 }
        );
        // underscores read as hyphens, the guides' soft hyphens as nothing
        assert_eq!(k("ip_address", ""), ArgKind::Ipv4);
        assert_eq!(k("vlan_ID", ""), ArgKind::Number { min: 1, max: 4094 });
        assert_eq!(k("interface-\u{ad}name", ""), ArgKind::Interface);
        // a split mask is its owner's kind
        assert_eq!(k("src-MAC-addr mask", ""), ArgKind::Mac);
        assert_eq!(k("ip-address mask", ""), ArgKind::Ipv4);
        assert_eq!(k("lsap mask", ""), ArgKind::Word);
        // one value written in parts
        assert_eq!(
            k(
                "network/length",
                "The bit mask can be a number from 1 to 32."
            ),
            ArgKind::Word
        );
        assert_eq!(k("ipv6-prefix/prefix-length", ""), ArgKind::Ipv6);
        assert_eq!(k("type slot/port", ""), ArgKind::Interface);
        // in a command, a split mask takes its owner's kind and the help of the book's mask row
        let names: BTreeSet<String> = [
            "src-MAC-addr",
            "src-MAC-addr mask",
            "type",
            "type mask",
            "ip-address",
            "ip-address mask",
        ]
        .map(String::from)
        .into();
        let rows = vec![
            vec![
                "host <src-MAC-addr> <src-MAC-addr mask>",
                "Defines a host MAC address and optional subnet mask.",
            ],
            vec![
                "<type mask>",
                "(Optional) Specifies the EtherType number. The type is 0 to 65535.",
            ],
            vec!["<mask>", "The address's mask."],
        ];
        let args = kinds_of(&names, &rows, &BTreeSet::new());
        assert_eq!(args["src-MAC-addr mask"], ArgKind::Mac);
        assert_eq!(
            args["type mask"],
            ArgKind::Word,
            "the type's, not its row's range"
        );
        let help = help_of(&names, &rows, &BTreeSet::new(), &args);
        assert_eq!(help["ip-address mask"], "The address's mask");
    }

    #[test]
    fn two_parts_written_as_one_value_are_one_argument() {
        let r = serde_json::json!({"release": "17.15", "platform": "Catalyst 9300", "chapters": [{"chapter": "c", "commands": [{
            "name": "ip prefix-list", "id": "a",
            "syntax": ["ip prefix-list {<list-name> [seq <number>] {deny | permit} <network>/<length> [ge <ge-length>] [le <le-length>] | description <description> | sequence-number}"],
            "sections": {
                "Command Modes": {"text": ["Global configuration (config)"]},
                "Syntax Description": {"rows": [
                    ["<number>", "(Optional) Integer from 1 to 4294967294."],
                    ["<network> / <length>", "Configures the network address and the length of the network mask in bits. The bit mask can be a number from 1 to 32."],
                    ["<description>", "(Optional) Descriptive name of the prefix list, from 1 to 80 characters in length."]
                ]}
            }
        }, {
            "name": "deny (IPv6)", "id": "b",
            "syntax": ["deny ipv6 <source-ipv6-prefix>/<prefix-length> any"],
            "sections": {"Command Modes": {"text": ["IPv6 access list configuration (config-ipv6-acl)"]}}
        }, {
            "name": "clock set", "id": "c",
            "syntax": ["clock set <hh> : <mm> : <ss> <day> <month> <year>"],
            "sections": {"Command Modes": {"text": ["Privileged EXEC (#)"]}}
        }]}]});
        let e = crate::cli::Engine::new(compile(&r));
        let takes = |m: &str, l: &str| matches!(e.parse(m, l), crate::cli::Outcome::Ok(_));
        assert!(takes("config", "ip prefix-list PL1 deny 10.0.0.0/8"));
        assert!(takes(
            "config",
            "ip prefix-list PL1 seq 5 permit 10.0.0.0/8 le 24"
        ));
        assert!(takes(
            "config",
            "ip prefix-list description routes to the core"
        ));
        assert!(takes("config-ipv6-acl", "deny ipv6 2001:db8::/32 any"));
        assert!(!takes("config-ipv6-acl", "deny ipv6 10.0.0.0/8 any"));
        assert!(takes("privileged", "clock set 10:30:00 1 Jan 2026"));
    }

    #[test]
    fn a_guide_step_with_a_digit_in_a_keyword_is_a_command_and_one_naming_a_value_is_not() {
        let mut g = book(vec![command(
            "interface",
            "config",
            "interface <interface-id>",
            Some("config-if"),
        )]);
        let step = |syntax: &str, example: &str| serde_json::json!({"syntax": syntax, "examples": [example]});
        let guides = serde_json::json!({"books": [{"chapters": [{"url": "x/cdp.html", "pres": [], "steps": [
            step("cdp advertise-v2", "Device(config)# cdp advertise-v2"),
            step("ipv6 unicast-routing", "Device(config)# ipv6 unicast-routing"),
            step("address-family ipv4 [unicast | multicast | vrf <vrf-name>]", "Device(config-router)# address-family ipv4 unicast"),
            step("interface gigabitethernet0/0", "Device(config)# interface gigabitethernet0/0"),
            step("interface vlan10", "Device(config)# interface vlan10"),
            step("line vty 0 98", "Device(config)# line vty 0 98"),
            step("ip address 192.168.20.1 255.255.255.0", "Device(config-if)# ip address 192.168.20.1 255.255.255.0"),
            step("dns-server 2001:100:0:1::1", "Device(config-dhcpv6)# dns-server 2001:100:0:1::1")
        ]}]}]});
        add_guides(&mut g, &guides);
        let names: Vec<&str> = g.commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "interface",
                "cdp advertise-v2",
                "ipv6 unicast-routing",
                "address-family ipv4"
            ]
        );
        // an instance is an example of its command
        assert_eq!(g.commands[0].examples.len(), 2);
        assert_eq!(g.commands[3].modes, ["config-router"]);
    }

    #[test]
    fn a_line_beginning_with_another_word_goes_where_the_book_types_it() {
        let ex = |pre: &str| serde_json::json!({"text": [], "rows": [], "pre": [pre]});
        let r = serde_json::json!({"release": "17.15", "platform": "Catalyst 9300", "chapters": [{"chapter": "security_commands", "commands": [{
            "name": "device-tracking (interface config)", "id": "a",
            "syntax": [
                "device-tracking attach-policy <policy-name> vlan { <vlan-id> | add <vlan-id> | all | except <vlan-id> | none | remove <vlan-id> }",
                "no device-tracking attach-policy <policy-name> vlan { <vlan-id> | add <vlan-id> | all | except <vlan-id> | none | remove <vlan-id> }",
                "attach-policy <policy-name>",
                "vlan { <vlan-id> | add <vlan-id> | all | except <vlan-id> | none | remove <vlan-id> }"
            ],
            "sections": {"Command Modes": {"text": ["Interface configuration [Device((config-if)# )]"]},
                "Examples": ex("Device(config)# interface tengigabitethernet1/0/1\nDevice(config-if)# no device-tracking attach-policy sisf-01\nDevice(config-if)# end")}
        }, {
            "name": "radius server", "id": "b",
            "syntax": ["radius server <name>", "key <string>", "automate tester <name>", "no radius server <name>"],
            "sections": {"Command Modes": {"text": ["Global configuration (config)"]},
                "Examples": ex("Device(config)# radius server ISE\nDevice(config-radius-server)# key INVENTED-KEY\nDevice(config-radius-server)# end")}
        }, {
            "name": "vrrp description", "id": "c",
            "syntax": ["description <text>", "no description"],
            "sections": {"Command Modes": {"text": ["VRRP configuration (config-if-vrrp)"]},
                "Examples": ex("Device(config-if-vrrp)# description Building A")}
        }, {
            "name": "tlv-type (device classifier condition)", "id": "d",
            "syntax": ["{cdp|dhcp|lldp}tlv-type <number> value{integer <num> | string <name>}"],
            "sections": {"Command Modes": {"text": ["Device classifier condition"]},
                "Examples": ex("Device(config-device-classifier-condition)# cdp tlv-type 1 value string TS01")}
        }]}]});
        let e = crate::cli::Engine::new(compile(&r));
        let reads = |m: &str, l: &str| match e.parse(m, l) {
            crate::cli::Outcome::Ok(p) => Some(e.grammar.commands[p.command].name.clone()),
            _ => None,
        };
        // `vlan 30` at an interface is no device-tracking line: global configuration takes it
        assert_eq!(reads("config-if", "vlan 30"), None);
        assert_eq!(reads("config-if", "attach-policy p1"), None);
        assert_eq!(
            reads("config-if", "device-tracking attach-policy p1 vlan 30").as_deref(),
            Some("device-tracking (interface config)")
        );
        // the book types `key` under the server it opens, and never `automate tester`
        assert_eq!(reads("config", "key INVENTED-KEY"), None);
        assert_eq!(
            reads("config-radius-server", "key INVENTED-KEY").as_deref(),
            Some("key (radius server)")
        );
        assert_eq!(reads("config-radius-server", "automate tester bob"), None);
        // a submode's command named after its feature keeps its lines there, and one the book gives
        // no mode takes its examples'
        assert_eq!(
            reads("config-if-vrrp", "description Building A").as_deref(),
            Some("vrrp description")
        );
        assert_eq!(
            reads(
                "config-device-classifier-condition",
                "cdp tlv-type 1 value string TS01"
            )
            .as_deref(),
            Some("tlv-type (device classifier condition)")
        );
    }

    #[test]
    fn a_line_that_starts_with_a_choice_of_keywords_is_kept() {
        let mut g = book(vec![
            command(
                "tlv-type",
                "config-device-classifier-condition",
                "{cdp | dhcp | lldp} tlv-type <number>",
                None,
            ),
            command(
                "ip route static bfd",
                "config",
                "<multihop-destination-address> <multihop-source-address>",
                None,
            ),
        ]);
        // a submode's free text, as an override writes it
        let mut data = command(
            "key data (ssh pubkey-chain)",
            "conf-ssh-pubkey-data",
            "<key-text>",
            None,
        );
        data.source = "data/overrides.json".into();
        g.commands.push(data);
        assert_eq!(prune(&mut g), 1);
        let names: Vec<&str> = g.commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["tlv-type", "key data (ssh pubkey-chain)"]);
        // an access list entry's leading sequence number is left off, the entry kept
        let mut acl = Grammar {
            release: "15.0(2)SE".into(),
            platform: "t".into(),
            source: "t".into(),
            transitions: vec![],
            commands: vec![command(
                "permit (IP)",
                "config-std-nacl",
                "<[sequence-number]> permit <source> <[source-wildcard]>",
                None,
            )],
        };
        assert_eq!(prune(&mut acl), 0);
        let e = crate::cli::Engine::new(acl);
        assert!(matches!(
            e.parse("config-std-nacl", "permit 10.0.0.9"),
            crate::cli::Outcome::Ok(_)
        ));
    }

    #[test]
    fn a_mode_s_parent_is_never_taken_from_a_misprinted_example() {
        let ex = |pre: &str| serde_json::json!({"text": [], "rows": [], "pre": [pre]});
        let r = serde_json::json!({"release": "17.15", "chapters": [{"chapter": "c", "commands": [{
            "name": "accept-lifetime", "id": "a", "syntax": ["accept-lifetime <start-time> infinite"],
            "sections": {"Command Modes": {"text": ["Key chain key configuration (config-keychain-key)"]},
                "Examples": ex("Device(config-keychain-key)# accept-lifetime 13:30:00 infinite\nDevice(config-keychain)# accept-lifetime 14:30:00 infinite")}
        }, {
            "name": "power-priority", "id": "b", "syntax": ["power-priority high <value>"],
            "sections": {"Command Modes": {"text": ["Switch stack-power configuration (config-stack)"]},
                "Examples": ex("Device(config-switch-stackpower)# power-priority high 11")}
        }]}]});
        let g = compile(&r);
        let modes_of = |n: &str| {
            g.commands
                .iter()
                .find(|c| c.name == n)
                .unwrap()
                .modes
                .clone()
        };
        assert_eq!(modes_of("accept-lifetime"), ["config-keychain-key"]);
        // a misprinted id, where the examples show the mode IOS prompts
        assert_eq!(
            modes_of("power-priority"),
            ["config-stack", "config-switch-stackpower"]
        );
    }

    #[test]
    fn a_session_s_configuration_lines_are_borrowed_in_their_modes() {
        let own = || {
            book(vec![
                command(
                    "configure terminal",
                    "privileged",
                    "configure terminal",
                    Some("config"),
                ),
                command("end", "config", "end", Some("privileged")),
                command("exit", "config-ext-nacl", "exit", None),
            ])
        };
        // the lab 2960G's access-list session at 15.0(2)SE, shortened
        let session = "C2960G#configure terminal\n\
            Enter configuration commands, one per line.  End with CNTL/Z.\n\
            C2960G(config)#ip access-list extended VSWITCH-TEST\n\
            C2960G(config-ext-nacl)#15 permit ip host 10.0.0.3 any\n\
            C2960G(config-ext-nacl)#10 permit ip host 10.0.0.5 any\n\
            % Duplicate sequence number.\n\
            C2960G(config-ext-nacl)#no 15\n\
            C2960G(config-ext-nacl)#no permit ip host 10.0.0.4 any\n\
            C2960G(config-ext-nacl)#remark a note\n\
            C2960G(config-ext-nacl)#do show ip access-lists VSWITCH-TEST\n\
            Extended IP access list VSWITCH-TEST\n\
            C2960G(config-ext-nacl)#exit\n\
            C2960G(config)#ntp master\n\
            % Invalid input detected at '^' marker.\n\
            C2960G(config)#end\n\
            C2960G#\n"
            .to_string();
        let pair = |m: &str, l: &str| (m.to_string(), l.to_string());
        assert_eq!(
            refused_commands(&own(), std::slice::from_ref(&session)),
            [
                pair("config", "ip access-list extended vswitch-test"),
                pair("config-ext-nacl", "permit ip host 10.0.0.3 any"),
                pair("config-ext-nacl", "permit ip host 10.0.0.4 any"),
                pair("config-ext-nacl", "remark a note"),
                pair("privileged", "show ip access-lists vswitch-test"),
            ]
        );
        // each in its mode from the general references; the line answered with `%` takes nothing
        let mut general = book(vec![
            command(
                "ip access-list extended",
                "config",
                "ip access-list extended <name>",
                Some("config-ext-nacl"),
            ),
            command(
                "permit",
                "config-ext-nacl",
                "permit ip host <source> any",
                None,
            ),
            command("remark", "config-ext-nacl", "remark <text>", None),
            command(
                "show ip access-lists",
                "privileged",
                "show ip access-lists [<name>]",
                None,
            ),
            command("ntp master", "config", "ntp master", None),
        ]);
        general.platform = "Cisco IOS general references".into();
        let mut general = General::new(general);
        let mut g = own();
        let taken = borrow_general(
            &mut g,
            &mut general,
            &[],
            std::slice::from_ref(&session),
            &[],
        );
        assert_eq!(
            taken,
            [
                "ip access-list extended",
                "permit",
                "remark",
                "show ip access-lists"
            ]
        );
        let permit = g.commands.iter().find(|c| c.name == "permit").unwrap();
        assert_eq!(permit.modes, ["config-ext-nacl"]);
        // the donor path matches a configuration line by name, as it does a capture's
        let donor = book(vec![
            command(
                "ip access-list",
                "config",
                "ip access-list extended <name>",
                Some("config-ext-nacl"),
            ),
            command(
                "permit",
                "config-ext-nacl",
                "permit ip host <source> any",
                None,
            ),
            command("ntp master", "config", "ntp master", None),
        ]);
        let taken = borrow_commands(&mut own(), &donor, &[], &[session]);
        assert_eq!(taken, ["ip access-list", "permit"]);
    }

    #[test]
    fn a_general_command_runs_only_where_the_books_have_none_of_its_name() {
        let mut own = book(vec![
            command(
                "channel-group",
                "config-if",
                "channel-group <n> mode on",
                None,
            ),
            command(
                "authentication periodic",
                "config-if",
                "authentication periodic",
                None,
            ),
        ]);
        let mut general = book(vec![
            command(
                "channel-group (interface)",
                "config-if",
                "channel-group <n> mode {active | on}",
                None,
            ),
            command(
                "authentication periodic",
                "config-if",
                "authentication periodic",
                None,
            ),
        ]);
        general.commands[1].modes.push("config-template".into());
        general.platform = "Cisco IOS general references".into();
        let mut general = General::new(general);
        // an interface range runs the books' own channel-group; a template has no command of its own
        let guided = [
            pair("config-if-range", "channel-group 5 mode active"),
            pair("config-template", "authentication periodic"),
        ];
        let taken = borrow_general(&mut own, &mut general, &[], &[], &guided);
        assert_eq!(taken, ["authentication periodic"]);
        let borrowed = own.commands.last().unwrap();
        assert_eq!(borrowed.modes, ["config-template"]);
    }

    fn pair(m: &str, l: &str) -> (String, String) {
        (m.to_string(), l.to_string())
    }

    /// The command a line reads as in a mode, by name; None where it is refused.
    fn reads(e: &crate::cli::Engine, mode: &str, line: &str) -> Option<String> {
        match e.parse(mode, line) {
            crate::cli::Outcome::Ok(p) => Some(e.grammar.commands[p.command].name.clone()),
            _ => None,
        }
    }

    /// data/overrides.json applied to a 9300 book at a release whose sources show the lines in `docs`.
    fn overridden(release: &str, commands: Vec<Command>, docs: &[&str]) -> crate::cli::Engine {
        let mut g = book(commands);
        g.platform = "Catalyst 9300".into();
        g.release = release.into();
        let docs: BTreeSet<String> = docs.iter().map(|d| d.to_string()).collect();
        apply_overrides(&mut g, include_str!("../data/overrides.json"), &docs).unwrap();
        prune(&mut g);
        crate::cli::Engine::new(g)
    }

    /// A command of several syntax lines, `no` lines among them.
    fn command_lines(name: &str, mode: &str, lines: &[&str]) -> Command {
        let mut c = command(name, mode, lines[0], None);
        c.lines = lines.iter().map(|l| syntax_line(l, None)).collect();
        for l in &c.lines {
            let mut names = BTreeSet::new();
            args_of(&l.node, &mut names);
            for a in names {
                c.args.entry(a.clone()).or_insert_with(|| arg_kind(&a, ""));
            }
        }
        c
    }

    #[test]
    fn transport_input_takes_none_and_both_protocols() {
        let e = overridden("17.15", vec![], &["transport input ssh"]);
        for l in [
            "transport input none",
            "transport input telnet ssh",
            "transport input ssh telnet",
            "transport input all",
            "no transport input",
        ] {
            assert_eq!(
                reads(&e, "config-line", l).as_deref(),
                Some("transport input"),
                "{l}"
            );
        }
        assert_eq!(reads(&e, "config-line", "transport input bogus"), None);
    }

    #[test]
    fn classic_ios_takes_no_ios_xe_form_of_an_override() {
        let docs = ["username a password 7 b", "ntp server 10.0.0.1"];
        let classic = overridden("12.2(44)SE", vec![], &docs);
        let xe = overridden("17.15", vec![], &docs);
        for (l, at_classic) in [
            ("username bob algorithm-type scrypt secret INVENTED", false),
            ("ntp server vrf Mgmt-vrf 10.0.0.1", false),
            ("username amy secret 5 $1$INVENTED", true),
            ("username bob privilege 15 password 7 INVENTED", true),
            ("ntp server 10.0.0.1 prefer", true),
        ] {
            assert_eq!(reads(&classic, "config", l).is_some(), at_classic, "{l}");
            assert!(reads(&xe, "config", l).is_some(), "{l}");
        }
    }

    #[test]
    fn the_ssh_pubkey_data_mode_takes_the_key_s_lines() {
        let chain = || {
            vec![command(
                "ip ssh pubkey-chain",
                "config",
                "ip ssh pubkey-chain",
                None,
            )]
        };
        let e = overridden("17.15", chain(), &["key-string", "username user1", "exit"]);
        assert_eq!(
            reads(
                &e,
                "conf-ssh-pubkey-data",
                "AAAAB3NzaC1yc2EAAAADAQABAAABAQC"
            )
            .as_deref(),
            Some("key data (ssh pubkey-chain)")
        );
        assert_eq!(
            reads(&e, "conf-ssh-pubkey-data", "exit").as_deref(),
            Some("exit")
        );
        // nowhere else, and not where nothing enters the mode
        assert_eq!(reads(&e, "conf-ssh-pubkey-user", "AAAAB3Nza"), None);
        let mut g = book(chain());
        g.release = "17.15".into();
        let (_, skipped) = apply_overrides(
            &mut g,
            include_str!("../data/overrides.json"),
            &BTreeSet::new(),
        )
        .unwrap();
        assert!(skipped.iter().any(|n| n == "key data (ssh pubkey-chain)"));
    }

    #[test]
    fn show_inventory_takes_an_entity_name() {
        let e = overridden("17.15", vec![], &["show inventory"]);
        for l in [
            "show inventory",
            "show inventory \"c93xx Stack\"",
            "show inventory raw chassis",
        ] {
            assert!(reads(&e, "privileged", l).is_some(), "{l}");
        }
        let (next, _) = e.next("privileged", &crate::cli::tokens("show inventory "));
        assert!(
            !next
                .iter()
                .any(|n| matches!(n, crate::cli::Next::Kw { kw, .. } if kw == "entity"))
        );
    }

    #[test]
    fn a_password_runs_to_the_end_of_its_line() {
        let e = overridden(
            "17.15",
            vec![],
            &["password 7 INVENTED", "username a password 0 b"],
        );
        assert_eq!(
            reads(&e, "config-line", "password my secret pw").as_deref(),
            Some("password")
        );
        assert_eq!(
            reads(&e, "config", "username bob password my pass").as_deref(),
            Some("username (privilege and secret)")
        );
    }

    #[test]
    fn a_name_server_is_an_ipv4_or_ipv6_address() {
        let e = overridden("17.15", vec![], &["ip name-server 10.0.0.1"]);
        for l in [
            "ip name-server 2001:db8::53",
            "ip name-server 10.0.0.1 2001:db8::1",
            "ip name-server vrf Mgmt-vrf 10.0.0.1",
        ] {
            assert!(reads(&e, "config", l).is_some(), "{l}");
        }
        assert_eq!(reads(&e, "config", "ip name-server bogus"), None);
    }

    #[test]
    fn a_key_chain_lifetime_takes_its_date_and_time() {
        let e = overridden(
            "17.15",
            vec![],
            &[
                "accept-lifetime 13:30:00 jan 25 1996 duration 7200",
                "send-lifetime 14:00:00 jan 25 1996 duration 3600",
            ],
        );
        for l in [
            "accept-lifetime 13:30:00 Jan 25 1996 duration 7200",
            "accept-lifetime local 12:30:00 25 Jan 2026 infinite",
            "send-lifetime 14:00:00 Jan 25 1996 15:00:00 Jan 26 1996",
            "no accept-lifetime",
        ] {
            assert!(reads(&e, "config-keychain-key", l).is_some(), "{l}");
        }
        assert_eq!(
            reads(
                &e,
                "config-keychain-key",
                "accept-lifetime 12:30:00 infinite"
            ),
            None
        );
    }

    #[test]
    fn a_no_form_the_books_state_only_in_full_takes_its_short_form() {
        let e = overridden(
            "17.15",
            vec![
                command_lines(
                    "access-list",
                    "config",
                    &["access-list <access-list-number> permit <source>"],
                ),
                command_lines(
                    "ip route",
                    "config",
                    &[
                        "ip route <prefix> <mask> <ip-address>",
                        "no ip route <prefix> <mask> <ip-address>",
                    ],
                ),
            ],
            &[],
        );
        assert_eq!(
            reads(&e, "config", "no access-list 103").as_deref(),
            Some("access-list")
        );
        assert_eq!(
            reads(&e, "config", "no ip route 10.0.0.0 255.0.0.0").as_deref(),
            Some("ip route")
        );
        assert_eq!(
            reads(&e, "config", "no ip route 10.0.0.0 255.0.0.0 10.1.1.1").as_deref(),
            Some("ip route")
        );
    }

    #[test]
    fn logging_console_alone_is_the_console_keyword() {
        let e = overridden(
            "17.15",
            vec![
                command("logging", "config", "logging <host>", None),
                command("logging console", "config", "logging console <level>", None),
            ],
            &["logging console 3"],
        );
        assert_eq!(
            reads(&e, "config", "logging console").as_deref(),
            Some("logging console")
        );
        assert_eq!(
            reads(&e, "config", "no logging console").as_deref(),
            Some("logging console")
        );
        assert_eq!(
            reads(&e, "config", "logging 10.0.0.5").as_deref(),
            Some("logging")
        );
    }

    #[test]
    fn the_classic_access_list_overrides_read_the_lab_2960g_s_lines() {
        let docs: BTreeSet<String> = [
            "ip access-list extended x",
            "ip access-list standard x",
            "ip access-list resequence x 10 10",
            "permit ip any any",
            "deny ip any any",
            "remark x",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let text = std::fs::read_to_string("data/overrides.json").unwrap();
        let ours = [
            "ip access-list extended",
            "ip access-list standard",
            "ip access-list resequence",
            "permit (IP extended)",
            "deny (IP extended)",
            "permit (IP standard)",
            "deny (IP standard)",
            "remark (IP access list)",
        ];
        let empty = |release: &str| Grammar {
            release: release.into(),
            platform: "Catalyst 2960".into(),
            source: "t".into(),
            transitions: vec![],
            commands: vec![],
        };
        // an IOS XE release keeps its books' own commands, and takes `remark` in either list
        let mut xe = empty("17.15");
        let (applied, _) = apply_overrides(&mut xe, &text, &docs).unwrap();
        let taken: Vec<&str> = ours
            .iter()
            .copied()
            .filter(|n| applied.iter().any(|a| a == n))
            .collect();
        assert_eq!(taken, ["remark (IP access list)"]);
        let e = crate::cli::Engine::new(xe);
        assert!(matches!(
            e.parse("config-std-nacl", "remark a note"),
            crate::cli::Outcome::Ok(_)
        ));
        // a classic release whose books show each command, as the 2960 guides do
        let mut g = empty("15.0(2)SE");
        let (applied, _) = apply_overrides(&mut g, &text, &docs).unwrap();
        for n in ours {
            assert!(applied.iter().any(|a| a == n), "{n}: {applied:?}");
        }
        let e = crate::cli::Engine::new(g);
        let reads =
            |mode: &str, line: &str| matches!(e.parse(mode, line), crate::cli::Outcome::Ok(_));
        let enters = |line: &str| match e.parse("config", line) {
            crate::cli::Outcome::Ok(p) => e.enters(&p).map(str::to_string),
            _ => None,
        };
        assert_eq!(
            enters("ip access-list extended VSWITCH-TEST").as_deref(),
            Some("config-ext-nacl")
        );
        assert_eq!(
            enters("ip access-list standard VSWITCH-STD").as_deref(),
            Some("config-std-nacl")
        );
        assert!(reads(
            "config",
            "ip access-list resequence VSWITCH-TEST 100 5"
        ));
        // the lab 2960G's entries, and the forms the reference gives beside them
        for line in [
            "permit ip host 10.0.0.1 any",
            "permit ip 10.0.0.0 0.0.0.255 host 10.1.1.1",
            "permit tcp any any eq 22",
            "permit tcp 10.0.0.0 0.0.0.255 gt 1023 host 10.1.1.1 range 1000 2000 established log",
            "permit udp host 10.0.0.1 eq domain any",
            "permit icmp any any echo",
            "permit icmp any any 8 0",
            "permit 89 any any",
            "deny ip any any log",
            "remark a note",
        ] {
            assert!(reads("config-ext-nacl", line), "{line}");
        }
        for line in [
            "permit 10.0.0.9",
            "permit 10.1.0.0 0.0.255.255",
            "permit host 10.0.0.7",
            "deny any",
            "deny 10.0.0.0 0.255.255.255 log",
            "remark a note",
        ] {
            assert!(reads("config-std-nacl", line), "{line}");
        }
        // each list takes its own kind of entry
        assert!(!reads("config-std-nacl", "permit ip any any"));
        assert!(!reads("config-ext-nacl", "permit 10.0.0.9"));
    }

    #[test]
    fn a_capture_s_numbered_access_list_entries_are_read_without_their_numbers() {
        let g = Grammar {
            release: "17.15".into(),
            platform: "t".into(),
            source: "t".into(),
            transitions: vec![],
            commands: vec![
                command(
                    "ip access-list standard",
                    "config",
                    "ip access-list standard <name>",
                    Some("config-std-nacl"),
                ),
                command("permit", "config-std-nacl", "permit <address>", None),
            ],
        };
        let e = crate::cli::Engine::new(g);
        // as IOS XE prints a standard list in its running configuration
        let cfg =
            "version 17.15\nip access-list standard X\n 10 permit 10.0.0.1\n 20 remark the servers\nend\n"
                .to_string();
        let refused: Vec<(String, String)> = refused_in_modes(&e, std::slice::from_ref(&cfg))
            .into_iter()
            .filter(|(m, _)| m == "config-std-nacl")
            .collect();
        assert_eq!(
            refused,
            [(
                "config-std-nacl".to_string(),
                "remark the servers".to_string()
            )]
        );
    }
}
