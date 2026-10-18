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

/// Whether `level` takes `line` in `mode` though its crawl offers no such line (`TAKEN`).
pub fn taken(level: &str, mode: &str, line: &str) -> bool {
    let words: Vec<String> = line.split_whitespace().map(str::to_lowercase).collect();
    TAKEN.iter().any(|(l, m, start)| {
        *l == level && *m == mode && {
            let want: Vec<&str> = start.split_whitespace().collect();
            words.len() >= want.len() && words.iter().zip(&want).all(|(w, s)| w == s)
        }
    })
}

/// Whether `level` takes `word` first on a line in `mode` though its `?` lists it nowhere
/// (`UNLISTED`).
pub fn unlisted(level: &str, mode: &str, word: &str) -> bool {
    UNLISTED
        .iter()
        .any(|(l, m, w)| *l == level && *m == mode && w.eq_ignore_ascii_case(word))
}

/// First words a gated level refuses in a mode where its crawl's `?` answers judge nothing: the lab
/// C9300-24T (17.6.4) at network-essentials refused `xconnect` on a dot1Q sub-interface at boot,
/// under the word; its crawled sub-interface's `?` listed it at neither level.
pub const LEFT_OUT: &[(&str, &str, &str)] = &[("network-essentials", "config-subif", "xconnect")];

/// Whether `level` refuses `word` first on a line in `mode` (`LEFT_OUT`).
pub fn left_out(level: &str, mode: &str, word: &str) -> bool {
    LEFT_OUT
        .iter()
        .any(|(l, m, w)| *l == level && *m == mode && w.eq_ignore_ascii_case(word))
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
            assert!(!unlisted(level, "config", "net"), "{level}");
        }
        assert_eq!(
            refused_types("network-essentials"),
            ["pseudowire", "Auto-Template"]
        );
        assert!(unlisted("network-essentials", "config", "NET"));
        assert!(!unlisted("network-essentials", "config-router", "net"));
        assert!(!unlisted("network-essentials", "config", "network"));
        assert!(left_out("network-essentials", "config-subif", "XCONNECT"));
        assert!(!left_out("network-essentials", "config-if", "xconnect"));
        assert!(!left_out("network-advantage", "config-subif", "xconnect"));
        assert!(taken("network-essentials", "config", "l2  Traceroute"));
        assert!(!taken(
            "network-essentials",
            "config",
            "l2 vfi customerA manual"
        ));
        assert!(!taken("network-advantage", "config", "l2 traceroute"));
        assert!(!taken("network-essentials", "config-if", "l2 traceroute"));
    }
}
