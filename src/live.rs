//! What real switches held: the running configurations captured from them (data/live), by platform
//! book and release. A line one of them held is a result, so a switch of that platform and release
//! takes it as captured whatever the books say (Lewis, 2026-10-05: derived commands "should never
//! override actual live switch results").

#[cfg(captures)]
use crate::grammars::{LIVE, LIVE_LOGS, LIVE_STACKS};
use std::collections::{BTreeMap, BTreeSet};

/// The lines real switches of one platform and release held.
#[derive(Debug, Default)]
pub struct Held {
    /// each line, trimmed, under the kind of block it sits in (`interface`, another block's own
    /// first line, or "" at the top level)
    pub lines: BTreeSet<(String, String)>,
    /// the indentation each line under a block had, as the first capture holding it printed it (one
    /// line has a tab), by (block, line)
    pub indent: BTreeMap<(String, String), String>,
    /// the indentation of a block's own lines (not a section's) by the block's first word and the
    /// line's, where every capture of the release, of any platform, printed it alike: a class map's
    /// `description`, two spaces in at 17.5 to 17.12 and one at 17.15
    pub word_indent: BTreeMap<(String, String), String>,
    /// top-level lines that open a block
    pub opens: BTreeSet<String>,
    /// the top-level line before each top-level line in its capture, where IOS prints it
    pub after: After,
    /// `lines`, `indent` and `opens` entries whose site values the corpus characterized (`fits`)
    pub fitted_lines: Vec<(String, String)>,
    pub fitted_indent: Vec<(String, String, String)>,
    pub fitted_opens: Vec<String>,
    /// the captures read
    pub captures: Vec<String>,
    /// what a switch printed for `?`, by its mode and the line typed before the `?` (`help_key`)
    pub help: BTreeMap<(String, String), Vec<String>>,
    /// what a crawl of a switch printed for `?` in each mode, by its mode and `context::key`: one
    /// answer per context (`context::of` the submodes entered), where it asked in several
    pub crawl: BTreeMap<(String, String), Vec<CrawlAnswer>>,
    /// the description switches of the family (classic IOS or IOS XE) printed beside each keyword
    /// for `?`, by the mode (`help_mode`), the line typed before the `?` and the keyword: for a
    /// keyword its own book and captures leave bare (the 9300's `show ?` at 17.15, from the 9200's)
    pub described: Option<&'static Described>,
    /// `described` for the other family, asked after it
    pub described_other: Option<&'static Described>,
}

/// A crawl's answer and the context it was asked in.
pub type CrawlAnswer = (String, Vec<String>);

/// Keyword descriptions by (`help_mode`, `help_key` of the line before `?`, keyword).
pub type Described = BTreeMap<(String, String, String), String>;

/// The mode a `?` description belongs to: EXEC and privileged EXEC share theirs; each configuration
/// mode has its own (`exit` is `Exit from the EXEC` at `#`, never at `(config)#`).
pub fn help_mode(mode: &str) -> &str {
    match mode {
        "exec" | "privileged" => "exec",
        m => m,
    }
}

/// A line typed before `?` as help answers are looked up: lowercase, one space between words, and the
/// trailing space kept (after a space `?` lists what comes next; after a word, what it begins).
pub fn help_key(text: &str) -> String {
    let mut key = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.ends_with(char::is_whitespace) && !key.is_empty() {
        key.push(' ');
    }
    key.to_lowercase()
}

/// The kind of block a top-level line opens, as lines are matched under it: every interface alike,
/// any other block by its own line.
pub fn kind(header: &str) -> String {
    if header.starts_with("interface ") {
        "interface".into()
    } else {
        header.to_string()
    }
}

