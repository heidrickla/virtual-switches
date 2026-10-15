//! What real switches' captures teach vswitch, compiled into one table set, `data/corpus.json.gz`,
//! which the binary embeds: the lines they held and where and how IOS printed them, their `?`
//! answers and keyword descriptions, IOS's printing order, each book's NVRAM size and the command
//! forms they used. `vswitch corpus <captures folder>` writes it from a folder laid out as
//! `<book>-<release>/<host>-running-config.txt`, `<host>-show.log` and `<host>-stack.txt`.

use crate::live;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::Path;

/// One file of a captures folder.
#[derive(Clone, Debug)]
pub struct File {
    pub book: String,
    pub release: String,
    pub name: String,
    pub text: String,
}

/// A captures folder, each kind of file sorted by book, release and name.
#[derive(Clone, Debug, Default)]
pub struct Source {
    pub running: Vec<File>,
    pub logs: Vec<File>,
    pub stacks: Vec<File>,
    /// each folder's `crawl/crawl.log`, the `?` probes of a crawl of the switch: read for the help
    /// it held, and for nothing else
    pub crawls: Vec<File>,
    /// the site's own designs and scripts, beside the captures folder (`../designs`): read only for
    /// the names they give
    pub designs: Vec<String>,
}

impl Source {
    /// Every `<book>-<release>` folder under `dir`.
    pub fn read(dir: &Path) -> Result<Source, String> {
        let mut src = Source::default();
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for d in entries.flatten() {
            let folder = d.file_name().to_string_lossy().to_string();
            let Some((book, release)) = folder.split_once('-') else {
                continue;
            };
            let files = std::fs::read_dir(d.path()).map_err(|e| format!("{folder}: {e}"))?;
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().to_string();
                let list = if name.ends_with("running-config.txt") {
                    &mut src.running
                } else if name.ends_with(".log") {
                    &mut src.logs
                } else if name.ends_with("-stack.txt") {
                    &mut src.stacks
                } else {
                    continue;
                };
                let text = std::fs::read_to_string(f.path())
                    .map_err(|e| format!("{folder}/{name}: {e}"))?;
                list.push(File {
                    book: book.into(),
                    release: release.into(),
                    name,
                    text,
                });
            }
        }
        src.crawls = Source::read_crawls(dir);
        if let Ok(designs) = std::fs::read_dir(dir.join("../designs")) {
            for f in designs.flatten() {
                if let Ok(text) = std::fs::read_to_string(f.path()) {
                    src.designs.push(text);
                }
            }
        }
        for list in [
            &mut src.running,
            &mut src.logs,
            &mut src.stacks,
            &mut src.crawls,
        ] {
            list.sort_by(|a, b| {
                (&a.book, &a.release, &a.name).cmp(&(&b.book, &b.release, &b.name))
            });
        }
        Ok(src)
    }

    /// Every `<book>-<release>/crawl/crawl.log` under `dir`, and no other file of a crawl's folder.
    pub fn read_crawls(dir: &Path) -> Vec<File> {
        let mut out = vec![];
        for d in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let folder = d.file_name().to_string_lossy().to_string();
            let Some((book, release)) = folder.split_once('-') else {
                continue;
            };
            if let Ok(text) = std::fs::read_to_string(d.path().join("crawl").join("crawl.log")) {
                out.push(File {
                    book: book.into(),
                    release: release.into(),
                    name: "crawl.log".into(),
                    text,
                });
            }
        }
        out
    }

    /// The captures compiled into a build that names a captures folder.
    #[cfg(all(captures, test))]
    pub fn embedded() -> Source {
        let files = |t: &[(&str, &str, &str, &str)]| {
            t.iter()
                .map(|&(book, release, name, text)| File {
                    book: book.into(),
                    release: release.into(),
                    name: name.into(),
                    text: text.into(),
                })
                .collect()
        };
        Source {
            running: files(crate::grammars::LIVE),
            logs: files(crate::grammars::LIVE_LOGS),
            stacks: files(crate::grammars::LIVE_STACKS),
            // a crawl's log is not embedded in the binary: read from the folder it was built against
            crawls: Source::read_crawls(Path::new(env!("VSWITCH_CAPTURES_DIR"))),
            designs: std::fs::read_dir(concat!(env!("VSWITCH_CAPTURES_DIR"), "/../designs"))
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|f| std::fs::read_to_string(f.path()).ok())
                .collect(),
        }
    }
}

/// What the captures of one book at one release held (`live::Held`).
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct HeldTable {
    /// (block kind, line)
    pub lines: Vec<(String, String)>,
    /// (block kind, line, indentation)
    pub indent: Vec<(String, String, String)>,
    pub opens: Vec<String>,
    /// (line, the top-level line before it)
    pub after: Vec<(String, String)>,
    /// (mode, `help_key` of the line before `?`, the answer)
    pub help: Vec<(String, String, Vec<String>)>,
    /// (mode, context, `context::key` of the line before `?`, the answer) from a crawl, which asked
    /// in each submode as each of its entering lines reached it; absent from a corpus without one
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crawl: Vec<(String, String, String, Vec<String>)>,
    /// the captures read, by name
    pub captures: Vec<String>,
}

/// Commands by number, and which pairs real switches printed in one order only (`show::Orders`).
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct Precedence {
    pub known: Vec<usize>,
    pub pairs: Vec<(usize, usize)>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct Corpus {
    /// (book, release, table)
    pub held: Vec<(String, String, HeldTable)>,
    /// (release, block's first word, line's first word, indentation)
    pub word_indent: Vec<(String, String, String, String)>,
    /// (classic IOS, mode, line before `?`, keyword, description)
    pub described: Vec<(bool, String, String, String, String)>,
    /// each command printing order knows, by number
    pub numbers: Vec<String>,
    /// (book, release, top-level order)
    pub release_order: Vec<(String, String, Precedence)>,
    /// classic IOS, then IOS XE
    pub family_order: [Precedence; 2],
    pub interface_order: Precedence,
    pub line_order: Precedence,
    /// (book, bytes of NVRAM its `show version` names)
    pub nvram: Vec<(String, u64)>,
    /// (book, release, block kind, the keywords of a command a capture used)
    pub forms: Vec<(String, String, String, Vec<String>)>,
}

impl Corpus {
    pub fn to_gzip(&self) -> Vec<u8> {
        let json = serde_json::to_vec(self).expect("the corpus serializes");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&json).expect("compressing in memory");
        gz.finish().expect("compressing in memory")
    }

    pub fn from_gzip(bytes: &[u8]) -> Result<Corpus, String> {
        let mut json = String::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_string(&mut json)
            .map_err(|e| format!("the corpus: {e}"))?;
        serde_json::from_str(&json).map_err(|e| format!("the corpus: {e}"))
    }
}

