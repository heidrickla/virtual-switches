//! What real switches held: the running configurations captured from them (data/live), by platform
//! book and release. A line one of them held is a result, so a switch of that platform and release
//! takes it as captured whatever the books say (Lewis, 2026-10-05: derived commands "should never
//! override actual live switch results").

#[cfg(captures)]
use crate::grammars::{LIVE, LIVE_LOGS, LIVE_STACKS};
use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
thread_local! {
    /// How many held-answer lookups this thread has made.
    static LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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
    /// what crawls of a switch of this image printed for `?`, by the license level it ran at
    pub crawls: BTreeMap<String, Crawl>,
    /// by a session's level, the crawl of another image that answers where this image's own
    /// answers hold none (`license::DONORS`), for the levels whose mapping is enabled
    pub donors: BTreeMap<String, Crawl>,
    /// the description switches of the family (classic IOS or IOS XE) printed beside each keyword
    /// for `?`, by the mode (`help_mode`), the line typed before the `?` and the keyword: for a
    /// keyword its own book and captures leave bare (the 9300's `show ?` at 17.15, from the 9200's)
    pub described: Option<&'static Described>,
    /// `described` for the other family, asked after it
    pub described_other: Option<&'static Described>,
}

/// A crawl's answer and the context it was asked in.
pub type CrawlAnswer = (String, Vec<String>);

/// What a crawl printed for `?` in each mode, by its mode and `context::key`: one answer per context
/// (`context::of` the submodes entered), where it asked in several.
pub type Crawl = BTreeMap<(String, String), Vec<CrawlAnswer>>;

/// Which answers gave a `?` answer (`Held::answered`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// the image's own: its captures and its crawl at the session's level
    Own,
    /// the donor crawl at the session's level
    Donor,
}

/// One tier's held answers: a capture's in every context, a crawl's in its own.
#[derive(Clone, Copy)]
struct Answers<'a> {
    help: Option<&'a BTreeMap<(String, String), Vec<String>>>,
    crawl: Option<&'a Crawl>,
}

/// Whether a held `?` answer is a refusal (`corpus::UNRECOGNIZED`, `corpus::AMBIGUOUS`).
pub fn is_refusal(answer: &[String]) -> bool {
    answer.iter().any(|l| l.starts_with('%'))
}

/// The blocks whose names a `?` answer offers, none configured when it was asked: `flow monitor `
/// for `No monitors configured`, `sampler ` for `No samplers configured`.
pub fn configured_kind(answer: &[String]) -> Option<&'static str> {
    answer.iter().find_map(|l| match l.trim() {
        "No monitors configured" | "No monitors available" => Some("flow monitor "),
        "No samplers configured" => Some("sampler "),
        "No records configured" => Some("flow record "),
        "No exporters configured" | "No flow exporters configured" => Some("flow exporter "),
        _ => None,
    })
}

/// The blocks whose names an answer's `User defined` rows are, the crawl's own, by the keyword
/// typed before them: `record` in a flow monitor offers the flow records configured.
pub fn defined_kind(answer: &[String], keyword: &str) -> Option<&'static str> {
    let kind = match keyword.to_lowercase().as_str() {
        "record" => "flow record ",
        "exporter" => "flow exporter ",
        "monitor" => "flow monitor ",
        "sampler" => "sampler ",
        _ => return None,
    };
    answer_rows(answer)
        .iter()
        .any(|(_, what)| what.eq_ignore_ascii_case("user defined"))
        .then_some(kind)
}

/// A `?` answer as the switch prints it after `typed`, the whole line before the `?` (`do-exec`
/// included): a held ambiguous refusal quotes that line as typed, as both crawls' switches did.
pub fn refusal_for(typed: &str, answer: Vec<String>) -> Vec<String> {
    if answer.len() == 1 && answer[0] == crate::corpus::AMBIGUOUS {
        vec![format!("% Ambiguous command:  \"{typed}\"")]
    } else {
        answer
    }
}

/// A line typed before `?` as the held answers read it (`read`).
#[derive(Debug, PartialEq, Eq)]
enum Read {
    /// the line, each abbreviated keyword written whole
    Line(String),
    /// a word begins two keywords or more and is neither, by the answer of the tier at this index
    Ambiguous(usize),
}

/// The held answer for an ambiguous line, which `refusal_for` quotes as typed.
fn ambiguous() -> &'static Vec<String> {
    static A: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    A.get_or_init(|| vec![crate::corpus::AMBIGUOUS.to_string()])
}

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

/// Each top-level line of a capture from `version` to `end`, with the `!` lines printed between it
/// and the top-level line before it; a banner's text belongs to its banner line.
pub fn separated(text: &str) -> Vec<(&str, usize)> {
    let mut out = vec![];
    let mut bangs = 0;
    let mut closes: Option<String> = None;
    for l in text.lines().skip_while(|l| !l.starts_with("version ")) {
        let l = l.trim_end();
        if let Some(d) = &closes {
            if l.contains(d.as_str()) {
                closes = None;
            }
            continue;
        }
        if l == "!" {
            bangs += 1;
            continue;
        }
        if l.is_empty() || l.starts_with([' ', '\t']) {
            continue;
        }
        out.push((l, bangs));
        bangs = 0;
        if l == "end" {
            break;
        }
        closes = banner_opens(l);
    }
    out
}