/// The lines of a capture from `version` to `end`, IOS's own `!` comments and blank lines left out
/// but a banner's text whole: from its `banner motd ^C` line through the line holding the delimiter
/// again, blank lines, `!` lines and an `end` among it are text.
pub fn body(text: &str) -> impl Iterator<Item = &str> {
    let mut lines = text.lines().skip_while(|l| !l.starts_with("version "));
    let mut closes: Option<String> = None;
    std::iter::from_fn(move || {
        loop {
            let l = lines.next()?;
            if let Some(d) = &closes {
                if l.contains(d.as_str()) {
                    closes = None;
                }
                return Some(l);
            }
            if l.trim_end() == "end" {
                return None;
            }
            if l.trim_start().starts_with('!') || l.trim().is_empty() {
                continue;
            }
            closes = banner_opens(l);
            return Some(l);
        }
    })
    .fuse()
}

/// The configuration lines of a capture (`body`) without a banner's text: `banner motd ^C` is a
/// configuration line, the lines after it through the one holding its delimiter again are the
/// banner's, which IOS takes only as its text.
pub fn config_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut closes: Option<String> = None;
    body(text).filter(move |l| {
        if let Some(d) = &closes {
            if l.contains(d.as_str()) {
                closes = None;
            }
            return false;
        }
        closes = banner_opens(l);
        true
    })
}

/// Whether a command a session typed is `full` as IOS reads it: each word the start of the full one,
/// two letters at least for the first and three for the rest, or the whole word (`sh inv` is `show
/// inventory`).
fn is_command(typed: &str, full: &str) -> bool {
    let typed: Vec<String> = typed.split_whitespace().map(str::to_lowercase).collect();
    let full: Vec<&str> = full.split_whitespace().collect();
    typed.len() == full.len()
        && typed.iter().zip(&full).enumerate().all(|(i, (t, f))| {
            t.len() >= f.len().min(if i == 0 { 2 } else { 3 }) && f.starts_with(t.as_str())
        })
}

/// The hardware a session's answers name, in `--stack` notation: its `show inventory`, else its
/// `show version`, each typed whole or abbreviated (`sh inv`).
pub fn stack_named(ex: &[crate::capture::Exchange]) -> Option<String> {
    let answer = |c: &str| {
        ex.iter()
            .rev()
            .find(|e| is_command(&e.command, c) && !e.output.is_empty())
            .map(|e| e.output.clone())
    };
    match answer("show inventory").map(|o| crate::capture::stack_of(&o)) {
        Some((Some(s), _)) => Some(s),
        _ => answer("show version").and_then(|o| crate::capture::stack_of_version(&o).0),
    }
}

/// The delimiter a `banner <kind> <delimiter>` line opens text with that runs on to later lines: `^C`
/// as a running configuration prints it, else the first character.
fn banner_opens(line: &str) -> Option<String> {
    let (_, text) = line.strip_prefix("banner ")?.split_once(' ')?;
    let text = text.trim_start();
    let d = if text.starts_with("^C") {
        "^C".to_string()
    } else {
        text.chars().next()?.to_string()
    };
    (!text[d.len()..].contains(d.as_str())).then_some(d)
}

/// Where IOS prints each held top-level line: after the top-level line before it in a capture, or
/// in a session, the line typed before it. A key or a line before it may carry characterized site
/// values (`fits`).
#[derive(Debug, Default, Clone)]
pub struct After {
    exact: BTreeMap<String, String>,
    fitted: Vec<(String, String)>,
}

impl After {
    /// A capture's pair, as the corpus has it.
    fn add(&mut self, line: String, before: String) {
        if fitted(&line) {
            self.fitted.push((line, before));
        } else {
            self.exact.insert(line, before);
        }
    }

    /// A session's pair: the line as typed, after the line typed before it.
    pub fn insert(&mut self, line: String, before: String) {
        self.exact.insert(line, before);
    }

    /// The line `line` prints after: a session's, else a capture's.
    pub fn get(&self, line: &str) -> Option<&str> {
        self.exact.get(line).map(String::as_str).or_else(|| {
            self.fitted
                .iter()
                .find(|(l, _)| fits(l, line))
                .map(|(_, b)| b.as_str())
        })
    }
}

