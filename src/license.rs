//! A switch's license level, which keys the crawls of its image, and the crawl of another image that
//! answers `?` where the image's own answers and readings hold none (the donor tier, `live::Held`).

/// The level a running configuration's `license boot level <level> ...` line names, lowercase.
pub fn stated<'a>(lines: impl IntoIterator<Item = &'a str>) -> Option<String> {
    lines.into_iter().find_map(|l| {
        let rest = l.trim().strip_prefix("license boot level ")?;
        rest.split_whitespace().next().map(str::to_lowercase)
    })
}

/// The level a book's switch runs where its configuration states none: the IE3300's `show version`
/// names network-essentials, and the classic books' captures run LAN Base. A Catalyst 9000 runs
/// network-advantage: the lab C9300-24T (IOS XE 17.6.4) came from the factory with no
/// startup-config running network-advantage with dna-advantage (console-bridge record, 2026-10-14).
pub fn default_for(book: &str) -> Option<&'static str> {
    match book {
        "ie3300" => Some("network-essentials"),
        "c2960" | "c2960x" | "ie3000" | "ie3010" => Some("lanbase"),
        "c9200" | "c9300" | "c9400" => Some("network-advantage"),
        _ => None,
    }
}

/// A switch's level: as its configuration states it, else its book's default.
#[cfg(test)]
pub fn of<'a>(book: &str, lines: impl IntoIterator<Item = &'a str>) -> Option<String> {
    stated(lines).or_else(|| default_for(book).map(str::to_string))
}

const IOS_XE: &[&str] = &["c9200", "c9300", "c9400", "ie3300"];
const CLASSIC: &[&str] = &["c2960", "c2960x", "ie3000", "ie3010"];

/// An image's crawl by its book, release and level.
pub type Image = (&'static str, &'static str, &'static str);

/// For a switch of one of the books at the level, at any release, the crawl that may answer for it:
/// IOS XE takes the C9300 17.6 crawl at its own level, classic IOS the C2960G 15.0(2)SE crawl.
pub const DONORS: &[(&[&str], &str, Image)] = &[
    (
        IOS_XE,
        "network-advantage",
        ("c9300", "17.6", "network-advantage"),
    ),
    (
        IOS_XE,
        "network-essentials",
        ("c9300", "17.6", "network-essentials"),
    ),
    (CLASSIC, "lanbase", ("c2960", "15.0(2)SE", "lanbase")),
];

/// The (book, release, level) of each switch whose donor answers; a mapping answers only once it is
/// listed here. Each is an image whose captures hold `?` answers the donor beat the books on with
/// those answers masked (`shell::tests::donor_answers_against_the_captures_answers`, 2026-10-15),
/// keyword recall 73-98% against the books' 32-51%. A Catalyst 9000 takes both levels: the
/// advantage donor scored 95-99% recall against essentials captures.
pub const ENABLED: &[(&str, &str, &str)] = &[
    ("c2960x", "15.2(7)E", "lanbase"),
    ("c9200", "17.6", "network-essentials"),
    ("c9200", "17.6", "network-advantage"),
    ("c9200", "17.15", "network-essentials"),
    ("c9200", "17.15", "network-advantage"),
    ("c9300", "17.15", "network-essentials"),
    ("c9300", "17.15", "network-advantage"),
    ("c9400", "17.5", "network-advantage"),
];

/// Each level an image runs with less of its software than another, with that other level: the lab
/// C9300-24T (17.6.4) at network-essentials refused, under a caret, the words its network-advantage
/// `?` answers offered and its network-essentials answers did not, and took no word network-advantage
/// refused.
pub const GATED: &[(&str, &str)] = &[("network-essentials", "network-advantage")];

/// The level whose crawl `level`'s is gated against (`GATED`).
pub fn gated_by(level: &str) -> Option<&'static str> {
    GATED.iter().find(|(l, _)| *l == level).map(|g| g.1)
}

/// Interface types a gated level's `?` lists that its parser refuses: the lab C9300-24T (17.6.4) at
/// network-essentials listed `pseudowire` and `Auto-Template` among the types and answered every
/// line naming one with a number `% Unrecognized command`, where network-advantage answered it;
/// `interface pseudowire 100` was refused under the type's second letter.
pub const REFUSED_TYPES: &[(&str, &[&str])] =
    &[("network-essentials", &["pseudowire", "Auto-Template"])];