/// The delimiter a `banner <kind> <delimiter>` line opens text with that runs on to later lines: `^C`
/// as a running configuration prints it, else the first character.
pub(crate) fn banner_opens(line: &str) -> Option<String> {
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
pub const PLACES: [&str; 6] = ["«ipv4»", "«ipv6»", "«mac»", "«number»", "«word»", "«text»"];

fn fitted(s: &str) -> bool {
    s.contains('«')
}

/// Whether `text` is `pattern` with each characterized value filled in by a value of its kind:
/// `«ipv4»` a dotted address, `«ipv6»` an IPv6 address, `«mac»` a dotted MAC address, `«number»`
/// digits (alone, or inside a word between the same text as the pattern's), `«word»` any word,
/// `«text»` the rest of the line.
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
        "«ipv6»" => g.contains(':') && g.parse::<std::net::Ipv6Addr>().is_ok(),
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
        Held::lent(c, platform, release, crate::license::ENABLED)
    }

    /// As `from_corpus`, with the donor mappings of the switches `enabled` lists answering.
    #[cfg(test)]
    pub fn with_donors(
        c: &crate::corpus::Corpus,
        platform: &str,
        release: &str,
        enabled: &[(&str, &str, &str)],
    ) -> Held {
        Held::lent(c, platform, release, enabled)
    }

    fn lent(
        c: &crate::corpus::Corpus,
        platform: &str,
        release: &str,
        enabled: &[(&str, &str, &str)],
    ) -> Held {
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
            for (l, m, c, k, a) in &t.crawl {
                h.crawls
                    .entry(l.clone())
                    .or_default()
                    .entry((m.clone(), k.clone()))
                    .or_default()
                    .push((c.clone(), a.clone()));
            }
            h.captures = t.captures.clone();
        }
        for &(_, _, level) in enabled
            .iter()
            .filter(|(b, r, _)| *b == book && *r == release)
        {
            let Some((db, dr, dl)) = crate::license::enabled_donor(book, release, level, enabled)
            else {
                continue;
            };
            let Some((_, _, t)) = c.held.iter().find(|(b, r, _)| b == db && r == dr) else {
                continue;
            };
            let crawl = h.donors.entry(level.to_string()).or_default();
            for (_, m, c, k, a) in t.crawl.iter().filter(|e| e.0 == dl) {
                crawl
                    .entry((m.clone(), k.clone()))
                    .or_default()
                    .push((c.clone(), a.clone()));
            }
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

    /// The image's own answers at a session's license level: its captures', and its crawl at that
    /// level.
    fn own(&self, level: Option<&str>) -> Answers<'_> {
        Answers {
            help: Some(&self.help),
            crawl: level.and_then(|l| self.crawls.get(l)),
        }
    }

    /// The donor crawl's answers at a session's level, where its mapping is enabled.
    fn donor(&self, level: Option<&str>) -> Option<Answers<'_>> {
        let crawl = self.donors.get(level?)?;
        Some(Answers {
            help: None,
            crawl: Some(crawl),
        })
    }

    /// What a real switch of this image printed for `?` typed after `text` in `mode`, entered through
    /// `context`, at the session's license `level` (`Answers::help_for`). The caret of a refused
    /// line reads this, never a donor's answer.
    pub fn help_for(
        &self,
        level: Option<&str>,
        mode: &str,
        context: Option<&str>,
        text: &str,
        configured: impl Fn(&str) -> bool,
    ) -> Option<&Vec<String>> {
        self.own(level).help_for(mode, context, text, &configured)
    }

    /// What the shell takes before the books: the image's own held answer, abbreviations read, else
    /// its derived answer; then the same from the donor crawl at the session's level, in the same
    /// mode and context. The line is read once across both (`read`), so a donor's answer never wins
    /// over the image's for the same line; a derived answer is built from one tier's answers. A
    /// donor's answer is never merged with the books'.
    pub fn answer(
        &self,
        level: Option<&str>,
        mode: &str,
        context: Option<&str>,
        text: &str,
        configured: impl Fn(&str) -> bool,
    ) -> Option<Vec<String>> {
        self.answered(level, mode, context, text, configured)
            .map(|(_, a)| a)
    }

    /// `answer`, with the tier that gave it.
    pub fn answered(
        &self,
        level: Option<&str>,
        mode: &str,
        context: Option<&str>,
        text: &str,
        configured: impl Fn(&str) -> bool,
    ) -> Option<(Tier, Vec<String>)> {
        let configured: &dyn Fn(&str) -> bool = &configured;
        let own = self.own(level);
        let donor = self.donor(level);
        let tiers: Vec<Answers<'_>> = std::iter::once(own).chain(donor).collect();
        let held = |a: Answers<'_>, t: &str| a.held_as_typed(mode, context, t, configured).cloned();
        if let Some(a) = held(own, text) {
            return Some((Tier::Own, a));
        }
        let line = match read(&tiers, mode, context, text, configured) {
            Read::Line(line) => Some(line),
            Read::Ambiguous(0) => return Some((Tier::Own, ambiguous().clone())),
            Read::Ambiguous(_) => None,
        };
        // the line as read, then a derived answer, each from one tier: the image's, then the donor's
        let expanded = |a: Answers<'_>, line: &str| {
            (line != text)
                .then(|| held(a, line))
                .flatten()
                .or_else(|| derived(line, |t| a.held_as_typed(mode, context, t, configured)))
        };
        if let Some(a) = line.as_deref().and_then(|l| expanded(own, l)) {
            return Some((Tier::Own, a));
        }
        let donor = donor?;
        if let Some(a) = held(donor, text) {
            return Some((Tier::Donor, a));
        }
        match line {
            None => Some((Tier::Donor, ambiguous().clone())),
            Some(l) => expanded(donor, &l).map(|a| (Tier::Donor, a)),
        }
    }

    /// The crawls at a session's level and at the level it is gated against (`license::GATED`): the
    /// image's own where it holds both, else the donor's at both.
    fn gate_crawls(&self, level: Option<&str>) -> Option<(&Crawl, &Crawl)> {
        let level = level?;
        let above = crate::license::gated_by(level)?;
        fn pair<'c>(
            m: &'c BTreeMap<String, Crawl>,
            level: &str,
            above: &str,
        ) -> Option<(&'c Crawl, &'c Crawl)> {
            Some((m.get(level)?, m.get(above)?))
        }
        pair(&self.crawls, level, above).or_else(|| pair(&self.donors, level, above))
    }

    /// Whether a session's level is gated here: crawls of the image, or of its donor, answer at both
    /// the level and the level it is gated against.
    pub fn gated(&self, level: Option<&str>) -> bool {
        self.gate_crawls(level).is_some()
    }

    /// What the gate's two crawls (`gate_crawls`) printed for `?` after `text` in `mode`, entered
    /// through `context`: the one at the session's level, then the one above it, each line read with
    /// its own crawl's answers. A line whose numbers neither crawl typed is read as the line both
    /// typed with other numbers in the same ranges (`numbers_as_crawled`). None where either holds
    /// no answer.
    pub fn gate_answers(
        &self,
        level: Option<&str>,
        mode: &str,
        context: Option<&str>,
        text: &str,
        configured: impl Fn(&str) -> bool,
    ) -> Option<(&Vec<String>, &Vec<String>)> {
        let (at, above) = self.gate_crawls(level)?;
        let configured: &dyn Fn(&str) -> bool = &configured;
        let answers = |c| Answers {
            help: None,
            crawl: Some(c),
        };
        let both = |t: &str| {
            Some((
                answers(at).help_for(mode, context, t, configured)?,
                answers(above).help_for(mode, context, t, configured)?,
            ))
        };
        if let Some(both) = both(text) {
            return Some(both);
        }
        let Read::Line(line) = read(&[answers(at)], mode, context, text, configured) else {
            return None;
        };
        let sibling = numbers_as_crawled(at, above, mode, context?, &crate::context::key(&line))?;
        both(&sibling)
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

impl<'a> Answers<'a> {
    /// What a real switch printed for `?` typed after `text` in `mode`, entered through `context`
    /// (`context::of`; none where the session's context is unknown). A capture's answer holds in
    /// every context; a crawl's holds in the context it was asked in, and in no other: where a crawl
    /// answered that mode and text under other contexts only, there is no held answer. A crawl's
    /// answer that a state predicate ends (`context::STATE_PREDICATES`) is no held answer either.
    /// The line is read with each keyword whole (`read`): a line held as typed is its own answer, an
    /// abbreviation that begins several keywords is refused as ambiguous.
    fn help_for(
        &self,
        mode: &str,
        context: Option<&str>,
        text: &str,
        configured: &dyn Fn(&str) -> bool,
    ) -> Option<&'a Vec<String>> {
        if let Some(a) = self.held_as_typed(mode, context, text, configured) {
            return Some(a);
        }
        match read(&[*self], mode, context, text, configured) {
            Read::Ambiguous(_) => Some(ambiguous()),
            Read::Line(line) if line != text => {
                self.held_as_typed(mode, context, &line, configured)
            }
            Read::Line(_) => None,
        }
    }

    /// `help_for` for the line as typed, its words as they stand.
    fn held_as_typed(
        &self,
        mode: &str,
        context: Option<&str>,
        text: &str,
        configured: &dyn Fn(&str) -> bool,
    ) -> Option<&'a Vec<String>> {
        let (help, crawled) = (help_key(text), crate::context::key(text));
        self.held_by_key(mode, context, &help, &crawled, configured)
    }

    /// `held_as_typed` for a line's keys (`help_key`, `context::key`).
    fn held_by_key(
        &self,
        mode: &str,
        context: Option<&str>,
        help_key: &str,
        crawled_key: &str,
        configured: &dyn Fn(&str) -> bool,
    ) -> Option<&'a Vec<String>> {
        #[cfg(test)]
        LOOKUPS.with(|n| n.set(n.get() + 1));
        let key = (mode.to_string(), help_key.to_string());
        let crawled = (mode.to_string(), crawled_key.to_string());
        let help = self.help.and_then(|h| h.get(&key));
        if context == Some("")
            && let Some(a) = help
        {
            return Some(a);
        }
        match self.crawl.and_then(|c| c.get(&crawled)) {
            None => help,
            Some(answers) => {
                let (_, a) = answers.iter().find(|(c, _)| Some(c.as_str()) == context)?;
                (!crate::context::stale(&crawled.1, configured)).then_some(a)
            }
        }
    }
}

/// `text` with each word typed before the last space that begins exactly one keyword the held
/// answer for the line before it offers written as that keyword, as IOS takes a unique start of a
/// keyword wherever one is expected (`ip acc ?` answers as `ip access-list ?`). The answer for the
/// line before a word is the first tier's of `tiers` that holds one, as typed or derived from its
/// own answers. A word that is a keyword whole, begins none (a value), stands where no answer is
/// held or is followed by a held answer is left as typed, and the words after it are read on; a
/// word beginning two keywords or more, none of them whole, makes the line ambiguous.
/// A word still being typed (no space after it) is left as typed.
fn read<'a>(
    tiers: &[Answers<'a>],
    mode: &str,
    context: Option<&str>,
    text: &str,
    configured: &dyn Fn(&str) -> bool,
) -> Read {
    let words: Vec<&str> = text.split_whitespace().collect();
    let whole = text.ends_with(char::is_whitespace);
    let complete = if whole {
        words.len()
    } else {
        words.len().saturating_sub(1)
    };
    let as_typed = |keys: &Keys| {
        tiers
            .iter()
            .find_map(|a| a.held_by_key(mode, context, &keys.help, &keys.crawled, configured))
    };
    let mut out: Vec<String> = vec![];
    let mut keys = Keys::default();
    // each tier's held answer after each prefix of `out` as it stands: a derivation takes its parent's
    // from here and is tried only where one is held
    let mut after: Vec<Vec<Option<&'a Vec<String>>>> = vec![vec![]; tiers.len()];
    for (i, w) in words.iter().enumerate() {
        if i >= complete {
            out.push(w.to_string());
            continue;
        }
        for (a, seen) in tiers.iter().zip(after.iter_mut()) {
            seen.push(a.held_by_key(mode, context, &keys.help, &keys.crawled, configured));
        }
        let held = tiers.iter().enumerate().find_map(|(n, a)| {
            after[n][i]
                .cloned()
                .or_else(|| {
                    derived_words(
                        &out,
                        &|t: &str| a.held_as_typed(mode, context, t, configured),
                        |k| after[n][k].is_some(),
                    )
                })
                .map(|h| (n, h))
        });
        // a word the switch answered after is one it took as typed, a keyword `?` leaves out
        // (the C9300's `ipv6 nd ra`, beside `ra-throttler` and `raguard`) or a value
        let took = || as_typed(&keys.with(w)).is_some_and(|a| !is_refusal(a));
        let next = match held.filter(|_| !took()) {
            None => w.to_string(),
            Some((n, held)) => {
                let known = keywords(&held);
                let known: Vec<&str> = known.iter().map(String::as_str).collect();
                match crate::context::reads(w, &known) {
                    crate::context::Reads::One(k) => k.to_string(),
                    crate::context::Reads::Several => return Read::Ambiguous(n),
                    _ => w.to_string(),
                }
            }
        };
        keys.push(&next);
        out.push(next);
    }
    let mut line = out.join(" ");
    if whole && !line.is_empty() {
        line.push(' ');
    }
    Read::Line(line)
}

