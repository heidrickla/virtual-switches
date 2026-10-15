//! The compiled grammars, one per platform book and IOS XE release, embedded gzip-compressed
//! (`data/grammars`, listed by build.rs). A switch runs its family's book at its release.

use crate::compile::Grammar;
use crate::platform::Family;
use crate::release::Release;
use std::collections::BTreeMap;
use std::io::Read;

include!(concat!(env!("OUT_DIR"), "/grammars.rs"));

/// Each book: its key, the platform name its grammar carries (tools/extract-releases.js writes it from
/// its PLATFORMS, which a test holds to this list) and its name in messages.
pub const BOOKS: &[(&str, &str, &str)] = &[
    ("c9300", "Catalyst 9300", "Catalyst 9300"),
    ("c9200", "Catalyst 9200", "Catalyst 9200"),
    ("c9400", "Catalyst 9400", "Catalyst 9400"),
    ("c2960x", "Catalyst 2960x", "Catalyst 2960-X"),
    ("c2960", "Catalyst 2960", "Catalyst 2960"),
    ("ie3300", "Catalyst IE3300", "Catalyst IE3300"),
    ("ie3000", "Cisco IE 3000", "Cisco IE 3000"),
    ("ie3010", "Cisco IE 3010", "Cisco IE 3010"),
];

/// A book Cisco publishes no command reference for, and the book whose grammar at the same release
/// it projects (`compile::project`): the IE3x00 runs the Catalyst 9300's IOS XE.
pub const PROJECTIONS: &[(&str, &str)] = &[("ie3300", "c9300")];

/// A book whose switches' general IOS commands (`hostname`, `line`) another book of the same IOS
/// documents: the 2960 books cover the switch's own commands, the 2960-X books both.
pub const DONORS: &[(&str, &str)] = &[
    ("c2960", "c2960x"),
    ("ie3000", "c2960x"),
    ("ie3010", "c2960x"),
];

/// The book a family's switches run: the 9200's for a Catalyst 9200, 9200L or 9200 B model, the 9400's for a 9400,
/// the 2960's for a 2960, 2960-Plus or 2960-C, else the 9300's.
pub fn book(family: Option<Family>) -> &'static str {
    match family {
        Some(Family::C9200 | Family::C9200L | Family::C9200B) => "c9200",
        Some(Family::C9400) => "c9400",
        Some(Family::C2960X) => "c2960x",
        Some(Family::C2960 | Family::C2960Plus | Family::C2960C) => "c2960",
        Some(Family::IE3300) => "ie3300",
        Some(Family::IE3000) => "ie3000",
        Some(Family::IE3010) => "ie3010",
        _ => "c9300",
    }
}

/// The book a grammar's platform name belongs to (`Catalyst 9400` is c9400).
pub fn book_of(platform: &str) -> Option<&'static str> {
    BOOKS.iter().find(|b| b.1 == platform).map(|b| b.0)
}

/// The release a book's switch runs when nothing names one: 17.15 for IOS XE where the book documents
/// it, else the book's newest release (the IE3300's 17.6, a classic book's newest train).
pub fn default_release(book: &str) -> Release {
    let documented = documented(book);
    if documented.contains(&crate::release::DEFAULT) {
        return crate::release::DEFAULT;
    }
    documented
        .last()
        .copied()
        .unwrap_or(crate::release::DEFAULT)
}

/// The release a stack runs when nothing names one: its book's default, else the newest release the
/// book documents that the hardware runs (a WS-C2960-24TC-L stops at 15.0(2)SE, before the 2960
/// book's 15.2(4)E); the one leaving fewest problems when none is clear.
pub fn fitting_release(book: &str, stack: &crate::platform::Stack) -> Release {
    let documented = documented(book);
    std::iter::once(default_release(book))
        .chain(documented.iter().rev().copied())
        .filter(|r| documented.contains(r))
        .min_by_key(|r| stack.problems(*r).len())
        .unwrap_or_else(|| default_release(book))
}

/// The book's human name in messages.
pub fn book_name(book: &str) -> &'static str {
    BOOKS
        .iter()
        .find(|b| b.0 == book)
        .map_or("Catalyst 9300", |b| b.2)
}

/// The releases a book has a grammar for, oldest first.
pub fn documented(book: &str) -> Vec<Release> {
    let mut v: Vec<Release> = GRAMMARS
        .iter()
        .filter(|g| g.0 == book)
        .filter_map(|g| Release::parse(g.1))
        .collect();
    v.sort();
    v
}

/// A release named in text (`17.9`, `17.9.3`, `26.1`) as the book documents it.
pub fn resolve(text: &str, book: &str) -> Result<Release, String> {
    Release::resolve(text, &documented(book)).ok_or_else(|| not_documented(text, book))
}

/// The refusal of a release the book has no grammar for, naming the release's own OS: a classic train
/// is IOS, `17.15` IOS XE; a train-less `15.2` is IOS where the book is classic.
fn not_documented(text: &str, book: &str) -> String {
    let list: Vec<String> = documented(book).iter().map(Release::to_string).collect();
    let classic = match Release::parse(text) {
        Some(r) if r.classic() => true,
        Some(r) if r.major() >= 16 => false,
        _ => documented(book).iter().any(|r| r.classic()),
    };
    let os = if classic { "IOS" } else { "IOS XE" };
    format!(
        "{os} {text} has no {} command reference; documented: {}",
        book_name(book),
        list.join(", ")
    )
}

