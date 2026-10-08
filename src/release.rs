//! IOS releases as Cisco documents them: an IOS XE command reference per train (`16.12`, `17.15`), one
//! `26.x` book for every 26 release, and classic IOS trains (`15.2(7)E`) for the Catalyst 2960-X.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Release {
    major: u32,
    /// None: the `<major>.x` book
    minor: Option<u32>,
    /// a classic IOS train: `15.2(7)E` is (7, "E"), its rebuilds (`E8`, `E0a`) ignored
    train: Option<(u32, [u8; 3])>,
}

/// The release a switch runs when nothing names one: Cisco's recommended 9200 and 9300 train.
pub const DEFAULT: Release = Release {
    major: 17,
    minor: Some(15),
    train: None,
};

impl Release {
    /// `17.15`, `17.9.3`, `16.11.1c`, `26.x`, `15.2(7)E8`: the train, the rest ignored; a rebuild
    /// lettered inside the train (`15.2(2a)E1`) is its train. A bootloader version (`15.2(6r)E`) is no
    /// release.
    pub fn parse(text: &str) -> Option<Release> {
        let text = text.trim();
        if let Some((head, rest)) = text.split_once('(') {
            let (major, minor) = head.split_once('.')?;
            let (number, after) = rest.split_once(')')?;
            if number.ends_with('r') {
                return None;
            }
            let number = number.trim_end_matches(|c: char| c.is_ascii_lowercase());
            let letters: String = after.chars().take_while(char::is_ascii_uppercase).collect();
            if letters.is_empty() || letters.len() > 3 {
                return None;
            }
            let mut code = [0u8; 3];
            code[..letters.len()].copy_from_slice(letters.as_bytes());
            return Some(Release {
                major: major.parse().ok()?,
                minor: Some(minor.parse().ok()?),
                train: Some((number.parse().ok()?, code)),
            });
        }
        let mut parts = text.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = match parts.next()? {
            "x" => None,
            m => Some(
                m.trim_end_matches(|c: char| c.is_ascii_alphabetic())
                    .parse()
                    .ok()?,
            ),
        };
        Some(Release {
            major,
            minor,
            train: None,
        })
    }

    pub fn major(self) -> u32 {
        self.major
    }

    /// Whether this is a classic IOS train (`15.2(7)E`) rather than IOS XE.
    pub fn classic(self) -> bool {
        self.train.is_some()
    }

    /// What a running configuration's `version` line names: `17.15`, or a classic train's `15.2`.
    pub fn config_version(self) -> String {
        match (self.classic(), self.minor) {
            (true, Some(m)) => format!("{}.{m}", self.major),
            _ => self.to_string(),
        }
    }

    /// The release with its operating system, for messages: `IOS XE 17.15`, `IOS 15.2(7)E`.
    pub fn named(self) -> String {
        format!("{} {self}", if self.classic() { "IOS" } else { "IOS XE" })
    }

    /// The documented release a running version falls in: its own train, else the `<major>.x` book, else,
    /// for a classic configuration's `version 15.2` line, which names no train, the newest documented
    /// `15.2(...)` train. A classic train past the last book of its line is its own release, read from
    /// that book (`book_for`).
    pub fn resolve(version: &str, documented: &[Release]) -> Option<Release> {
        let r = Release::parse(version)?;
        if r.book_for(documented).is_some() {
            return Some(r);
        }
        documented
            .iter()
            .find(|d| **d == r)
            .or_else(|| {
                documented
                    .iter()
                    .find(|d| d.major == r.major && d.minor.is_none())
            })
            .or_else(|| {
                documented
                    .iter()
                    .filter(|d| {
                        r.train.is_none()
                            && d.train.is_some()
                            && (d.major, d.minor) == (r.major, r.minor)
                    })
                    .max()
            })
            .copied()
    }

    /// The documented release whose book this one reads: itself, or for a classic train past the last
    /// book of its line, that book (the 2960-Plus books stop at 15.2(4)E, "and later", while switches
    /// run 15.2(6)E1). A train between two books has none.
    pub fn book_for(self, documented: &[Release]) -> Option<Release> {
        if documented.contains(&self) {
            return Some(self);
        }
        let (n, code) = self.train?;
        let last = documented
            .iter()
            .filter(|d| {
                (d.major, d.minor) == (self.major, self.minor)
                    && d.train.is_some_and(|t| t.1 == code)
            })
            .max()?;
        last.train.is_some_and(|t| t.0 < n).then_some(*last)
    }

    /// Whether a release named for a switch agrees with the one its file's version line resolves to:
    /// the same, or any train of a classic `version 15.2` line, which names none.
    pub fn agrees(self, own: Release, version: &str) -> bool {
        self == own
            || (Release::parse(version).is_some_and(|v| v.train.is_none())
                && self.classic()
                && own.classic()
                && self.config_version() == own.config_version())
    }

    /// Whether hardware Cisco introduced in `since` runs on this release.
    pub fn runs(self, since: &str) -> bool {
        Release::parse(since).is_none_or(|s| self.includes(s))
    }

    /// Whether `other` is this release or an earlier one; the `<major>.x` book holds every `<major>`
    /// release.
    pub fn includes(self, other: Release) -> bool {
        match (other.minor, self.minor) {
            (_, None) => other.major <= self.major,
            (Some(o), Some(m)) => {
                (other.major, o, other.train.map(|t| t.0))
                    <= (self.major, m, self.train.map(|t| t.0))
            }
            (None, Some(_)) => other.major < self.major,
        }
    }
}