/// The site values the corpus characterized, each standing for any value of its kind.
#[cfg(test)]
pub const PLACES: [&str; 5] = ["«ipv4»", "«mac»", "«number»", "«word»", "«text»"];

fn fitted(s: &str) -> bool {
    s.contains('«')
}

/// Whether `text` is `pattern` with each characterized value filled in by a value of its kind:
/// `«ipv4»` a dotted address, `«mac»` a dotted MAC address, `«number»` digits (alone, or inside a
/// word between the same text as the pattern's), `«word»` any word, `«text»` the rest of the line.
/// A pattern with none is the text itself.
pub fn fits(pattern: &str, text: &str) -> bool {
    if !fitted(pattern) {
        return pattern == text;
    }
    let want: Vec<&str> = pattern.split_whitespace().collect();
    let got: Vec<&str> = text.split_whitespace().collect();
    for (i, p) in want.iter().enumerate() {
        if *p == "«text»" {
            return i == want.len() - 1 && got.len() > i;
        }
        let Some(g) = got.get(i) else {
            return false;
        };
        if !fits_word(p, g) {
            return false;
        }
    }
    want.len() == got.len()
}

fn fits_word(p: &str, g: &str) -> bool {
    match p {
        "«ipv4»" => {
            let parts: Vec<&str> = g.split('.').collect();
            parts.len() == 4
                && parts
                    .iter()
                    .all(|o| !o.is_empty() && o.len() <= 3 && o.parse::<u8>().is_ok())
        }
        "«mac»" => {
            let parts: Vec<&str> = g.split('.').collect();
            parts.len() == 3
                && parts
                    .iter()
                    .all(|h| h.len() == 4 && h.chars().all(|c| c.is_ascii_hexdigit()))
        }
        "«number»" => !g.is_empty() && g.chars().all(|c| c.is_ascii_digit()),
        "«word»" => !g.is_empty(),
        _ => match p.split_once("«number»") {
            Some((before, after)) if !fitted(before) && !fitted(after) => g
                .strip_prefix(before)
                .and_then(|r| r.strip_suffix(after))
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())),
            _ => p == g,
        },
    }
}

impl Held {
    /// What the captures of `platform` (a grammar's platform name) at `release` held, as the
    /// embedded corpus has it.
    pub fn of(platform: &str, release: &str) -> Held {
        Held::from_corpus(crate::corpus::get(), platform, release)
    }

    /// As `of`, from a corpus being built.
    pub fn from_corpus(c: &crate::corpus::Corpus, platform: &str, release: &str) -> Held {
        let mut h = Held::default();
        let Some(book) = crate::grammars::book_of(platform) else {
            return h;
        };
        if let Some((_, _, t)) = c.held.iter().find(|(b, r, _)| b == book && r == release) {
            for (k, l) in &t.lines {
                if fitted(k) || fitted(l) {
                    h.fitted_lines.push((k.clone(), l.clone()));
                } else {
                    h.lines.insert((k.clone(), l.clone()));
                }
            }
            for (k, l, i) in &t.indent {
                if fitted(k) || fitted(l) {
                    h.fitted_indent.push((k.clone(), l.clone(), i.clone()));
                } else {
                    h.indent.insert((k.clone(), l.clone()), i.clone());
                }
            }
            for o in &t.opens {
                if fitted(o) {
                    h.fitted_opens.push(o.clone());
                } else {
                    h.opens.insert(o.clone());
                }
            }
            for (l, prev) in &t.after {
                h.after.add(l.clone(), prev.clone());
            }
            h.help = t
                .help
                .iter()
                .map(|(m, k, a)| ((m.clone(), k.clone()), a.clone()))
                .collect();
            for (m, c, k, a) in &t.crawl {
                h.crawl
                    .entry((m.clone(), k.clone()))
                    .or_default()
                    .push((c.clone(), a.clone()));
            }
            h.captures = t.captures.clone();
        }
        // every platform's captures at the release: IOS XE moved a class map's `description` from
        // two spaces in to one between 17.12 and 17.15, alike on the 9200, 9300 and 9400
        h.word_indent = c
            .word_indent
            .iter()
            .filter(|(r, ..)| r == release)
            .map(|(_, b, l, i)| ((b.clone(), l.clone()), i.clone()))
            .collect();
        let classic = crate::release::Release::parse(release).is_none_or(|r| r.classic());
        h.described = Some(described(c, classic));
        h.described_other = Some(described(c, !classic));
        h
    }