/// The corpus the binary embeds; empty when the build had none.
pub fn get() -> &'static Corpus {
    static C: std::sync::OnceLock<Corpus> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        if crate::grammars::CORPUS.is_empty() {
            return Corpus::default();
        }
        Corpus::from_gzip(crate::grammars::CORPUS).expect("the embedded corpus reads")
    })
}

fn classic(release: &str) -> bool {
    crate::release::Release::parse(release).is_none_or(|r| r.classic())
}

/// The tables of one book's captures at one release.
fn held_table(src: &Source, book: &str, release: &str) -> HeldTable {
    let mut lines: BTreeSet<(String, String)> = BTreeSet::new();
    let mut indent: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut opens: BTreeSet<String> = BTreeSet::new();
    let mut after: BTreeMap<String, String> = BTreeMap::new();
    let mut help: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let mut captures = vec![];
    for f in src
        .running
        .iter()
        .filter(|f| f.book == book && f.release == release)
    {
        captures.push(f.name.clone());
        let mut top = String::new();
        for l in live::config_lines(&f.text) {
            let ind = &l[..l.len() - l.trim_start().len()];
            let line = l.trim().to_string();
            if ind.is_empty() {
                if !top.is_empty() {
                    after.entry(line.clone()).or_insert(top.clone());
                }
                top = line.clone();
                lines.insert((String::new(), line));
            } else {
                opens.insert(top.clone());
                let place = (live::kind(&top), line);
                indent
                    .entry(place.clone())
                    .or_insert_with(|| ind.to_string());
                lines.insert(place);
            }
        }
    }
    for f in src
        .logs
        .iter()
        .filter(|f| f.book == book && f.release == release)
    {
        let Some((_, ex)) = crate::capture::exchanges(&f.text) else {
            continue;
        };
        for e in ex {
            // an error answer depends on that switch's configuration, not the release
            if e.output.iter().any(|l| l.starts_with('%')) {
                continue;
            }
            if let Some(before) = e.command.strip_suffix('?') {
                help.entry((e.mode.clone(), live::help_key(before)))
                    .or_insert(e.output.clone());
            }
        }
    }
    let mut crawl: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
    for f in src
        .crawls
        .iter()
        .filter(|f| f.book == book && f.release == release)
    {
        for (mode, context, key, answer) in crawled_help(&f.text) {
            crawl.entry((mode, context, key)).or_insert(answer);
        }
    }
    HeldTable {
        lines: lines.into_iter().collect(),
        indent: indent.into_iter().map(|((k, l), i)| (k, l, i)).collect(),
        opens: opens.into_iter().collect(),
        after: after.into_iter().collect(),
        help: help.into_iter().map(|((m, k), a)| (m, k, a)).collect(),
        crawl: crawl
            .into_iter()
            .map(|((m, c, k), a)| (m, c, k, a))
            .collect(),
        captures,
    }
}

/// The `?` answers a crawl's log holds, as (mode, context, key, answer): the mode its prompt showed
/// and the context its entering lines give it, or privileged EXEC for a line typed after `do-exec`,
/// which runs the rest of the line there. Only a complete exchange is an answer: `swcon crawl` writes
/// a probe to the log once the switch has printed its answer and the prompt again, and leaves a probe
/// that timed out, left the mode or met a question out, so a probe is complete when another prompt
/// follows it. The last probe of a log read while the crawl writes it may be cut short and is left
/// out. A refused probe, one asked where the context is unknown and one whose answer holds a pager's
/// `--More--`, a control character (an erase's backspaces) or undecodable bytes are left out.
pub(crate) fn crawled_help(text: &str) -> Vec<(String, String, String, Vec<String>)> {
    let Some((_, exchanges)) = crate::capture::exchanges(text) else {
        return vec![];
    };
    let residue = |l: &str| {
        l.contains("--More--")
            || l.contains('\u{fffd}')
            || l.chars().any(|c| c.is_control() && c != '\t')
    };
    let complete = exchanges.len().saturating_sub(1);
    let mut out = vec![];
    for e in exchanges.into_iter().take(complete) {
        let Some(before) = e.command.strip_suffix('?') else {
            continue;
        };
        if e.output.iter().any(|l| l.starts_with('%') || residue(l)) || residue(&e.command) {
            continue;
        }
        let exec = before
            .trim_start()
            .split_once(char::is_whitespace)
            .filter(|(w, _)| crate::cli::names_do_exec(w) && crate::cli::is_config(&e.mode))
            .map(|(_, rest)| rest);
        let (mode, context, typed) = match exec {
            Some(rest) => ("privileged".to_string(), String::new(), rest),
            None => {
                let Some(context) = e.context.clone() else {
                    continue;
                };
                (e.mode.clone(), context, before)
            }
        };
        out.push((mode, context, crate::context::key(typed), e.output.clone()));
    }
    out
}

/// The indentation every capture at a release printed a block's own lines with, by the block's first
/// word and the line's; a pair printed two ways is left out.
pub(crate) fn word_indents<'a>(
    texts: impl Iterator<Item = &'a str>,
) -> BTreeMap<(String, String), String> {
    let mut seen: BTreeMap<(String, String), Option<String>> = BTreeMap::new();
    for text in texts {
        let mut top = "";
        // the least indentation in the block so far: a deeper line is in a section
        let mut least = usize::MAX;
        for l in live::config_lines(text) {
            let ind = &l[..l.len() - l.trim_start().len()];
            if ind.is_empty() {
                top = l.trim();
                least = usize::MAX;
                continue;
            }
            if ind.len() > least {
                continue;
            }
            least = ind.len();
            let first = |s: &str| s.split_whitespace().next().unwrap_or("").to_string();
            let agreed = seen
                .entry((first(top), first(l)))
                .or_insert_with(|| Some(ind.to_string()));
            if agreed.as_deref() != Some(ind) {
                *agreed = None;
            }
        }
    }
    seen.into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect()
}

/// The description beside each keyword in the `?` answers of every session log of the family, by
/// the mode asked in, the line typed before the `?` and the keyword, lowercase. A keyword row is two
/// spaces in; a row indented further continues a description and is left out.
fn described(src: &Source, classic_family: bool) -> BTreeMap<(String, String, String), String> {
    let mut out = BTreeMap::new();
    for f in src
        .logs
        .iter()
        .filter(|f| classic(&f.release) == classic_family)
    {
        let Some((_, ex)) = crate::capture::exchanges(&f.text) else {
            continue;
        };
        for e in ex {
            let Some(before) = e.command.strip_suffix('?') else {
                continue;
            };
            if e.output.iter().any(|l| l.starts_with('%')) {
                continue;
            }
            for row in &e.output {
                let Some(rest) = row.strip_prefix("  ") else {
                    continue;
                };
                if rest.starts_with(char::is_whitespace) {
                    continue;
                }
                let Some((word, what)) = rest.split_once(char::is_whitespace) else {
                    continue;
                };
                let what = what.trim();
                if !what.is_empty() {
                    let mode = live::help_mode(&e.mode).to_string();
                    out.entry((mode, live::help_key(before), word.to_lowercase()))
                        .or_insert_with(|| what.to_string());
                }
            }
        }
    }
    out
}

