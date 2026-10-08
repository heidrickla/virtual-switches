//! A session on the switch: the mode stack and its prompt, a line typed at the prompt, and the
//! output IOS prints for it.

use crate::cli::{self, Engine, Item, Next, Outcome, Parse};
use crate::compile::ArgKind;
use crate::device::{Device, Entry};
use crate::show;
use std::collections::{BTreeMap, BTreeSet};

/// What `copy running-config startup-config` asks before it saves (IOS XE command reference example).
const DESTINATION: &str = "Destination filename [startup-config]? ";

#[derive(Clone, Debug)]
struct Frame {
    mode: String,
    /// the running-configuration blocks lines typed here go to: one, or each member of a range
    blocks: Vec<usize>,
    /// nesting inside the blocks: a submode of a submode (address-family) writes its lines indented
    prefix: String,
    /// the key of the line that opened this indented section, which its `exit-` line closes
    section: Option<String>,
}

impl Frame {
    fn top(mode: &str) -> Self {
        Frame {
            mode: mode.into(),
            blocks: vec![],
            prefix: String::new(),
            section: None,
        }
    }
}

/// What `?` lists and Tab completes to.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Offered {
    Keyword,
    /// an interface type, followed by its number
    Type,
    /// an argument's placeholder (`WORD`, `<1-4094>`), not typed as shown
    Placeholder,
}

struct Offer {
    shown: String,
    help: String,
    kind: Offered,
}

/// A configuration line the switch took: the block it went into ("" at the top level), the line,
/// and its command's key (none for a line taken as a real switch held it).
#[derive(Clone, Debug, PartialEq)]
pub struct Accepted {
    pub block: String,
    pub line: String,
    pub key: Option<String>,
}

impl Accepted {
    /// The kind of block it went into (`live::kind`).
    pub fn kind(&self) -> String {
        crate::live::kind(&self.block)
    }
}

/// Where `write memory` keeps the configuration besides the session: the switch list.
type Keep<'a> = Box<dyn FnMut(&str) -> std::result::Result<(), String> + Send + 'a>;

pub struct Session<'a> {
    pub engine: &'a Engine,
    pub device: Device,
    frames: Vec<Frame>,
    pub closed: bool,
    /// the configuration `write memory` saved last
    pub startup: Option<String>,
    keep: Option<Keep<'a>>,
    /// `copy running-config startup-config` waits for its destination
    asking: bool,
    /// the console's width in columns, which `show vlan brief` wraps to; 80 off a terminal
    pub width: usize,
    /// the console's page length in rows, which `--More--` paging uses; usize::MAX is no paging
    /// (`terminal length 0`). Off a terminal it is unused, since only the console pages.
    pub length: usize,
    /// whether `terminal width`/`terminal length` set the value, so a window resize leaves it alone
    pub width_set: bool,
    pub length_set: bool,
    /// every configuration line taken, in order (`vswitch verify`)
    pub accepted: Vec<Accepted>,
    /// a banner being typed: its global entry and the delimiter that ends it
    banner: Option<(usize, String)>,
    /// a certificate's hex being typed after `certificate` in a chain: the chain's block
    certificate: Option<usize>,
    /// the top-level line or block header written last, as the running configuration prints it: a
    /// line a real switch held prints after the line typed before it
    last_top: Option<String>,
    /// the configuration line being typed, as typed
    raw: String,
    /// the columns before a line `do` hands to EXEC, its configuration prompt and `do `: a refused
    /// word's caret stands under the line as typed
    lead: Option<usize>,
    /// a configuration being replayed (`check::type_into`): the secret sweep's `<removed>` stands
    /// for the value it took out; a console types none
    pub redacted: bool,
    /// the startup configuration's lines, as `show startup-config` prints them: the one `write
    /// memory` saved, else the one replayed before the first typed line (the switch booted from it)
    pub saved: Option<Vec<String>>,
    replayed: bool,
}

/// What a typed line did.
#[derive(Debug, PartialEq)]
pub enum Result {
    Ok,
    Error,
}

impl<'a> Session<'a> {
    pub fn new(engine: &'a Engine, mut device: Device) -> Self {
        if device.release.is_empty() {
            device.release = engine.grammar.release.clone();
        }
        device.held_after = engine.held.after.clone();
        Session {
            engine,
            device,
            frames: vec![Frame::top("exec")],
            closed: false,
            startup: None,
            keep: None,
            asking: false,
            width: 80,
            length: 24,
            width_set: false,
            length_set: false,
            accepted: vec![],
            banner: None,
            certificate: None,
            last_top: None,
            raw: String::new(),
            lead: None,
            redacted: false,
            saved: None,
            replayed: false,
        }
    }

    /// The commands `privilege exec level <n> <command>` raises above level 1, where `>` runs, as
    /// their words (c9200 17.15 capture 27 raises 14, `show` and `terminal` among them).
    fn raised(&self) -> Vec<Vec<String>> {
        self.device
            .global
            .iter()
            .filter_map(|e| {
                let rest = e.text.strip_prefix("privilege exec ")?;
                let rest = rest.strip_prefix("all ").unwrap_or(rest);
                let (n, cmd) = rest.strip_prefix("level ")?.split_once(' ')?;
                let words: Vec<String> = cmd.split_whitespace().map(str::to_lowercase).collect();
                (n.parse::<u32>().ok()? > 1 && !words.is_empty()).then_some(words)
            })
            .collect()
    }

    /// Whether a line's words start a command raised above `>`'s level, at `>`.
    fn above_exec(&self, words: &[String]) -> bool {
        self.mode() == "exec"
            && self
                .raised()
                .iter()
                .any(|r| r.len() <= words.len() && r.iter().zip(words).all(|(a, b)| a == b))
    }

    /// Records a top-level line or block header as written last; returns the one written before it.
    fn wrote_top(&mut self, text: &str) -> Option<String> {
        self.last_top.replace(text.to_string())
    }

    /// `terminal length <0-512>` and `terminal width <0-512>` at an EXEC prompt: the general command
    /// reference's, in no platform book. Every capture types `terminal length 0`; the width is the one
    /// `show vlan brief` wraps to, 0 for none. `terminal` is read by any start no other command at the
    /// prompt has (`ter len 30`), and a value out of range is refused under it.
    fn terminal(&mut self, line: &str, prompt_len: usize) -> Option<(Vec<String>, Result)> {
        if cli::is_config(self.mode()) {
            return None;
        }
        let toks = cli::tokens(line);
        let [t, what, rest @ ..] = &toks[..] else {
            return None;
        };
        let what_word = what.text.to_lowercase();
        if !self.names(&t.text.to_lowercase(), "terminal", &[self.mode()]) || what_word.len() < 2 {
            return None;
        }
        if self.above_exec(&["terminal".to_string()]) {
            return None;
        }
        let width = if "width".starts_with(what_word.as_str()) {
            true
        } else if "length".starts_with(what_word.as_str()) {
            false
        } else {
            return None;
        };
        let refuse = |o: Outcome| Some((self.error(prompt_len, line, &o), Result::Error));
        let n = match rest {
            [] => return refuse(Outcome::Incomplete),
            [n] => match n.text.parse::<usize>().ok().filter(|n| *n <= 512) {
                Some(n) => n,
                None => return refuse(Outcome::Invalid(n.at)),
            },
            [_, extra, ..] => return refuse(Outcome::Invalid(extra.at)),
        };
        if width {
            self.width = if n == 0 { usize::MAX } else { n };
            self.width_set = true;
        } else {
            // `terminal length 0` turns paging off; otherwise the page is n rows
            self.length = if n == 0 { usize::MAX } else { n };
            self.length_set = true;
        }
        Some((vec![], Result::Ok))
    }

    /// `banner <kind> <delimiter><text>`: the delimiter is `^C` as a running configuration prints it,
    /// else the first character, and the text runs to the delimiter again, on this line or a later
    /// one. The banner is one entry, its lines kept whole (c2960x 15.2(7)E capture 8's has two identical lines),
    /// delimited by `^C` whatever was typed, as every captured banner prints, and nothing kept past
    /// its closing delimiter. `banner` and its kind are read as IOS reads keywords: in any case, by
    /// a start no other keyword there has (`ban mo #`).
    fn banner_start(&mut self, line: &str) -> Option<(Vec<String>, Result)> {
        if !cli::is_config(self.mode()) {
            return None;
        }
        const KINDS: &[&str] = &[
            "motd",
            "login",
            "exec",
            "incoming",
            "slip-ppp",
            "prompt-timeout",
            "config-save",
        ];
        let toks = cli::tokens(line);
        let (word, kind) = (toks.first()?, toks.get(1)?);
        if !self.names(
            &word.text.to_lowercase(),
            "banner",
            &[self.mode(), "config"],
        ) {
            return None;
        }
        let typed = kind.text.to_lowercase();
        let [kind] = KINDS
            .iter()
            .filter(|k| k.starts_with(&typed))
            .collect::<Vec<_>>()[..]
        else {
            return None;
        };
        let body = line[toks[1].at + toks[1].text.len()..].trim_start();
        let delim = if body.starts_with("^C") {
            "^C".to_string()
        } else {
            body.chars().next()?.to_string()
        };
        let rest = &body[delim.len()..];
        let closed = rest.find(delim.as_str());
        let key = format!("banner {kind}");
        let text = match closed {
            Some(i) => format!("banner {kind} ^C{}^C", &rest[..i]),
            None => format!("banner {kind} ^C{rest}"),
        };
        // typed in a submode, it goes to global configuration (c9200 17.6 capture 1's follows control-plane)
        self.frames.truncate(3);
        let at = match self.device.global.iter().position(|e| e.key == key) {
            Some(i) => {
                self.device.global[i].text = text.clone();
                i
            }
            None => {
                self.device.global.push(Entry {
                    key: key.clone(),
                    text: text.clone(),
                });
                self.device.global.len() - 1
            }
        };
        self.accepted.push(Accepted {
            block: String::new(),
            line: text,
            key: Some(key),
        });
        if closed.is_some() {
            return Some((vec![], Result::Ok));
        }
        let ask = format!("Enter TEXT message.  End with the character '{delim}'.");
        self.banner = Some((at, delim));
        Some((vec![ask], Result::Ok))
    }

    /// Whether a typed first word names `command`: the whole word, or a start of it no other
    /// command of these modes begins with.
    fn names(&self, word: &str, command: &str, modes: &[&str]) -> bool {
        if word == command {
            return true;
        }
        if word.is_empty() || !command.starts_with(word) {
            return false;
        }
        modes.iter().all(|m| {
            self.engine.next(m, &[]).0.iter().all(|n| match n {
                Next::Kw { kw, .. } => {
                    let kw = kw.to_lowercase();
                    kw == command || !kw.starts_with(word)
                }
                Next::Arg { .. } => true,
            })
        })
    }

    /// `write`, `write memory` and `copy running-config startup-config` at the privileged prompt: the
    /// console's ways to save, whatever a platform's books spell (the 2960 books give no `write` and
    /// read `copy` as the bootloader's). `write` alone is `write memory`, as on IOS.
    fn save_line(&mut self, line: &str) -> Option<(Vec<String>, Result)> {
        if self.mode() != "privileged" {
            return None;
        }
        let words: Vec<String> = line.split_whitespace().map(str::to_lowercase).collect();
        let starts = |w: &str, of: &str, least: usize| w.len() >= least && of.starts_with(w);
        match &words[..] {
            [w] if self.names(w, "write", &["privileged"]) => Some(self.save()),
            [w, m] if self.names(w, "write", &["privileged"]) && starts(m, "memory", 1) => {
                Some(self.save())
            }
            [c, from, to]
                if self.names(c, "copy", &["privileged"])
                    && starts(from, "running-config", 3)
                    && starts(to, "startup-config", 3) =>
            {
                self.asking = true;
                Some((vec![], Result::Ok))
            }
            _ => None,
        }
    }

    /// `ip access-list resequence <name> <start> <step>`, each keyword by any unique start: the named
    /// list's entries renumbered, a remark with the entry after it (`acl::resequence`).
    fn resequence(&mut self, line: &str) -> Option<(Vec<String>, Result)> {
        let w: Vec<&str> = line.split_whitespace().collect();
        let starts = |typed: &str, word: &str, least: usize| {
            typed.len() >= least && word.starts_with(&typed.to_lowercase())
        };
        if w.len() != 6
            || !w[0].eq_ignore_ascii_case("ip")
            || !starts(w[1], "access-list", 8)
            || !starts(w[2], "resequence", 2)
        {
            return None;
        }
        let (Ok(start), Ok(step)) = (w[4].parse::<u32>(), w[5].parse::<u32>()) else {
            return None;
        };
        if let Some(b) = self
            .device
            .blocks
            .iter()
            .position(|b| crate::acl::of(&b.header).is_some_and(|(_, n)| n == w[3]))
        {
            crate::acl::resequence(&mut self.device.blocks[b].lines, start, step);
        }
        Some((vec![], Result::Ok))
    }

    /// Whether a banner's text is being typed: every line is text until its delimiter, a `!`, a
    /// blank line, `end` or a `?` among them, so a caller that skips such lines types them here.
    pub fn in_banner(&self) -> bool {
        self.banner.is_some()
    }

    /// The block the current mode writes into, "" at the top level.
    fn block_header(&self) -> String {
        self.frames
            .last()
            .and_then(|f| f.blocks.first())
            .map_or(String::new(), |&b| self.device.blocks[b].header.clone())
    }