/// The interface types `level` lists and refuses (`REFUSED_TYPES`).
pub fn refused_types(level: &str) -> &'static [&'static str] {
    REFUSED_TYPES
        .iter()
        .find(|(l, _)| *l == level)
        .map_or(&[], |r| r.1)
}

/// First words a gated level takes in a mode though its `?` lists none, refusing the word after
/// them: the lab C9300-24T (17.6.4) at network-essentials took `net` at global configuration and
/// refused the NET after it under its first character, as typed and under a refused `router isis`.
pub const UNLISTED: &[(&str, &str, &str)] = &[("network-essentials", "config", "net")];

/// Line starts a gated level takes in a mode though its crawl's `?` answers offer it no such line:
/// the lab C9300-24T (17.6.4) at network-essentials took `l2 traceroute`, its `?` listing no `l2`
/// (only `l2protocol-tunnel`), and refused `l2 vfi`.
pub const TAKEN: &[(&str, &str, &str)] = &[("network-essentials", "config", "l2 traceroute")];

/// Whether `level` takes `line` in `mode` though its crawl offers no such line (`TAKEN`): each word
/// of its start as measured or a unique start of it among the keywords offered before it
/// (`keywords`, by the text before the word; `context::stands_for`).
pub fn taken(level: &str, mode: &str, line: &str, keywords: impl Fn(&str) -> Vec<String>) -> bool {
    TAKEN
        .iter()
        .any(|(l, m, start)| *l == level && *m == mode && begins(line, start, &keywords))
}

/// `TAKEN` entries measured on one image (book, release) at a level its own crawl keys: the lab
/// C9200L-48T-4G (17.6.4) took `l2 traceroute` at network-advantage too, its `?` listing no `l2`.
pub const TAKEN_ON: &[((&str, &str), &str, &str, &str)] = &[(
    crate::context::C9200_17_6,
    "network-advantage",
    "config",
    "l2 traceroute",
)];

/// `taken`, or a `TAKEN_ON` entry for `image` (book, release).
pub fn taken_on(
    image: (&str, &str),
    level: &str,
    mode: &str,
    line: &str,
    keywords: impl Fn(&str) -> Vec<String>,
) -> bool {
    taken(level, mode, line, &keywords)
        || TAKEN_ON.iter().any(|(i, l, m, start)| {
            *i == image && *l == level && *m == mode && begins(line, start, &keywords)
        })
}

/// Whether `line` begins with `start`, each word as measured or a unique start of it.
fn begins(line: &str, start: &str, keywords: impl Fn(&str) -> Vec<String>) -> bool {
    let toks = crate::cli::tokens(line);
    let want: Vec<&str> = start.split_whitespace().collect();
    toks.len() >= want.len()
        && toks.iter().zip(&want).all(|(t, w)| {
            t.text.eq_ignore_ascii_case(w)
                || crate::context::stands_for(&t.text, w, &keywords(&line[..t.at])) == Some(false)
        })
}

/// How `word`, first on a line in `mode`, stands for one `level` takes though its `?` lists it
/// nowhere (`UNLISTED`), `offered` the keywords offered there (`context::stands_for`).
pub fn unlisted(
    level: &str,
    mode: &str,
    word: &str,
    offered: impl Fn() -> Vec<String>,
) -> Option<bool> {
    first_word(UNLISTED, level, mode, word, offered)
}

/// How `word` stands for a measured first word of `table` at `level` in `mode`.
fn first_word(
    table: &[(&str, &str, &str)],
    level: &str,
    mode: &str,
    word: &str,
    offered: impl Fn() -> Vec<String>,
) -> Option<bool> {
    let mut entries = table
        .iter()
        .filter(|(l, m, _)| *l == level && *m == mode)
        .peekable();
    entries.peek()?;
    let offered = offered();
    entries.find_map(|(_, _, w)| crate::context::stands_for(word, w, &offered))
}