fn gunzip(bytes: &[u8], what: &str) -> Result<String, String> {
    let mut out = String::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_string(&mut out)
        .map_err(|e| format!("{what}: {e}"))?;
    Ok(out)
}

/// The first `n` bytes of the book's grammar at a release, decompressed: the compiler writes the
/// release and the platform first.
#[cfg(test)]
pub fn head(book: &str, release: Release, n: u64) -> Result<String, String> {
    let (_, _, bytes) = GRAMMARS
        .iter()
        .find(|g| g.0 == book && Release::parse(g.1) == Some(release))
        .ok_or_else(|| not_documented(&release.to_string(), book))?;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(*bytes)
        .take(n)
        .read_to_end(&mut out)
        .map_err(|e| format!("grammar {book} {release}: {e}"))?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// The book's grammar at a release, as JSON.
pub fn text(book: &str, release: Release) -> Result<String, String> {
    let read = release.book_for(&documented(book)).unwrap_or(release);
    let (_, _, bytes) = GRAMMARS
        .iter()
        .find(|g| g.0 == book && Release::parse(g.1) == Some(read))
        .ok_or_else(|| not_documented(&release.to_string(), book))?;
    gunzip(bytes, &format!("grammar {book} {release}"))
}

/// The book's grammar at a release, each command's help filled from the book's help table. A train read
/// from an earlier book keeps its own release, which the captures held at it are filed under.
pub fn grammar(book: &str, release: Release) -> Result<Grammar, String> {
    let mut g: Grammar =
        serde_json::from_str(&text(book, release)?).map_err(|e| format!("grammar: {e}"))?;
    g.release = release.to_string();
    privilege_as_measured(&mut g);
    if let Some((_, bytes)) = HELP.iter().find(|h| h.0 == book) {
        let table: BTreeMap<String, BTreeMap<String, String>> =
            serde_json::from_str(&gunzip(bytes, &format!("help {book}"))?)
                .map_err(|e| format!("help {book}: {e}"))?;
        for c in &mut g.commands {
            if c.help.is_empty()
                && let Some(h) = table.get(&c.name)
            {
                c.help = h.clone();
            }
        }
    }
    Ok(g)
}

/// `privilege <mode> level <level> <command>` as the lab C9300-24T (17.6.4) took it, where the books'
/// lines read one word for each: the command is every word after the level (`show version`), and
/// outside the `no` form the level is 0 to 15 (`level 99` refused).
fn privilege_as_measured(g: &mut Grammar) {
    use crate::compile::ArgKind;
    use crate::syntax::Node;
    fn rename(n: &mut Node) {
        match n {
            Node::Arg(a) if a == "level" => *a = "privilege-level".into(),
            Node::Seq(v) | Node::Alt(v) => v.iter_mut().for_each(rename),
            Node::Opt(o) => rename(o),
            _ => {}
        }
    }
    for c in g.commands.iter_mut().filter(|c| c.name == "privilege") {
        if let Some(k) = c.args.get_mut("command") {
            *k = ArgKind::Text;
        }
        if !c.args.contains_key("level") {
            continue;
        }
        c.args.insert(
            "privilege-level".into(),
            ArgKind::Number { min: 0, max: 15 },
        );
        for l in c.lines.iter_mut().filter(|l| !l.negated) {
            rename(&mut l.node);
            l.text = l.text.replace("<level>", "<privilege-level>");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extractor_names_each_book_s_platform_as_its_grammar_carries_it() {
        // a grammar finds its book by the platform name the extractor wrote into its reference
        let js = include_str!("../tools/extract-releases.js");
        let table = js
            .split("const PLATFORMS = {")
            .nth(1)
            .and_then(|t| t.split("};").next())
            .expect("PLATFORMS in tools/extract-releases.js");
        let named: BTreeMap<&str, &str> = table
            .lines()
            .filter_map(|l| {
                let (book, name) = l.trim().trim_end_matches(',').split_once(": ")?;
                Some((book, name.trim_matches('\'')))
            })
            .collect();
        let books: BTreeMap<&str, &str> = BOOKS.iter().map(|b| (b.0, b.1)).collect();
        assert_eq!(named, books);
    }

    #[test]
    fn a_release_no_book_documents_is_named_with_its_os() {
        assert!(not_documented("15.2(2)E", "none").starts_with("IOS 15.2(2)E has no"));
        assert!(not_documented("17.3", "none").starts_with("IOS XE 17.3 has no"));
        // an IOS XE release asked of a classic book is IOS XE; a train-less `15.2` is the book's IOS
        assert!(
            not_documented("17.15", "c2960x").starts_with("IOS XE 17.15 has no Catalyst 2960-X")
        );
        assert!(not_documented("15.1", "c2960x").starts_with("IOS 15.1 has no"));
    }

    #[test]
    fn every_book_s_default_release_is_one_it_documents() {
        for (book, _, _) in BOOKS {
            let r = default_release(book);
            assert!(documented(book).contains(&r), "{book} {r}");
        }
        // the IE3300 book holds 17.6 alone; an IOS XE book with 17.15 runs it
        assert_eq!(default_release("ie3300").to_string(), "17.6");
        assert_eq!(default_release("c9200"), crate::release::DEFAULT);
    }
}
