//! How much of the real switches' configurations a platform's books take on their own: each capture
//! typed at its platform and release with the lines real switches held set aside. A refused line is
//! a root when every line it sits under was taken; the lines under a refused block opener, at any
//! depth, follow from it. `vswitch coverage` prints the roots; a test holds them against
//! `data/coverage.txt`, so a grammar change that refuses more, or less, says so.

use crate::cli::Engine;
use crate::live::{Capture, Held};
use crate::release::Release;
use std::collections::BTreeMap;

/// A configuration line the books alone refuse, under a line they take.
#[derive(Clone, Debug, PartialEq)]
pub struct Root {
    pub book: &'static str,
    pub release: &'static str,
    pub host: &'static str,
    /// the line's number in the capture, from 1
    pub number: usize,
    pub text: String,
    pub error: String,
}

impl Root {
    /// The baseline's line: what is refused where, without the line number a re-capture moves.
    pub fn key(&self) -> String {
        format!(
            "{} {} {}: {}",
            self.book, self.release, self.host, self.text
        )
    }
}

/// A platform book's engine at a release with the lines real switches held set aside.
pub fn books_alone(book: &str, release: &str) -> Result<Engine, String> {
    let r = Release::parse(release).ok_or_else(|| format!("release {release}"))?;
    let mut e = Engine::new(crate::grammars::grammar(book, r)?);
    e.held = Held::default();
    Ok(e)
}

/// The roots of one capture against its books alone.
pub fn roots(engine: &Engine, c: &Capture) -> Result<Vec<Root>, String> {
    let stack = crate::platform::parse_stack(c.model)?;
    let (_, typed) = crate::check::type_config(engine, stack, c.running);
    let raw: Vec<&str> = c.running.lines().collect();
    let indent = |n: usize| {
        raw.get(n.saturating_sub(1))
            .map_or(0, |l| l.len() - l.trim_start().len())
    };
    // the lines above each one, by indentation: a refused one takes its block with it
    let mut above: Vec<(usize, bool)> = Vec::new();
    let mut out = Vec::new();
    for t in &typed {
        let depth = indent(t.number);
        while above.last().is_some_and(|(d, _)| *d >= depth) {
            above.pop();
        }
        let under_taken = above.last().is_none_or(|(_, ok)| *ok);
        if let Some(e) = &t.error
            && under_taken
        {
            out.push(Root {
                book: c.book,
                release: c.release,
                host: c.host,
                number: t.number,
                text: t.line.trim().to_string(),
                error: e.lines().last().unwrap_or("").trim().to_string(),
            });
        }
        // a line under a refused one follows from it however deep, taken or not
        above.push((depth, t.error.is_none() && under_taken));
    }
    Ok(out)
}

/// Every capture's roots, captures in the order `live::captures` gives: an engine per platform book
/// and release, and the captures typed side by side.
pub fn all_roots(captures: &[Capture]) -> Result<Vec<Root>, String> {
    let keys: Vec<(&'static str, &'static str)> = captures
        .iter()
        .map(|c| (c.book, c.release))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let built = crate::live::par_map(&keys, |&(book, release)| books_alone(book, release));
    let mut engines = BTreeMap::new();
    for (k, e) in keys.into_iter().zip(built) {
        engines.insert(k, e?);
    }
    let mut out = Vec::new();
    for r in crate::live::par_map(captures, |c| roots(&engines[&(c.book, c.release)], c)) {
        out.extend(r?);
    }
    Ok(out)
}