    /// Takes a line the grammar refuses that a real switch of this platform and release held in a
    /// block of this kind, as it held it.
    fn held_in_block(&mut self, line: &str) -> Option<(Vec<String>, Result)> {
        let header = self.block_header();
        let text = line.trim();
        let kind = crate::live::kind(&header);
        if header.is_empty() || !self.engine.held.has(&kind, text) {
            return None;
        }
        // as indented under the block (a block prints its lines one space in): as typed, else as the
        // first capture that held it indented it (a terminal log writes a tab as spaces)
        let typed = &line[..line.len() - line.trim_start().len()];
        let indent = if typed.is_empty() {
            self.engine
                .held
                .indent(&kind, text)
                .map_or("", |i| i.strip_prefix(' ').unwrap_or(i))
        } else {
            typed.strip_prefix(' ').unwrap_or(typed)
        };
        let frame = self
            .frames
            .last()
            .cloned()
            .unwrap_or_else(|| Frame::top("config"));
        // an access list's held entry takes its place among the list's entries by its number
        if let Some(list) = crate::acl::of(&header).map(|(k, _)| k) {
            let (seq, entry) =
                crate::acl::numbered(text).map_or((None, text), |(n, t)| (Some(n), t));
            for &b in &frame.blocks {
                let _ = crate::acl::add(&mut self.device.blocks[b].lines, list, seq, entry);
            }
            self.accepted.push(Accepted {
                block: header,
                line: text.to_string(),
                key: None,
            });
            return Some((vec![], Result::Ok));
        }
        // typed in a section, at least as deep as the section's own lines
        let indent = if indent.len() < frame.prefix.len() {
            frame.prefix.as_str()
        } else {
            indent
        };
        let entry = format!("{indent}{text}");
        // a line typed again in its section is not a second line; the same line under another
        // section is (a policy map's classes each `police` alike, c9300 17.15 capture 1). It goes at the end
        // of its section's lines, before the line that closes it.
        let depth = |s: &str| s.len() - s.trim_start().len();
        for &b in &frame.blocks {
            let lines = &mut self.device.blocks[b].lines;
            let from = match frame
                .section
                .as_ref()
                .and_then(|s| lines.iter().rposition(|e| &e.key == s))
            {
                Some(h) => h + 1,
                None => lines
                    .iter()
                    .rposition(|e| depth(&e.text) < depth(&entry))
                    .map_or(0, |i| i + 1),
            };
            let to = lines[from..]
                .iter()
                .position(|e| depth(&e.text) < depth(&entry))
                .map_or(lines.len(), |i| from + i);
            if !lines[from..to].iter().any(|e| e.key == entry) {
                lines.insert(
                    to,
                    Entry {
                        key: entry.clone(),
                        text: entry.clone(),
                    },
                );
            }
        }
        self.accepted.push(Accepted {
            block: header,
            line: text.to_string(),
            key: None,
        });
        Some((vec![], Result::Ok))
    }

    /// Takes a top-level line the grammar refuses that a real switch of this platform and release
    /// held, back in global configuration; one that opened a block there opens it here.
    fn held_at_top(&mut self, line: &str) -> Option<(Vec<String>, Result)> {
        let text = line.trim();
        if !cli::is_config(self.mode()) || !self.engine.held.has("", text) {
            return None;
        }
        // exec, privileged, global configuration
        self.frames.truncate(3);
        self.device.held_top.insert(text.to_string());
        // it prints after the line typed before it, where a typed running configuration put it
        // (two 2960-X captures order their held lines apart); a line typed again keeps its place
        let again = self.device.global.iter().any(|e| e.key == text);
        if let Some(prev) = self.wrote_top(text)
            && !again
            && prev != text
        {
            self.device.held_after.insert(text.to_string(), prev);
        }
        if self.engine.held.opens(text) {
            let b = self.device.block(text);
            self.frames.push(Frame {
                mode: self.held_mode(text),
                blocks: vec![b],
                prefix: String::new(),
                section: None,
            });
        } else if !self.device.global.iter().any(|e| e.key == text) {
            self.device.global.push(Entry {
                key: text.to_string(),
                text: text.to_string(),
            });
        }
        self.accepted.push(Accepted {
            block: String::new(),
            line: text.to_string(),
            key: None,
        });
        Some((vec![], Result::Ok))
    }

    /// The mode a block real switches held runs in at the console: the submode the books' examples
    /// show after its line, though too seldom to learn (the 9200 17.15 book's one `redundancy`, then
    /// `(config-red)`), else `config-live`, a mode no command of the books runs in.
    fn held_mode(&self, line: &str) -> String {
        let fallback = || "config-live".to_string();
        let Outcome::Ok(p) = self.parse("config", line) else {
            return fallback();
        };
        let first = |s: &str| s.split_whitespace().next().unwrap_or("").to_lowercase();
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for t in &self.engine.grammar.transitions {
            if t.from == "config"
                && cli::is_submode(&t.to)
                && self.engine.has_mode(&t.to)
                && first(&t.line) == first(line)
                && matches!(self.parse("config", &t.line), Outcome::Ok(q) if q.command == p.command)
            {
                *seen.entry(t.to.as_str()).or_default() += 1;
            }
        }
        seen.into_iter()
            .max_by_key(|(_, n)| *n)
            .map_or_else(fallback, |(m, _)| m.to_string())
    }

    /// Where `write memory` keeps the configuration, besides the session.
    pub fn on_save(
        &mut self,
        keep: impl FnMut(&str) -> std::result::Result<(), String> + Send + 'a,
    ) {
        self.keep = Some(Box::new(keep));
    }

    pub fn mode(&self) -> &str {
        self.frames.last().map_or("exec", |f| f.mode.as_str())
    }

    /// A line matched in `mode`, as a replayed configuration is when the session replays one.
    fn parse(&self, mode: &str, line: &str) -> Outcome {
        if self.redacted {
            self.engine.parse_redacted(mode, line)
        } else {
            self.engine.parse(mode, line)
        }
    }

    /// Ctrl-C at a question (`copy`'s destination): the question is dropped and nothing is saved.
    pub fn cancel(&mut self) {
        self.asking = false;
    }

    pub fn prompt(&self) -> String {
        if self.asking {
            return DESTINATION.into();
        }
        let h = &self.device.hostname;
        match self.mode() {
            "exec" => format!("{h}>"),
            "privileged" => format!("{h}#"),
            m => format!("{h}({m})#"),
        }
    }

    /// Where IOS puts the caret in a word it refuses, as a column after the prompt (characters, not
    /// bytes): past the longest start of it some keyword there begins with (`show config` at `>`,
    /// where `controllers` begins `con`: under the `f`, c2960x 15.2(7)E capture 8). A word where an argument may
    /// stand is refused at its start.
    fn caret(&self, line: &str, at: usize) -> usize {
        let column = line.get(..at).map_or(at, |s| s.chars().count());
        let toks = cli::tokens(line);
        let Some(i) = toks.iter().position(|t| t.at == at) else {
            return column;
        };
        // the words a real switch offered there when asked `?`, else the books'
        let key = (self.mode().to_string(), crate::live::help_key(&line[..at]));
        let mut words: Vec<String> = match self.engine.held.help.get(&key) {
            Some(lines) => lines
                .iter()
                .filter_map(|l| l.split_whitespace().next())
                .map(str::to_string)
                .collect(),
            None => {
                let (next, _) = self.engine.next(self.mode(), &toks[..i]);
                self.offered(&next).into_iter().map(|o| o.shown).collect()
            }
        };
        // a command raised above `>`'s level is no word there
        if i == 0 {
            words.retain(|w| !self.above_exec(&[w.to_lowercase()]));
        }
        let typed = toks[i].text.to_lowercase();
        // an argument the typed word could be stands here (`WORD`, `A.B.C.D`, `<1-4094>`), and the
        // caret marks the word's start; one it cannot be does not (17.15's `<1-99>` session number
        // at `#` against `shw`); `<cr>` ends the line and `|` starts a modifier
        let could_be = |w: &str| {
            if w == "<cr>"
                || w == "|"
                || w.chars().any(|c| c.is_ascii_lowercase()) && !w.starts_with('<')
            {
                return false;
            }
            let inner = w.trim_start_matches('<').trim_end_matches('>');
            let range = inner.split_once('-').is_some_and(|(a, b)| {
                [a, b]
                    .iter()
                    .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            });
            if range {
                typed.chars().all(|c| c.is_ascii_digit())
            } else if inner.contains('.')
                && inner.chars().all(|c| c.is_ascii_uppercase() || c == '.')
            {
                typed.chars().all(|c| c.is_ascii_digit() || c == '.')
            } else {
                true
            }
        };
        if words.iter().any(|w| could_be(w)) {
            return column;
        }
        let matched = words
            .iter()
            .map(|w| {
                typed
                    .chars()
                    .zip(w.to_lowercase().chars())
                    .take_while(|(a, b)| a == b)
                    .count()
            })
            .max()
            .unwrap_or(0);
        if matched < typed.chars().count() {
            column + matched
        } else {
            column
        }
    }

    /// IOS's three errors, the caret aligned under the typed line after the prompt (`prompt_len`
    /// columns).
    fn error(&self, prompt_len: usize, line: &str, o: &Outcome) -> Vec<String> {
        match o {
            Outcome::Invalid(at) => vec![
                format!("{}^", " ".repeat(prompt_len + self.caret(line, *at))),
                "% Invalid input detected at '^' marker.".into(),
                String::new(),
            ],
            Outcome::Incomplete => vec!["% Incomplete command.".into(), String::new()],
            Outcome::Ambiguous => vec![format!("% Ambiguous command:  \"{line}\"")],
            Outcome::Ok(_) => vec![],
        }
    }

