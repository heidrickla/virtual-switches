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
/// listed here.
pub const ENABLED: &[(&str, &str, &str)] = &[];

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
        assert!(ENABLED.is_empty(), "every mapping ships off");
        assert_eq!(
            enabled_donor("c2960x", "15.2(7)E", "lanbase", ENABLED),
            None
        );
        let on = [("c2960x", "15.2(7)E", "lanbase")];
        assert_eq!(
            enabled_donor("c2960x", "15.2(7)E", "lanbase", &on),
            Some(("c2960", "15.0(2)SE", "lanbase"))
        );
        assert_eq!(enabled_donor("c2960x", "15.2(6)E", "lanbase", &on), None);
    }
}