/// A precedence being gathered: every command printed, and every pair printed in that order.
#[derive(Default)]
struct Gathering {
    known: BTreeSet<usize>,
    pairs: BTreeSet<(usize, usize)>,
}

impl Gathering {
    fn add(&mut self, mut commands: Vec<usize>) {
        // a run of one command is one place
        commands.dedup();
        self.known.extend(&commands);
        for (i, &a) in commands.iter().enumerate() {
            for &b in &commands[i + 1..] {
                if a != b {
                    self.pairs.insert((a, b));
                }
            }
        }
    }

    fn join(&mut self, other: &Gathering) {
        self.known.extend(&other.known);
        self.pairs.extend(&other.pairs);
    }

    /// Without the pairs printed both ways round, which order nothing.
    fn settled(&self) -> Precedence {
        Precedence {
            known: self.known.iter().copied().collect(),
            pairs: self
                .pairs
                .iter()
                .filter(|(a, b)| !self.pairs.contains(&(*b, *a)))
                .copied()
                .collect(),
        }
    }
}

/// The NVRAM a book's switches have, as the first of its session logs naming it says.
fn nvram(src: &Source, book: &str) -> Option<u64> {
    src.logs
        .iter()
        .filter(|f| f.book == book)
        .flat_map(|f| f.text.lines())
        .find_map(|l| {
            let k = l
                .strip_suffix("bytes of non-volatile configuration memory.")
                .or_else(|| {
                    l.strip_suffix("bytes of flash-simulated non-volatile configuration memory.")
                })?;
            k.trim().strip_suffix('K')?.parse::<u64>().ok()
        })
        .map(|k| k * 1024)
}

/// Every table but the command forms, which need an engine on these tables (`with_forms`).
pub fn build(src: &Source) -> Corpus {
    let mut c = Corpus::default();
    let keys: BTreeSet<(String, String)> = src
        .running
        .iter()
        .chain(&src.logs)
        .chain(&src.crawls)
        .map(|f| (f.book.clone(), f.release.clone()))
        .collect();
    for (book, release) in &keys {
        c.held.push((
            book.clone(),
            release.clone(),
            held_table(src, book, release),
        ));
    }
    let releases: BTreeSet<&str> = src.running.iter().map(|f| f.release.as_str()).collect();
    for r in releases {
        let texts = src
            .running
            .iter()
            .filter(|f| f.release == r)
            .map(|f| f.text.as_str());
        for ((block, line), ind) in word_indents(texts) {
            c.word_indent.push((r.to_string(), block, line, ind));
        }
    }
    for family in [true, false] {
        for ((mode, key, word), what) in described(src, family) {
            c.described.push((family, mode, key, word, what));
        }
    }
    // printing order: each capture's top-level commands, and the own lines (one space in, not a
    // section's) of each of its interface and line blocks
    let mut numbers: BTreeMap<String, usize> = BTreeMap::new();
    let mut order: Vec<String> = vec![];
    let mut number = |line: &str| {
        let cmd = crate::show::command(line);
        *numbers.entry(cmd.clone()).or_insert_with(|| {
            order.push(cmd);
            order.len() - 1
        })
    };
    let mut release: BTreeMap<(String, String), Gathering> = BTreeMap::new();
    let (mut interface, mut line) = (Gathering::default(), Gathering::default());
    for f in &src.running {
        let mut top = vec![];
        let mut block: Option<(bool, Vec<usize>)> = None;
        let mut close = |block: Option<(bool, Vec<usize>)>| match block {
            Some((true, commands)) => interface.add(commands),
            Some((false, commands)) => line.add(commands),
            None => {}
        };
        for l in live::body(&f.text) {
            if !l.starts_with([' ', '\t']) {
                close(block.take());
                top.push(number(l));
                block = if l.starts_with("interface ") {
                    Some((true, vec![]))
                } else if l.starts_with("line ") {
                    Some((false, vec![]))
                } else {
                    None
                };
            } else if let Some((_, commands)) = block.as_mut()
                && l.len() - l.trim_start().len() == 1
            {
                commands.push(number(l));
            }
        }
        close(block);
        release
            .entry((f.book.clone(), f.release.clone()))
            .or_default()
            .add(top);
    }
    let mut family = [Gathering::default(), Gathering::default()];
    for ((_, at), g) in &release {
        family[usize::from(!classic(at))].join(g);
    }
    c.numbers = order;
    c.release_order = release
        .iter()
        .map(|((b, r), g)| (b.clone(), r.clone(), g.settled()))
        .collect();
    c.family_order = [family[0].settled(), family[1].settled()];
    c.interface_order = interface.settled();
    c.line_order = line.settled();
    let books: BTreeSet<&str> = src.logs.iter().map(|f| f.book.as_str()).collect();
    for b in books {
        if let Some(n) = nvram(src, b) {
            c.nvram.push((b.to_string(), n));
        }
    }
    c
}

/// The tables with the command forms each book's captures used at their release, typed on an
/// engine of that book and release that reads these tables.
pub fn with_forms(mut c: Corpus, src: &Source) -> Result<Corpus, String> {
    let mut forms = BTreeSet::new();
    let keys: BTreeSet<(String, String)> = src
        .running
        .iter()
        .map(|f| (f.book.clone(), f.release.clone()))
        .collect();
    for (book, release) in &keys {
        let Some(r) = crate::release::Release::parse(release) else {
            continue;
        };
        let Ok(grammar) = crate::grammars::grammar(book, r) else {
            continue;
        };
        let held = live::Held::from_corpus(&c, &grammar.platform, &grammar.release);
        let engine = crate::cli::Engine::with_held(grammar, held);
        let captures: Vec<(&str, crate::platform::Stack)> = src
            .running
            .iter()
            .filter(|f| &f.book == book && &f.release == release)
            .filter_map(|f| Some((f.text.as_str(), stack_of(src, f)?)))
            .collect();
        for (kind, form) in crate::verify::capture_forms(&engine, &captures) {
            forms.insert((book.clone(), release.clone(), kind, form));
        }
    }
    c.forms = forms.into_iter().collect();
    Ok(c)
}