    /// Types `line` at the current prompt; returns what IOS prints and whether it was accepted.
    pub fn type_line(&mut self, line: &str) -> (Vec<String>, Result) {
        let lead = self.lead.take();
        // a switch booted from the configuration replayed before its first typed line
        if self.redacted {
            self.replayed = true;
        } else if self.replayed && self.saved.is_none() {
            self.saved = Some(show::configuration(&self.device));
        }
        if self.asking {
            self.asking = false;
            let to = line.trim();
            if to.is_empty() || to.eq_ignore_ascii_case("startup-config") {
                return self.save();
            }
            return (
                vec!["% vswitch saves only to startup-config".into()],
                Result::Error,
            );
        }
        if let Some((at, delim)) = self.banner.take() {
            let text = line.trim_end_matches(['\r', '\n']);
            let e = &mut self.device.global[at];
            e.text.push('\n');
            match text.find(delim.as_str()) {
                Some(i) => {
                    e.text.push_str(&text[..i]);
                    e.text.push_str("^C");
                }
                None => {
                    e.text.push_str(text);
                    self.banner = Some((at, delim));
                }
            }
            return (vec![], Result::Ok);
        }
        if let Some(done) = self.banner_start(line) {
            return done;
        }
        if let Some(done) = self.certificate_data(line) {
            return done;
        }
        let prompt_len = lead.unwrap_or_else(|| self.prompt().chars().count());
        if let Some(done) = self.terminal(line, prompt_len) {
            return done;
        }
        if let Some(done) = self.save_line(line) {
            return done;
        }
        let whole = line.trim_end();
        // a blank line and a `!` comment do nothing, as a pasted running configuration's do
        if whole.trim().is_empty() || whole.trim_start().starts_with('!') {
            return (vec![], Result::Ok);
        }
        // `logout` and `quit` end an EXEC session as `exit` does
        if matches!(self.mode(), "exec" | "privileged")
            && matches!(whole.trim().to_lowercase().as_str(), "logout" | "quit")
        {
            self.closed = true;
            return (vec![], Result::Ok);
        }
        // an EXEC line's output passes the modifier after `|` (`show running-config | include x`);
        // in configuration mode `|` is text (`do` hands its line to EXEC whole)
        let bar = (!cli::is_config(self.mode()))
            .then(|| whole.find('|'))
            .flatten();
        let mut trimmed = bar.map_or(whole, |b| whole[..b].trim_end());
        // `ip access-list resequence <name> <start> <step>` renumbers a list's entries and is no line
        // of the configuration (the lab 2960G took it at 15.0(2)SE11)
        if self.mode() == "config"
            && let Some(done) = self.resequence(trimmed)
        {
            return done;
        }
        // an IPv4 access list's entries carry sequence numbers (`acl`): `no <n>` removes one, and a
        // number before an entry places it, the entry read as typed without it
        let full = trimmed;
        let mut seq = None;
        if crate::acl::in_mode(self.mode()) {
            if let Some(n) = crate::acl::unnumbered(full) {
                if let Some(b) = self.frames.last().and_then(|f| f.blocks.first().copied()) {
                    crate::acl::remove_number(&mut self.device.blocks[b].lines, n);
                }
                return (vec![], Result::Ok);
            }
            if let Some((n, entry)) = crate::acl::numbered(full) {
                seq = Some(n);
                trimmed = entry;
            }
        }
        // a typed configuration's indentation closes a section: a line no deeper than the
        // section's own lines belongs to the block around it (c9300 17.15 capture 1's ` class
        // <class>` after `  priority level 1`); a line typed unindented closes nothing,
        // and a section's `exit-` line, printed at its header's depth, closes its own
        let indent = line.len() - line.trim_start().len();
        // the sections it closes, open again if the line is refused: the mode leaves a submode only
        // for a line that is accepted
        let mut closed: Vec<Frame> = Vec::new();
        if indent > 0 && cli::is_config(self.mode()) && !line.trim_start().starts_with("exit") {
            while self
                .frames
                .last()
                .is_some_and(|f| f.section.is_some() && indent <= f.prefix.len())
            {
                closed.extend(self.frames.pop());
            }
        }
        let mut mode = self.mode().to_string();
        let mut outcome = self.parse(&mode, trimmed);
        // a line a real switch of this platform and release held in this block is taken as it held it
        if !matches!(outcome, Outcome::Ok(_))
            && let Some(done) = self.held_in_block(full)
        {
            return done;
        }
        // a configuration submode hands a line it does not know to the nearest enclosing mode that
        // takes it (`no ip address` after `vrf forwarding` goes to the interface), global
        // configuration last; a numbered entry stays in its list
        let mut back_to = None;
        if cli::is_submode(&mode) && seq.is_none() && !matches!(outcome, Outcome::Ok(_)) {
            for i in (0..self.frames.len().saturating_sub(1)).rev() {
                let m = self.frames[i].mode.clone();
                if !cli::is_config(&m) {
                    break;
                }
                let o = self.parse(&m, trimmed);
                if matches!(o, Outcome::Ok(_)) {
                    back_to = Some(i);
                    mode = m;
                    outcome = o;
                    break;
                }
            }
        }
        let p = match outcome {
            Outcome::Ok(p) => p,
            other => {
                if let Some(done) = self.held_at_top(trimmed) {
                    return done;
                }
                self.frames.extend(closed.drain(..).rev());
                // a refused entry's caret stands under the line as typed, past its number
                let number = full.chars().count() - trimmed.chars().count();
                return (
                    self.error(prompt_len + number, trimmed, &other),
                    Result::Error,
                );
            }
        };
        // a command `privilege exec level` raised is refused at `>` as one IOS does not know there
        let words: Vec<String> = p
            .canonical()
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        if self.above_exec(&words) {
            return (
                self.error(prompt_len, trimmed, &Outcome::Invalid(0)),
                Result::Error,
            );
        }
        // a line a real switch printed that the books read as another command is taken as that
        // switch held it: IOS writes a command's first word whole, so a first word the books read as
        // an abbreviation names a command they lack (c9200 17.6 capture 1's `service call-home`: the 9200 book
        // has no `service`, so it read `service-template`)
        let first_abbreviated = p.items.iter().find_map(|i| match i {
            Item::Kw { kw, exact, .. } if !kw.eq_ignore_ascii_case("no") => Some(!*exact),
            _ => None,
        });
        if cli::is_config(&mode) && first_abbreviated == Some(true) {
            if let Some(done) = self.held_in_block(trimmed) {
                return done;
            }
            if let Some(done) = self.held_at_top(trimmed) {
                return done;
            }
        }
        // an indented line a real switch held under a top-level line the books give no submode
        // stays in that block, though the books read it globally (the C2960G's `username vswitch`
        // under `ip ssh pubkey-chain`)
        if mode == "config"
            && indent > 0
            && let Some(done) = self.held_in_block(trimmed)
        {
            return done;
        }
        // a top-level line real switches held opening a block, which the books give no submode,
        // opens it as they held it (every dprL2cok capture's `redundancy` at 9200 17.15)
        if mode == "config"
            && self.engine.enters(&p).is_none()
            && self.engine.held.opens(trimmed.trim())
            && let Some(done) = self.held_at_top(trimmed)
        {
            return done;
        }
        // an interface the stack does not have is refused at its name; a range, at its start. Only an
        // interface argument names one: free text that begins with a name is text (`description
        // Vlan10 uplink`)
        let args = &self.engine.grammar.commands[p.command].args;
        let refused = p.items.iter().find_map(|it| match it {
            Item::Arg {
                name, value, tok, ..
            } if matches!(
                args.get(name),
                Some(ArgKind::Interface | ArgKind::InterfaceRange)
            ) && !on_stack(value, &self.device.stack) =>
            {
                Some(*tok)
            }
            _ => None,
        });
        // a block's interface written apart from its number (`interface vlan 5000`), at the number
        let refused = refused.or_else(|| {
            let opens = self.engine.enters(&p).is_some_and(cli::is_config);
            let named = headers(&p).into_iter().any(|h| {
                h.strip_prefix("interface ")
                    .is_some_and(|n| looks_like_interface(n) && !self.device.stack.has(n))
            });
            (opens && named)
                .then(|| {
                    p.items.iter().rev().find_map(|i| match i {
                        Item::Arg { tok, .. } => Some(*tok),
                        Item::Kw { .. } => None,
                    })
                })
                .flatten()
        });
        if let Some(tok) = refused {
            self.frames.extend(closed.drain(..).rev());
            let at = crate::cli::tokens(trimmed).get(tok).map_or(0, |t| t.at);
            return (
                self.error(prompt_len, trimmed, &Outcome::Invalid(at)),
                Result::Error,
            );
        }
        let modifier = match bar.map(|b| crate::filter::parse(&whole[b + 1..], b + 1)) {
            None => None,
            Some(Ok(m)) => Some(m),
            Some(Err(o)) => return (self.error(prompt_len, whole, &o), Result::Error),
        };
        // the mode leaves the submode only for a line that is accepted
        if let Some(i) = back_to {
            self.frames.truncate(i + 1);
        }
        // an access list's entry goes among its entries by number: one typed again adds nothing, a
        // number in use is refused, `no` removes it
        if back_to.is_none()
            && crate::acl::in_mode(&mode)
            && let Some(b) = self.frames.last().and_then(|f| f.blocks.first().copied())
            && let Some(kind) = crate::acl::of(&self.device.blocks[b].header).map(|(k, _)| k)
        {
            let canonical = p.canonical();
            let text = canonical.strip_prefix("no ").unwrap_or(&canonical);
            if matches!(
                text.split_whitespace().next(),
                Some("permit" | "deny" | "remark")
            ) {
                let lines = &mut self.device.blocks[b].lines;
                if p.negated {
                    crate::acl::remove(lines, kind, text);
                } else if let Err(e) = crate::acl::add(lines, kind, seq, text) {
                    return (vec![e, String::new()], Result::Error);
                }
                let block = self.block_header();
                self.accepted.push(Accepted {
                    block,
                    line: full.trim().to_string(),
                    key: Some(crate::device::key(&p)),
                });
                return (vec![], Result::Ok);
            }
        }
        // a line that only moves between modes or runs EXEC (`exit`, `end`, `do`) configures
        // nothing; an `exit-` line closes its section, which IOS prints
        let navigates = matches!(
            self.engine.grammar.commands[p.command].name.as_str(),
            "do" | "exit" | "end"
        );
        if cli::is_config(&mode) && !navigates {
            let block = self.block_header();
            self.accepted.push(Accepted {
                block,
                line: trimmed.trim().to_string(),
                key: Some(crate::device::key(&p)),
            });
        }
        let (mut out, r) = self.run(&mode, trimmed, &p);
        // at the console an access or voice VLAN the switch lacks is made, and said so, as IOS does;
        // a replayed configuration defines its own
        if r == Result::Ok
            && !self.redacted
            && !p.negated
            && let Some(made) = self.vlan_made(&p.canonical())
        {
            out.insert(0, made);
        }
        // IOS reads a certificate's hex after `certificate` in a chain, until `quit`
        if r == Result::Ok
            && mode == "config-cert-chain"
            && !p.negated
            && self.engine.grammar.commands[p.command].name == "certificate"
        {
            self.certificate = self.frames.last().and_then(|f| f.blocks.first().copied());
        }
        match modifier {
            Some(m) if r == Result::Ok => (crate::filter::apply(&m, out), r),
            _ => (out, r),
        }
    }

    /// The VLAN a `switchport access vlan <n>` or `switchport voice vlan <n>` names when the switch
    /// has none, made as IOS makes it, and what IOS says: `% Access VLAN does not exist. Creating
    /// vlan <n>`. A VTP client makes none, nor a server an extended-range VLAN.
    fn vlan_made(&mut self, line: &str) -> Option<String> {
        let (what, rest) = if let Some(r) = line.strip_prefix("switchport access vlan ") {
            ("Access", r)
        } else {
            ("Voice", line.strip_prefix("switchport voice vlan ")?)
        };
        let id: u32 = rest.trim().parse().ok()?;
        let vtp = |m: &str| {
            self.device
                .global
                .iter()
                .any(|e| e.text == format!("vtp mode {m}"))
        };
        if id == 1
            || (1002..=1005).contains(&id)
            || id > 4094
            || vtp("client")
            || id > 1005 && !self.device.vlans_in_running_config()
            || show::defined_vlans(&self.device).contains(&id)
        {
            return None;
        }
        self.device.block(&format!("vlan {id}"));
        Some(format!("% {what} VLAN does not exist. Creating vlan {id}"))
    }

    /// A line of the certificate being typed: hex, or `quit`, which ends it. Kept in the chain's
    /// block as typed, else as switches print it (two spaces in, `quit` after a tab); any other
    /// line ends the certificate and is typed as itself.
    fn certificate_data(&mut self, line: &str) -> Option<(Vec<String>, Result)> {
        let b = self.certificate?;
        let t = line.trim();
        let quit = t.eq_ignore_ascii_case("quit");
        let hex = !t.is_empty() && t.chars().all(|c| c.is_ascii_hexdigit() || c == ' ');
        if !(quit || hex) {
            self.certificate = None;
            return None;
        }
        let typed = &line[..line.len() - line.trim_start().len()];
        let header = self.device.blocks.get(b)?.header.clone();
        let indent = if !typed.is_empty() {
            typed.to_string()
        } else if let Some(i) = self.engine.held.indent_of(&header, t) {
            i.to_string()
        } else if quit {
            "  \t".to_string()
        } else {
            "  ".to_string()
        };
        // a block prints its lines one space in
        let stored = format!("{}{t}", indent.strip_prefix(' ').unwrap_or(&indent));
        let lines = &mut self.device.blocks[b].lines;
        let key = format!("certificate data {}", lines.len());
        lines.push(Entry { key, text: stored });
        if quit {
            self.certificate = None;
        }
        Some((vec![], Result::Ok))
    }

    fn run(&mut self, mode: &str, line: &str, p: &Parse) -> (Vec<String>, Result) {
        self.raw = line.to_string();
        let cmd = &self.engine.grammar.commands[p.command];
        let name = cmd.name.as_str();
        let enters = self.engine.enters(p).map(str::to_string);
        match name {
            "do" => {
                // the rest of the line as typed, run at the privileged prompt; a refused word's
                // caret stands under it after the configuration prompt and `do `
                let at = cli::tokens(line).get(1).map_or(line.len(), |t| t.at);
                let lead = self.prompt().chars().count() + line[..at].chars().count();
                let saved = std::mem::replace(&mut self.frames, vec![Frame::top("privileged")]);
                self.lead = Some(lead);
                let out = self.type_line(&line[at..]);
                self.frames = saved;
                return out;
            }
            "exit" => {
                if mode == "privileged" || mode == "exec" {
                    self.closed = true;
                } else {
                    self.frames.pop();
                }
                return (vec![], Result::Ok);
            }
            "end" => {
                self.frames.truncate(2.min(self.frames.len()));
                return (vec![], Result::Ok);
            }
            "enable" => {
                if mode == "exec" {
                    self.frames.push(Frame::top("privileged"));
                }
                return (vec![], Result::Ok);
            }
            "disable" => {
                self.frames.truncate(1);
                return (vec![], Result::Ok);
            }
            "configure terminal" => {
                self.frames.push(Frame::top("config"));
                return (
                    vec!["Enter configuration commands, one per line.  End with CNTL/Z.".into()],
                    Result::Ok,
                );
            }
            "write memory" => return self.save(),
            "copy running-config startup-config" => {
                self.asking = true;
                return (vec![], Result::Ok);
            }
            _ => {}
        }
        if !cli::is_config(mode) {
            // EXEC: show commands print; the rest run silently
            return (show::render(self, name, p, line), Result::Ok);
        }
        // an `exit-` line (`exit-address-family`) closes the section it is typed in: IOS prints it
        // under the section's lines, and the mode goes to the one it names or else the enclosing one
        if name.starts_with("exit-") && !p.negated && self.frames.len() > 3 {
            let target = enters
                .as_deref()
                .and_then(|to| self.frames.iter().rposition(|f| f.mode == to))
                .filter(|&i| i + 1 < self.frames.len())
                .unwrap_or(self.frames.len() - 2);
            let leaving = self.frames[self.frames.len() - 1].clone();
            if let Some(section) = &leaving.section {
                let outer = &leaving.prefix[..leaving.prefix.len().saturating_sub(1)];
                for &b in &leaving.blocks {
                    let lines = &mut self.device.blocks[b].lines;
                    let key = crate::device::closing(section);
                    if lines.iter().any(|e| e.key == key) {
                        continue;
                    }
                    if let Some(h) = lines.iter().position(|e| &e.key == section) {
                        let mut at = h + 1;
                        while at < lines.len() && lines[at].key.starts_with(&leaving.prefix) {
                            at += 1;
                        }
                        let text = format!("{outer}{}", p.canonical());
                        lines.insert(at, Entry { key, text });
                    }
                }
            }
            self.frames.truncate(target + 1);
            return (vec![], Result::Ok);
        }
        // a line that opens a block of its own in global configuration, typed in another such
        // block's submode, leaves it for global configuration as IOS does (`interface vlan 10`
        // after `vlan 10`, `interface port-channel 5` in an interface), though a guide lists it there
        if enters.as_deref().is_some_and(cli::is_config)
            && self.frames.len() > 3
            && headers(p).first().is_some_and(|h| top_block(h))
            && self.frames[3]
                .blocks
                .first()
                .is_some_and(|&b| top_block(&self.device.blocks[b].header))
            && matches!(self.parse("config", line), Outcome::Ok(_))
        {
            self.frames.truncate(3);
        }
        // a line whose example prompts return to an enclosing mode (`ip address` after
        // `vrf forwarding`) goes back to that mode first
        if let Some(to) = enters.as_deref()
            && let Some(i) = self.frames.iter().rposition(|f| f.mode == to)
            && i + 1 < self.frames.len()
        {
            self.frames.truncate(i + 1);
            self.write(p);
            return (vec![], Result::Ok);
        }
        let top = self
            .frames
            .last()
            .cloned()
            .unwrap_or_else(|| Frame::top("config"));
        match enters.as_deref() {
            // learned as returning to the mode it is typed in: an ordinary line here
            Some(sub) if sub == top.mode => self.write(p),
            Some(sub) if cli::is_config(sub) && p.negated => {
                // `no interface ...`, `no vlan ...`: the block goes
                for header in headers(p) {
                    self.device.remove_block(&header);
                }
            }
            Some(sub) if cli::is_config(sub) && top.blocks.is_empty() => {
                // a top-level block, or one per member of an interface range
                let hs = headers(p);
                let prev = hs.first().and_then(|h| self.wrote_top(h));
                // a block a real switch of this platform and release held prints where that switch
                // printed it, where the running configuration has no place of its own for it
                // (`crypto pki trustpoint`): after the line typed before it, as a held block does
                for h in &hs {
                    if self.engine.held.has("", h) && !crate::show::has_place(h) {
                        self.device.held_top.insert(h.clone());
                        if let Some(prev) = prev.clone().filter(|p| p != h) {
                            self.device.held_after.insert(h.clone(), prev);
                        }
                    }
                }
                let blocks = hs.iter().map(|h| self.device.block(h)).collect();
                self.frames.push(Frame {
                    mode: sub.into(),
                    blocks,
                    prefix: String::new(),
                    section: None,
                });
            }
            Some(sub) if cli::is_config(sub) && top.mode.starts_with("config-if") => {
                // a submode of an interface writes into the interface itself (`vrf forwarding`)
                self.write(p);
                self.frames.push(Frame {
                    mode: sub.into(),
                    blocks: top.blocks.clone(),
                    prefix: top.prefix.clone(),
                    section: None,
                });
            }
            Some(sub) if cli::is_config(sub) => {
                // a submode inside a block opens an indented section (address-family), which IOS
                // closes with its `exit-` line however the mode is left (c9200 17.12 capture 1's empty ones)
                let header = headers(p).remove(0);
                let close = self.closes(sub, &header);
                for &b in &top.blocks {
                    let key = format!("{}{header}", top.prefix);
                    let lines = &mut self.device.blocks[b].lines;
                    if !lines.iter().any(|e| e.key == key) {
                        lines.push(Entry {
                            key: key.clone(),
                            text: key.clone(),
                        });
                        if let Some(c) = &close {
                            lines.push(Entry {
                                key: crate::device::closing(&key),
                                text: format!("{}{c}", top.prefix),
                            });
                        }
                    }
                }
                self.frames.push(Frame {
                    mode: sub.into(),
                    blocks: top.blocks.clone(),
                    prefix: format!("{} ", top.prefix),
                    section: Some(format!("{}{header}", top.prefix)),
                });
            }
            _ => self.write(p),
        }
        (vec![], Result::Ok)
    }