impl fmt::Display for Release {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.minor, self.train) {
            (Some(m), Some((n, code))) => {
                let letters: String = code
                    .iter()
                    .take_while(|&&b| b != 0)
                    .map(|&b| b as char)
                    .collect();
                write!(f, "{}.{m}({n}){letters}", self.major)
            }
            (Some(m), None) => write!(f, "{}.{m}", self.major),
            (None, _) => write!(f, "{}.x", self.major),
        }
    }
}

/// The `version 17.9` line a running configuration opens with, at the top level: an indented
/// `version 2` is a submode's (`router rip`).
pub fn version_line(config: &str) -> Option<&str> {
    config
        .lines()
        .find_map(|l| l.strip_prefix("version ")?.split_whitespace().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(t: &str) -> Release {
        Release::parse(t).unwrap()
    }

    #[test]
    fn trains_parse_order_and_resolve() {
        assert_eq!(r("17.9.3"), r("17.9"));
        assert_eq!(r("16.11.1c").to_string(), "16.11");
        assert_eq!(r("26.x").to_string(), "26.x");
        assert!(r("16.12") < r("17.1") && r("17.9") < r("17.10") && r("17.18") < r("26.x"));
        let documented = [r("16.12"), r("17.9"), r("26.x")];
        assert_eq!(Release::resolve("17.9.4", &documented), Some(r("17.9")));
        assert_eq!(Release::resolve("26.1", &documented), Some(r("26.x")));
        assert_eq!(Release::resolve("17.8", &documented), None);
        assert!(r("17.5").runs("17.5.1") && !r("17.4").runs("17.5.1"));
        assert!(r("26.x").runs("17.18.1") && r("17.15").runs("16.5.1a"));
        assert_eq!(version_line("!\nversion 17.9\nhostname a\n"), Some("17.9"));
        assert_eq!(version_line("hostname a\n"), None);
        // RIP's ` version 2` is a submode's line, not the configuration's release
        let rip = "hostname lab\nrouter rip\n version 2\n network 10.0.0.0\nend\n";
        assert_eq!(version_line(rip), None);
        assert_eq!(
            version_line(&format!("{rip}version 17.12\n")),
            Some("17.12")
        );
    }

    #[test]
    fn classic_trains_parse_order_and_resolve() {
        // a rebuild belongs to its train; a bootloader version is no release
        assert_eq!(r("15.2(7)E8"), r("15.2(7)E"));
        assert_eq!(r("15.2(7)E0a").to_string(), "15.2(7)E");
        assert_eq!(r("12.2(55)SE11").to_string(), "12.2(55)SE");
        assert!(Release::parse("15.2(6r)E").is_none() && Release::parse("15.2(7)").is_none());
        // a rebuild lettered inside its train (`15.2(2a)E1`) is the train
        assert_eq!(r("15.2(2a)E1"), r("15.2(2)E"));
        assert_eq!(r("15.0(2a)EX5").to_string(), "15.0(2)EX");
        assert!(
            r("15.2(2)E") < r("15.2(4)E")
                && r("15.2(6)E") < r("15.2(7)E")
                && r("15.2(7)E") < r("16.12")
        );
        assert!(r("15.2(7)E").classic() && !r("17.15").classic());
        let documented = [r("15.2(2)E"), r("15.2(4)E"), r("15.2(6)E"), r("15.2(7)E")];
        assert_eq!(
            Release::resolve("15.2(6)E1", &documented),
            Some(r("15.2(6)E"))
        );
        assert_eq!(Release::resolve("15.2(5)E2", &documented), None);
        // a classic configuration's version line names no train: the newest one of its release
        assert_eq!(Release::resolve("15.2", &documented), Some(r("15.2(7)E")));
        assert_eq!(Release::resolve("15.0", &documented), None);
        assert!(r("15.2(7)E").runs("15.2(4)E") && !r("15.2(2)E").runs("15.2(4)E"));
        // past the last book of its line: its own release, read from that book
        let plus = [
            r("12.2(44)SE"),
            r("12.2(55)SE"),
            r("15.2(2)E"),
            r("15.2(4)E"),
        ];
        assert_eq!(Release::resolve("15.2(6)E1", &plus), Some(r("15.2(6)E")));
        assert_eq!(r("15.2(6)E").book_for(&plus), Some(r("15.2(4)E")));
        assert_eq!(r("15.2(2)E").book_for(&plus), Some(r("15.2(2)E")));
        assert_eq!(r("15.2(3)E").book_for(&plus), None);
        assert_eq!(r("12.2(58)SE").book_for(&plus), Some(r("12.2(55)SE")));
        assert_eq!(r("15.2(6)E").book_for(&documented), Some(r("15.2(6)E")));
        // a `version 12.2` line names no train: every 12.2 train agrees with it
        assert!(r("12.2(44)SE").agrees(r("12.2(55)SE"), "12.2"));
        assert!(!r("12.2(44)SE").agrees(r("12.2(55)SE"), "12.2(55)SE"));
        assert!(!r("15.2(4)E").agrees(r("12.2(55)SE"), "12.2"));
        assert!(!r("17.9").agrees(r("17.12"), "17.12"));
    }
}