/// The first line both crawls answered `?` after in `mode` and `context` that is `key` with other
/// numbers where it has numbers: each pair in one `<lo-hi>` range of the answer before it at the
/// level (`at`). The lab C9300-24T (17.6.4) crawls asked `monitor session 1 ?`, and its
/// network-essentials refused `type` after `monitor session 2` as its answer for session 1 left it out.
fn numbers_as_crawled(
    at: &Crawl,
    above: &Crawl,
    mode: &str,
    context: &str,
    key: &str,
) -> Option<String> {
    let number = |w: &str| !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit());
    let words: Vec<&str> = key.split(' ').collect();
    let first = words.iter().position(|w| number(w))?;
    if first == 0 || !key.ends_with(' ') {
        return None;
    }
    let prefix = format!("{} ", words[..first].join(" "));
    fn answered<'c>(c: &'c Crawl, mode: &str, context: &str, k: &str) -> Option<&'c Vec<String>> {
        c.get(&(mode.to_string(), k.to_string()))?
            .iter()
            .find(|(c, _)| c == context)
            .map(|(_, a)| a)
    }
    let held = |c, k: &str| answered(c, mode, context, k);
    let in_one_range = |answer: &Vec<String>, a: &str, b: &str| {
        answer_rows(answer).iter().any(|(w, _)| {
            let range = w.strip_prefix('<').and_then(|r| r.strip_suffix('>'));
            range
                .and_then(|r| r.split_once('-'))
                .and_then(|(lo, hi)| Some((lo.parse::<u64>().ok()?, hi.parse::<u64>().ok()?)))
                .is_some_and(|(lo, hi)| {
                    [a, b]
                        .iter()
                        .all(|n| n.parse::<u64>().is_ok_and(|n| n >= lo && n <= hi))
                })
        })
    };
    at.range((mode.to_string(), prefix.clone())..)
        .map(|((m, k), _)| (m, k))
        .take_while(|(m, k)| m.as_str() == mode && k.starts_with(&prefix))
        .map(|(_, k)| k)
        .find(|k| {
            let theirs: Vec<&str> = k.split(' ').collect();
            theirs.len() == words.len()
                && *k != key
                && held(at, k).is_some()
                && held(above, k).is_some()
                && words.iter().zip(&theirs).enumerate().all(|(i, (w, t))| {
                    if !number(w) {
                        return w == t;
                    }
                    let before = format!("{} ", theirs[..i].join(" "));
                    number(t) && held(at, &before).is_some_and(|a| in_one_range(a, w, t))
                })
        })
        .cloned()
}

/// The lookup keys of the words of a line read so far (`help_key` and `context::key` of the line,
/// a space after each word), kept as the line grows.
#[derive(Default)]
struct Keys {
    help: String,
    crawled: String,
}

impl Keys {
    fn push(&mut self, word: &str) {
        let word = word.to_lowercase();
        self.crawled.push_str(&crate::context::values(&word));
        self.crawled.push(' ');
        self.help.push_str(&word);
        self.help.push(' ');
    }

    fn with(&self, word: &str) -> Keys {
        let mut keys = Keys {
            help: self.help.clone(),
            crawled: self.crawled.clone(),
        };
        keys.push(word);
        keys
    }
}

/// A row of a `?` answer: its word, its description and the lines continuing the description.
struct Row<'a> {
    word: &'a str,
    what: &'a str,
    more: Vec<&'a String>,
}

/// The rows of a `?` answer: a row is two spaces in, a line further in continues the row before it;
/// a refusal has none.
fn rows(answer: &[String]) -> Vec<Row<'_>> {
    let mut out: Vec<Row<'_>> = vec![];
    if is_refusal(answer) {
        return out;
    }
    for l in answer {
        let indent = l.len() - l.trim_start_matches(' ').len();
        let t = l.trim();
        if t.is_empty() {
            continue;
        }
        if indent == 2 {
            let (word, what) = t
                .split_once(char::is_whitespace)
                .map_or((t, ""), |(w, d)| (w, d.trim()));
            out.push(Row {
                word,
                what,
                more: vec![],
            });
        } else if indent > 2
            && let Some(last) = out.last_mut()
        {
            last.more.push(l);
        }
    }
    out
}

/// Whether a row's word is a keyword, as the crawler classes it: not `<cr>`, `|`, a range or other
/// `<...>` form, nor a word in capitals (`WORD`, `LINE`, `A.B.C.D`, `X:X:X:X::X/<0-128>`).
fn keyword(word: &str) -> bool {
    let caps = word.chars().any(|c| c.is_ascii_uppercase())
        && word
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || "_./:-<>".contains(c));
    !(word == "|" || word.starts_with('<') || caps)
}

/// The word and description of each row of a `?` answer, keywords and placeholders alike; a
/// refusal has none.
pub fn answer_rows(answer: &[String]) -> Vec<(&str, &str)> {
    rows(answer).iter().map(|r| (r.word, r.what)).collect()
}

/// The keywords a `?` answer offers, in its order (`keyword`); a refusal offers none.
pub fn keywords(answer: &[String]) -> Vec<String> {
    rows(answer)
        .iter()
        .map(|r| r.word)
        .filter(|w| keyword(w))
        .map(str::to_string)
        .collect()
}