    /// What a real switch printed for `?` typed after `text` in `mode`, entered through `context`
    /// (`context::of`; none where the session's context is unknown). A capture's answer holds in
    /// every context; a crawl's holds in the context it was asked in, and in no other: where a crawl
    /// answered that mode and text under other contexts only, there is no held answer. A crawl's
    /// answer that a state predicate ends (`context::STATE_PREDICATES`) is no held answer either.
    pub fn help_for(
        &self,
        mode: &str,
        context: Option<&str>,
        text: &str,
        configured: impl Fn(&str) -> bool,
    ) -> Option<&Vec<String>> {
        let key = (mode.to_string(), help_key(text));
        let crawled = (mode.to_string(), crate::context::key(text));
        if context == Some("")
            && let Some(a) = self.help.get(&key)
        {
            return Some(a);
        }
        match self.crawl.get(&crawled) {
            None => self.help.get(&key),
            Some(answers) => {
                let (_, a) = answers.iter().find(|(c, _)| Some(c.as_str()) == context)?;
                (!crate::context::stale(&crawled.1, configured)).then_some(a)
            }
        }
    }

    /// Whether a real switch held `line` under a block of this kind ("" at the top level).
    pub fn has(&self, kind: &str, line: &str) -> bool {
        let line = line.trim();
        self.lines.contains(&(kind.to_string(), line.to_string()))
            || self
                .fitted_lines
                .iter()
                .any(|(k, l)| fits(k, kind) && fits(l, line))
    }

    /// Whether a real switch held `line` itself, not only a line of its form with other values.
    pub fn has_exact(&self, kind: &str, line: &str) -> bool {
        self.lines
            .contains(&(kind.to_string(), line.trim().to_string()))
    }

    /// Whether a real switch's top-level `line` opened a block.
    pub fn opens(&self, line: &str) -> bool {
        let line = line.trim();
        self.opens.contains(line) || self.fitted_opens.iter().any(|o| fits(o, line))
    }

    /// The indentation the first capture holding `line` under a block of this kind printed it with.
    pub fn indent(&self, kind: &str, line: &str) -> Option<&str> {
        let line = line.trim();
        if let Some(i) = self.indent.get(&(kind.to_string(), line.to_string())) {
            return Some(i);
        }
        self.fitted_indent
            .iter()
            .find(|(k, l, _)| fits(k, kind) && fits(l, line))
            .map(|(_, _, i)| i.as_str())
    }

    /// The indentation a switch prints `line` with directly under the block `header` opens: as a
    /// capture held that line, else as captures of the family printed lines of its first word under
    /// blocks of the header's first word.
    pub fn indent_of(&self, header: &str, line: &str) -> Option<&str> {
        let line = line.trim();
        if let Some(i) = self.indent(&kind(header), line) {
            return Some(i);
        }
        let first = |s: &str| s.split_whitespace().next().unwrap_or("").to_string();
        self.word_indent
            .get(&(first(header), first(line)))
            .map(String::as_str)
    }
}