    /// The line that closes a section of mode `sub` that `header` opens: the `exit-` command the mode
    /// runs (`exit-address-family`), where it runs several the one the header names.
    fn closes(&self, sub: &str, header: &str) -> Option<String> {
        let exits: BTreeSet<&str> = self
            .engine
            .grammar
            .commands
            .iter()
            .filter(|c| c.modes.iter().any(|m| m == sub))
            .flat_map(|c| c.lines.iter().filter(|l| !l.negated))
            .map(|l| l.text.trim())
            .filter(|t| t.starts_with("exit-") && !t.contains(' '))
            .collect();
        let words = |s: &str| s.replace('-', " ");
        match exits.into_iter().collect::<Vec<_>>()[..] {
            [one] => Some(one.to_string()),
            ref many => many
                .iter()
                .find(|t| words(header).starts_with(&words(&t["exit-".len()..])))
                .map(|t| t.to_string()),
        }
    }

    /// Saves the running configuration as the startup configuration, and into the switch list.
    fn save(&mut self) -> (Vec<String>, Result) {
        // a VTP server keeps its VLANs in vlan.dat, which survives a reload beside the startup
        // configuration: they go first, so a boot types them back
        let mut text = String::new();
        let db = show::vlan_database(&self.device);
        if !db.is_empty() {
            text.push_str(
                "! vlan.dat: the VLAN database a VTP server keeps outside the configuration\n",
            );
            for l in db {
                text.push_str(&l);
                text.push('\n');
            }
        }
        let config = show::configuration(&self.device);
        text.push_str(&config.join("\n"));
        text.push('\n');
        let mut out = vec!["Building configuration...".to_string()];
        if let Some(keep) = self.keep.as_mut()
            && let Err(e) = keep(&text)
        {
            out.push(format!("% vswitch could not keep the configuration: {e}"));
            return (out, Result::Error);
        }
        self.startup = Some(text);
        self.saved = Some(config);
        out.push("[OK]".into());
        (out, Result::Ok)
    }

    /// The interface types `?` and Tab offer: the stack's front-panel types and the logical ones.
    fn interface_types(&self) -> Vec<String> {
        let physical = self.device.stack.physical();
        crate::cli::interface_types()
            .iter()
            .map(|(full, _)| *full)
            .filter(|full| {
                ["Loopback", "Port-channel", "Tunnel", "Vlan"].contains(full)
                    || physical.iter().any(|p| {
                        p.strip_prefix(full)
                            .is_some_and(|n| n.starts_with(|c: char| c.is_ascii_digit()))
                    })
            })
            .map(str::to_string)
            .collect()
    }

    /// How `?` shows what may come next: a keyword, an interface type, or an argument's placeholder,
    /// with its help.
    fn words(&self, n: &Next) -> Vec<Offer> {
        match n {
            Next::Kw { kw, help } => vec![Offer {
                shown: kw.clone(),
                help: help.clone().unwrap_or_default(),
                kind: Offered::Keyword,
            }],
            Next::Arg { kind, help, .. } => {
                let shown = match kind {
                    ArgKind::Interface | ArgKind::InterfaceRange => {
                        return self
                            .interface_types()
                            .into_iter()
                            .map(|t| Offer {
                                shown: t,
                                help: String::new(),
                                kind: Offered::Type,
                            })
                            .collect();
                    }
                    ArgKind::Number { min, max } => format!("<{min}-{max}>"),
                    ArgKind::Dscp => "<0-63>".into(),
                    ArgKind::VlanList | ArgKind::Word => "WORD".into(),
                    ArgKind::Ipv4 => "A.B.C.D".into(),
                    ArgKind::Ipv6 => "X:X:X:X::X".into(),
                    ArgKind::Mac => "H.H.H".into(),
                    ArgKind::Text => "LINE".into(),
                };
                vec![Offer {
                    shown,
                    help: help.clone().unwrap_or_default(),
                    kind: Offered::Placeholder,
                }]
            }
        }
    }

    /// What `?` and Tab offer after `next`, each once: a keyword the books spell in two cases in its
    /// lowercase spelling, and where an interface goes its type as IOS spells it (`Port-channel`, not
    /// the books' `port-channel` keyword).
    fn offered(&self, next: &[Next]) -> Vec<Offer> {
        let interface = next.iter().any(|n| {
            matches!(
                n,
                Next::Arg {
                    kind: ArgKind::Interface | ArgKind::InterfaceRange,
                    ..
                }
            )
        });
        let a_type = |w: &str| {
            crate::cli::interface_types()
                .iter()
                .any(|(full, _)| full.eq_ignore_ascii_case(w))
        };
        let mut by: BTreeMap<String, Offer> = BTreeMap::new();
        for o in next.iter().flat_map(|n| self.words(n)) {
            if interface && o.kind == Offered::Keyword && a_type(&o.shown) {
                continue;
            }
            let key = match o.kind {
                Offered::Placeholder => o.shown.clone(),
                _ => o.shown.to_lowercase(),
            };
            match by.get_mut(&key) {
                None => {
                    by.insert(key, o);
                }
                Some(e) => {
                    if o.shown == o.shown.to_lowercase() && e.kind == o.kind {
                        e.shown = o.shown;
                    }
                    if e.help.is_empty() {
                        e.help = o.help;
                    }
                }
            }
        }
        by.into_values().collect()
    }

    /// What IOS prints for `?` typed after `text`: what a real switch of this platform and release
    /// printed for the same line in the same mode, where a capture holds it; else, after a space, what
    /// may come next with its help and `<cr>` when the line is complete, and right after a word, the
    /// keywords that word begins.
    pub fn help(&self, text: &str) -> Vec<String> {
        match self.after_do(text) {
            Some(rest) => self.help_in("privileged", rest),
            None => self.help_in(self.mode(), text),
        }
    }

    /// The text after a configuration line's `do` and a space, which EXEC reads: `?` and Tab
    /// answer it as the privileged prompt would.
    fn after_do<'t>(&self, text: &'t str) -> Option<&'t str> {
        let toks = cli::tokens(text);
        let first = toks.first()?;
        let spaced = text.len() > first.at + first.text.len();
        (cli::is_config(self.mode()) && first.text.eq_ignore_ascii_case("do") && spaced)
            .then(|| &text[toks.get(1).map_or(text.len(), |t| t.at)..])
    }

    /// What may follow the whole tokens `done` at `mode`, and whether the line is complete there; at
    /// `>`, no command `privilege exec level` raised above it.
    fn offered_at(&self, mode: &str, done: &[cli::Tok]) -> (Vec<Offer>, bool) {
        let (next, mut complete) = self.engine.next(mode, done);
        let mut offered = self.offered(&next);
        if done.is_empty() && mode == "exec" {
            offered.retain(|o| !self.above_exec(&[o.shown.to_lowercase()]));
        }
        // `terminal length` and `terminal width`, which the shell takes (`terminal`): both words,
        // then the range, described as the books describe the length's value where they do
        let terminal = |t: &cli::Tok| {
            matches!(mode, "exec" | "privileged")
                && !self.above_exec(&["terminal".to_string()])
                && self.names(&t.text.to_lowercase(), "terminal", &[mode])
        };
        match done {
            [t] if terminal(t) => {
                for w in ["length", "width"] {
                    if !offered.iter().any(|o| o.shown == w) {
                        offered.push(Offer {
                            shown: w.to_string(),
                            help: String::new(),
                            kind: Offered::Keyword,
                        });
                    }
                }
            }
            [t, w] if terminal(t) && w.text.len() >= 2 => {
                let w = w.text.to_lowercase();
                if "length".starts_with(&w) || "width".starts_with(&w) {
                    let help = offered
                        .iter()
                        .find(|o| o.kind == Offered::Placeholder)
                        .map_or(String::new(), |o| o.help.clone());
                    offered = vec![Offer {
                        shown: "<0-512>".into(),
                        help,
                        kind: Offered::Placeholder,
                    }];
                    complete = false;
                }
            }
            _ => {}
        }
        (offered, complete)
    }

    /// `help` at `mode`.
    fn help_in(&self, mode: &str, text: &str) -> Vec<String> {
        // after a command raised above `>`'s level, at `>` (c9200 17.15 capture 27's `show ?`)
        let typed: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
        if text.ends_with(char::is_whitespace)
            && typed.first().is_some_and(|w| {
                self.raised()
                    .iter()
                    .any(|r| r.len() == 1 && r[0].starts_with(w.as_str()))
            })
            && mode == "exec"
        {
            return vec!["% Unrecognized command".into()];
        }
        let key = (mode.to_string(), crate::live::help_key(text));
        if let Some(lines) = self.engine.held.help.get(&key) {
            return lines.clone();
        }
        let toks = crate::cli::tokens(text);
        let whole = text.is_empty() || text.ends_with(char::is_whitespace);
        let done = if whole {
            &toks[..]
        } else {
            &toks[..toks.len() - 1]
        };
        let (offered, complete) = self.offered_at(mode, done);
        let unknown = vec!["% Unrecognized command".to_string()];
        if !whole {
            let typed = toks.last().map_or(String::new(), |t| t.text.to_lowercase());
            let words: BTreeSet<String> = offered
                .into_iter()
                .filter(|o| {
                    o.kind != Offered::Placeholder && o.shown.to_lowercase().starts_with(&typed)
                })
                .map(|o| o.shown)
                .collect();
            if words.is_empty() {
                return unknown;
            }
            // the words across the line, two spaces apart, wrapped before the console's last column
            let limit = self.width.saturating_sub(1);
            let mut out = vec![String::new()];
            for w in words {
                let n = out.len() - 1;
                if !out[n].is_empty() && out[n].len() + 2 + w.len() > limit {
                    out.push(String::new());
                }
                let n = out.len() - 1;
                if !out[n].is_empty() {
                    out[n].push_str("  ");
                }
                out[n].push_str(&w);
            }
            out.push(String::new());
            return out;
        }
        // a keyword the book leaves bare takes what switches of the family printed beside it, else
        // what the other family's did (IOS and IOS XE both say `enable  Turn on privileged commands`)
        let at = crate::live::help_key(text);
        let rows: BTreeMap<String, String> = offered
            .into_iter()
            .map(|o| {
                let help = if o.help.is_empty() {
                    let mode = crate::live::help_mode(self.mode()).to_string();
                    let key = (mode, at.clone(), o.shown.to_lowercase());
                    self.engine
                        .held
                        .described
                        .into_iter()
                        .chain(self.engine.held.described_other)
                        .find_map(|d| d.get(&key))
                        .cloned()
                        .unwrap_or_default()
                } else {
                    o.help
                };
                (o.shown, help)
            })
            .collect();
        if rows.is_empty() && !complete {
            return unknown;
        }
        // a show line that may end here takes an output modifier; at an IOS XE EXEC prompt `<cr>` is
        // described as itself, in the keywords' column, where classic IOS leaves it bare
        // (c9200 17.6 capture 1 at 9200 17.6, the C2960G's `show vlan ?`)
        let classic =
            crate::release::Release::parse(&self.device.release).is_some_and(|r| r.classic());
        let exec = matches!(mode, "exec" | "privileged");
        let show = done
            .first()
            .is_some_and(|t| self.names(&t.text.to_lowercase(), "show", &[mode]));
        let mut rows: Vec<(String, String)> = rows.into_iter().collect();
        if complete && exec && show {
            rows.push(("|".into(), "Output modifiers".into()));
        }
        let described = complete && exec && !classic;
        if described {
            rows.push(("<cr>".into(), "<cr>".into()));
        }
        let width = rows.iter().map(|(w, _)| w.len()).max().unwrap_or(0);
        let limit = self.width.saturating_sub(1);
        let mut out: Vec<String> = rows
            .iter()
            .flat_map(|(w, h)| wrapped(&format!("  {w:<width$}  "), h, limit))
            .collect();
        if complete && !described {
            out.push("  <cr>".into());
        }
        // classic IOS heads the EXEC commands (c2960x 15.2(7)E capture 7's `>?`)
        if text.trim().is_empty() && exec && classic {
            out.insert(0, "Exec commands:".into());
        }
        out.push(String::new());
        out
    }

    /// The line Tab makes of `text`: its last word completed to the one keyword or interface type it
    /// begins (a whole keyword is taken as itself), or none when it begins none or several.
    pub fn complete(&self, text: &str) -> Option<String> {
        match self.after_do(text) {
            Some(rest) => {
                let head = &text[..text.len() - rest.len()];
                self.complete_in("privileged", rest)
                    .map(|done| format!("{head}{done}"))
            }
            None => self.complete_in(self.mode(), text),
        }
    }

    /// `complete` at `mode`.
    fn complete_in(&self, mode: &str, text: &str) -> Option<String> {
        if text.is_empty() || text.ends_with(char::is_whitespace) {
            return None;
        }
        let toks = crate::cli::tokens(text);
        let last = toks.last()?;
        let (offered, _) = self.offered_at(mode, &toks[..toks.len() - 1]);
        let typed = last.text.to_lowercase();
        let kws: Vec<&String> = offered
            .iter()
            .filter(|o| o.kind == Offered::Keyword && o.shown.to_lowercase().starts_with(&typed))
            .map(|o| &o.shown)
            .collect();
        let head = &text[..last.at];
        let pick = kws
            .iter()
            .find(|k| k.to_lowercase() == typed)
            .or(if kws.len() == 1 { kws.first() } else { None });
        if let Some(k) = pick {
            return Some(format!("{head}{k} "));
        }
        let interface = offered.iter().any(|o| o.kind == Offered::Type);
        if !kws.is_empty() || !interface {
            return None;
        }
        // `twe` or `twe1/0/1`: the type written out, a space after a whole name
        let split = last
            .text
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(last.text.len());
        let (ty, num) = last.text.split_at(split);
        let ty = ty.to_lowercase();
        let types: Vec<String> = self
            .interface_types()
            .into_iter()
            .filter(|t| t.to_lowercase().starts_with(&ty))
            .collect();
        let full = types
            .iter()
            .find(|t| t.to_lowercase() == ty)
            .or(if types.len() == 1 {
                types.first()
            } else {
                None
            })?;
        let space = if num.is_empty() { "" } else { " " };
        Some(format!("{head}{full}{num}{space}"))
    }

    /// Writes an accepted configuration line where the current mode keeps it.
    fn write(&mut self, p: &Parse) {
        let top = self
            .frames
            .last()
            .cloned()
            .unwrap_or_else(|| Frame::top("config"));
        // a line a real switch of this platform and release printed is kept as it printed it
        let printed = |d: &Device, b: Option<usize>| {
            let header = b.map_or("", |b| d.blocks[b].header.as_str());
            self.engine
                .held
                .has(&crate::live::kind(header), &p.canonical())
        };
        let whole = self.whole(&top.mode, p);
        if top.blocks.is_empty() {
            let kept = printed(&self.device, None);
            self.device.apply_as(p, None, kept, whole);
            let key = crate::device::key(p);
            if let Some(i) = self.device.global.iter().position(|e| e.key == key) {
                if let Some(t) = self.as_printed("", &self.device.global[i].text) {
                    self.device.global[i].text = t;
                }
                let text = self.device.global[i].text.clone();
                self.wrote_top(&text);
            }
            return;
        }
        // a member's `channel-group <n>` makes its port-channel interface, as IOS does; the interface
        // stays when its members leave
        if !p.negated
            && top
                .blocks
                .iter()
                .any(|&b| self.device.blocks[b].header.starts_with("interface "))
            && let Some(n) = p
                .canonical()
                .strip_prefix("channel-group ")
                .and_then(|r| r.split_whitespace().next())
                .filter(|n| n.chars().all(|c| c.is_ascii_digit()))
        {
            self.device.block(&format!("interface Port-channel{n}"));
        }
        for &b in &top.blocks {
            let kept = printed(&self.device, Some(b));
            if top.prefix.is_empty() {
                self.device.apply_as(p, Some(b), kept, whole);
                let key = crate::device::key(p);
                let header = self.device.blocks[b].header.clone();
                let kind = crate::live::kind(&header);
                if let Some(i) = self.device.blocks[b]
                    .lines
                    .iter()
                    .position(|e| e.key == key)
                {
                    if let Some(t) = self.as_printed(&kind, &self.device.blocks[b].lines[i].text) {
                        self.device.blocks[b].lines[i].text = t;
                    }
                    // indented as switches print it: a block prints its lines one space in, an IOS XE
                    // class map its `description` two (c9200 17.12 capture 1)
                    let text = self.device.blocks[b].lines[i].text.trim_start().to_string();
                    if let Some(indent) = self.engine.held.indent_of(&header, &text) {
                        let indent = indent.strip_prefix(' ').unwrap_or(indent);
                        self.device.blocks[b].lines[i].text = format!("{indent}{text}");
                    }
                }
            } else {
                let prefix = top.prefix.as_str();
                let lines = &mut self.device.blocks[b].lines;
                // within this section only: a policy map's classes each set their own `priority
                // level` (c9300 17.15 capture 1)
                let start = top
                    .section
                    .as_ref()
                    .and_then(|s| lines.iter().rposition(|e| &e.key == s))
                    .map_or(0, |i| i + 1);
                let depth = |s: &str| s.len() - s.trim_start().len();
                let end = lines[start..]
                    .iter()
                    .position(|e| depth(&e.text) < prefix.len())
                    .map_or(lines.len(), |i| start + i);
                // the section's lines as a block's, its indentation off, changed as a block's are
                let strip = |s: String| s.strip_prefix(prefix).map_or(s.clone(), str::to_string);
                let mut section: Vec<Entry> = lines
                    .drain(start..end)
                    .map(|e| Entry {
                        key: strip(e.key),
                        text: strip(e.text),
                    })
                    .collect();
                crate::device::apply_to(&mut section, p, crate::device::key(p), kept, whole);
                let back = section.into_iter().map(|e| Entry {
                    key: format!("{prefix}{}", e.key),
                    text: format!("{prefix}{}", e.text),
                });
                lines.splice(start..start, back);
            }
        }
    }

    /// Whether a line names a whole command: a `no` line the books take without its `no` here
    /// (`no ip dhcp snooping`, where `no spanning-tree bpduguard` names part of one).
    fn whole(&self, mode: &str, p: &Parse) -> bool {
        if !p.negated {
            return true;
        }
        let c = p.canonical();
        let positive = c.strip_prefix("no ").unwrap_or(&c);
        matches!(self.parse(mode, positive), Outcome::Ok(_))
    }

    /// The line being typed as a real switch printed it, where its spacing is the switch's own and the
    /// stored text the same words (`snmp-server host 192.0.2.147 <removed>  snmp`, c2960 15.2(6)E capture 5).
    fn as_printed(&self, kind: &str, stored: &str) -> Option<String> {
        let raw = self.raw.trim();
        // the same words, but for spacing or a keyword's case (`RO`, where a book writes `ro`)
        let words = |s: &str| {
            s.split_whitespace()
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        };
        (raw != stored && words(raw) == words(stored) && self.engine.held.has(kind, raw))
            .then(|| raw.to_string())
    }

    /// Types every line of a script, echoing each at its prompt as a terminal session shows it.
    /// Returns the transcript and the number of lines refused.
    pub fn script(&mut self, text: &str) -> (Vec<String>, usize) {
        let mut out = Vec::new();
        let mut refused = 0;
        for line in text.lines() {
            if self.closed {
                break;
            }
            let l = line.trim_end();
            // a banner's text is typed as it is, `!` and `?` included
            let text = self.in_banner();
            if !text && l.trim_start().starts_with('!') && cli::is_config(self.mode()) {
                continue;
            }
            out.push(format!("{}{}", self.prompt(), l));
            // `?` prints help for the line before it, which is not entered
            if let Some(before) = l.strip_suffix('?').filter(|_| !text) {
                out.extend(self.help(before));
                continue;
            }
            let (o, r) = self.type_line(l);
            if r == Result::Error {
                refused += 1;
            }
            out.extend(o);
        }
        (out, refused)
    }
}