/// A digest of every capture the roots come from, so a capture added or changed asks for a new
/// baseline rather than reading as a grammar change (FNV-1a, 64 bits).
pub fn corpus_digest(captures: &[Capture]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for c in captures {
        for part in [c.book, c.release, c.host, c.model, c.running] {
            for b in part.bytes().chain([0u8]) {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    format!("{h:016x}")
}

/// The baseline file: a header naming the corpus, then each root's key, sorted.
pub fn baseline(captures: &[Capture], roots: &[Root]) -> String {
    let mut keys: Vec<String> = roots.iter().map(Root::key).collect();
    keys.sort();
    let mut out = String::from(
        "# Lines real switches held that their books alone refuse, under a line the books take.\n\
         # `vswitch coverage --baseline > data/coverage.txt` writes it again after a grammar change.\n",
    );
    out.push_str(&format!("corpus {}\n", corpus_digest(captures)));
    for k in keys {
        out.push_str(&k);
        out.push('\n');
    }
    out
}

/// The count of roots by platform book and release, and the commands refused most, for the report.
pub fn report(captures: &[Capture], roots: &[Root]) -> Vec<String> {
    let mut lines: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for c in captures {
        *lines.entry((c.book, c.release)).or_default() +=
            crate::live::config_lines(c.running).count();
    }
    let mut by: BTreeMap<(&str, &str), Vec<&Root>> = BTreeMap::new();
    for r in roots {
        by.entry((r.book, r.release)).or_default().push(r);
    }
    let mut out = vec![format!(
        "{:<8} {:<11} {:>6} {:>6}  most refused",
        "book", "release", "lines", "roots"
    )];
    for ((book, release), n) in &lines {
        let rs = by.get(&(*book, *release)).cloned().unwrap_or_default();
        let mut stems: BTreeMap<String, usize> = BTreeMap::new();
        for r in &rs {
            let stem: Vec<&str> = r.text.split_whitespace().take(2).collect();
            *stems.entry(stem.join(" ")).or_default() += 1;
        }
        let mut top: Vec<(String, usize)> = stems.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let names: Vec<String> = top
            .iter()
            .take(4)
            .map(|(s, n)| format!("{s} ({n})"))
            .collect();
        out.push(format!(
            "{book:<8} {release:<11} {n:>6} {:>6}  {}",
            rs.len(),
            names.join(", ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(
        book: &'static str,
        release: &'static str,
        model: &'static str,
        running: &'static str,
    ) -> Capture {
        Capture {
            book,
            release,
            model,
            host: "sw",
            running,
            log: "",
        }
    }

    #[test]
    fn a_refused_block_opener_takes_its_lines_and_a_taken_one_does_not() {
        let c = capture(
            "c9300",
            "17.15",
            "C9300-24T",
            "version 17.15\nhostname sw\nnosuchblock one\n nosuchline two\ninterface GigabitEthernet1/0/1\n nosuchline three\n  deeper four\n description ok\nend\n",
        );
        let e = books_alone(c.book, c.release).unwrap();
        let r = roots(&e, &c).unwrap();
        let texts: Vec<&str> = r.iter().map(|x| x.text.as_str()).collect();
        // the line under the refused opener follows from it; under the taken interface the
        // refused line is a root and the line nested under it is not
        assert_eq!(texts, ["nosuchblock one", "nosuchline three"]);
        assert_eq!(r[1].number, 6);
        // a refused line under a taken one under a refused opener follows from the opener too
        let c = capture(
            "c9300",
            "17.15",
            "C9300-24T",
            "version 17.15\nnosuchblock one\n hostname sw2\n  nosuchline two\nend\n",
        );
        let r = roots(&e, &c).unwrap();
        let texts: Vec<&str> = r.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, ["nosuchblock one"]);
    }

    #[test]
    fn the_corpus_digest_changes_with_any_capture() {
        let a = capture(
            "c9300",
            "17.15",
            "C9300-24T",
            "version 17.15\nhostname a\nend\n",
        );
        let b = capture(
            "c9300",
            "17.15",
            "C9300-24T",
            "version 17.15\nhostname b\nend\n",
        );
        assert_ne!(corpus_digest(&[a]), corpus_digest(&[b]));
        assert_eq!(corpus_digest(&[a]), corpus_digest(&[a]));
    }
}