/// A capture's hardware: what its configuration names, else what its session's `show inventory` or
/// `show version` names, else its `<host>-stack.txt`.
fn stack_of(src: &Source, f: &File) -> Option<crate::platform::Stack> {
    let host = f.name.strip_suffix("-running-config.txt")?;
    let beside = |suffix: &str| {
        let name = format!("{host}{suffix}");
        move |g: &&File| g.book == f.book && g.release == f.release && g.name == name
    };
    let named = crate::platform::infer_stack(&f.text)
        .or_else(|| {
            let log = src.logs.iter().find(beside("-show.log"))?;
            let (_, ex) = crate::capture::exchanges(&log.text)?;
            live::stack_named(&ex)
        })
        .or_else(|| {
            let s = src.stacks.iter().find(beside("-stack.txt"))?;
            Some(s.text.trim().to_string())
        })?;
    crate::platform::parse_stack(&named).ok()
}

/// The words of a text as the deny-list reads them: runs of letters, digits and `_ - . @ /`,
/// lowercase, a trailing `.` or `-` dropped.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || "_-.@/".contains(c)))
        .map(|w| w.trim_end_matches(['.', '-']).to_lowercase())
        .filter(|w| !w.is_empty())
}

/// Every word of Cisco's own text in the grammars and help tables: names, syntax lines, example
/// lines and their output, and `?` descriptions. A word there names nothing of a site.
fn public_words() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (book, release, _) in crate::grammars::GRAMMARS {
        let Some(r) = crate::release::Release::parse(release) else {
            continue;
        };
        let Ok(g) = crate::grammars::grammar(book, r) else {
            continue;
        };
        for c in &g.commands {
            out.extend(words(&c.name));
            for l in &c.lines {
                out.extend(words(&l.text));
            }
            for e in &c.examples {
                out.extend(words(&e.line));
                for o in &e.output {
                    out.extend(words(o));
                }
            }
            for h in c.help.values() {
                out.extend(words(h));
            }
        }
        for tr in &g.transitions {
            out.extend(words(&tr.line));
        }
    }
    // and the parts of each between `-`, `.`, `/`, `@` and `_`
    let parts: Vec<String> = out
        .iter()
        .flat_map(|w| {
            w.split(['-', '.', '/', '@', '_'])
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect();
    out.extend(parts);
    out
}

/// A mask, a wildcard or the any address: dotted octets that are all 0, 255 or a contiguous mask's.
fn mask_like(ip: &str) -> bool {
    ip.split('.').all(|o| {
        matches!(
            o,
            "0" | "128"
                | "192"
                | "224"
                | "240"
                | "248"
                | "252"
                | "254"
                | "255"
                | "1"
                | "3"
                | "7"
                | "15"
                | "31"
                | "63"
                | "127"
        )
    })
}

/// What a site's captures name it by: host names, domains, users, free text, the names it gave
/// configured objects, addresses, MAC addresses, serial and certificate numbers. Built in memory from
/// the captures and never written: the list itself names the site.
/// Whether a word from a site's names can name the site: a number, a range, an interface or a
/// model cannot; an address can, unless it is mask-shaped.
fn names_a_thing(w: &str) -> bool {
    if w.split('.').count() == 4 && w.split('.').all(|o| o.parse::<u8>().is_ok()) {
        return !mask_like(w);
    }
    let plain = regex::Regex::new(r"^[0-9.,:/-]+$").unwrap();
    // IOS interface types, whole and abbreviated, and Onyx's MLAG port-channel, with a slot, port
    // and range
    let port = regex::Regex::new(
        r"^(ethernet|eth|et|mpo|fastethernet|fa|gigabitethernet|gig|gi|twogigabitethernet|tw|fivegigabitethernet|fi|tengigabitethernet|te|twentyfivegige|twe|fortygigabitethernet|fo|hundredgige|hu|appgigabitethernet|ap|port-channel|po|vlan|vl|loopback|lo|tunnel|tu|null|nu|serial|se|bdi|nve|mgmt|[a-z])[0-9]+(/[0-9]+)*(-[0-9]+)?$",
    )
    .unwrap();
    let model = regex::Regex::new(
        r"^(c9[0-9]{3}|ws-c|ie-|c3[0-9]{3}|c2960|cat[0-9]|sfp|qsfp|glc-|nm-|hp-sn|sn2[0-9]{3}|ios-xe)",
    )
    .unwrap();
    !(plain.is_match(w) || port.is_match(w) || model.is_match(w))
}

pub struct Deny {
    tokens: BTreeSet<String>,
    // the words of six characters or more, looked for inside longer words
    long: std::collections::HashSet<String>,
    longest: usize,
}

impl Deny {
    pub fn of(src: &Source) -> Deny {
        let mut public = public_words();
        // the keywords IOS lists for `?` with its descriptions of them, and its `<cr>`: the rows
        // whose first column is a lowercase keyword-shaped word, which a site also uses in free text
        let keyword = regex::Regex::new(r"^  ([a-z][a-z0-9-]*:?)\s+(.*)$").unwrap();
        public.insert("cr".into());
        for f in &src.logs {
            for l in f.text.lines() {
                if let Some(c) = keyword.captures(l) {
                    public.extend(words(&c[1]));
                    public.extend(words(&c[2]));
                }
            }
        }
        let mut raw: BTreeSet<String> = BTreeSet::new();
        let ip = regex::Regex::new(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b").unwrap();
        let mac = regex::Regex::new(r"\b[0-9a-fA-F]{4}\.[0-9a-fA-F]{4}\.[0-9a-fA-F]{4}\b").unwrap();
        let serial = regex::Regex::new(
            r"(?:SN:|Processor board ID|[Ss]erial [Nn]umber\s*:|System serial number\s*:)\s*([A-Za-z0-9]{6,})",
        )
        .unwrap();
        let numbered =
            regex::Regex::new(r"\b(?:TP-self-signed|IOS-Self-Signed-Certificate)-\d+\b").unwrap();
        let named = regex::Regex::new(
            r"^\s*(?:hostname|username|ip domain[ -]name|ip domain list|vtp domain|snmp-server community|snmp-server location|snmp-server contact|description|name|alias \S+|banner \S+|ip access-list(?: standard| extended| resequence)?|ipv6 access-list|mac access-list extended|(?:class-map|policy-map)(?: type \S+)?(?: match-(?:any|all))?|route-map|key chain|crypto pki trustpoint|crypto pki certificate chain|vrf definition|ip vrf|object-group \S+|logging discriminator|time-range|ip prefix-list|aaa group server \S+|radius server|tacacs server|service-policy (?:input|output)|class|match access-group name)\s+(.+)$",
        )
        .unwrap();
        let hosts = src
            .running
            .iter()
            .chain(&src.logs)
            .chain(&src.stacks)
            .filter_map(|f| {
                ["-running-config.txt", "-show.log", "-stack.txt"]
                    .iter()
                    .find_map(|s| f.name.strip_suffix(s))
                    .map(|h| h.split("-session-").next().unwrap_or(h).to_string())
            });
        raw.extend(hosts);
        for f in src.running.iter().chain(&src.logs).chain(&src.stacks) {
            for line in f.text.lines() {
                raw.extend(
                    ip.find_iter(line)
                        .map(|m| m.as_str().to_string())
                        .filter(|a| !mask_like(a)),
                );
                raw.extend(mac.find_iter(line).map(|m| m.as_str().to_string()));
                raw.extend(serial.captures_iter(line).map(|c| c[1].to_string()));
                raw.extend(numbered.find_iter(line).map(|m| m.as_str().to_string()));
                if let Some(c) = named.captures(line) {
                    raw.extend(words(&c[1]).filter(|w| !w.chars().all(|c| c.is_ascii_digit())));
                }
            }
            // a banner's text, delimiter to delimiter (below)
            let mut closes: Option<String> = None;
            for line in f.text.lines() {
                match &closes {
                    Some(d) => {
                        raw.extend(words(line));
                        if line.contains(d.as_str()) {
                            closes = None;
                        }
                    }
                    None => {
                        if let Some(rest) = line.strip_prefix("banner ")
                            && let Some((_, text)) = rest.split_once(' ')
                        {
                            let text = text.trim_start();
                            let d = if text.starts_with("^C") {
                                "^C".to_string()
                            } else {
                                text.chars().take(1).collect()
                            };
                            raw.extend(words(&text[d.len().min(text.len())..]));
                            if !text[d.len().min(text.len())..].contains(d.as_str()) {
                                closes = Some(d);
                            }
                        }
                    }
                }
            }
        }
        // words a site's free text shares with ordinary use, and the lab's own test names and
        // addresses, which name nothing of a site
        const GENERIC: [&str; 45] = [
            "vswitch",
            "classic",
            "print",
            "computer",
            "closet",
            "comm",
            "pace",
            "vm",
            "rc",
            "oob",
            "fg",
            "fw",
            "h1",
            "h2",
            "cccc",
            "cccccc",
            "mdf",
            "idf",
            "lewis",
            "c2960g",
            "vswitch-test",
            "vswitch-std",
            "10.0.0.3",
            "10.0.0.4",
            "10.0.0.5",
            "10.0.0.7",
            "10.0.0.9",
            "10.1.0.0",
            "proxmox",
            "bond0",
            "bond1",
            "pve_migration",
            "pve_ceph_public",
            "inter-site",
            "top-of-rack",
            "inter-vrf",
            "inter-process",
            "inter-switch",
            "https_ss_cert_keypair",
            "rack",
            "east",
            "west",
            "inter",
            "sr",
            "interconnect",
        ];
        public.extend(GENERIC.iter().map(|w| w.to_string()));
        // the designs: their addresses and the names they give hosts, VLANs, ports and changes
        let field = regex::Regex::new(
            r#"\b(?:name|description|hostname|domain|title|connects)\s*=\s*"([^"]*)""#,
        )
        .unwrap();
        for text in &src.designs {
            for line in text.lines() {
                raw.extend(
                    ip.find_iter(line)
                        .map(|m| m.as_str().to_string())
                        .filter(|a| !mask_like(a)),
                );
                for c in field.captures_iter(line) {
                    raw.extend(words(&c[1]).filter(|w| !w.chars().all(|c| c.is_ascii_digit())));
                }
                if let Some(h) = line.strip_prefix("hostname ") {
                    raw.insert(h.trim().to_string());
                }
            }
        }
        // a number, a release, a range, an interface or a model names nothing of a site; an
        // address does
        let tokens: BTreeSet<String> = raw
            .into_iter()
            .flat_map(|t| {
                let whole = t.to_lowercase();
                let mut out: Vec<String> = words(&t).collect();
                out.push(whole);
                out
            })
            .filter(|w| w.len() >= 2 && !public.contains(w) && names_a_thing(w))
            .collect();
        Deny::from_tokens(tokens)
    }

    fn from_tokens(tokens: BTreeSet<String>) -> Deny {
        let long: std::collections::HashSet<String> =
            tokens.iter().filter(|t| t.len() >= 6).cloned().collect();
        let longest = long.iter().map(String::len).max().unwrap_or(0);
        Deny {
            tokens,
            long,
            longest,
        }
    }

    /// Whether `text` holds a word the site is named by: a word, a part of one between `-`, `.`,
    /// `/`, `@` or `_`, or a word of six characters or more inside another.
    pub fn hits(&self, text: &str) -> bool {
        words(text).any(|w| self.denied(&w))
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    fn denied(&self, w: &str) -> bool {
        self.tokens.contains(w)
            || w.split(['-', '.', '/', '@', '_'])
                .any(|part| part.len() >= 4 && self.tokens.contains(part))
            || self.inside(w)
    }

    fn inside(&self, w: &str) -> bool {
        let b = w.as_bytes();
        (0..b.len()).any(|i| {
            (6..=self.longest.min(b.len() - i))
                .any(|n| std::str::from_utf8(&b[i..i + n]).is_ok_and(|s| self.long.contains(s)))
        })
    }

    /// The denied word `text` holds, for `vswitch corpus --why`.
    pub fn hit(&self, text: &str) -> Option<String> {
        words(text).find(|w| self.denied(w))
    }

    /// As `hit`, and also a word holding a denied word of four or five characters, which `hits`
    /// does not look for inside a longer word: for a person to review (`vswitch corpus --loose`).
    pub fn hit_loose(&self, text: &str) -> Option<String> {
        self.hit(text).or_else(|| {
            words(text).find_map(|w| {
                let b = w.as_bytes();
                (0..b.len()).find_map(|i| {
                    (4..=5.min(b.len() - i)).find_map(|n| {
                        std::str::from_utf8(&b[i..i + n])
                            .ok()
                            .filter(|s| s.len() < w.len() && self.tokens.contains(*s))
                            .map(|s| format!("{w} ({s})"))
                    })
                })
            })
        })
    }
}

/// `text` with each value that names the site characterized (`live::fits`): the free text after
/// `description`, `name`, `remark`, `location` or `contact` as `«text»`, an address as `«ipv4»`, a
/// MAC address as `«mac»`, the number of a self-signed trustpoint or certificate as `«number»`, and
/// any other such word as `«word»`. Spacing is kept.
pub(crate) fn characterize(text: &str, deny: &Deny) -> String {
    const FREE: [&str; 5] = ["description", "name", "remark", "location", "contact"];
    let starts: Vec<(usize, &str)> = text
        .split_whitespace()
        .map(|w| (w.as_ptr() as usize - text.as_ptr() as usize, w))
        .collect();
    // free text and a switch's own name are the site's whatever their words
    for (i, (_, w)) in starts.iter().enumerate() {
        if FREE.contains(w)
            && let Some(&(at, _)) = starts.get(i + 1)
        {
            return format!("{}«text»", characterize_words(&text[..at], deny));
        }
    }
    if let [(_, "hostname"), (at, _)] = starts[..] {
        return format!("{}«word»", &text[..at]);
    }
    if !deny.hits(text) {
        return text.to_string();
    }
    characterize_words(text, deny)
}

fn characterize_words(text: &str, deny: &Deny) -> String {
    let ip = regex::Regex::new(r"^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}$").unwrap();
    let mac = regex::Regex::new(r"^[0-9a-fA-F]{4}\.[0-9a-fA-F]{4}\.[0-9a-fA-F]{4}$").unwrap();
    let numbered = regex::Regex::new(r"(TP-self-signed-|IOS-Self-Signed-Certificate-)\d+").unwrap();
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        if !deny.hits(word) {
            out.push_str(word);
        } else if ip.is_match(word) {
            out.push_str("«ipv4»");
        } else if mac.is_match(word) {
            out.push_str("«mac»");
        } else {
            let renumbered = numbered.replace_all(word, "${1}«number»").to_string();
            if renumbered != *word && !deny.hits(&renumbered.replace("«number»", "")) {
                out.push_str(&renumbered);
            } else {
                out.push_str("«word»");
            }
        }
        word.clear();
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// The corpus with nothing that names the site: each line, block, order key and command form
/// keeps its shape with its site values characterized, a `?` answer or description naming the site
/// goes (a configured object's name is the switch's, not the release's), and a capture is named by
/// its book, release and place.
pub fn filtered(mut c: Corpus, deny: &Deny) -> Corpus {
    let ch = |s: &str| characterize(s, deny);
    for (book, release, t) in &mut c.held {
        let lines: BTreeSet<(String, String)> =
            t.lines.iter().map(|(k, l)| (ch(k), ch(l))).collect();
        t.lines = lines.into_iter().collect();
        let mut indent: BTreeMap<(String, String), String> = BTreeMap::new();
        for (k, l, i) in &t.indent {
            indent.entry((ch(k), ch(l))).or_insert_with(|| i.clone());
        }
        t.indent = indent.into_iter().map(|((k, l), i)| (k, l, i)).collect();
        let opens: BTreeSet<String> = t.opens.iter().map(|o| ch(o)).collect();
        t.opens = opens.into_iter().collect();
        let mut after: BTreeMap<String, String> = BTreeMap::new();
        for (l, p) in &t.after {
            after.entry(ch(l)).or_insert_with(|| ch(p));
        }
        t.after = after.into_iter().collect();
        t.help
            .retain(|(_, k, a)| !deny.hits(k) && !a.iter().any(|r| deny.hits(r)));
        // a crawl's key and context hold no address however it was typed: one that does slipped
        // past `context::values` and goes, as a capture's site words do
        t.crawl.retain(|(_, c, k, a)| {
            !deny.hits(k)
                && !deny.hits(c)
                && !crate::context::holds_address(k)
                && !crate::context::holds_address(c)
                && !a.iter().any(|r| deny.hits(r))
        });
        let n = t.captures.len();
        t.captures = (1..=n)
            .map(|i| format!("{book} {release} capture {i}"))
            .collect();
    }
    c.word_indent
        .retain(|(_, b, l, _)| !deny.hits(b) && !deny.hits(l));
    c.described
        .retain(|(_, _, k, w, what)| !deny.hits(k) && !deny.hits(w) && !deny.hits(what));
    for cmd in c.numbers.iter_mut() {
        *cmd = ch(cmd);
    }
    let forms: BTreeSet<(String, String, String, Vec<String>)> = c
        .forms
        .iter()
        .map(|(b, r, k, f)| {
            (
                b.clone(),
                r.clone(),
                ch(k),
                f.iter().map(|w| ch(w)).collect(),
            )
        })
        .collect();
    c.forms = forms.into_iter().collect();
    c
}

/// Every string in the corpus that still holds a site word, and every crawl key or context that
/// holds an address (none, for a corpus `filtered` wrote).
pub fn leaks(c: &Corpus, deny: &Deny) -> Vec<String> {
    let json = serde_json::to_value(c).expect("the corpus serializes");
    let mut out: Vec<String> = c
        .held
        .iter()
        .flat_map(|(_, _, t)| &t.crawl)
        .flat_map(|(_, ctx, k, _)| [ctx, k])
        .filter(|s| crate::context::holds_address(s))
        .cloned()
        .collect();
    fn walk(v: &serde_json::Value, deny: &Deny, out: &mut Vec<String>) {
        match v {
            serde_json::Value::String(s) if deny.hits(s) => out.push(s.clone()),
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, deny, out)),
            serde_json::Value::Object(o) => o.values().for_each(|x| walk(x, deny, out)),
            _ => {}
        }
    }
    walk(&json, deny, &mut out);
    out
}

/// Each file under `root` that would be published and names the site, with the line and the word:
/// every file but `.git`, `target`, the Cisco references and the private captures, a `.gz` read
/// decompressed.
pub fn scan_tree(root: &Path, deny: &Deny, loose: bool) -> Vec<(String, usize, String)> {
    const SKIP: [&str; 6] = [
        ".git",
        "target",
        "reference",
        "live",
        "captures",
        "coverage.txt",
    ];
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let text = if name.ends_with(".gz") {
                let mut s = String::new();
                if flate2::read::GzDecoder::new(&bytes[..])
                    .read_to_string(&mut s)
                    .is_err()
                {
                    continue;
                }
                s
            } else {
                match String::from_utf8(bytes) {
                    Ok(s) => s,
                    Err(_) => continue,
                }
            };
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string()
                .replace('\\', "/");
            for (i, line) in text.lines().enumerate() {
                let hit = if loose {
                    deny.hit_loose(line)
                } else {
                    deny.hit(line)
                };
                if let Some(w) = hit {
                    out.push((rel.clone(), i + 1, w));
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interface_a_number_or_a_model_names_nothing_and_a_host_name_does() {
        for w in [
            "gi1/0/1",
            "te2/0/1-4",
            "vlan10",
            "port-channel11",
            "po11",
            "mpo2",
            "twe1/0/24",
            "e1",
        ] {
            assert!(!names_a_thing(w), "{w}");
        }
        for w in [
            "451",
            "1/0/1",
            "c9300x-24y",
            "ws-c2960x-48fpd-l",
            "255.255.255.0",
        ] {
            assert!(!names_a_thing(w), "{w}");
        }
        for w in [
            "plantsw01",
            "lobbyhmi11",
            "abcrest50",
            "officepc50",
            "192.0.2.10",
        ] {
            assert!(names_a_thing(w), "{w}");
        }
    }

    #[test]
    fn a_site_word_is_found_whole_in_a_part_or_inside_a_longer_word() {
        let deny = Deny::from_tokens(["plantx".into(), "abcd".into()].into());
        assert!(deny.hits("hostname plantx"));
        assert!(deny.hits("hostname core-abcd-1"));
        assert!(deny.hits("hostname plantxsw01"));
        assert!(deny.hits("description uplink to PLANTX"));
        assert!(
            !deny.hits("hostname abcdsw01"),
            "a short word counts only whole or as a part"
        );
        assert!(!deny.hits("hostname plant-x"));
    }

    /// A build that names a captures folder checks the embedded corpus against it: the corpus is what
    /// `vswitch corpus` writes from those captures, so a capture added or changed without writing it
    /// again fails here.
    #[cfg(captures)]
    #[test]
    fn the_embedded_corpus_is_the_captures_compiled() {
        let src = Source::embedded();
        let built = filtered(with_forms(build(&src), &src).unwrap(), &Deny::of(&src));
        assert!(
            built == *get(),
            "data/corpus.json.gz is stale: run vswitch corpus on the captures folder"
        );
    }

    /// The public corpus names no site: every characterized value is one of `live::PLACES`, and no
    /// address but a mask, no MAC address and no serial number is left.
    #[test]
    fn the_corpus_holds_only_characterized_values() {
        let json = String::from_utf8(serde_json::to_vec(get()).unwrap()).unwrap();
        let place = regex::Regex::new("«[^»]*»").unwrap();
        for m in place.find_iter(&json) {
            assert!(crate::live::PLACES.contains(&m.as_str()), "{}", m.as_str());
        }
        let ip = regex::Regex::new(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b").unwrap();
        let addresses: Vec<&str> = ip
            .find_iter(&json)
            .map(|m| m.as_str())
            .filter(|a| !mask_like(a))
            .collect();
        assert!(addresses.is_empty(), "{addresses:?}");
        let mac = regex::Regex::new(r"\b[0-9a-fA-F]{4}\.[0-9a-fA-F]{4}\.[0-9a-fA-F]{4}\b").unwrap();
        assert!(!mac.is_match(&json));
        // free text and a switch's name, in every held line and block
        let free = regex::Regex::new(
            r"^\s*(?:\S+ )*?(?:description|name|remark|location|contact) (.+)$|^\s*hostname (\S+)$",
        )
        .unwrap();
        let mut seen = 0;
        for (_, _, h) in &get().held {
            for (k, l) in &h.lines {
                for s in [k, l] {
                    if let Some(c) = free.captures(s) {
                        seen += 1;
                        let v = c.get(1).or(c.get(2)).unwrap().as_str();
                        assert!(v == "«text»" || v == "«word»", "{s}");
                    }
                }
            }
        }
        assert!(seen > 0 || get().held.is_empty());
    }

    #[test]
    fn free_text_and_a_hostname_are_characterized_whatever_their_words() {
        let none = Deny::from_tokens(BTreeSet::new());
        assert_eq!(
            characterize(" description to the core", &none),
            " description «text»"
        );
        assert_eq!(
            characterize("snmp-server location rack 1", &none),
            "snmp-server location «text»"
        );
        assert_eq!(characterize("hostname core1", &none), "hostname «word»");
        assert_eq!(
            characterize("interface Gi1/0/1", &none),
            "interface Gi1/0/1"
        );
        assert_eq!(characterize("hostname", &none), "hostname");
    }

    #[test]
    fn a_characterized_line_takes_any_value_of_its_kind_and_no_other() {
        use crate::live::fits;
        assert!(fits("logging host «ipv4»", "logging host 192.0.2.10"));
        assert!(!fits("logging host «ipv4»", "logging host 192.0.2.300"));
        assert!(!fits(
            "logging host «ipv4»",
            "logging host 192.0.2.10 vrf x"
        ));
        assert!(fits(
            "rsakeypair TP-self-signed-«number»",
            "rsakeypair TP-self-signed-12345"
        ));
        assert!(!fits(
            "rsakeypair TP-self-signed-«number»",
            "rsakeypair TP-self-signed-12a45"
        ));
        assert!(fits(
            "description «text»",
            "description to the core, port 4"
        ));
        assert!(!fits("description «text»", "description"));
        assert!(fits(
            "username «word» privilege 15",
            "username jdoe privilege 15"
        ));
        assert!(fits("«mac»", "0011.2233.44ff") && !fits("«mac»", "0011.2233.44fg"));
        assert!(fits("no shutdown", "no shutdown") && !fits("no shutdown", "shutdown"));
    }

    /// A build that names a captures folder scans every file this repository publishes, `.gz`
    /// files decompressed, for anything that names the site.
    #[cfg(captures)]
    #[test]
    fn no_published_file_names_the_site() {
        let deny = Deny::of(&Source::embedded());
        let hits = scan_tree(Path::new(env!("CARGO_MANIFEST_DIR")), &deny, false);
        let shown: Vec<String> = hits
            .iter()
            .take(20)
            .map(|(f, l, _)| format!("{f}:{l}"))
            .collect();
        assert!(
            hits.is_empty(),
            "{} lines name the site: {shown:?}",
            hits.len()
        );
    }

    const CRAWL: &str = "Switch#configure terminal\nEnter configuration commands, one per line.  End with CNTL/Z.\nSwitch(config)# ?\nConfigure commands:\n  auto  Configure Automation\n  auto  Configure Automation\nSwitch(config)#do-exec show ?\n  vlan  VTP VLAN status\nSwitch(config)#bogus ?\n% Unrecognized command\nSwitch(config)#router ospf 1\nSwitch(config-router)# ?\n  area  OSPF area parameters\nSwitch(config-router)#end\n";

    #[test]
    fn a_crawl_log_under_crawl_reaches_the_corpus_and_no_other_file_of_its_folder_does() {
        let dir = std::env::temp_dir().join(format!("vswitch-crawl-test-{}", std::process::id()));
        let folder = dir.join("c9300-17.6");
        std::fs::create_dir_all(folder.join("crawl")).unwrap();
        let put = |name: &str, text: &str| std::fs::write(folder.join(name), text).unwrap();
        put("crawl/crawl.log", CRAWL);
        put(
            "crawl/notes.log",
            "Switch#show version\nnot a session of the crawl\n",
        );
        put("crawl/state.json", "{}");
        put(
            "lab-show.log",
            "lab#show vtp ?\n  status  VTP status\nlab#\n",
        );
        let src = Source::read(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(src.crawls.len(), 1);
        assert_eq!(
            (&src.crawls[0].book[..], &src.crawls[0].release[..]),
            ("c9300", "17.6")
        );
        assert_eq!(src.crawls[0].text, CRAWL);
        let logs: Vec<&str> = src.logs.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(logs, ["lab-show.log"]);
        let built = build(&src);
        let (_, _, table) = built
            .held
            .iter()
            .find(|(b, r, _)| b == "c9300" && r == "17.6")
            .unwrap();
        let held: Vec<(&str, &str, &str, usize)> = table
            .crawl
            .iter()
            .map(|(m, c, k, a)| (m.as_str(), c.as_str(), k.as_str(), a.len()))
            .collect();
        // the repeated row stays; `do-exec show ?` is privileged EXEC's `show ?`; a refusal is no answer
        assert_eq!(
            held,
            [
                ("config", "", "", 3),
                ("config-router", "router ospf", "", 1),
                ("privileged", "", "show ", 1),
            ]
        );
        assert_eq!(table.help.len(), 1);
    }

    /// A crawl's log is read by `vswitch corpus` from its folder and never embedded: the build's
    /// `LIVE_LOGS` holds the top-level session logs of each folder only.
    #[cfg(captures)]
    #[test]
    fn a_crawl_log_is_not_embedded_in_the_binary() {
        for f in Source::read_crawls(Path::new(env!("VSWITCH_CAPTURES_DIR"))) {
            assert!(
                crate::grammars::LIVE_LOGS.iter().all(|l| l.3 != f.text),
                "{} {}'s crawl log is embedded",
                f.book,
                f.release
            );
        }
        assert!(
            crate::grammars::LIVE_LOGS
                .iter()
                .all(|l| l.2 != "crawl.log" && !l.2.contains(['/', '\\']))
        );
    }

    #[test]
    fn a_crawl_answer_naming_the_site_is_left_out() {
        let deny = Deny::from_tokens(["plantx".into()].into());
        let mut c = Corpus::default();
        c.held.push((
            "c9300".into(),
            "17.6".into(),
            HeldTable {
                crawl: vec![
                    (
                        "config".into(),
                        "".into(),
                        "ip ".into(),
                        vec!["  routing  ok".into()],
                    ),
                    (
                        "config".into(),
                        "".into(),
                        "snmp ".into(),
                        vec!["  plantx  site".into()],
                    ),
                ],
                ..Default::default()
            },
        ));
        let kept = filtered(c, &deny);
        let keys: Vec<&str> = kept.held[0].2.crawl.iter().map(|e| e.2.as_str()).collect();
        assert_eq!(keys, ["ip "]);
        assert!(leaks(&kept, &deny).is_empty());
    }

    #[test]
    fn only_a_crawl_s_complete_exchanges_are_held() {
        let log = [
            "Switch#configure terminal",
            "Enter configuration commands, one per line.  End with CNTL/Z.",
            "Switch(config)#ip ?",
            "  routing  Enable IP routing",
            // the switch printed nothing, and the next prompt shows the probe was answered
            "Switch(config)#banner motd x ?",
            "Switch(config)#vtp ?",
            "  mode  Set the mode",
            " --More-- \u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}\u{8}",
            "Switch(config)#snmp-server ?",
            "  host  Specify hosts\u{8}\u{8}\u{8} \u{8}",
            "Switch(config)#\u{8}\u{8}lldp ?",
            "  run  Enable LLDP",
            "Switch(config)#cdp ?",
            "  advertise-v2  CDP sends version-2 advertisements",
            "Switch(config)#bogus ?",
            "% Unrecognized command",
            "Switch(config)#spanning-tree ?",
            "  mode  Spanning tree operating mode",
            // the log was read while this answer was being written
            "  portfast  Spanning tree portfast",
        ]
        .join("\n");
        let held: Vec<(String, Vec<String>)> = crawled_help(&log)
            .into_iter()
            .map(|(_, _, k, a)| (k, a))
            .collect();
        assert_eq!(
            held,
            [
                (
                    "ip ".to_string(),
                    vec!["  routing  Enable IP routing".to_string()]
                ),
                ("banner motd x ".to_string(), vec![]),
                (
                    "cdp ".to_string(),
                    vec!["  advertise-v2  CDP sends version-2 advertisements".to_string()]
                ),
            ]
        );
    }

    #[test]
    fn a_crawl_s_abbreviated_do_exec_line_is_privileged_exec_help() {
        let log = "Switch#configure terminal\nSwitch(config)#do-ex show ?\n  vlan  VTP VLAN status\nSwitch(config)#do-e show vtp ?\n  status  VTP status\nSwitch(config)#do ?\n  LINE  Exec Command\nSwitch(config)#end\n";
        let held: Vec<(String, String, String)> = crawled_help(log)
            .into_iter()
            .map(|(m, c, k, _)| (m, c, k))
            .collect();
        let row = |m: &str, k: &str| (m.to_string(), String::new(), k.to_string());
        assert_eq!(
            held,
            [
                row("privileged", "show "),
                row("privileged", "show vtp "),
                row("config", "do "),
            ]
        );
    }

    #[test]
    fn a_crawl_key_or_context_holding_an_address_is_dropped_and_counted_a_leak() {
        let deny = Deny::from_tokens(BTreeSet::new());
        let entry = |c: &str, k: &str| ("config".to_string(), c.to_string(), k.to_string(), vec![]);
        let table = HeldTable {
            crawl: vec![
                entry("", "ip "),
                entry("", "ipv6 route 2001:db8::/32 "),
                entry("", "copy tftp://10.1.1.1/x "),
                entry("ip sla «number» > icmp-echo 10.1.1.1", ""),
                entry("", "arp 0011.2233.4455 "),
                entry("", &crate::context::key("ipv6 route 2001:db8::/32 ")),
            ],
            ..Default::default()
        };
        let c = Corpus {
            held: vec![("c9300".into(), "17.6".into(), table)],
            ..Default::default()
        };
        assert_eq!(leaks(&c, &deny).len(), 4);
        let kept = filtered(c, &deny);
        let keys: Vec<&str> = kept.held[0].2.crawl.iter().map(|e| e.2.as_str()).collect();
        assert_eq!(keys, ["ip ", "ipv6 route «ipv6»/«number» "]);
        assert!(leaks(&kept, &deny).is_empty());
    }

    #[test]
    fn a_corpus_without_a_crawl_has_no_crawl_field_and_one_with_it_round_trips() {
        let plain = Corpus {
            held: vec![("c9300".into(), "17.6".into(), HeldTable::default())],
            ..Default::default()
        };
        let json = String::from_utf8(serde_json::to_vec(&plain).unwrap()).unwrap();
        assert!(!json.contains("crawl"), "{json}");
        assert_eq!(Corpus::from_gzip(&plain.to_gzip()).unwrap(), plain);
        let mut with = plain.clone();
        with.held[0].2.crawl.push((
            "config".into(),
            "router bgp".into(),
            "".into(),
            vec!["  neighbor  x".into()],
        ));
        assert_eq!(Corpus::from_gzip(&with.to_gzip()).unwrap(), with);
    }

    #[test]
    fn the_corpus_round_trips_through_its_file_form() {
        let c = get();
        assert!(!c.held.is_empty() && !c.numbers.is_empty());
        assert_eq!(&Corpus::from_gzip(&c.to_gzip()).unwrap(), c);
    }
}