/// First words a gated level refuses in a mode where its crawl's `?` answers judge nothing: the lab
/// C9300-24T (17.6.4) at network-essentials refused `xconnect` on a dot1Q sub-interface at boot,
/// under the word; its crawled sub-interface's `?` listed it at neither level.
pub const LEFT_OUT: &[(&str, &str, &str)] = &[("network-essentials", "config-subif", "xconnect")];

/// How `word`, first on a line in `mode`, stands for one `level` refuses there (`LEFT_OUT`),
/// `offered` the keywords offered there (`context::stands_for`).
pub fn left_out(
    level: &str,
    mode: &str,
    word: &str,
    offered: impl Fn() -> Vec<String>,
) -> Option<bool> {
    first_word(LEFT_OUT, level, mode, word, offered)
}

/// The donor `DONORS` names for a switch, none where that is the switch's own crawl.
pub fn donor(book: &str, release: &str, level: &str) -> Option<Image> {
    DONORS
        .iter()
        .find(|(books, l, _)| books.contains(&book) && *l == level)
        .map(|d| d.2)
        .filter(|d| (d.0, d.1, d.2) != (book, release, level))
}

/// `donor`, where `enabled` lists the switch.
pub fn enabled_donor(
    book: &str,
    release: &str,
    level: &str,
    enabled: &[(&str, &str, &str)],
) -> Option<Image> {
    enabled
        .contains(&(book, release, level))
        .then(|| donor(book, release, level))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_is_the_configuration_s_else_the_book_s_default() {
        let config = [
            "hostname x",
            "license boot level network-advantage addon dna-advantage",
        ];
        assert_eq!(stated(config).as_deref(), Some("network-advantage"));
        assert_eq!(
            stated(["license boot level network-essentials"]).as_deref(),
            Some("network-essentials")
        );
        // other license lines name no level
        assert_eq!(stated(["license smart transport off"]), None);
        assert_eq!(
            of("ie3300", ["license boot level network-advantage"]).as_deref(),
            Some("network-advantage")
        );
        assert_eq!(of("ie3300", []).as_deref(), Some("network-essentials"));
        for book in ["c2960", "c2960x", "ie3000", "ie3010"] {
            assert_eq!(
                of(book, ["hostname x"]).as_deref(),
                Some("lanbase"),
                "{book}"
            );
        }
        for book in ["c9200", "c9300", "c9400"] {
            assert_eq!(
                of(book, ["hostname x"]).as_deref(),
                Some("network-advantage"),
                "{book}"
            );
            assert_eq!(
                of(book, ["license boot level network-essentials"]).as_deref(),
                Some("network-essentials"),
                "{book}"
            );
        }
    }

    #[test]
    fn a_donor_answers_only_where_enabled_and_never_for_its_own_image() {
        assert_eq!(
            donor("c9200", "17.15", "network-essentials"),
            Some(("c9300", "17.6", "network-essentials"))
        );
        assert_eq!(
            donor("ie3300", "17.6", "network-advantage"),
            Some(("c9300", "17.6", "network-advantage"))
        );
        assert_eq!(
            donor("c2960x", "15.2(7)E", "lanbase"),
            Some(("c2960", "15.0(2)SE", "lanbase"))
        );
        assert_eq!(donor("c2960x", "15.2(7)E", "network-advantage"), None);
        assert_eq!(donor("c9300", "17.6", "network-advantage"), None);
        assert_eq!(donor("c2960", "15.0(2)SE", "lanbase"), None);
        assert_eq!(enabled_donor("c2960x", "15.2(7)E", "lanbase", &[]), None);
        // every enabled switch has a donor that is not its own image
        for &(b, r, l) in ENABLED {
            assert!(enabled_donor(b, r, l, ENABLED).is_some(), "{b} {r} {l}");
        }
        assert_eq!(
            enabled_donor("c9300", "17.15", "network-essentials", ENABLED),
            Some(("c9300", "17.6", "network-essentials"))
        );
        assert_eq!(
            enabled_donor("c2960x", "15.2(2)E", "lanbase", ENABLED),
            None
        );
        let on = [("c2960x", "15.2(7)E", "lanbase")];
        assert_eq!(
            enabled_donor("c2960x", "15.2(7)E", "lanbase", &on),
            Some(("c2960", "15.0(2)SE", "lanbase"))
        );
        assert_eq!(enabled_donor("c2960x", "15.2(6)E", "lanbase", &on), None);
    }

    #[test]
    fn network_essentials_is_gated_against_network_advantage_alone() {
        assert_eq!(gated_by("network-essentials"), Some("network-advantage"));
        for level in ["network-advantage", "lanbase", "ipservices", ""] {
            assert_eq!(gated_by(level), None, "{level}");
            assert!(refused_types(level).is_empty(), "{level}");
            assert_eq!(unlisted(level, "config", "net", Vec::new), None, "{level}");
        }
        assert_eq!(
            refused_types("network-essentials"),
            ["pseudowire", "Auto-Template"]
        );
        let none = Vec::new;
        let ess = "network-essentials";
        assert_eq!(unlisted(ess, "config", "NET", none), Some(false));
        assert_eq!(unlisted(ess, "config-router", "net", none), None);
        assert_eq!(unlisted(ess, "config", "network", none), None);
        assert_eq!(left_out(ess, "config-subif", "XCONNECT", none), Some(false));
        assert_eq!(left_out(ess, "config-if", "xconnect", none), None);
        assert_eq!(
            left_out("network-advantage", "config-subif", "xconnect", none),
            None
        );
        assert!(taken(ess, "config", "l2  Traceroute", |_| vec![]));
        assert!(!taken(ess, "config", "l2 vfi customerA manual", |_| vec![]));
        assert!(!taken(
            "network-advantage",
            "config",
            "l2 traceroute",
            |_| vec![]
        ));
        assert!(!taken(ess, "config-if", "l2 traceroute", |_| vec![]));
    }

    /// An image's own measured start holds on that image and level alone; the level-wide ones hold
    /// on every image.
    #[test]
    fn an_image_s_taken_start_holds_on_that_image_and_level_alone() {
        let adv = "network-advantage";
        let none = |_: &str| vec![];
        let c9200 = ("c9200", "17.6");
        assert!(taken_on(c9200, adv, "config", "l2 traceroute", none));
        assert!(taken_on(c9200, adv, "config", "L2 TRACEROUTE", none));
        assert!(!taken_on(c9200, adv, "config", "l2 vfi v1 manual", none));
        assert!(!taken_on(c9200, adv, "config-if", "l2 traceroute", none));
        assert!(!taken_on(
            ("c9200", "17.15"),
            adv,
            "config",
            "l2 traceroute",
            none
        ));
        assert!(!taken_on(
            ("c9300", "17.6"),
            adv,
            "config",
            "l2 traceroute",
            none
        ));
        assert!(!taken_on(c9200, "lanbase", "config", "l2 traceroute", none));
        assert!(taken_on(
            ("c9300", "17.6"),
            "network-essentials",
            "config",
            "l2 traceroute",
            none
        ));
        // a unique start stands for the measured word among the keywords offered before it
        let offered = |text: &str| -> Vec<String> {
            if text.is_empty() {
                vec!["l2protocol-tunnel".into(), "lacp".into()]
            } else {
                vec!["traceroute".into(), "vfi".into()]
            }
        };
        assert!(taken_on(c9200, adv, "config", "l2 tracer", offered));
    }

    /// A measured first word or line start stands for its unique abbreviations among the keywords
    /// offered where it is typed; one that begins another keyword too is ambiguous.
    #[test]
    fn a_measured_word_stands_for_its_unique_abbreviations() {
        let ess = "network-essentials";
        let offered = || vec!["exit".to_string(), "encapsulation".to_string()];
        assert_eq!(left_out(ess, "config-subif", "xconn", offered), Some(false));
        assert_eq!(left_out(ess, "config-subif", "e", offered), None);
        let config = || vec!["netconf-yang".to_string(), "ntp".to_string()];
        assert_eq!(unlisted(ess, "config", "ne", config), Some(true));
        assert_eq!(unlisted(ess, "config", "nt", config), None);
        let after = |text: &str| match text {
            "" => vec!["l2protocol-tunnel".to_string()],
            _ => vec!["vfi".to_string(), "traceroute".to_string()],
        };
        assert!(taken(ess, "config", "l2 trace", after));
        assert!(!taken(ess, "config", "l2 v", after));
    }
}