/// A `?` row: `head`, then its help, wrapped between words under its own column where a line would
/// pass `limit` columns (c9200 17.6 capture 1's `stats  Show interface packets & octets, ...` at 80).
fn wrapped(head: &str, help: &str, limit: usize) -> Vec<String> {
    let column = head.chars().count();
    let mut out = Vec::new();
    let mut line = head.to_string();
    let mut words = 0;
    for word in help.split_whitespace() {
        if words > 0 && line.chars().count() + 1 + word.chars().count() > limit {
            out.push(std::mem::replace(&mut line, " ".repeat(column)));
            words = 0;
        }
        if words > 0 {
            line.push(' ');
        }
        line.push_str(word);
        words += 1;
    }
    out.push(line.trim_end().to_string());
    out
}

/// Whether a block is one IOS opens from global configuration whatever mode it is typed in.
fn top_block(header: &str) -> bool {
    [
        "interface ",
        "vlan ",
        "line ",
        "router ",
        "vrf definition ",
        "ip vrf ",
        "route-map ",
        "class-map ",
        "policy-map ",
        "ip access-list ",
    ]
    .iter()
    .any(|k| header.starts_with(k))
}

/// The blocks a submode-entering line opens: `interface TwentyFiveGigE1/0/1`, `vlan 459`, one per
/// member of an interface range; an interface as IOS names it, its type and number joined
/// (`interface Port-channel11` for `interface port-channel 11`, `interface Vlan20` for
/// `interface vlan 20`).
fn headers(p: &Parse) -> Vec<String> {
    let c = p.canonical();
    let c = c.trim_start_matches("no ");
    if let Some(r) = c.strip_prefix("interface range ") {
        return expand(r)
            .into_iter()
            .map(|i| format!("interface {i}"))
            .collect();
    }
    if let Some(rest) = c.strip_prefix("interface ") {
        let words: Vec<&str> = rest.split_whitespace().collect();
        if let Some((full, num, used)) = words
            .first()
            .and_then(|w| cli::interface_name(w, words.get(1).copied()))
            && used == words.len()
        {
            return vec![format!("interface {full}{num}")];
        }
    }
    // IOS prints the console line `line con 0` (the cores' running configurations, the guides'
    // `show run | sec line` examples)
    if let Some(n) = c.strip_prefix("line console ") {
        return vec![format!("line con {n}")];
    }
    vec![c.to_string()]
}

/// The interfaces an interface or range value names: `TwentyFiveGigE1/0/11 - 12 , Hu1/1/1` gives
/// TwentyFiveGigE1/0/11, TwentyFiveGigE1/0/12 and HundredGigE1/1/1; `Vlan10 - 11` Vlan10 and
/// Vlan11. A segment naming more than any range can is kept as written, which no stack has.
fn expand(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    for seg in value.split(" , ") {
        match span(seg) {
            Some((stem, sep, a, b)) if a <= b && b - a < MOST => {
                out.extend((a..=b).map(|n| format!("{stem}{sep}{n}")));
            }
            _ => out.push(seg.trim().to_string()),
        }
    }
    out
}

/// The most interfaces one range names: every VLAN interface (`interface range vlan 1 - 4094`).
const MOST: u32 = 4094;

/// A range segment's stem, the separator before its number, and its first and last numbers: a
/// port's after its slot (`GigabitEthernet1/0/1 - 4`), a slotless interface's after its type
/// (`Vlan10 - 11`, `Port-channel1 - 2`).
fn span(seg: &str) -> Option<(&str, &str, u32, u32)> {
    let (first, last) = seg.split_once(" - ")?;
    let first = first.trim();
    let (stem, sep, n) = match first.rsplit_once('/') {
        Some((stem, n)) => (stem, "/", n),
        None => {
            let at = first.trim_end_matches(|c: char| c.is_ascii_digit()).len();
            (&first[..at], "", &first[at..])
        }
    };
    Some((stem, sep, n.parse().ok()?, last.trim().parse().ok()?))
}

/// Whether the stack has every interface a value names: a range's ends first, so a mistyped end
/// (`Gi1/0/1 - 4294967295`) is refused without naming each port between.
fn on_stack(value: &str, stack: &crate::platform::Stack) -> bool {
    value.split(" , ").all(|seg| match span(seg) {
        Some((stem, sep, a, b)) => {
            let name = |n: u32| format!("{stem}{sep}{n}");
            stack.has(&name(a)) && stack.has(&name(b)) && expand(seg).iter().all(|n| stack.has(n))
        }
        None => stack.has(seg.trim()),
    })
}