/// `answer` with the rows whose word `keep` refuses dropped and `add`'s (word, description) rows put
/// in ASCII order before `|` and `<cr>`, the column laid out again as IOS lays it: the longest word
/// printed plus two, a bare `<cr>` not counted, a word with no description printed bare. Lines that
/// are not rows (a heading, the closing blank) stay where they were.
pub fn relaid(answer: &[String], keep: impl Fn(&str) -> bool, add: &[(&str, &str)]) -> Vec<String> {
    enum Line<'a> {
        Row(&'a str, &'a str, Vec<&'a str>),
        Other(&'a str),
    }
    let mut lines: Vec<Line<'_>> = vec![];
    if is_refusal(answer) {
        return answer.to_vec();
    }
    for l in answer {
        let indent = l.len() - l.trim_start_matches(' ').len();
        let t = l.trim();
        match lines.last_mut() {
            _ if indent == 2 && !t.is_empty() => {
                let (word, what) = t
                    .split_once(char::is_whitespace)
                    .map_or((t, ""), |(w, d)| (w, d.trim()));
                lines.push(Line::Row(word, what, vec![]));
            }
            Some(Line::Row(_, _, more)) if indent > 2 && !t.is_empty() => more.push(t),
            _ => lines.push(Line::Other(l)),
        }
    }
    lines.retain(|l| !matches!(l, Line::Row(w, ..) if !keep(w)));
    for &(word, what) in add {
        let at = lines
            .iter()
            .position(|l| matches!(l, Line::Row(w, ..) if *w == "<cr>" || *w == "|" || *w > word))
            .or_else(|| {
                lines
                    .iter()
                    .rposition(|l| matches!(l, Line::Row(..)))
                    .map(|i| i + 1)
            })
            .unwrap_or(0);
        lines.insert(at, Line::Row(word, what, vec![]));
    }
    let width = lines
        .iter()
        .filter_map(|l| match l {
            Line::Row(w, what, _) if !(*w == "<cr>" && what.is_empty()) => Some(w.chars().count()),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let mut out = vec![];
    for l in lines {
        match l {
            Line::Row(w, "", _) => out.push(format!("  {w}")),
            Line::Row(w, what, more) => {
                out.push(format!("  {w:<width$}  {what}"));
                out.extend(more.iter().map(|m| format!("{}{m}", " ".repeat(width + 4))));
            }
            Line::Other(l) => out.push(l.to_string()),
        }
    }
    out
}

/// The words an answer offers, `<cr>` left out.
fn offered(rows: &[Row<'_>]) -> BTreeSet<String> {
    rows.iter()
        .filter(|r| r.word != "<cr>")
        .map(|r| r.word.to_string())
        .collect()
}

/// The crawler's rule (`swcon crawl`, `is_subset_answer`) for a pick it does not expand: its answer
/// offers only words its parent's offered, at least two and at least half as many; `no` and
/// `default` restate commands and are never such a pick.
fn subset_answer(parent: &BTreeSet<String>, picked: &str, child: &BTreeSet<String>) -> bool {
    !matches!(picked, "no" | "default")
        && child.len() >= 2
        && child.len() * 2 >= parent.len()
        && parent.contains(picked)
        && child.is_subset(parent)
}

/// The answer after `Q t1 .. tn ` (two picks or more) from the answers `lookup` holds for `Q ` and for
/// each `Q ti ` alone, where each pick is a keyword `Q` offers and its answer a subset answer of
/// `Q`'s: `Q`'s rows in its order, each kept only where every pick's answer offers it (a pick too,
/// where its own answer offers it again, as `ntp peer .. normal-sync ?` does on the C2960G), and
/// `<cr>` last where every pick's answer offers it. Each pick must be offered after every other, as
/// the switch takes it then. The word column is as wide as the longest word printed with a
/// description, and each description is `Q`'s. The longest `Q` that qualifies is taken; none, and
/// there is no answer.
pub fn derived<'a>(
    typed: &str,
    lookup: impl Fn(&str) -> Option<&'a Vec<String>>,
) -> Option<Vec<String>> {
    if !typed.ends_with(char::is_whitespace) {
        return None;
    }
    let words: Vec<&str> = typed.split_whitespace().collect();
    derived_words(&words, &lookup, |_| true)
}

/// `derived` for the words of the line, trying only the splits `viable` allows (a split whose first
/// part has no held answer derives nothing).
fn derived_words<'a, S: Borrow<str>>(
    words: &[S],
    lookup: &impl Fn(&str) -> Option<&'a Vec<String>>,
    viable: impl Fn(usize) -> bool,
) -> Option<Vec<String>> {
    (1..words.len().saturating_sub(1))
        .rev()
        .filter(|&k| viable(k))
        .find_map(|k| derive_at(&words[..k], &words[k..], lookup))
}

fn derive_at<'a, S: Borrow<str>>(
    q: &[S],
    picks: &[S],
    lookup: &impl Fn(&str) -> Option<&'a Vec<String>>,
) -> Option<Vec<String>> {
    let base = q.join(" ");
    let parent = rows(lookup(&format!("{base} "))?);
    let parent_words = offered(&parent);
    let mut picked: Vec<&str> = vec![];
    let mut children: Vec<Vec<Row<'_>>> = vec![];
    for p in picks {
        let p: &str = p.borrow();
        let row = parent
            .iter()
            .find(|r| r.word.eq_ignore_ascii_case(p) && keyword(r.word))?;
        let child = rows(lookup(&format!("{base} {p} "))?);
        if !subset_answer(&parent_words, row.word, &offered(&child)) {
            return None;
        }
        picked.push(row.word);
        children.push(child);
    }
    let offers = |c: &[Row<'_>], w: &str| c.iter().any(|r| r.word == w);
    // a pick the switch no longer offers after another is refused there
    for (i, c) in children.iter().enumerate() {
        if picked
            .iter()
            .enumerate()
            .any(|(j, w)| i != j && !offers(c, w))
        {
            return None;
        }
    }
    let mut kept: Vec<&Row<'_>> = parent
        .iter()
        .filter(|r| r.word != "<cr>")
        .filter(|r| children.iter().all(|c| offers(c, r.word)))
        .collect();
    let cr = children
        .first()
        .and_then(|c| c.iter().find(|r| r.word == "<cr>"));
    if let Some(cr) = cr
        && children.iter().all(|c| offers(c, "<cr>"))
    {
        kept.push(cr);
    }
    if kept.is_empty() {
        return None;
    }
    // a row printed without a description (classic IOS's `<cr>`) does not widen the column
    let width = kept
        .iter()
        .filter(|r| !r.what.is_empty())
        .map(|r| r.word.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = vec![];
    for r in kept {
        out.push(if r.what.is_empty() {
            format!("  {}", r.word)
        } else {
            format!("  {:<width$}  {}", r.word, r.what)
        });
        out.extend(r.more.iter().map(|l| l.to_string()));
    }
    Some(out)
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

    #[test]
    fn each_top_level_line_counts_the_separators_before_it() {
        let text = "Building configuration...\n!\nversion 17.6\nhostname a\n!\n!\nvrf definition v\n !\n address-family ipv4\n exit-address-family\n!\nbanner motd ^C\n!\nhi\n^C\n!\nline con 0\n stopbits 1\nend\n!\n";
        assert_eq!(
            separated(text),
            vec![
                ("version 17.6", 0),
                ("hostname a", 0),
                ("vrf definition v", 2),
                ("banner motd ^C", 1),
                ("line con 0", 1),
                ("end", 0),
            ]
        );
    }

    #[cfg(all(test, captures))]
    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/live-tests.rs"));

    fn answer(s: &str) -> Vec<String> {
        vec![s.to_string()]
    }

    /// The level the tests' sessions run at, and the image's own crawl at it.
    const L: Option<&str> = Some("network-advantage");

    fn own(h: &mut Held) -> &mut Crawl {
        h.crawls.entry(L.unwrap().to_string()).or_default()
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
        own(&mut h).insert(
            key("config-router", ""),
            vec![
                ("router ospf".into(), answer("ospf")),
                ("router bgp".into(), answer("bgp")),
            ],
        );
        own(&mut h).insert(
            key("privileged", "show vtp "),
            vec![("".into(), answer("crawl"))],
        );
        let ask = |m: &str, c: Option<&str>, t: &str| h.help_for(L, m, c, t, |_| false).cloned();
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
        own(&mut h).insert(
            ("config".into(), "aaa ".into()),
            vec![("".into(), answer("crawl"))],
        );
        own(&mut h).insert(
            ("config".into(), "ip ".into()),
            vec![("".into(), answer("crawl ip"))],
        );
        let ask = |t: &str, on: bool| {
            h.help_for(L, "config", Some(""), t, |l| on && l == "aaa new-model")
                .cloned()
        };
        assert_eq!(ask("aaa ", false), Some(answer("crawl")));
        assert_eq!(ask("aaa ", true), None);
        // another line of the session's does not end it, nor does the state end another answer
        assert_eq!(ask("ip ", true), Some(answer("crawl ip")));
    }

    #[test]
    fn a_crawl_s_refusal_takes_the_place_of_a_crawl_s_answer_in_the_lookup_order() {
        let refused = answer(crate::corpus::UNRECOGNIZED);
        let mut h = Held::default();
        let key = |m: &str, k: &str| (m.to_string(), k.to_string());
        // a production capture's answer, and the crawl's refusal of the same line
        h.help
            .insert(key("privileged", "show vtp "), answer("capture"));
        own(&mut h).insert(
            key("privileged", "show vtp "),
            vec![("".into(), refused.clone())],
        );
        own(&mut h).insert(
            key("config-router", "bogus "),
            vec![("router ospf".into(), refused.clone())],
        );
        h.help.insert(key("config", "bogus "), answer("capture"));
        own(&mut h).insert(
            key("config", "aaa authentication "),
            vec![("".into(), refused.clone())],
        );
        let ask = |m: &str, c: Option<&str>, t: &str, aaa: bool| {
            h.help_for(L, m, c, t, |l| aaa && l == "aaa new-model")
                .cloned()
        };
        // with no context, the capture's answer first
        assert_eq!(
            ask("privileged", Some(""), "show vtp ", false),
            Some(answer("capture"))
        );
        // the crawl's refusal in the context it was asked in; in another, none: the books answer
        assert_eq!(
            ask("config-router", Some("router ospf"), "bogus ", false),
            Some(refused.clone())
        );
        assert_eq!(
            ask("config-router", Some("router bgp"), "bogus ", false),
            None
        );
        // a line the crawl never asked in that mode takes the capture's answer
        assert_eq!(
            ask("config", Some(""), "bogus ", false),
            Some(answer("capture"))
        );
        // the state predicate ends a refusal as it ends an answer
        assert_eq!(
            ask("config", Some(""), "aaa authentication ", false),
            Some(refused)
        );
        assert_eq!(ask("config", Some(""), "aaa authentication ", true), None);
    }

    #[test]
    fn a_held_ambiguous_refusal_quotes_the_line_and_any_other_answer_is_as_held() {
        let ambiguous = answer(crate::corpus::AMBIGUOUS);
        assert!(is_refusal(&ambiguous) && is_refusal(&answer(crate::corpus::UNRECOGNIZED)));
        assert!(!is_refusal(&answer("  <cr>")));
        assert_eq!(
            refusal_for("alias  Filter ", ambiguous),
            ["% Ambiguous command:  \"alias  Filter \""]
        );
        for held in [
            answer(crate::corpus::UNRECOGNIZED),
            answer("  <cr>"),
            vec![],
        ] {
            assert_eq!(refusal_for("alias filter ", held.clone()), held);
        }
    }

    /// A crawl's answers in global configuration with no context, as (key, rows).
    fn crawled(answers: &[(&str, &[&str])]) -> Held {
        let mut h = Held::default();
        for (k, rows) in answers {
            own(&mut h).insert(
                ("config".into(), k.to_string()),
                vec![("".into(), rows.iter().map(|r| r.to_string()).collect())],
            );
        }
        h
    }

    const TOP: (&str, &[&str]) = (
        "",
        &[
            "  interface  Select an interface to configure",
            "  ip         Global IP configuration subcommands",
            "  ipv6       Global IPv6 configuration commands",
            "  vlan       Vlan commands",
        ],
    );
    const IP: (&str, &[&str]) = (
        "ip ",
        &["  access-list  Named access-list", "  address      Address"],
    );
    const ACL: (&str, &[&str]) = (
        "ip access-list ",
        &[
            "  extended  Extended Access List",
            "  standard  Standard Access List",
        ],
    );

    #[test]
    fn an_abbreviated_keyword_reaches_the_held_answer_for_the_keyword() {
        let h = crawled(&[TOP, IP, ACL]);
        let ask = |t: &str| h.help_for(L, "config", Some(""), t, |_| false).cloned();
        let want = Some(ACL.1.iter().map(|r| r.to_string()).collect::<Vec<_>>());
        for typed in ["ip acc ", "IP  ACC ", "ip ac ", "ip access-list "] {
            assert_eq!(ask(typed), want, "{typed}");
        }
        // a word that begins no keyword, and a word still being typed, are left as they are
        assert_eq!(ask("ip a-l "), None);
        assert_eq!(ask("ip acc"), None);
        // the derivation and the shell's order take the same reading
        assert_eq!(h.answer(L, "config", Some(""), "ip acc ", |_| false), want);
    }

    #[test]
    fn a_keyword_typed_whole_wins_over_the_longer_keywords_it_begins() {
        let h = crawled(&[TOP, IP, ("ipv6 ", &["  route  Configure static routes"])]);
        let ask = |t: &str| h.help_for(L, "config", Some(""), t, |_| false).cloned();
        assert_eq!(
            ask("ip "),
            Some(vec![IP.1[0].to_string(), IP.1[1].to_string()])
        );
        assert_eq!(
            ask("ipv "),
            Some(vec!["  route  Configure static routes".to_string()])
        );
    }

    #[test]
    fn a_start_of_several_keywords_is_refused_as_the_switch_refuses_it() {
        let h = crawled(&[
            TOP,
            IP,
            (
                "alias ",
                &[
                    "  exec          Exec (prompt) mode",
                    "  filter        Filter",
                    "  filter-group  Filter group",
                ],
            ),
            ("alias filter ", &[crate::corpus::AMBIGUOUS]),
        ]);
        let ask = |t: &str| {
            h.help_for(L, "config", Some(""), t, |_| false)
                .map(|a| refusal_for(t, a.clone()))
        };
        // at the end of the line and before other words, the whole line quoted as typed
        for typed in ["i ", "i access-list ", "alias fil ", "Alias  Fil  x "] {
            assert_eq!(
                ask(typed),
                Some(vec![format!("% Ambiguous command:  \"{typed}\"")]),
                "{typed}"
            );
        }
        // a keyword typed whole that begins another is the switch's held answer for it
        assert_eq!(
            ask("alias filter "),
            Some(vec!["% Ambiguous command:  \"alias filter \"".to_string()])
        );
        assert_eq!(ask("alias filter-g "), None);
    }

    /// `read` as it walked a line before the prefix keys were kept: each word's preceding line
    /// rebuilt whole, a derivation tried at every split of it.
    fn read_before(
        tiers: &[Answers<'_>],
        mode: &str,
        context: Option<&str>,
        text: &str,
        configured: &dyn Fn(&str) -> bool,
    ) -> Read {
        let words: Vec<&str> = text.split_whitespace().collect();
        let whole = text.ends_with(char::is_whitespace);
        let complete = if whole {
            words.len()
        } else {
            words.len().saturating_sub(1)
        };
        let as_typed = |t: &str| {
            tiers
                .iter()
                .find_map(|a| a.held_as_typed(mode, context, t, configured))
        };
        let mut out: Vec<String> = vec![];
        for (i, w) in words.iter().enumerate() {
            if i >= complete {
                out.push(w.to_string());
                continue;
            }
            let before = if out.is_empty() {
                String::new()
            } else {
                format!("{} ", out.join(" "))
            };
            let held = tiers.iter().enumerate().find_map(|(n, a)| {
                a.held_as_typed(mode, context, &before, configured)
                    .cloned()
                    .or_else(|| derived(&before, |t| a.held_as_typed(mode, context, t, configured)))
                    .map(|h| (n, h))
            });
            let took = || as_typed(&format!("{before}{w} ")).is_some_and(|a| !is_refusal(a));
            let Some((n, held)) = held.filter(|_| !took()) else {
                out.push(w.to_string());
                continue;
            };
            let known = keywords(&held);
            let known: Vec<&str> = known.iter().map(String::as_str).collect();
            match crate::context::reads(w, &known) {
                crate::context::Reads::One(k) => out.push(k.to_string()),
                crate::context::Reads::Several => return Read::Ambiguous(n),
                _ => out.push(w.to_string()),
            }
        }
        let mut line = out.join(" ");
        if whole && !line.is_empty() {
            line.push(' ');
        }
        Read::Line(line)
    }

    /// The tests' answers: the image's own, and a donor's that holds a few others.
    fn two_tiers() -> Held {
        let mut h = crawled(&[
            (
                "",
                &[
                    "  interface  Select an interface to configure",
                    "  ip         Global IP configuration subcommands",
                    "  tr         Traps",
                    "  vlan       Vlan commands",
                ],
            ),
            IP,
            ACL,
            ("ip access-list standard ", &["  <1-99>  Number"]),
            (
                "tr ",
                &[
                    "  alpha  First",
                    "  beta   Second",
                    "  gamma  Third",
                    "  <cr>   <cr>",
                ],
            ),
            (
                "tr alpha ",
                &["  beta   Second", "  gamma  Third", "  <cr>   <cr>"],
            ),
            (
                "tr beta ",
                &["  alpha  First", "  gamma  Third", "  <cr>   <cr>"],
            ),
            (
                "tr gamma ",
                &["  alpha  First", "  beta   Second", "  <cr>   <cr>"],
            ),
            ("vlan ", &["  <1-4094>  VLAN ID"]),
            ("vlan 10 ", &["  <cr>   <cr>"]),
        ]);
        let donor = h.donors.entry(L.unwrap().to_string()).or_default();
        for (k, rows) in [
            ("", &["  vtp  VTP", "  vlan  Vlan commands"][..]),
            ("vtp ", &["  mode  Mode", "  domain  Domain"][..]),
            ("ip ", &["  multicast  Multicast", "  address  Address"][..]),
        ] {
            donor.insert(
                ("config".into(), k.to_string()),
                vec![("".into(), rows.iter().map(|r| r.to_string()).collect())],
            );
        }
        h
    }

    /// Held-answer lookups `f` made, and what it returned.
    fn lookups<T>(f: impl FnOnce() -> T) -> (T, usize) {
        LOOKUPS.with(|n| n.set(0));
        let out = f();
        (out, LOOKUPS.with(std::cell::Cell::get))
    }

    #[test]
    fn a_line_is_read_as_it_was_before_the_prefix_keys_were_kept() {
        let h = two_tiers();
        let (own, donor) = (h.own(L), h.donor(L).unwrap());
        let lines = [
            "",
            " ",
            "tr ",
            "tr al ",
            "tr alpha beta ",
            "tr al be ",
            "tr alp be ga ",
            "t alpha b g ",
            "tr beta alpha g",
            "TR  ALPHA   BETA ",
            "tr alpha beta gamma delta ",
            "ip acc ext ",
            "ip acc e ",
            "ip acc st 5 ",
            "ip acc st 5 6 7 ",
            "ip a ",
            "ip address 1.2.3.4 255.0.0.0 ",
            "i ac ",
            "i",
            "vlan 10 ",
            "vl 10 20 30 ",
            "vlan 10 tr alpha beta ",
            "vt mo ",
            "vtp d x ",
            "v ",
            "ip m ",
            "ip mu x ",
            "zz yy xx ",
        ];
        for tiers in [vec![own], vec![own, donor], vec![donor, own]] {
            for ctx in [Some(""), Some("router ospf"), None] {
                for typed in lines {
                    assert_eq!(
                        read(&tiers, "config", ctx, typed, &|_| false),
                        read_before(&tiers, "config", ctx, typed, &|_| false),
                        "{typed:?} in {ctx:?} through {} tiers",
                        tiers.len()
                    );
                }
            }
        }
        // the lines read through a held answer are not all left as typed
        let tiers = [own, donor];
        let read_as = |t: &str| read(&tiers, "config", Some(""), t, &|_| false);
        assert_eq!(
            read_as("ip acc ext "),
            Read::Line("ip access-list extended ".into())
        );
        assert_eq!(read_as("ip a "), Read::Ambiguous(0));
        assert_eq!(
            read(&[donor], "config", Some(""), "vt mo ", &|_| false),
            Read::Line("vtp mode ".into())
        );
    }

    #[test]
    fn a_long_line_is_read_with_lookups_in_step_with_its_words() {
        let h = two_tiers();
        let tiers = [h.own(L), h.donor(L).unwrap()];
        let words = 200;
        let tail: String = (0..words).map(|i| format!("w{i} ")).collect();
        // values after a held word, after a held keyword set, and with no held word at all
        for head in ["ip ", "tr alpha ", ""] {
            let typed = format!("{head}{tail}");
            let (now, took) = lookups(|| read(&tiers, "config", Some(""), &typed, &|_| false));
            let (before, was) =
                lookups(|| read_before(&tiers, "config", Some(""), &typed, &|_| false));
            assert_eq!(now, before, "{head:?}");
            // each word costs a lookup in each tier, one for the word as typed and a few for the
            // derivation's parent, not one per split of the line before it
            assert!(took <= 8 * words, "{head:?}: {took} lookups");
            assert!(was > 4 * took, "{head:?}: {was} lookups before, {took} now");
        }
    }

    #[test]
    fn a_value_is_left_as_typed_and_the_words_after_it_are_read() {
        let h = crawled(&[
            TOP,
            (
                "vlan ",
                &[
                    "  <1-4094>    VLAN ID",
                    "  access-map  Create vlan access-map",
                ],
            ),
            (
                "access-class ",
                &[
                    "  <1-199>  IP access list",
                    "  WORD     Access-list name",
                    "  in       Filter incoming connections",
                    "  out      Filter outgoing connections",
                ],
            ),
            ("access-class 1 ", &["  in   Filter incoming connections"]),
            ("access-class 1 in ", &["  <cr>"]),
            ("vlan 10 ", &["  <cr>"]),
        ]);
        let ask = |t: &str| h.help_for(L, "config", Some(""), t, |_| false).cloned();
        assert_eq!(ask("vlan 10 "), Some(vec!["  <cr>".to_string()]));
        assert_eq!(ask("vl 10 "), Some(vec!["  <cr>".to_string()]));
        assert_eq!(ask("access-class 1 i "), Some(vec!["  <cr>".to_string()]));
        // a name that begins no keyword is a value; nothing is held after it
        assert_eq!(ask("access-class acl1 "), None);
    }

    #[test]
    fn a_word_the_switch_answered_after_is_taken_as_typed() {
        // the C9300's `ipv6 nd ?` leaves out `ra`, which it answers after
        let h = crawled(&[
            TOP,
            (
                "ipv6 ",
                &["  nd  IPv6 interface Neighbor Discovery subcommands"],
            ),
            (
                "ipv6 nd ",
                &[
                    "  ra-throttler  Configure RA throttler on the interface",
                    "  raguard       Configure RA guard on the interface",
                ],
            ),
            ("ipv6 nd ra ", &["  dns  DNS"]),
            ("ipv6 nd ra dns ", &["  server  DNS Server"]),
        ]);
        let ask = |t: &str| h.help_for(L, "config", Some(""), t, |_| false).cloned();
        assert_eq!(
            ask("ipv n ra d "),
            Some(vec!["  server  DNS Server".to_string()])
        );
        assert_eq!(
            ask("ipv6 nd r ").map(|a| refusal_for("ipv6 nd r ", a)),
            Some(vec!["% Ambiguous command:  \"ipv6 nd r \"".to_string()])
        );
    }

    #[test]
    fn a_word_where_no_answer_is_held_leaves_the_rest_of_the_line_as_typed() {
        // `snmp-server ?` is not held: its next word cannot be read, nor anything after it
        let h = crawled(&[
            TOP,
            (
                "snmp-server enable traps ",
                &["  stackwise  STACKWISE traps"],
            ),
        ]);
        let ask = |t: &str| h.help_for(L, "config", Some(""), t, |_| false).cloned();
        assert_eq!(
            ask("snmp-server enable traps "),
            Some(vec!["  stackwise  STACKWISE traps".to_string()])
        );
        assert_eq!(ask("snmp-server ena traps "), None);
        assert_eq!(
            h.answer(L, "config", Some(""), "snmp-server ena traps ", |_| false),
            None
        );
    }

    #[test]
    fn abbreviations_in_a_submode_are_read_against_its_context_s_answers() {
        let mut h = Held::default();
        let ospf = |a: &str| vec![("router ospf".to_string(), vec![a.to_string()])];
        own(&mut h).insert(
            ("config-router".into(), "".into()),
            vec![
                (
                    "router ospf".into(),
                    vec![
                        "  area       OSPF area parameters".into(),
                        "  auto-cost  Calculate OSPF interface cost".into(),
                    ],
                ),
                ("router bgp".into(), vec!["  address-family  Enter".into()]),
            ],
        );
        own(&mut h).insert(
            ("config-router".into(), "area ".into()),
            ospf("  <0-4294967295>  OSPF area ID"),
        );
        let ask = |c: &str, t: &str| {
            h.help_for(L, "config-router", Some(c), t, |_| false)
                .cloned()
        };
        assert_eq!(
            ask("router ospf", "ar "),
            Some(vec!["  <0-4294967295>  OSPF area ID".to_string()])
        );
        // `a` begins both of OSPF's keywords; under BGP `ar` begins none and nothing is held
        assert_eq!(
            ask("router ospf", "a ").map(|a| refusal_for("a ", a)),
            Some(vec!["% Ambiguous command:  \"a \"".to_string()])
        );
        assert_eq!(ask("router bgp", "ar "), None);
    }

    #[test]
    fn an_image_s_own_crawl_answers_only_at_the_session_s_level() {
        let mut h = Held::default();
        let key = ("config".to_string(), "ip ".to_string());
        for (level, a) in [
            ("network-advantage", "advantage"),
            ("network-essentials", "essentials"),
        ] {
            h.crawls
                .entry(level.into())
                .or_default()
                .insert(key.clone(), vec![("".into(), answer(a))]);
        }
        h.help
            .insert(("privileged".into(), "show ".into()), answer("capture"));
        let ask = |l: Option<&str>| h.help_for(l, "config", Some(""), "ip ", |_| false).cloned();
        assert_eq!(ask(Some("network-advantage")), Some(answer("advantage")));
        assert_eq!(ask(Some("network-essentials")), Some(answer("essentials")));
        assert_eq!(ask(Some("lanbase")), None);
        assert_eq!(ask(None), None);
        // a capture's answer holds at any level, and with none
        for l in [Some("lanbase"), None] {
            assert_eq!(
                h.answer(l, "privileged", Some(""), "show ", |_| false),
                Some(answer("capture"))
            );
        }
    }

    const ADV: &str = "network-advantage";
    const ESS: &str = "network-essentials";

    fn row(
        level: &str,
        mode: &str,
        context: &str,
        key: &str,
        a: &[&str],
    ) -> crate::corpus::CrawlRow {
        (
            level.into(),
            mode.into(),
            context.into(),
            key.into(),
            a.iter().map(|r| r.to_string()).collect(),
        )
    }

    const STACKWISE: &str = "snmp-server enable traps stackwise ";

    /// A C9300 17.6 crawl at both levels, and a C9200 17.15 capture's `show ?` and crawl at
    /// network-advantage.
    fn lent() -> crate::corpus::Corpus {
        let donor = crate::corpus::HeldTable {
            crawl: vec![
                row(
                    ADV,
                    "config",
                    "",
                    "",
                    &[
                        "  ip      Global IP configuration subcommands",
                        "  router  Enable a routing process",
                    ],
                ),
                row(ADV, "config", "", "ip ", &["  routing  Enable IP routing"]),
                row(ADV, "config", "", "bogus ", &[crate::corpus::UNRECOGNIZED]),
                row(
                    ADV,
                    "config",
                    "",
                    "logging ",
                    &["  host  Set syslog server"],
                ),
                row(
                    ADV,
                    "config-router",
                    "router ospf",
                    "",
                    &["  area  OSPF area parameters"],
                ),
                row(
                    ADV,
                    "privileged",
                    "",
                    "",
                    &[
                        "  show  Show running system information",
                        "  ssh   Open a secure shell client connection",
                    ],
                ),
                row(ADV, "privileged", "", "show ", &["  vlan  VTP VLAN status"]),
                row(
                    ADV,
                    "config",
                    "",
                    STACKWISE,
                    &["  a     A", "  b     B", "  c     C", "  <cr>  <cr>"],
                ),
                row(
                    ADV,
                    "config",
                    "",
                    &format!("{STACKWISE}a "),
                    &["  b     B", "  c     C", "  <cr>  <cr>"],
                ),
                row(
                    ADV,
                    "config",
                    "",
                    &format!("{STACKWISE}b "),
                    &["  a     A", "  c     C", "  <cr>  <cr>"],
                ),
                row(
                    ESS,
                    "config",
                    "",
                    "ip ",
                    &["  multicast  Global IP multicast commands"],
                ),
            ],
            ..Default::default()
        };
        let target = crate::corpus::HeldTable {
            help: vec![(
                "privileged".into(),
                "show ".into(),
                vec!["  vlan  capture".into()],
            )],
            crawl: vec![
                row(ADV, "config", "", "logging ", &["  buffered  own crawl"]),
                row(
                    ADV,
                    "config",
                    "",
                    "ip routing ",
                    &[crate::corpus::UNRECOGNIZED],
                ),
            ],
            ..Default::default()
        };
        crate::corpus::Corpus {
            held: vec![
                ("c9200".into(), "17.15".into(), target),
                ("c9300".into(), "17.6".into(), donor),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn a_donor_answers_after_the_image_s_own_answers_and_before_the_books() {
        let c = lent();
        let on = [("c9200", "17.15", ADV)];
        let h = Held::with_donors(&c, "Catalyst 9200", "17.15", &on);
        let ask = |l: Option<&str>, m: &str, ctx: Option<&str>, t: &str| {
            h.answered(l, m, ctx, t, |_| false)
        };
        let adv = Some(ADV);
        // the image's capture and its own crawl, a refusal too, before the donor
        assert_eq!(
            ask(adv, "privileged", Some(""), "show "),
            Some((Tier::Own, answer("  vlan  capture")))
        );
        assert_eq!(
            ask(adv, "config", Some(""), "logging "),
            Some((Tier::Own, answer("  buffered  own crawl")))
        );
        assert_eq!(
            ask(adv, "config", Some(""), "ip routing "),
            Some((Tier::Own, answer(crate::corpus::UNRECOGNIZED)))
        );
        // where the image holds none, the donor's, its refusals among them
        assert_eq!(
            ask(adv, "config", Some(""), "ip "),
            Some((Tier::Donor, answer("  routing  Enable IP routing")))
        );
        assert_eq!(
            ask(adv, "config", Some(""), "bogus "),
            Some((Tier::Donor, answer(crate::corpus::UNRECOGNIZED)))
        );
        // in the context the donor asked in only
        assert_eq!(
            ask(adv, "config-router", Some("router ospf"), ""),
            Some((Tier::Donor, answer("  area  OSPF area parameters")))
        );
        for ctx in [Some("router bgp"), Some(""), None] {
            assert_eq!(ask(adv, "config-router", ctx, ""), None, "{ctx:?}");
        }
        // a line neither holds is the books'
        assert_eq!(ask(adv, "config", Some(""), "vtp "), None);
        // another level's mapping is off, and a session with no level has no donor
        assert_eq!(ask(Some(ESS), "config", Some(""), "ip "), None);
        assert_eq!(ask(None, "config", Some(""), "ip "), None);
        // at a level whose mapping is on, the donor's answer at that level
        let both = Held::with_donors(
            &c,
            "Catalyst 9200",
            "17.15",
            &[on[0], ("c9200", "17.15", ESS)],
        );
        assert_eq!(
            both.answer(Some(ESS), "config", Some(""), "ip ", |_| false),
            Some(answer("  multicast  Global IP multicast commands"))
        );
        // with no mapping enabled the books answer
        let off = Held::with_donors(&c, "Catalyst 9200", "17.15", &[]);
        assert!(off.donors.is_empty());
        assert_eq!(off.answer(adv, "config", Some(""), "ip ", |_| false), None);
        // shipped, `license::ENABLED` lists the 9200 at 17.15: its donor answers
        let shipped = Held::from_corpus(&c, "Catalyst 9200", "17.15");
        assert_eq!(
            shipped.answer(adv, "config", Some(""), "ip ", |_| false),
            Some(answer("  routing  Enable IP routing"))
        );
        // the caret's lookup never reads a donor
        assert_eq!(h.help_for(adv, "config", Some(""), "ip ", |_| false), None);
    }

    #[test]
    fn a_line_read_through_a_donor_s_answer_takes_the_image_s_answer_for_it() {
        let c = lent();
        let h = Held::with_donors(&c, "Catalyst 9200", "17.15", &[("c9200", "17.15", ADV)]);
        // the image holds `show ?` and no answer before it; the donor holds both
        for typed in ["show ", "sh ", "SH  "] {
            assert_eq!(
                h.answered(Some(ADV), "privileged", Some(""), typed, |_| false),
                Some((Tier::Own, answer("  vlan  capture"))),
                "{typed:?}"
            );
        }
        // a line read through the donor's answer that neither holds is the books'
        assert_eq!(
            h.answered(Some(ADV), "privileged", Some(""), "ss ", |_| false),
            None
        );
    }

    #[test]
    fn a_donor_s_answers_read_abbreviations_and_derive_several_picks() {
        let c = lent();
        let h = Held::with_donors(&c, "Catalyst 9200", "17.15", &[("c9200", "17.15", ADV)]);
        let ask = |t: &str| h.answered(Some(ADV), "config", Some(""), t, |_| false);
        assert_eq!(
            ask("i "),
            Some((Tier::Donor, answer("  routing  Enable IP routing")))
        );
        assert_eq!(
            ask("ro"),
            None,
            "a word still being typed is left to the books"
        );
        let derived = ask(&format!("{STACKWISE}a b ")).unwrap();
        assert_eq!(derived.0, Tier::Donor);
        assert_eq!(keywords(&derived.1), ["c"]);
        assert_eq!(derived.1.last().map(String::as_str), Some("  <cr>  <cr>"));
        // the same lines with the mapping off
        let off = Held::with_donors(&c, "Catalyst 9200", "17.15", &[]);
        for t in ["i ", &format!("{STACKWISE}a b ")] {
            assert_eq!(
                off.answer(Some(ADV), "config", Some(""), t, |_| false),
                None,
                "{t}"
            );
        }
    }

    #[test]
    fn an_answer_relaid_drops_and_adds_rows_and_widens_its_column_to_fit() {
        let lines = |s: &str| -> Vec<String> { s.split('\n').map(str::to_string).collect() };
        let held = lines(
            "Exec commands:\n  AppGigabitEthernet    App-hosting Gigabit Ethernet\n  \
             FortyGigabitEthernet  Forty Gigabit Ethernet\n  Loopback              Loopback \
             interface which\n                        wraps\n  range                 interface \
             range command\n  |                     Output modifiers\n  <cr>                  <cr>\n",
        );
        let out = relaid(
            &held,
            |w| w != "FortyGigabitEthernet",
            &[
                ("HundredGigE", ""),
                ("TwentyFiveGigE", "Twenty Five Gigabit Ethernet"),
            ],
        );
        assert_eq!(
            out,
            [
                "Exec commands:",
                "  AppGigabitEthernet  App-hosting Gigabit Ethernet",
                "  HundredGigE",
                "  Loopback            Loopback interface which",
                "                      wraps",
                "  TwentyFiveGigE      Twenty Five Gigabit Ethernet",
                "  range               interface range command",
                "  |                   Output modifiers",
                "  <cr>                <cr>",
                "",
            ]
        );
        // a word sorting after every row goes before `|` and `<cr>`; a bare `<cr>` sets no column
        let classic = lines("  ab  A\n  <cr>\n");
        assert_eq!(
            relaid(&classic, |_| true, &[("zz", "Z")]),
            ["  ab  A", "  zz  Z", "  <cr>", ""]
        );
        assert_eq!(relaid(&classic, |w| w != "ab", &[]), ["  <cr>", ""]);
        let refusal = answer(crate::corpus::UNRECOGNIZED);
        assert_eq!(relaid(&refusal, |_| false, &[("x", "")]), refusal);
    }

    #[test]
    fn a_crawl_answer_is_found_whatever_address_was_typed() {
        let mut h = Held::default();
        own(&mut h).insert(
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
                h.help_for(L, "config", Some(""), typed, |_| false).cloned(),
                Some(answer("arpa")),
                "{typed}"
            );
        }
        assert_eq!(
            h.help_for(L, "config", Some(""), "arp 10.1.1.1 ", |_| false),
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

    #[test]
    fn a_derived_answer_s_column_and_cr_are_as_classic_ios_prints_them() {
        // the C2960G at 15.0(2)SE: a bare `<cr>` does not widen the column
        let held: BTreeMap<&str, Vec<String>> = [
            (
                "ip arp inspection validate ",
                &[
                    "  dst-mac  Validate destination MAC address",
                    "  ip       Validate IP addresses",
                    "  src-mac  Validate source MAC address",
                ][..],
            ),
            (
                "ip arp inspection validate dst-mac ",
                &[
                    "  ip       Validate IP addresses",
                    "  src-mac  Validate source MAC address",
                    "  <cr>",
                ],
            ),
            (
                "ip arp inspection validate src-mac ",
                &[
                    "  dst-mac  Validate destination MAC address",
                    "  ip       Validate IP addresses",
                    "  <cr>",
                ],
            ),
        ]
        .into_iter()
        .map(|(k, a)| (k, a.iter().map(|l| l.to_string()).collect()))
        .collect();
        let lookup = |t: &str| held.get(t);
        for typed in [
            "ip arp inspection validate dst-mac src-mac ",
            "ip arp inspection validate src-mac dst-mac ",
        ] {
            assert_eq!(
                derived(typed, lookup),
                Some(vec![
                    "  ip  Validate IP addresses".to_string(),
                    "  <cr>".to_string()
                ]),
                "{typed}"
            );
        }
        // one pick is held, not derived; a word asked after is not
        assert_eq!(derived("ip arp inspection validate dst-mac ", lookup), None);
        assert_eq!(
            derived("ip arp inspection validate dst-mac src-mac", lookup),
            None
        );
        // a pick repeated is offered again only where its own answer offers it
        assert_eq!(
            derived("ip arp inspection validate dst-mac dst-mac ", lookup),
            None
        );
    }

    /// Leave-one-out over the crawl logs under `VSWITCH_CRAWLS` (each crawl folder with a `level`
    /// file): every held answer `derived` can give with that answer itself set aside, compared with
    /// what the switch printed. `--nocapture` prints the counts and up to three keys per mismatch
    /// class.
    #[test]
    #[ignore = "an audit: reads the crawl logs VSWITCH_CRAWLS names"]
    fn derived_answers_against_the_held_ones() {
        let Ok(dir) = std::env::var("VSWITCH_CRAWLS") else {
            return;
        };
        for f in crate::corpus::Source::read_crawls(std::path::Path::new(&dir)) {
            let mut held: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
            for (m, c, k, a) in crate::corpus::crawled_help(&f.text) {
                held.entry((m, c, k)).or_insert(a);
            }
            for longest in [true, false] {
                let (mut tested, mut exact) = (0, 0);
                let mut classes: BTreeMap<&str, Vec<String>> = BTreeMap::new();
                for ((m, c, k), a) in &held {
                    let lookup = |t: &str| {
                        let t = crate::context::key(t);
                        (t != *k)
                            .then(|| held.get(&(m.clone(), c.clone(), t)))
                            .flatten()
                    };
                    let words: Vec<&str> = k.split_whitespace().collect();
                    let mut splits: Vec<usize> = (1..words.len().saturating_sub(1)).collect();
                    if longest {
                        splits.reverse();
                    }
                    if !k.ends_with(' ') {
                        continue;
                    }
                    let Some(d) = splits
                        .into_iter()
                        .find_map(|i| derive_at(&words[..i], &words[i..], &lookup))
                    else {
                        continue;
                    };
                    tested += 1;
                    if d == *a {
                        exact += 1;
                        continue;
                    }
                    let names = |x: &[String]| -> Vec<String> {
                        rows(x).iter().map(|r| r.word.to_string()).collect()
                    };
                    let (dn, an) = (names(&d), names(a));
                    let extra: Vec<&String> = dn.iter().filter(|w| !an.contains(w)).collect();
                    let missing: Vec<&String> = an.iter().filter(|w| !dn.contains(w)).collect();
                    let class = if is_refusal(a) {
                        "the switch refused"
                    } else if dn == an {
                        "same rows, other text"
                    } else if dn.iter().collect::<BTreeSet<_>>() == an.iter().collect() {
                        "same rows, other order"
                    } else if extra.is_empty() {
                        "the switch offered more"
                    } else if missing.is_empty() {
                        "the switch offered fewer"
                    } else {
                        "other rows"
                    };
                    let e = classes.entry(class).or_default();
                    let shown = e.iter().filter(|x| !x.is_empty()).count();
                    let prefix = |x: &str| x.split(' ').take(4).collect::<Vec<_>>().join(" ");
                    let new_command = !e
                        .iter()
                        .any(|x| prefix(x) == prefix(&format!("{m} [{c}] {k:?}")));
                    if shown < 8 && new_command {
                        let first = d
                            .iter()
                            .zip(a)
                            .find(|(x, y)| x != y)
                            .map(|(x, y)| format!("derived {x:?} switch {y:?}"))
                            .unwrap_or_default();
                        e.push(format!(
                            "{m} [{c}] {k:?}: extra {extra:?} missing {missing:?} {first}"
                        ));
                    } else {
                        e.push(String::new());
                    }
                }
                println!(
                    "{}-{} {}: {tested} tested, {exact} exact",
                    f.book,
                    f.release,
                    if longest { "longest Q" } else { "shortest Q" }
                );
                for (class, ex) in classes {
                    println!("  {class}: {}", ex.len());
                    for x in ex.iter().filter(|x| !x.is_empty()) {
                        println!("    {x}");
                    }
                }
            }
        }
    }

    /// Over the crawl logs under `VSWITCH_CRAWLS`: every held answer whose key has two keywords or
    /// more (each a keyword row of the held answer for the line before it, in the same mode and
    /// context), asked through `Held::answer` with each keyword cut to its shortest unique start
    /// among those rows, and again to three characters where those are unique, compared with the
    /// held answer as the switch would print it for the line typed. Also counts the words the crawl
    /// itself typed as the start of a keyword, and the keys with a word whose line before it holds
    /// no answer in that context.
    #[test]
    #[ignore = "an audit: reads the crawl logs VSWITCH_CRAWLS names"]
    fn abbreviated_keywords_against_the_held_answers() {
        use crate::context::{Reads, reads};
        let Ok(dir) = std::env::var("VSWITCH_CRAWLS") else {
            return;
        };
        for f in crate::corpus::Source::read_crawls(std::path::Path::new(&dir)) {
            let mut first: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
            for (m, c, k, a) in crate::corpus::crawled_help(&f.text) {
                first.entry((m, c, k)).or_insert(a);
            }
            let mut held = Held::default();
            for ((m, c, k), a) in &first {
                held.crawls
                    .entry(f.level.clone())
                    .or_default()
                    .entry((m.clone(), k.clone()))
                    .or_default()
                    .push((c.clone(), a.clone()));
            }
            let contexts_of: BTreeMap<(&str, &str), usize> =
                first.keys().fold(BTreeMap::new(), |mut acc, (m, _, k)| {
                    *acc.entry((m.as_str(), k.as_str())).or_default() += 1;
                    acc
                });
            let name = format!("{}-{}", f.book, f.release);
            // the keyword rows of the held answer before each word, none where none is held
            let parents = |m: &str, c: &str, words: &[&str]| -> Vec<Option<Vec<String>>> {
                (0..words.len())
                    .map(|i| {
                        let before = if i == 0 {
                            String::new()
                        } else {
                            format!("{} ", words[..i].join(" "))
                        };
                        first.get(&(m.to_string(), c.to_string(), before)).map(|a| {
                            rows(a)
                                .iter()
                                .map(|r| r.word.to_string())
                                .filter(|w| keyword(w))
                                .collect()
                        })
                    })
                    .collect()
            };
            // the crawl's own abbreviations, and keys below a line no answer is held for
            let (mut typed_short, mut deep, mut orphaned, mut elsewhere) = (0, 0, 0, 0);
            let mut short_examples: Vec<String> = vec![];
            for ((m, c, k), a) in &first {
                let words: Vec<&str> = k.split_whitespace().collect();
                if words.len() < 2 {
                    continue;
                }
                deep += 1;
                let ps = parents(m, c, &words);
                let missing: Vec<usize> = (1..words.len()).filter(|&i| ps[i].is_none()).collect();
                if let Some(&i) = missing.first() {
                    orphaned += 1;
                    let before = format!("{} ", words[..i].join(" "));
                    if contexts_of.contains_key(&(m.as_str(), before.as_str())) {
                        elsewhere += 1;
                    }
                }
                for (i, w) in words.iter().enumerate() {
                    let Some(known) = &ps[i] else { continue };
                    let known: Vec<&str> = known.iter().map(String::as_str).collect();
                    if !matches!(reads(w, &known), Reads::Exact | Reads::None) {
                        typed_short += 1;
                        if short_examples.len() < 5 {
                            short_examples
                                .push(format!("{m} [{c}] {k:?} word {w:?}: {:?}", a.first()));
                        }
                    }
                }
            }
            println!(
                "{name}: {} keys, {deep} of two words or more; {orphaned} with a word whose line before it holds no answer in its context ({elsewhere} held in another context); {typed_short} words the crawl typed as a keyword's start",
                first.len()
            );
            for x in short_examples {
                println!("    {x}");
            }
            for three in [false, true] {
                let (mut tested, mut exact) = (0, 0);
                let mut classes: BTreeMap<&str, (usize, Vec<String>)> = BTreeMap::new();
                for ((m, c, k), a) in &first {
                    let words: Vec<&str> = k.split_whitespace().collect();
                    let whole = k.ends_with(' ');
                    let ps = parents(m, c, &words);
                    let mut keywords = 0;
                    let mut cut: Vec<String> = vec![];
                    for (i, w) in words.iter().enumerate() {
                        let known: Vec<&str> = ps[i]
                            .as_ref()
                            .map(|k| k.iter().map(String::as_str).collect())
                            .unwrap_or_default();
                        let last = i + 1 == words.len() && !whole;
                        if last || reads(w, &known) != Reads::Exact {
                            cut.push(w.to_string());
                            continue;
                        }
                        keywords += 1;
                        let unique = |p: &str| matches!(reads(p, &known), Reads::One(x) if x.eq_ignore_ascii_case(w));
                        let short = if three {
                            (w.len() > 3 && w.is_char_boundary(3) && unique(&w[..3]))
                                .then(|| w[..3].to_string())
                        } else {
                            (1..w.len())
                                .filter(|&n| w.is_char_boundary(n))
                                .map(|n| &w[..n])
                                .find(|p| unique(p))
                                .map(str::to_string)
                        };
                        cut.push(short.unwrap_or_else(|| w.to_string()));
                    }
                    let mut typed = cut.join(" ");
                    if whole {
                        typed.push(' ');
                    }
                    if keywords < 2 || typed == *k {
                        continue;
                    }
                    tested += 1;
                    let want = refusal_for(&typed, a.clone());
                    let got = held
                        .answer(Some(&f.level), m, Some(c), &typed, |_| false)
                        .map(|g| refusal_for(&typed, g));
                    if got.as_ref() == Some(&want) {
                        exact += 1;
                        continue;
                    }
                    let class = match &got {
                        None => "no held answer: the books answer",
                        Some(g) if is_refusal(g) && !is_refusal(&want) => {
                            "refused where the switch answered"
                        }
                        Some(_) if is_refusal(&want) => "the switch refused",
                        Some(g) if rows(g).len() == rows(&want).len() => "same rows, other text",
                        Some(_) => "other rows",
                    };
                    let e = classes.entry(class).or_default();
                    e.0 += 1;
                    if e.1.len() < 4 {
                        e.1.push(format!(
                            "{m} [{c}] {typed:?} for {k:?}: got {:?}",
                            got.as_ref().and_then(|g| g.first())
                        ));
                    }
                }
                println!(
                    "{name} {}: {tested} tested, {exact} exact",
                    if three {
                        "three characters"
                    } else {
                        "shortest unique start"
                    }
                );
                for (class, (n, ex)) in classes {
                    println!("  {class}: {n}");
                    for x in ex {
                        println!("    {x}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_line_with_other_numbers_is_read_as_the_line_both_crawls_answered() {
        let answer =
            |rows: &[&str]| -> Vec<String> { rows.iter().map(|r| format!("  {r}")).collect() };
        let crawl = |entries: &[(&str, &[&str])]| -> Crawl {
            entries
                .iter()
                .map(|(k, rows)| {
                    (
                        ("config".to_string(), k.to_string()),
                        vec![(String::new(), answer(rows))],
                    )
                })
                .collect()
        };
        let session = &["<1-66>  SPAN session number"][..];
        let one = &["destination  SPAN destination", "type  session type"][..];
        let at = crawl(&[("monitor session ", session), ("monitor session 1 ", one)]);
        let above = crawl(&[("monitor session ", session), ("monitor session 1 ", one)]);
        let read = |key: &str| numbers_as_crawled(&at, &above, "config", "", key);
        assert_eq!(
            read("monitor session 2 ").as_deref(),
            Some("monitor session 1 ")
        );
        // out of the range, the crawled line itself, a number first, a word unfinished
        assert_eq!(read("monitor session 99 "), None);
        assert_eq!(read("monitor session 1 "), None);
        assert_eq!(read("2 "), None);
        assert_eq!(read("monitor session 2"), None);
        // in another context, or answered at one level alone
        assert_eq!(
            numbers_as_crawled(
                &at,
                &above,
                "config",
                "interface vlan",
                "monitor session 2 "
            ),
            None
        );
        let alone = crawl(&[("monitor session ", session)]);
        assert_eq!(
            numbers_as_crawled(&at, &alone, "config", "", "monitor session 2 "),
            None
        );
    }
}