/// The keyword descriptions of a family (classic IOS, or IOS XE) in a corpus: built once for the
/// embedded corpus, and leaked for one being built, which is read once.
fn described(c: &crate::corpus::Corpus, classic: bool) -> &'static Described {
    static CLASSIC: std::sync::OnceLock<Described> = std::sync::OnceLock::new();
    static XE: std::sync::OnceLock<Described> = std::sync::OnceLock::new();
    let table = || {
        c.described
            .iter()
            .filter(|d| d.0 == classic)
            .map(|(_, m, k, w, what)| ((m.clone(), k.clone(), w.clone()), what.clone()))
            .collect::<Described>()
    };
    if !std::ptr::eq(c, crate::corpus::get()) {
        return Box::leak(Box::new(table()));
    }
    let cell = if classic { &CLASSIC } else { &XE };
    cell.get_or_init(table)
}

/// `f` over every item, the items shared out over the machine's threads, results in item order: the
/// captures are typed 102 switches at a time by the capture tests and `vswitch coverage`.
pub fn par_map<'a, T: Sync, R: Send>(items: &'a [T], f: impl Fn(&'a T) -> R + Sync) -> Vec<R> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut done: Vec<(usize, R)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads.min(items.len()))
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else {
                            break mine;
                        };
                        mine.push((i, f(item)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

/// A real switch's capture with its session: platform book and release, hardware in `--stack`
/// notation, running configuration and session log.
#[cfg(captures)]
#[derive(Clone, Copy, Debug)]
pub struct Capture {
    pub book: &'static str,
    pub release: &'static str,
    pub model: &'static str,
    pub host: &'static str,
    pub running: &'static str,
    /// read by the capture tests, which compare each show command with it
    #[cfg_attr(not(test), allow(dead_code))]
    pub log: &'static str,
}

/// Every capture with a session log, its hardware read from the session's `show inventory` or
/// `show version`, else from `<host>-stack.txt` beside it (the cores'). A capture whose hardware
/// none of them names is left out; `live_capture_tests` fails on one.
#[cfg(captures)]
pub fn captures() -> Vec<Capture> {
    static ALL: std::sync::OnceLock<Vec<Capture>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        LIVE.iter()
            .filter_map(|&(book, release, file, running)| {
                let host = file.strip_suffix("-running-config.txt")?;
                let log_name = format!("{host}-show.log");
                let log = LIVE_LOGS
                    .iter()
                    .find(|l| l.0 == book && l.1 == release && l.2 == log_name)?
                    .3;
                let (_, ex) = crate::capture::exchanges(log)?;
                let stack_file = format!("{host}-stack.txt");
                let beside = || {
                    LIVE_STACKS
                        .iter()
                        .find(|s| s.0 == book && s.1 == release && s.2 == stack_file)
                        .map(|s| s.3.trim().to_string())
                };
                let stack = stack_named(&ex).or_else(beside)?;
                Some(Capture {
                    book,
                    release,
                    model: leak(stack),
                    host: leak(host.to_string()),
                    running,
                    log,
                })
            })
            .collect()
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_s_own_lines_take_the_indentation_every_capture_gave_them() {
        let one = "version 17.12\nclass-map match-any a\n  description x\n match y\npolicy-map p\n class c\n  police 1\nline con 0\n  exec-timeout 5\nend\n";
        let two = "version 17.12\nclass-map match-any b\n  description z\nline vty 0 4\n exec-timeout 9\nend\n";
        let w = crate::corpus::word_indents([one, two].into_iter());
        let at = |b: &str, l: &str| w.get(&(b.to_string(), l.to_string())).map(String::as_str);
        assert_eq!(at("class-map", "description"), Some("  "));
        assert_eq!(at("class-map", "match"), Some(" "));
        // a section's line is not the block's own
        assert_eq!(at("policy-map", "class"), Some(" "));
        assert_eq!(at("policy-map", "police"), None);
        // captures that disagree give no rule
        assert_eq!(at("line", "exec-timeout"), None);
    }

    #[cfg(all(test, captures))]
    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/live-tests.rs"));

    fn answer(s: &str) -> Vec<String> {
        vec![s.to_string()]
    }

    #[test]
    fn a_crawl_answer_holds_in_its_own_context_alone_and_a_capture_s_in_every_one() {
        let mut h = Held::default();
        let key = |m: &str, k: &str| (m.to_string(), k.to_string());
        h.help
            .insert(key("config-if", "switchport "), answer("capture"));
        h.help.insert(key("config-router", ""), answer("capture"));
        h.help
            .insert(key("privileged", "show vtp "), answer("capture"));
        h.crawl.insert(
            key("config-router", ""),
            vec![
                ("router ospf".into(), answer("ospf")),
                ("router bgp".into(), answer("bgp")),
            ],
        );
        h.crawl.insert(
            key("privileged", "show vtp "),
            vec![("".into(), answer("crawl"))],
        );
        let ask = |m: &str, c: Option<&str>, t: &str| h.help_for(m, c, t, |_| false).cloned();
        // where no crawl asked, the capture's answer: in any context, the unknown one too
        for c in [Some(""), Some("interface vlan"), None] {
            assert_eq!(ask("config-if", c, "switchport "), Some(answer("capture")));
        }
        // the context the crawl asked in, its answer; another context, none, whatever a capture held
        assert_eq!(
            ask("config-router", Some("router ospf"), ""),
            Some(answer("ospf"))
        );
        assert_eq!(
            ask("config-router", Some("router bgp"), ""),
            Some(answer("bgp"))
        );
        for c in [
            Some("router rip"),
            Some("router bgp > address-family ipv4"),
            None,
        ] {
            assert_eq!(ask("config-router", c, ""), None, "{c:?}");
        }
        // where both held it with no context, the capture's
        assert_eq!(
            ask("privileged", Some(""), "show vtp "),
            Some(answer("capture"))
        );
        // a mode and a line neither held
        assert_eq!(ask("config", Some(""), "ip "), None);
    }

    #[test]
    fn a_crawl_answer_ended_by_the_session_s_state_is_no_held_answer() {
        let mut h = Held::default();
        h.crawl.insert(
            ("config".into(), "aaa ".into()),
            vec![("".into(), answer("crawl"))],
        );
        h.crawl.insert(
            ("config".into(), "ip ".into()),
            vec![("".into(), answer("crawl ip"))],
        );
        let ask = |t: &str, on: bool| {
            h.help_for("config", Some(""), t, |l| on && l == "aaa new-model")
                .cloned()
        };
        assert_eq!(ask("aaa ", false), Some(answer("crawl")));
        assert_eq!(ask("aaa ", true), None);
        // another line of the session's does not end it, nor does the state end another answer
        assert_eq!(ask("ip ", true), Some(answer("crawl ip")));
    }

    #[test]
    fn a_crawl_answer_is_found_whatever_address_was_typed() {
        let mut h = Held::default();
        h.crawl.insert(
            (
                "config".into(),
                crate::context::key("arp 10.1.1.1 0011.2233.4455 "),
            ),
            vec![("".into(), answer("arpa"))],
        );
        for typed in [
            "arp 192.168.5.5 aabb.ccdd.eeff ",
            "ARP  10.1.1.1  0011.2233.4455 ",
        ] {
            assert_eq!(
                h.help_for("config", Some(""), typed, |_| false).cloned(),
                Some(answer("arpa")),
                "{typed}"
            );
        }
        assert_eq!(
            h.help_for("config", Some(""), "arp 10.1.1.1 ", |_| false),
            None
        );
    }

    #[test]
    fn a_session_s_abbreviated_show_inventory_names_its_hardware() {
        let log = "sw1#sh inv\nNAME: \"Switch 1\", DESCR: \"C9300-48P\"\nPID: C9300-48P         , VID: V02  , SN: FCW0000A000\n\nsw1#\n";
        let (_, ex) = crate::capture::exchanges(log).unwrap();
        assert_eq!(stack_named(&ex).as_deref(), Some("C9300-48P"));
        assert!(is_command("SHOW  ver", "show version") && is_command("sh inv", "show inventory"));
        assert!(
            !is_command("s inv", "show inventory") && !is_command("sh inv raw", "show inventory")
        );
    }
}