fn looks_like_interface(v: &str) -> bool {
    crate::cli::interface_types().iter().any(|(full, _)| {
        v.starts_with(full) && v[full.len()..].starts_with(|c: char| c.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{Command, Grammar, Line};

    fn engine(lines: &[&str]) -> Engine {
        Engine::new(Grammar {
            release: "t".into(),
            platform: "t".into(),
            source: "t".into(),
            transitions: vec![],
            commands: lines
                .iter()
                .map(|l| Command {
                    name: l.to_string(),
                    source: "test".into(),
                    modes: vec!["config".into()],
                    lines: vec![Line {
                        text: l.to_string(),
                        node: crate::syntax::parse(l).0,
                        negated: false,
                        repairs: vec![],
                    }],
                    args: Default::default(),
                    examples: vec![],
                    enters: None,
                    evidence: None,
                    help: Default::default(),
                })
                .collect(),
        })
    }

    #[test]
    fn a_line_opens_the_submode_its_own_form_enters() {
        // one command for both lists, as the classic books document it; their examples enter each
        // list's own submode
        let mut g = engine(&["ip access-list {standard | extended} <name>"]).grammar;
        let mut entry = g.commands[0].clone();
        entry.name = "remark".into();
        entry.modes = vec!["config-ext-nacl".into(), "config-std-nacl".into()];
        entry.lines = vec![Line {
            text: "remark <text>".into(),
            node: crate::syntax::parse("remark <text>").0,
            negated: false,
            repairs: vec![],
        }];
        g.commands.push(entry);
        let t = |line: &str, to: &str| crate::compile::Transition {
            from: "config".into(),
            line: line.into(),
            to: to.into(),
        };
        g.transitions = vec![
            t("ip access-list extended a", "config-ext-nacl"),
            t("ip access-list extended b", "config-ext-nacl"),
            t("ip access-list standard c", "config-std-nacl"),
            t("ip access-list standard d", "config-std-nacl"),
        ];
        let e = Engine::new(g);
        let stack = crate::platform::parse_stack("C9300X-24Y").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.frames.push(Frame::top("config"));
        assert_eq!(s.type_line("ip access-list standard S").1, Result::Ok);
        assert_eq!(s.mode(), "config-std-nacl");
        s.type_line("exit");
        assert_eq!(s.type_line("ip access-list extended T").1, Result::Ok);
        assert_eq!(s.mode(), "config-ext-nacl");
    }

    #[test]
    fn a_comment_line_does_nothing_in_any_mode() {
        let e = engine(&["hostname <name>"]);
        let stack = crate::platform::parse_stack("C9300X-24Y").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        for line in ["!", "! a comment", "  !"] {
            assert_eq!(s.type_line(line), (vec![], Result::Ok), "exec {line:?}");
        }
        s.frames.push(Frame::top("config"));
        for line in ["!", " !", "!hostname x"] {
            assert_eq!(s.type_line(line), (vec![], Result::Ok), "config {line:?}");
            assert_eq!(s.mode(), "config");
        }
        assert!(s.device.global.is_empty(), "{:?}", s.device.global);
    }

    #[test]
    fn logout_and_quit_end_an_exec_session_and_nothing_in_configuration() {
        let e = engine(&["hostname <name>"]);
        let stack = || crate::platform::parse_stack("C9300X-24Y").unwrap();
        for word in ["logout", "quit", "QUIT"] {
            let mut s = Session::new(&e, Device::new(stack()));
            assert_eq!(s.type_line(word).1, Result::Ok);
            assert!(s.closed, "{word}");
        }
        let mut s = Session::new(&e, Device::new(stack()));
        s.frames.push(Frame::top("config"));
        assert_eq!(s.type_line("quit").1, Result::Error);
        assert!(!s.closed);
    }

    #[test]
    fn a_keyword_the_books_spell_in_two_cases_is_offered_in_lowercase() {
        // a book's `IP` comes before the `ip` IOS prints
        let e = engine(&["IP route", "ip address", "ipv6 address"]);
        let stack = crate::platform::parse_stack("C9300X-24Y").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.frames.push(Frame::top("config"));
        assert_eq!(s.help("i"), vec!["ip  ipv6", ""]);
        assert_eq!(s.complete("IP").as_deref(), Some("ip "));
    }

    #[test]
    fn the_ssh_pubkey_chain_walks_its_conf_submodes() {
        let r = crate::release::Release::parse("17.15").unwrap();
        let e = Engine::new(crate::grammars::grammar("c9300", r).unwrap());
        let stack = crate::platform::parse_stack("C9300-48P").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.frames.push(Frame::top("config"));
        for (line, mode) in [
            ("ip ssh pubkey-chain", "conf-ssh-pubkey"),
            ("username claude", "conf-ssh-pubkey-user"),
            ("key-hash ssh-rsa 0123456789ABCDEF", "conf-ssh-pubkey-user"),
            ("key-string", "conf-ssh-pubkey-data"),
            ("exit", "conf-ssh-pubkey-user"),
            ("exit", "conf-ssh-pubkey"),
            ("exit", "config"),
            // the global username takes no submode
            ("username bob privilege 15", "config"),
        ] {
            s.type_line(line);
            assert_eq!(s.mode(), mode, "after {line}");
        }
    }

    #[test]
    fn a_line_the_books_take_is_indented_as_switches_of_its_release_printed_it() {
        // a class map's `description`: two spaces in at 17.12 (c9200 17.12 capture 1), one at 17.15
        // (c9200 17.15 capture 5), for a class map no capture holds
        for (release, indent) in [("17.12", "  "), ("17.15", " ")] {
            let r = crate::release::Release::parse(release).unwrap();
            let e = Engine::new(crate::grammars::grammar("c9200", r).unwrap());
            let stack = crate::platform::parse_stack("C9200-24P").unwrap();
            let mut s = Session::new(&e, Device::new(stack));
            s.frames.push(Frame::top("config"));
            for line in ["class-map match-any lab-class", "description lab traffic"] {
                assert_eq!(s.type_line(line).1, Result::Ok, "{release}: {line}");
            }
            let shown = crate::show::configuration(&s.device);
            let at = shown
                .iter()
                .position(|l| l == "class-map match-any lab-class")
                .unwrap_or_else(|| panic!("{release}: {shown:?}"));
            assert_eq!(shown[at + 1], format!("{indent}description lab traffic"));
        }
    }

    #[test]
    fn a_certificate_s_hex_is_read_until_quit_and_prints_as_typed() {
        // a 9300 at 17.15, no capture of whose chains vswitch holds
        let r = crate::release::Release::parse("17.15").unwrap();
        let e = Engine::new(crate::grammars::grammar("c9300", r).unwrap());
        let stack = crate::platform::parse_stack("C9300-48P").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.frames.push(Frame::top("config"));
        let chain = [
            "crypto pki certificate chain TP-self-signed-42",
            " certificate self-signed 01",
            "  3082032E 30820216 A0030201 02020101",
            "  D697DF7F 28",
            "  \tquit",
        ];
        for (n, line) in chain.iter().enumerate() {
            assert_eq!(s.type_line(line).1, Result::Ok, "line {n}: {line}");
        }
        assert_eq!(s.mode(), "config-cert-chain");
        // the next line is itself again
        assert_eq!(s.type_line("hostname lab42").1, Result::Ok);
        let shown = crate::show::configuration(&s.device);
        let at = shown
            .iter()
            .position(|l| l == chain[0])
            .unwrap_or_else(|| panic!("{shown:?}"));
        assert_eq!(shown[at..at + chain.len()], chain);
        assert!(shown.contains(&"hostname lab42".to_string()));
        // a trustpoint's lines in its own mode
        for line in [
            "crypto pki trustpoint TP-self-signed-42",
            " enrollment selfsigned",
            " revocation-check none",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        assert_eq!(s.mode(), "ca-trustpoint");
    }

    #[test]
    fn a_keyword_its_book_leaves_bare_is_described_as_switches_described_it() {
        // no 9300 at 17.15 was asked `show ?`; the 9200s were
        let r = crate::release::Release::parse("17.15").unwrap();
        let e = Engine::new(crate::grammars::grammar("c9300", r).unwrap());
        let stack = crate::platform::parse_stack("C9300-48P").unwrap();
        let s = Session::new(&e, Device::new(stack));
        let row = |text: &str, word: &str| {
            s.help(text)
                .into_iter()
                .find(|l| l.split_whitespace().next() == Some(word))
                .unwrap_or_default()
        };
        assert!(row("show ", "aaa").ends_with("Show AAA values"));
        // at `>`, from the classic switches' answers where no IOS XE switch was asked
        assert!(row("", "enable").ends_with("Turn on privileged commands"));
        // an EXEC answer describes no configuration mode's keyword (c2960x 15.2(7)E capture 7's `>?` says
        // `exit  Exit from the EXEC`; at `(config)#` IOS leaves EXEC)
        let r = crate::release::Release::parse("15.2(7)E").unwrap();
        let e = Engine::new(crate::grammars::grammar("c2960x", r).unwrap());
        let stack = crate::platform::parse_stack("WS-C2960X-48FPD-L").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.type_line("enable");
        assert!(s.help("").iter().any(|l| l.ends_with("Exit from the EXEC")));
        s.type_line("configure terminal");
        let exit = s
            .help("")
            .into_iter()
            .find(|l| l.split_whitespace().next() == Some("exit"));
        assert!(
            !exit
                .as_deref()
                .unwrap_or("")
                .ends_with("Exit from the EXEC"),
            "{exit:?}"
        );
    }

    #[test]
    fn a_command_kept_once_per_instance_keeps_each_line_typed() {
        let r = crate::release::Release::parse("15.2(7)E").unwrap();
        let e = Engine::new(crate::grammars::grammar("c2960x", r).unwrap());
        let stack = crate::platform::parse_stack("WS-C2960X-48FPD-L").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.frames.push(Frame::top("config"));
        // c2960x 15.2(7)E capture 9 peers with two servers; an SVI relays DHCP to two
        for line in [
            "ntp peer 192.0.2.121",
            "ntp peer 192.0.2.122",
            "interface Vlan10",
            "ip helper-address 192.0.2.5",
            "ip helper-address 192.0.2.6",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        let shown = crate::show::configuration(&s.device);
        for line in [
            "ntp peer 192.0.2.121",
            "ntp peer 192.0.2.122",
            " ip helper-address 192.0.2.5",
            " ip helper-address 192.0.2.6",
        ] {
            assert!(shown.contains(&line.to_string()), "{line}: {shown:?}");
        }
    }

    /// A 9300 at 17.15, its books' grammar.
    fn c9300() -> Engine {
        let r = crate::release::Release::parse("17.15").unwrap();
        Engine::new(crate::grammars::grammar("c9300", r).unwrap())
    }

    /// A C9300-48P in global configuration with each line typed, every one taken.
    fn configured<'a>(e: &'a Engine, lines: &[&str]) -> Session<'a> {
        let stack = crate::platform::parse_stack("C9300-48P").unwrap();
        let mut s = Session::new(e, Device::new(stack));
        for line in ["enable", "configure terminal"].iter().chain(lines) {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Ok, "{line}: {out:?}");
        }
        s
    }

    /// The running configuration's lines that start with one of `starts`, past their indentation.
    fn shown(s: &Session, starts: &[&str]) -> Vec<String> {
        crate::show::configuration(&s.device)
            .into_iter()
            .filter(|l| starts.iter().any(|w| l.trim_start().starts_with(w)))
            .collect()
    }

    #[test]
    fn a_later_value_of_a_setting_replaces_the_earlier() {
        let e = c9300();
        let s = configured(
            &e,
            &[
                "spanning-tree mode rapid-pvst",
                "spanning-tree mode mst",
                "vtp mode transparent",
                "vtp mode off",
                "interface GigabitEthernet1/0/1",
                "switchport mode access",
                "switchport mode trunk",
                "duplex half",
                "duplex full",
                "speed 100",
                "speed 10",
            ],
        );
        assert_eq!(
            shown(
                &s,
                &[
                    "spanning-tree mode",
                    "vtp mode",
                    "switchport mode",
                    "duplex",
                    "speed"
                ]
            ),
            // in the order the captures print them
            [
                "vtp mode off",
                "spanning-tree mode mst",
                " switchport mode trunk",
                " speed 10",
                " duplex full"
            ]
        );
    }

    #[test]
    fn an_instance_is_named_by_its_arguments_not_by_what_it_sets() {
        let e = c9300();
        let mut s = configured(
            &e,
            &[
                "username admin privilege 15 secret 0 INVENTED1",
                "username admin secret 0 INVENTED2",
                "username ops secret 0 INVENTED3",
                "snmp-server community INVENTEDC RO",
                "snmp-server community INVENTEDC RW",
                "ntp server 10.0.0.1",
                "ntp server 10.0.0.1 prefer",
                "ntp server 10.0.0.2",
                "spanning-tree vlan 10 priority 4096",
                "spanning-tree vlan 10 priority 8192",
                "spanning-tree vlan 10 forward-time 10",
                "ip route 10.1.0.0 255.255.0.0 10.0.0.254",
                "ip route 10.1.0.0 255.255.0.0 10.0.0.254 5",
                "ip route 10.1.0.0 255.255.0.0 10.0.0.253",
            ],
        );
        let starts = [
            "username",
            "snmp-server community",
            "ntp server",
            "spanning-tree vlan",
            "ip route",
        ];
        // in the order the captures print them
        assert_eq!(
            shown(&s, &starts),
            [
                "spanning-tree vlan 10 priority 8192",
                "spanning-tree vlan 10 forward-time 10",
                "username admin secret 0 INVENTED2",
                "username ops secret 0 INVENTED3",
                "ip route 10.1.0.0 255.255.0.0 10.0.0.254 5",
                "ip route 10.1.0.0 255.255.0.0 10.0.0.253",
                "snmp-server community INVENTEDC RW",
                "ntp server 10.0.0.1 prefer",
                "ntp server 10.0.0.2",
            ]
        );
        // `no` naming the instance removes it, whatever it sets
        for line in [
            "no username admin",
            "no snmp-server community INVENTEDC",
            "no ntp server 10.0.0.1",
            "no spanning-tree vlan 10 priority",
            "no ip route 10.1.0.0 255.255.0.0 10.0.0.254",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        assert_eq!(
            shown(&s, &starts),
            [
                "spanning-tree vlan 10 forward-time 10",
                "username ops secret 0 INVENTED3",
                "ip route 10.1.0.0 255.255.0.0 10.0.0.253",
                "ntp server 10.0.0.2",
            ]
        );
    }

    #[test]
    fn no_turns_off_its_own_command_and_leaves_the_longer_ones() {
        let e = c9300();
        let s = configured(
            &e,
            &[
                "ip dhcp snooping vlan 10",
                "ip dhcp snooping",
                "no ip dhcp snooping",
                "interface GigabitEthernet1/0/1",
                "switchport mode access",
                "switchport port-security maximum 3",
                "switchport port-security violation restrict",
                "switchport port-security",
                "no switchport port-security",
                "spanning-tree bpduguard enable",
                "no spanning-tree bpduguard",
            ],
        );
        assert_eq!(
            shown(
                &s,
                &[
                    "ip dhcp snooping",
                    "switchport port-security",
                    "spanning-tree bpduguard"
                ]
            ),
            [
                "ip dhcp snooping vlan 10",
                " switchport port-security maximum 3",
                " switchport port-security violation restrict"
            ]
        );
    }

    #[test]
    fn no_naming_an_instance_s_first_words_removes_every_instance_they_begin() {
        let e = c9300();
        let mut s = configured(
            &e,
            &[
                "ip prefix-list LAN seq 5 permit 10.0.0.0/8",
                "ip prefix-list LAN seq 10 permit 192.168.0.0/16",
                "ip prefix-list WAN seq 5 permit 0.0.0.0/0",
            ],
        );
        let (out, r) = s.type_line("no ip prefix-list LAN");
        assert_eq!(r, Result::Ok, "{out:?}");
        assert_eq!(
            shown(&s, &["ip prefix-list"]),
            ["ip prefix-list WAN seq 5 permit 0.0.0.0/0"]
        );
    }

    #[test]
    fn each_neighbor_and_network_keeps_its_own_lines() {
        let e = c9300();
        let s = configured(
            &e,
            &[
                "router bgp 65000",
                "neighbor 10.0.0.1 remote-as 65001",
                "neighbor 10.0.0.2 remote-as 65002",
                "address-family ipv4",
                "neighbor 10.0.0.1 activate",
                "neighbor 10.0.0.2 activate",
                "network 10.1.0.0 mask 255.255.0.0",
                "network 10.2.0.0 mask 255.255.0.0",
                "exit-address-family",
                "exit",
                "router ospf 1",
                "network 10.1.0.0 0.0.0.255 area 1",
                "network 10.2.0.0 0.0.0.255 area 1",
            ],
        );
        let shown = crate::show::configuration(&s.device);
        let at = shown
            .iter()
            .position(|l| l == "router bgp 65000")
            .unwrap_or_else(|| panic!("{shown:?}"));
        assert_eq!(
            shown[at..at + 14],
            [
                "router bgp 65000",
                " neighbor 10.0.0.1 remote-as 65001",
                " neighbor 10.0.0.2 remote-as 65002",
                " address-family ipv4",
                "  neighbor 10.0.0.1 activate",
                "  neighbor 10.0.0.2 activate",
                "  network 10.1.0.0 mask 255.255.0.0",
                "  network 10.2.0.0 mask 255.255.0.0",
                " exit-address-family",
                "!",
                "router ospf 1",
                " network 10.1.0.0 0.0.0.255 area 1",
                " network 10.2.0.0 0.0.0.255 area 1",
                "!"
            ]
        );
    }

    /// The block whose header is `header`, as the running configuration prints it, through its `!`.
    fn block_of(s: &Session, header: &str) -> Vec<String> {
        let shown = crate::show::configuration(&s.device);
        let at = shown
            .iter()
            .position(|l| l == header)
            .unwrap_or_else(|| panic!("{header}: {shown:?}"));
        let end = shown[at..].iter().position(|l| l == "!").unwrap_or(0);
        shown[at..at + end].to_vec()
    }

    #[test]
    fn no_in_a_section_removes_the_line_it_negates() {
        let e = c9300();
        let s = configured(
            &e,
            &[
                "router bgp 65000",
                "address-family ipv4",
                "network 10.1.0.0 mask 255.255.0.0",
                "network 10.2.0.0 mask 255.255.0.0",
                "no network 10.1.0.0 mask 255.255.0.0",
                "no network 10.3.0.0 mask 255.255.0.0",
                "exit-address-family",
            ],
        );
        assert_eq!(
            block_of(&s, "router bgp 65000"),
            [
                "router bgp 65000",
                " address-family ipv4",
                "  network 10.2.0.0 mask 255.255.0.0",
                " exit-address-family"
            ]
        );
    }

    #[test]
    fn a_section_closes_however_it_is_left_and_its_block_s_lines_print_first() {
        let e = c9300();
        let s = configured(
            &e,
            &[
                "router bgp 65000",
                "address-family ipv4",
                "neighbor 10.0.0.1 activate",
                "exit",
                "address-family ipv6",
                "neighbor 10.0.0.1 activate",
                "end",
                "configure terminal",
                "router bgp 65000",
                "bgp log-neighbor-changes",
                "neighbor 10.0.0.1 remote-as 65001",
                "vrf definition X",
                "address-family ipv4",
                "exit",
                "address-family ipv6",
                "end",
            ],
        );
        assert_eq!(
            block_of(&s, "router bgp 65000"),
            [
                "router bgp 65000",
                " bgp log-neighbor-changes",
                " neighbor 10.0.0.1 remote-as 65001",
                " address-family ipv4",
                "  neighbor 10.0.0.1 activate",
                " exit-address-family",
                " address-family ipv6",
                "  neighbor 10.0.0.1 activate",
                " exit-address-family"
            ]
        );
        assert_eq!(
            block_of(&s, "vrf definition X"),
            [
                "vrf definition X",
                " address-family ipv4",
                " exit-address-family",
                " address-family ipv6",
                " exit-address-family"
            ]
        );
    }

    #[test]
    fn an_interface_typed_in_another_block_s_mode_opens_its_own_block() {
        let e = c9300();
        let mut s = configured(
            &e,
            &[
                "vtp mode transparent",
                "vlan 10",
                "name users",
                "interface vlan 10",
                "ip address 10.0.0.1 255.255.255.0",
            ],
        );
        assert_eq!(s.mode(), "config-if");
        assert_eq!(s.type_line("exit").1, Result::Ok);
        assert_eq!(s.mode(), "config");
        for line in [
            "interface Vlan60",
            "description a",
            "interface port-channel 5",
            "description e",
            "interface GigabitEthernet1/0/2",
            "description b",
            "interface loopback 1",
            "description c",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        assert_eq!(block_of(&s, "vlan 10"), ["vlan 10", " name users"]);
        assert_eq!(
            block_of(&s, "interface Vlan10"),
            ["interface Vlan10", " ip address 10.0.0.1 255.255.255.0"]
        );
        assert_eq!(
            block_of(&s, "interface Vlan60"),
            ["interface Vlan60", " description a"]
        );
        assert_eq!(
            block_of(&s, "interface Port-channel5"),
            ["interface Port-channel5", " description e"]
        );
        assert_eq!(
            block_of(&s, "interface GigabitEthernet1/0/2"),
            ["interface GigabitEthernet1/0/2", " description b"]
        );
        assert_eq!(
            block_of(&s, "interface Loopback1"),
            ["interface Loopback1", " description c"]
        );
    }

    #[test]
    fn a_range_names_each_member() {
        assert_eq!(
            expand("TwentyFiveGigE1/0/11 - 12 , HundredGigE1/1/1"),
            vec![
                "TwentyFiveGigE1/0/11",
                "TwentyFiveGigE1/0/12",
                "HundredGigE1/1/1"
            ]
        );
        // a member with no slot counts on its own number
        assert_eq!(
            expand("Port-channel1 - 2 , Vlan10 - 11"),
            vec!["Port-channel1", "Port-channel2", "Vlan10", "Vlan11"]
        );
    }

    #[test]
    fn an_interface_is_one_block_however_its_name_is_spaced_and_must_be_on_the_stack() {
        let e = c9300();
        let mut s = configured(
            &e,
            &[
                "interface vlan 20",
                "description a",
                "interface Vlan20",
                "description b",
                "interface tunnel 1",
                "interface port-channel 2",
                "description bundle",
                "interface range Vlan20 - 21",
                "description svi",
            ],
        );
        assert_eq!(
            block_of(&s, "interface Vlan20"),
            ["interface Vlan20", " description svi"]
        );
        assert_eq!(
            block_of(&s, "interface Vlan21"),
            ["interface Vlan21", " description svi"]
        );
        assert_eq!(block_of(&s, "interface Tunnel1"), ["interface Tunnel1"]);
        assert_eq!(
            block_of(&s, "interface Port-channel2"),
            ["interface Port-channel2", " description bundle"]
        );
        // no block keeps a spaced name (`interface vlan 20`)
        assert!(
            s.device.blocks.iter().all(|b| b
                .header
                .strip_prefix("interface ")
                .is_none_or(|n| !n.contains(' '))),
            "{:?}",
            s.device.blocks
        );
        // VLAN interfaces run 1 to 4094, spaced or not
        for line in ["interface vlan 5000", "interface Vlan5000"] {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Error, "{line}");
            assert_eq!(out[1], "% Invalid input detected at '^' marker.", "{line}");
        }
    }

    #[test]
    fn a_range_with_an_end_the_stack_does_not_have_is_refused_without_naming_each_port() {
        let e = c9300();
        let mut s = configured(&e, &[]);
        for line in [
            "interface range GigabitEthernet1/0/1 - 100000",
            "interface range Vlan1 - 4095",
            // both ends exist, but no range names this many
            "interface range Loopback0 - 100000",
        ] {
            assert_eq!(s.type_line(line).1, Result::Error, "{line}");
        }
        assert_eq!(s.mode(), "config");
        assert!(s.device.blocks.is_empty(), "{}", s.device.blocks.len());
        // a range past the most any names stays as written, which no stack has
        assert_eq!(expand("Loopback0 - 100000"), ["Loopback0 - 100000"]);
        assert_eq!(expand("Vlan1 - 4094").len(), 4094);
    }

    #[test]
    fn free_text_that_begins_with_an_interface_name_is_text() {
        let e = c9300();
        let s = configured(
            &e,
            &[
                "snmp-server location Vlan10 closet",
                "interface GigabitEthernet1/0/1",
                "description GigabitEthernet1/0/49 to core",
                "exit",
                "interface GigabitEthernet1/0/2",
                "description Vlan10 uplink",
            ],
        );
        assert_eq!(
            shown(&s, &["snmp-server location", "description"]),
            [
                " description GigabitEthernet1/0/49 to core",
                " description Vlan10 uplink",
                "snmp-server location Vlan10 closet"
            ]
        );
    }

    /// The running configuration's banners, line by line.
    fn banners(s: &Session) -> Vec<String> {
        s.device
            .global
            .iter()
            .filter(|e| e.key.starts_with("banner "))
            .flat_map(|e| e.text.lines().map(str::to_string).collect::<Vec<_>>())
            .collect()
    }

    #[test]
    fn a_banner_prints_with_the_delimiter_a_running_configuration_prints() {
        let e = c9300();
        let s = configured(
            &e,
            &[
                "banner motd #",
                "Welcome",
                "#",
                "banner login #hello",
                "world# trailing",
                "banner exec #one line# after",
                "banner incoming ^Ckept^C",
            ],
        );
        assert_eq!(
            banners(&s),
            [
                "banner motd ^C",
                "Welcome",
                "^C",
                "banner login ^Chello",
                "world^C",
                "banner exec ^Cone line^C",
                "banner incoming ^Ckept^C"
            ]
        );
    }

    #[test]
    fn a_member_s_channel_group_makes_its_port_channel_and_a_missing_vlan_is_made() {
        let e = c9300();
        let mut s = configured(
            &e,
            &[
                "vtp mode transparent",
                "vlan 10",
                "interface GigabitEthernet1/0/1",
                "channel-group 5 mode active",
                "switchport access vlan 10",
            ],
        );
        // IOS makes the port-channel when its first member joins, and keeps it
        assert_eq!(s.type_line("no channel-group").1, Result::Ok);
        let run = show::running_config(&s.device);
        assert!(
            run.contains(&"interface Port-channel5".to_string()),
            "{run:?}"
        );
        // a VLAN the switch has makes nothing; one it lacks is made at the console
        assert_eq!(
            s.type_line("switchport access vlan 30").0,
            ["% Access VLAN does not exist. Creating vlan 30"]
        );
        assert_eq!(
            s.type_line("switchport voice vlan 40").0,
            ["% Voice VLAN does not exist. Creating vlan 40"]
        );
        assert!(s.type_line("switchport access vlan 30").0.is_empty());
        let run = show::running_config(&s.device);
        assert!(run.contains(&"vlan 30".to_string()) && run.contains(&"vlan 40".to_string()));
        // a replayed configuration defines its own VLANs
        let (replayed, _) = crate::check::type_config(
            &e,
            crate::platform::parse_stack("C9300-48P").unwrap(),
            "vtp mode transparent\ninterface GigabitEthernet1/0/2\n switchport access vlan 50\n",
        );
        assert!(!show::defined_vlans(&replayed.device).contains(&50));
    }

    #[test]
    fn a_banner_s_words_are_taken_as_ios_takes_them() {
        let e = c9300();
        let s = configured(
            &e,
            &[
                "ban login #",
                "hi",
                "#",
                "banner mo #",
                "hello",
                "#",
                "BANNER  EXEC  #x#",
            ],
        );
        assert_eq!(
            banners(&s),
            [
                "banner login ^C",
                "hi",
                "^C",
                "banner motd ^C",
                "hello",
                "^C",
                "banner exec ^Cx^C"
            ]
        );
    }

    #[test]
    fn a_script_types_a_banner_s_lines_as_they_are() {
        let e = c9300();
        let stack = crate::platform::parse_stack("C9300-48P").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        let (_, refused) = s.script(
            "enable\nconfigure terminal\nbanner motd ^C\n!!! AUTHORIZED ONLY !!!\n\nAre you allowed?\n^C\nhostname lab\n",
        );
        assert_eq!(refused, 0);
        assert!(!s.in_banner());
        assert_eq!(
            banners(&s),
            [
                "banner motd ^C",
                "!!! AUTHORIZED ONLY !!!",
                "",
                "Are you allowed?",
                "^C"
            ]
        );
        assert_eq!(s.device.hostname, "lab");
    }

    #[test]
    fn write_and_copy_save_at_every_platform_s_privileged_prompt() {
        for (book, release, model) in [
            ("c9300", "17.15", "C9300-48P"),
            ("c2960x", "15.2(7)E", "WS-C2960X-48FPD-L"),
            ("c2960", "15.0(2)SE", "WS-C2960G-24TC-L"),
        ] {
            let r = crate::release::Release::parse(release).unwrap();
            let e = Engine::new(crate::grammars::grammar(book, r).unwrap());
            let stack = crate::platform::parse_stack(model).unwrap();
            let mut s = Session::new(&e, Device::new(stack));
            s.type_line("enable");
            for (n, line) in ["wr", "write", "wr mem", "write memory"].iter().enumerate() {
                s.type_line("configure terminal");
                s.type_line(&format!("hostname lab{n}"));
                s.type_line("end");
                let (out, r) = s.type_line(line);
                assert_eq!(r, Result::Ok, "{model}: {line}: {out:?}");
                assert_eq!(
                    out,
                    ["Building configuration...", "[OK]"],
                    "{model}: {line}"
                );
                let kept = s.startup.clone().unwrap_or_default();
                assert!(
                    kept.contains(&format!("hostname lab{n}")),
                    "{model}: {line}"
                );
            }
            s.type_line("configure terminal");
            s.type_line("hostname copied");
            s.type_line("end");
            assert_eq!(s.type_line("copy run start").1, Result::Ok, "{model}");
            assert!(s.prompt().starts_with("Destination filename"), "{model}");
            assert_eq!(s.type_line("").0, ["Building configuration...", "[OK]"]);
            assert!(s.startup.unwrap().contains("hostname copied"), "{model}");
        }
    }

    /// The caret line's column, under a refused line.
    fn caret_at(out: &[String]) -> usize {
        let line = out
            .iter()
            .find(|l| l.trim_start() == "^")
            .unwrap_or_else(|| panic!("no caret: {out:?}"));
        line.chars().count() - 1
    }

    #[test]
    fn a_caret_stands_under_the_refused_word_of_the_line_as_typed() {
        let e = c9300();
        let mut s = configured(&e, &[]);
        // `Switch(config)#do shw run`: under the `w`
        assert_eq!(caret_at(&s.type_line("do shw run").0), 20);
        // past the `b` of `bogus`, which `brief` begins
        assert_eq!(caret_at(&s.type_line("do  show   vlan   bogus").0), 34);
        s.type_line("interface GigabitEthernet1/0/1");
        assert_eq!(caret_at(&s.type_line("do shw run").0), 23);
        s.type_line("end");
        assert_eq!(caret_at(&s.type_line("shw run").0), 9);
        // a modifier's word under `do` as under `#`, past `Switch(config)#do ` where `Switch#` was
        let at_exec = caret_at(&s.type_line("show running-config | bogus x").0);
        s.type_line("configure terminal");
        let under_do = caret_at(&s.type_line("do show running-config | bogus x").0);
        assert_eq!(under_do, at_exec + "(config)".len() + "do ".len());
        // columns, not bytes: `Zürich(config)#bogus` under the `g` past `bo`
        s.type_line("end");
        s.type_line("configure terminal");
        s.type_line("hostname Zürich");
        assert_eq!(caret_at(&s.type_line("bogus").0), 17);
        s.type_line("vlan 10");
        assert_eq!(caret_at(&s.type_line("name café x").0), 30);
    }

    #[test]
    fn do_hands_exec_the_line_as_typed_and_its_help() {
        let e = c9300();
        let mut s = configured(&e, &["interface Vlan10", "exit"]);
        // two spaces in the expression match nothing, as at EXEC
        assert_eq!(
            s.type_line("do show running-config | include interface  Vlan"),
            (vec![], Result::Ok)
        );
        assert_eq!(
            s.type_line("do show running-config | include interface Vlan10")
                .0,
            ["interface Vlan10"]
        );
        assert_eq!(s.help("do show vl"), ["vlan", ""]);
        assert!(
            s.help("do show vlan ")
                .iter()
                .any(|l| l.starts_with("  brief"))
        );
        assert_eq!(s.complete("do sh").as_deref(), Some("do show "));
        assert_eq!(s.mode(), "config");
    }

    #[test]
    fn only_lines_that_configure_are_taken_as_configuration() {
        let e = c9300();
        let s = configured(
            &e,
            &[
                "interface GigabitEthernet1/0/5",
                "description x",
                "exit",
                "do show vlan brief",
                "vlan 10",
                "name x",
                "exit",
                "router bgp 65000",
                "address-family ipv4",
                "exit-address-family",
                "end",
            ],
        );
        let lines: Vec<&str> = s.accepted.iter().map(|a| a.line.as_str()).collect();
        assert_eq!(
            lines,
            [
                "interface GigabitEthernet1/0/5",
                "description x",
                "vlan 10",
                "name x",
                "router bgp 65000",
                "address-family ipv4",
                "exit-address-family"
            ]
        );
    }

    #[test]
    fn a_block_real_switches_hold_opens_at_the_console_in_the_mode_the_books_name() {
        // every dprL2cok capture at 9200 17.15 holds `redundancy` / ` mode sso`; the book's line
        // enters no submode, and its one example enters `(config-red)`
        let r = crate::release::Release::parse("17.15").unwrap();
        let e = Engine::new(crate::grammars::grammar("c9200", r).unwrap());
        let stack = || crate::platform::parse_stack("C9200-48P").unwrap();
        for sub in ["mode sso", " mode sso"] {
            let mut s = Session::new(&e, Device::new(stack()));
            for line in ["enable", "configure terminal", "redundancy", sub] {
                let (out, r) = s.type_line(line);
                assert_eq!(r, Result::Ok, "{line:?}: {out:?}");
            }
            assert_eq!(s.mode(), "config-red", "{sub:?}");
            assert_eq!(block_of(&s, "redundancy"), ["redundancy", " mode sso"]);
        }
    }

    #[test]
    fn a_command_raised_above_the_user_level_is_neither_listed_nor_completed_there() {
        let e = c9300();
        let mut s = configured(
            &e,
            &[
                "privilege exec level 14 show",
                "privilege exec level 14 terminal",
            ],
        );
        s.type_line("end");
        s.type_line("disable");
        assert_eq!(s.mode(), "exec");
        let first = |h: Vec<String>| -> Vec<String> {
            h.iter()
                .flat_map(|l| l.split_whitespace().map(str::to_string).collect::<Vec<_>>())
                .collect()
        };
        assert!(!first(s.help("s")).contains(&"show".to_string()));
        let listed: Vec<String> = s
            .help("")
            .iter()
            .filter_map(|l| l.split_whitespace().next().map(str::to_string))
            .collect();
        assert!(listed.contains(&"enable".to_string()), "{listed:?}");
        assert!(!listed.contains(&"show".to_string()), "{listed:?}");
        assert!(!listed.contains(&"terminal".to_string()), "{listed:?}");
        assert_eq!(s.complete("sho"), None);
        // at `#` they are there
        s.type_line("enable");
        assert_eq!(s.complete("sho").as_deref(), Some("show "));
    }

    #[test]
    fn a_default_turned_back_on_prints_nothing_and_an_interface_keeps_its_no_lines() {
        let e = c9300();
        let s = configured(
            &e,
            &[
                "no spanning-tree vlan 5",
                "spanning-tree vlan 5",
                "no cdp run",
                "cdp run",
                "no ip domain lookup",
                "ip domain lookup",
                "ip igmp snooping",
                "no lldp run",
                "interface GigabitEthernet1/0/1",
                "no cdp enable",
                "no lldp transmit",
                "no lldp receive",
                "interface GigabitEthernet1/0/2",
                "no cdp enable",
                "cdp enable",
            ],
        );
        let starts = [
            "spanning-tree vlan",
            "cdp",
            "no cdp",
            "ip domain",
            "no ip domain",
            "ip igmp",
            "lldp",
            "no lldp",
        ];
        assert_eq!(
            shown(&s, &starts),
            [" no cdp enable", " no lldp transmit", " no lldp receive"]
        );
        assert_eq!(
            block_of(&s, "interface GigabitEthernet1/0/2"),
            ["interface GigabitEthernet1/0/2"]
        );
    }

    #[test]
    fn a_port_turned_routed_or_back_keeps_only_its_layer_s_lines() {
        let e = c9300();
        let mut s = configured(
            &e,
            &[
                "interface GigabitEthernet1/0/1",
                "switchport access vlan 10",
                "switchport mode access",
                "no switchport",
            ],
        );
        assert_eq!(
            block_of(&s, "interface GigabitEthernet1/0/1"),
            ["interface GigabitEthernet1/0/1", " no switchport"]
        );
        for line in [
            "ip address 10.1.1.1 255.255.255.0",
            "no ip address",
            "switchport",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        assert_eq!(
            block_of(&s, "interface GigabitEthernet1/0/1"),
            ["interface GigabitEthernet1/0/1"]
        );
    }

    #[test]
    fn a_refused_indented_line_leaves_the_mode_as_it_was() {
        let e = c9300();
        let mut s = configured(
            &e,
            &[
                "router bgp 65000",
                " address-family ipv4",
                "  neighbor 10.0.0.1 activate",
            ],
        );
        assert_eq!(s.type_line(" bogus line").1, Result::Error);
        assert_eq!(s.mode(), "config-router-af");
        assert_eq!(s.type_line("neighbor 10.0.0.2 activate").1, Result::Ok);
        assert_eq!(
            block_of(&s, "router bgp 65000"),
            [
                "router bgp 65000",
                " address-family ipv4",
                "  neighbor 10.0.0.1 activate",
                "  neighbor 10.0.0.2 activate",
                " exit-address-family"
            ]
        );
    }

    #[test]
    fn terminal_takes_a_length_or_width_of_0_to_512_and_refuses_another_at_it() {
        let e = c9300();
        let stack = crate::platform::parse_stack("C9300-48P").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.type_line("enable");
        // `Switch#terminal length 600`: under the value
        for (line, at) in [
            ("terminal length 600", 23),
            ("terminal length abc", 23),
            ("terminal width 600", 22),
        ] {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Error, "{line}");
            assert_eq!(caret_at(&out), at, "{line}");
        }
        assert_eq!(s.length, 24);
        assert_eq!(s.type_line("ter len 30"), (vec![], Result::Ok));
        assert_eq!(s.length, 30);
        assert_eq!(s.type_line("term wid 100"), (vec![], Result::Ok));
        assert_eq!(s.width, 100);
        // `?` and Tab know both words and the range
        assert!(s.help("terminal ").iter().any(|l| l.trim() == "width"));
        assert_eq!(s.help("terminal width ")[0].trim_end(), "  <0-512>");
        assert!(s.help("terminal length ")[0].starts_with("  <0-512>"));
        assert_eq!(
            s.complete("terminal wi").as_deref(),
            Some("terminal width ")
        );
    }

    #[test]
    fn question_mark_lays_out_as_ios_and_ios_xe_print_it() {
        // IOS XE (c9200 17.6 capture 1 at 9200 17.6): `|` and `<cr>` padded to the keywords, `<cr>` described
        // as `<cr>`; classic IOS (C2960G): `<cr>` alone; help past the width wraps under its column
        let e = c9300();
        let stack = crate::platform::parse_stack("C9300-48P").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.type_line("enable");
        let h = s.help("show vlan ");
        let n = h.len();
        assert_eq!(h[n - 1], "");
        assert_eq!(
            h[n - 2].split_whitespace().collect::<Vec<_>>(),
            ["<cr>", "<cr>"]
        );
        assert!(h[n - 3].starts_with("  | ") && h[n - 3].ends_with(" Output modifiers"));
        // the column each row's help starts at
        let at = |l: &str| {
            l.len()
                - l[2..]
                    .trim_start_matches(|c: char| c != ' ')
                    .trim_start()
                    .len()
        };
        assert_eq!(at(&h[n - 3]), at(&h[0]), "{h:?}");
        assert_eq!(at(&h[n - 2]), at(&h[0]), "{h:?}");
        assert!(h.iter().all(|l| l.chars().count() < 80), "{h:?}");
        s.width = 40;
        assert!(s.help("s").iter().all(|l| l.len() < 40));
        let r = crate::release::Release::parse("15.2(4)E").unwrap();
        let e = Engine::new(crate::grammars::grammar("c2960x", r).unwrap());
        let stack = crate::platform::parse_stack("WS-C2960X-48FPD-L").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        assert_eq!(s.help("")[0], "Exec commands:");
        s.type_line("enable");
        let h = s.help("show vlan ");
        let n = h.len();
        assert_eq!(h[n - 2], "  <cr>");
        assert!(h[n - 3].starts_with("  | ") && h[n - 3].ends_with(" Output modifiers"));
    }

    #[test]
    fn a_held_line_typed_in_a_section_stays_in_it() {
        // a line a capture held under the block, typed in its address family: inside the family,
        // before its closing line
        let mut e = c9300();
        e.held
            .lines
            .insert(("router bgp 65000".into(), "frobnicate x".into()));
        let s = configured(
            &e,
            &["router bgp 65000", "address-family ipv4", "frobnicate x"],
        );
        assert_eq!(
            block_of(&s, "router bgp 65000"),
            [
                "router bgp 65000",
                " address-family ipv4",
                "  frobnicate x",
                " exit-address-family"
            ]
        );
    }

    /// A session on `e` in global configuration, its frames as `configure terminal` leaves them.
    fn in_config(e: &Engine) -> Session<'_> {
        let stack = crate::platform::parse_stack("C9300X-24Y").unwrap();
        let mut s = Session::new(e, Device::new(stack));
        s.frames.push(Frame::top("privileged"));
        s.frames.push(Frame::top("config"));
        s
    }

    #[test]
    fn a_held_line_whose_first_word_the_books_abbreviate_prints_as_held() {
        // IOS writes a command's first word whole, so a held line whose first word the books read
        // as the start of a longer keyword names a command they lack (`service call-home` against
        // a book with `service-template` alone)
        let mut e = engine(&["service-template <name>"]);
        e.held
            .lines
            .insert((String::new(), "service call-home".into()));
        let mut s = in_config(&e);
        assert_eq!(s.type_line("service call-home").1, Result::Ok);
        let run = crate::show::configuration(&s.device);
        assert!(run.contains(&"service call-home".to_string()), "{run:?}");
        assert!(
            !run.iter().any(|l| l.starts_with("service-template")),
            "{run:?}"
        );
    }

    #[test]
    fn a_held_line_typed_again_keeps_its_place() {
        // a held top-level line prints after the line typed before it the first time; typed again
        // after another line, or its held block entered again after the block's own lines, it stays
        let mut e = engine(&["alpha", "beta", "gamma"]);
        for (block, line) in [
            ("", "frob on"),
            ("", "frob block"),
            ("frob block", "inner x"),
        ] {
            e.held.lines.insert((block.into(), line.into()));
        }
        e.held.opens.insert("frob block".into());
        let mut s = in_config(&e);
        for line in [
            "alpha",
            "frob on",
            "beta",
            "frob block",
            " inner x",
            "frob block",
            "frob on",
            "gamma",
        ] {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Ok, "{line:?}: {out:?}");
        }
        let ours = [
            "alpha",
            "beta",
            "gamma",
            "frob on",
            "frob block",
            " inner x",
        ];
        let run: Vec<String> = crate::show::configuration(&s.device)
            .into_iter()
            .filter(|l| ours.contains(&l.as_str()))
            .collect();
        assert_eq!(
            run,
            [
                "alpha",
                "frob on",
                "beta",
                "frob block",
                " inner x",
                "gamma"
            ]
        );
    }

    #[test]
    fn a_held_block_line_keeps_the_indentation_it_was_typed_with() {
        // a held line under a block prints as indented when typed (c9200 17.15 capture 27's terminal log wrote
        // eight spaces where the show techs print two and a tab); typed with none, as the first
        // capture that held it indented it
        let mut e = engine(&["alpha"]);
        for (block, line) in [("", "frob block"), ("frob block", "inner x")] {
            e.held.lines.insert((block.into(), line.into()));
        }
        e.held.opens.insert("frob block".into());
        e.held
            .indent
            .insert(("frob block".into(), "inner x".into()), "  \t".into());
        for (typed, printed) in [
            ("        inner x", "        inner x"),
            ("inner x", "  \tinner x"),
        ] {
            let mut s = in_config(&e);
            for line in ["frob block", typed] {
                let (out, r) = s.type_line(line);
                assert_eq!(r, Result::Ok, "{line:?}: {out:?}");
            }
            let run = crate::show::configuration(&s.device);
            assert!(run.contains(&printed.to_string()), "{typed:?}: {run:?}");
        }
    }
}
