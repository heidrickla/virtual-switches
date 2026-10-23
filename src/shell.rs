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
    /// the line as typed that entered this mode, which its help context is derived from
    entered: String,
}

impl Frame {
    fn top(mode: &str) -> Self {
        Frame {
            mode: mode.into(),
            blocks: vec![],
            prefix: String::new(),
            section: None,
            entered: String::new(),
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
    /// a public key's text typed after `key-string` in `ip ssh pubkey-chain`, until `exit`
    pubkey: Option<String>,
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
    /// the lab the switch is cabled into, as it stood once every cable came up (`lab::connect`):
    /// spanning tree reads it with this session's own configuration in its place
    pub lab: Option<crate::spanning::LabView>,
    /// the switch's clock, as seconds since 1970 UTC, which a VLAN database change stamps; the
    /// computer's clock when `None`
    pub clock: Option<u64>,
    /// when the switch booted, as seconds since 1970 UTC: the session's start on the computer's
    /// clock; `show version` counts its uptime from it
    pub booted: u64,
    /// the VLAN database as it stood when VTP last counted a change
    vlans_seen: Option<Vec<(String, Vec<String>)>>,
    /// the lines entered that are known to crash a switch of this family (`enter`)
    pub warned: usize,
    /// the licence level the switch booted at, as a replayed configuration's `license boot level`
    /// line states it wherever it stands in the file (`check::type_traced`)
    pub boot_level: Option<String>,
}

/// A time as VTP's last-change line prints it: month and day unpadded, the year in two digits
/// (`10-16-26 18:30:17`, the lab C9300, 17.6.4; `3-10-93 03:49:32`, the lab C2960G).
pub fn vtp_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let (year, month, day) = civil(days);
    format!(
        "{month}-{day}-{:02} {:02}:{:02}:{:02}",
        year.rem_euclid(100),
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Days since 1970 as a civil year, month (1-12) and day (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// A time as the running configuration's last-change line prints it: `12:34:56 UTC Thu Mar 5
/// 2026` (the lab C9300, 17.6.4, at two-digit days; a day under 10 unpadded, inferred).
pub fn config_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let (year, month, day) = civil(days);
    format!(
        "{:02}:{:02}:{:02} UTC {} {} {day} {year}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60,
        DAYS[days.rem_euclid(7) as usize],
        MONTHS[(month - 1) as usize]
    )
}

/// The time a last-change line names (`12:34:56 UTC Thu Mar 5 2026`, a classic switch's `... by
/// vswitch` after it), as seconds since 1970 UTC.
pub fn parse_stamp(text: &str) -> Option<u64> {
    let w: Vec<&str> = text.split_whitespace().collect();
    let (time, month, day, year) = (w.first()?, w.get(3)?, w.get(4)?, w.get(5)?);
    let hms: Vec<u64> = time
        .split(':')
        .map(|x| x.parse().ok())
        .collect::<Option<_>>()?;
    let [h, m, s] = hms[..] else {
        return None;
    };
    let month = MONTHS.iter().position(|x| x == month)? as i64 + 1;
    let (day, year): (i64, i64) = (day.parse().ok()?, year.parse().ok()?);
    // days from civil (Howard Hinnant's algorithm)
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days)
        .ok()
        .map(|d| d * 86_400 + h * 3600 + m * 60 + s)
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
            pubkey: None,
            last_top: None,
            raw: String::new(),
            lead: None,
            redacted: false,
            saved: None,
            replayed: false,
            lab: None,
            clock: None,
            booted: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            vlans_seen: None,
            warned: 0,
            boot_level: None,
        }
    }

    /// Boots the switch as it comes from the factory with no startup configuration, where vswitch
    /// models that default (`startup::factory`): its lines taken as the switch's own, the startup
    /// configuration still absent, the session at `>`.
    pub fn factory_default(&mut self) {
        self.factory_default_at(crate::startup::FACTORY_LEVEL);
    }

    /// `factory_default` at a licence level (`network-essentials addon dna-essentials`): the switch
    /// boots now on its clock, which is the configuration's last change.
    pub fn factory_default_at(&mut self, level: &str) {
        let at = self.now();
        let Some(lines) = crate::startup::factory(&self.device, level, at) else {
            return;
        };
        crate::check::type_into(self, &lines.join("\n"));
        self.frames = vec![Frame::top("exec")];
        self.replayed = false;
        self.accepted.clear();
        self.device.factory = true;
        self.device.last_change = Some(at);
        self.device.last_change_by = None;
        self.booted = at;
        self.vlans_seen = Some(self.device.vlan_contents());
    }

    /// The switch's clock, as seconds since 1970 UTC.
    pub fn now(&self) -> u64 {
        self.clock.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        })
    }

    /// What VTP makes of the line just typed: a new domain, transparent or off mode, or version 3
    /// starts the revision at 0 again (VLAN configuration guide; version 3 measured on the lab
    /// C9300, 17.6.4); a VLAN database change a version 1 or 2 server takes at the console counts
    /// one revision and stamps the updater and the clock once the VLAN submode is left (the lab
    /// C9300, 17.6.4: `vlan 140`, `exit`, revision 1). A replayed configuration is the boot state
    /// and counts nothing.
    fn vtp_after(&mut self, before: (String, String, String)) {
        let (version, mode, domain) = self.device.vtp_settings();
        if domain != before.2
            || (mode != before.1 && matches!(mode.as_str(), "transparent" | "off"))
            || (version != before.0 && version == "3")
        {
            self.device.vtp.revision = 0;
        }
        // a VLAN submode's changes count when it is left (inferred: one change per `vlan` entry,
        // a range included)
        if self.mode() == "config-vlan" {
            return;
        }
        let contents = self.device.vlan_contents();
        let changed = self.vlans_seen.as_ref().is_some_and(|s| *s != contents);
        // a version 3 server is no primary server, which vswitch does not model: its revision stays
        // (inferred)
        if changed && !self.redacted && mode == "server" && version != "3" {
            self.device.vtp.revision += 1;
            self.device.vtp.modified_by =
                show::vtp_updater(&self.device).map_or_else(|| "0.0.0.0".to_string(), |u| u.1);
            self.device.vtp.modified_at = vtp_stamp(self.now());
        }
        self.vlans_seen = Some(contents);
    }

    /// The forced speeds of the ports a `no speed` is about to clear, by full name.
    fn forced_speeds(&self, p: &Parse) -> Vec<(String, u32)> {
        if !p.negated || crate::device::key(p) != "speed" {
            return vec![];
        }
        let Some(f) = self.frames.last() else {
            return vec![];
        };
        f.blocks
            .iter()
            .filter_map(|&b| {
                let name = self.device.blocks[b].header.strip_prefix("interface ")?;
                let mbps =
                    self.device.config_of(name).iter().find_map(|l| {
                        l.strip_prefix("speed ").and_then(|v| v.parse::<u32>().ok())
                    })?;
                Some((name.to_string(), mbps))
            })
            .collect()
    }

    /// The lab C9300, 17.6.4 kept costing a port at the speed `no speed` cleared, with its Type back
    /// to `P2p`, 30 seconds on. That the cost holds until the port is given a speed again, and that
    /// other platforms and releases do the same, is inferred.
    fn hold_speeds(&mut self, p: &Parse, cleared: Vec<(String, u32)>) {
        self.device.held_speed.extend(cleared);
        if !p.negated && crate::device::key(p) == "speed" {
            let Some(f) = self.frames.last() else {
                return;
            };
            for &b in &f.blocks {
                if let Some(name) = self.device.blocks[b].header.strip_prefix("interface ") {
                    self.device.held_speed.remove(name);
                }
            }
        }
    }

    /// The commands `privilege exec level <n> <command>` raises above level 1, where `>` runs, as
    /// their words (c9200 17.15 capture 27 raises 14, `show` and `terminal` among them), each start of
    /// a raised command with them unless a line of its own sets it.
    fn raised(&self) -> Vec<Vec<String>> {
        let levels: BTreeMap<Vec<String>, u32> = self
            .device
            .global
            .iter()
            .filter_map(|e| {
                let rest = e.text.strip_prefix("privilege exec ")?;
                let rest = rest.strip_prefix("all ").unwrap_or(rest);
                let (n, cmd) = rest.strip_prefix("level ")?.split_once(' ')?;
                let words: Vec<String> = cmd.split_whitespace().map(str::to_lowercase).collect();
                (!words.is_empty()).then_some((words, n.parse::<u32>().ok()?))
            })
            .collect();
        let mut out: BTreeSet<Vec<String>> = BTreeSet::new();
        for (words, _) in levels.iter().filter(|(_, n)| **n > 1) {
            for k in 1..=words.len() {
                let start = words[..k].to_vec();
                if k == words.len() || !levels.contains_key(&start) {
                    out.insert(start);
                }
            }
        }
        out.into_iter().collect()
    }

    /// Whether a line's words start a command raised above `>`'s level, at `>`.
    fn above_exec(&self, words: &[String]) -> bool {
        self.mode() == "exec"
            && self
                .raised()
                .iter()
                .any(|r| r.len() <= words.len() && r.iter().zip(words).all(|(a, b)| a == b))
    }

    /// Whether words as typed, each whole or a start of its word, begin a raised command at `mode`.
    fn raised_typed(raised: &[Vec<String>], mode: &str, words: &[String]) -> bool {
        mode == "exec"
            && raised.iter().any(|r| {
                r.len() <= words.len()
                    && r.iter().zip(words).all(|(a, b)| a.starts_with(b.as_str()))
            })
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

    /// `vlan <n>` past 4094 in global configuration, refused by the VLAN list reader with no caret
    /// (lab C9300-24T 17.6.4: `vlan 5000`), the character number the one after the number. A list of
    /// several VLANs or a range is left to the grammar: its message is derived, not measured.
    fn vlan_past_4094(&self, line: &str) -> Option<(Vec<String>, Result)> {
        if self.mode() != "config" {
            return None;
        }
        let toks = cli::tokens(line);
        let [t, n] = &toks[..] else {
            return None;
        };
        let value = n.text.trim_start_matches('0');
        let past = value.len() > 4 || value.parse::<u32>().is_ok_and(|v| v > 4094);
        if !past
            || !n.text.bytes().all(|b| b.is_ascii_digit())
            || !self.names(&t.text.to_lowercase(), "vlan", &["config"])
        {
            return None;
        }
        Some((
            vec![
                format!(
                    "Command rejected: Bad VLAN list - character #{} (EOL) delimits a VLAN",
                    n.text.len() + 1
                ),
                "number which is out of the range 1..4094.".into(),
            ],
            Result::Error,
        ))
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
                entered: text.to_string(),
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

    /// A line matched in `mode`, as a replayed configuration is when the session replays one: as the
    /// books read it, else as a form the switch answered at the session's level (`measured`).
    fn parse(&self, mode: &str, line: &str) -> Outcome {
        let o = self.books(mode, line);
        if matches!(o, Outcome::Ok(_)) {
            return o;
        }
        self.measured(mode, line).map_or(o, Outcome::Ok)
    }

    /// `parse` against the books alone.
    fn books(&self, mode: &str, line: &str) -> Outcome {
        if self.redacted {
            self.engine.parse_redacted(mode, line)
        } else {
            self.engine.parse(mode, line)
        }
    }

    /// The form a line reads as among those the switch of this image answered on Enter at the
    /// session's level (`cli::Engine::parse_measured`).
    fn measured(&self, mode: &str, line: &str) -> Option<Parse> {
        if mode != "privileged" {
            return None;
        }
        let level = self.level()?;
        match self.engine.parse_measured(mode, line, &level) {
            Outcome::Ok(p) => Some(p),
            _ => None,
        }
    }

    /// What a privileged line prints where the form the session's level measured for it holds the
    /// switch's `%` lines (`messages::print`): those that stand, else the command's output `out`.
    /// A line the messages stand in place of is refused, as `unconfigured`'s are; a line the shell
    /// warns of before it runs (`taken_with_warning`) prints that warning and its output instead.
    fn said(&self, mode: &str, line: &str, p: &Parse, out: Vec<String>) -> (Vec<String>, Result) {
        if self.taken_with_warning(mode, p).is_some() {
            return (out, Result::Ok);
        }
        let held = self.measured(mode, line).and_then(|m| {
            let level = self.level()?;
            let swept = self.engine.held.swept.get(&level)?;
            // IOS reads a word that is a keyword where it stands as that keyword: the line is that
            // keyword's command, not the form's value (`show ip vrf detail` beside `show ip vrf <word>`)
            let toks = cli::tokens(line);
            let keyword = |at: usize| {
                let word = &toks[at].text;
                let crawled = self
                    .engine
                    .held
                    .help_for(Some(&level), mode, Some(""), &line[..toks[at].at], |l| {
                        self.running_has(l)
                    })
                    .filter(|a| !crate::live::is_refusal(a))
                    .is_some_and(|a| {
                        crate::live::keywords(a)
                            .iter()
                            .any(|k| k.eq_ignore_ascii_case(word))
                    });
                let (next, _) = self.engine.next_at(mode, &toks[..at], Some(&level));
                crawled
                    || next
                        .iter()
                        .any(|n| matches!(n, Next::Kw { kw, .. } if kw.eq_ignore_ascii_case(word)))
            };
            if m.items
                .iter()
                .any(|i| matches!(i, Item::Arg { tok, .. } if *tok < toks.len() && keyword(*tok)))
            {
                return None;
            }
            let form: Vec<String> = m
                .items
                .iter()
                .map(|i| match i {
                    Item::Kw { kw, .. } => kw.clone(),
                    Item::Arg { name, .. } => format!("<{name}>"),
                })
                .collect();
            Some((swept.said.get(&form)?, m))
        });
        let Some((held, m)) = held else {
            return (out, Result::Ok);
        };
        match crate::messages::print(held, &m.items, &self.device, out) {
            (lines, true) => (lines, Result::Error),
            (lines, false) => (lines, Result::Ok),
        }
    }

    /// The refusal the switch of this image gave a line on Enter at the session's level, where it is
    /// a measured one (`swept::Swept::refused`): its words read against the keywords the held answer
    /// offers before each, else the books' and forms'.
    fn measured_refusal(&self, mode: &str, line: &str) -> Option<Outcome> {
        if mode != "privileged" {
            return None;
        }
        let level = self.level()?;
        let swept = self.engine.held.swept.get(&level)?;
        let keywords = |text: &str| -> Option<Vec<String>> {
            let held = self
                .engine
                .held
                .help_for(Some(&level), mode, Some(""), text, |l| self.running_has(l))
                .filter(|a| !crate::live::is_refusal(a));
            if let Some(a) = held {
                return Some(crate::live::keywords(a));
            }
            let toks = cli::tokens(text);
            let (next, _) = self.engine.next_at(mode, &toks, Some(&level));
            Some(
                next.into_iter()
                    .filter_map(|n| match n {
                        Next::Kw { kw, .. } => Some(kw),
                        Next::Arg { .. } => None,
                    })
                    .collect(),
            )
        };
        swept.refused(line, &keywords)
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
        // a caret the switch drew for this very line stands where it drew it
        if self.measured_refusal(self.mode(), line) == Some(Outcome::Invalid(at)) {
            return column;
        }
        let toks = cli::tokens(line);
        let Some(i) = toks.iter().position(|t| t.at == at) else {
            return column;
        };
        // the words a real switch offered there when asked `?`, else the books' (a held refusal
        // offers none); a heading (`VLAN configuration commands:`) offers no word
        let held = self
            .held_help(self.mode(), &line[..at])
            .filter(|a| !crate::live::is_refusal(a));
        let mut words: Vec<String> = match held {
            Some(lines) => crate::live::answer_rows(lines)
                .into_iter()
                .map(|(w, _)| w.to_string())
                .collect(),
            None => self
                .offered_at(self.mode(), &toks[..i])
                .0
                .into_iter()
                .map(|o| o.shown)
                .collect(),
        };
        // a command raised above `>`'s level is no word there
        if i == 0 {
            words.retain(|w| !self.above_exec(&[w.to_lowercase()]));
        }
        let typed = toks[i].text.to_lowercase();
        // an argument the typed word could be stands here (`WORD`, `A.B.C.D`, `<1-4094>`), and the
        // caret marks the word's start; one it cannot be does not (17.15's `<1-99>` session number
        // at `#` against `shw`); `<cr>` ends the line and `|` starts a modifier. Among interface
        // types a word in capitals is a type (`PROTECTION_GROUP`, `BDI`): the lab C9300-24T (17.6.4)
        // at network-essentials refused `interface nve1` past the `n` its `Null` begins
        let types = words
            .iter()
            .filter(|w| {
                cli::interface_types()
                    .iter()
                    .chain(cli::PHYSICAL_TYPES)
                    .any(|(full, _)| *full == w.as_str())
            })
            .count();
        let could_be = |w: &str| {
            if w == "<cr>"
                || w == "|"
                || w.chars().any(|c| c.is_ascii_lowercase()) && !w.starts_with('<')
                || types >= 2 && !matches!(w, "WORD" | "LINE") && !w.contains(['<', '.', ':'])
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
        // a word running past a listed interface type's whole name into no type's (`Tunnel-tp`
        // where the level lists `Tunnel` alone) is refused past the name, though a host name
        // could stand there (the lab C9300-24T, 17.6.4, at network-essentials)
        let past_type = words
            .iter()
            .filter(|w| {
                cli::interface_types()
                    .iter()
                    .chain(cli::PHYSICAL_TYPES)
                    .any(|(full, _)| *full == w.as_str())
            })
            .map(|w| w.to_lowercase())
            .filter(|w| {
                typed.starts_with(w.as_str())
                    && typed[w.len()..].starts_with(|c: char| !c.is_ascii_digit())
            })
            .map(|w| w.len())
            .max();
        if let Some(n) = past_type
            && !words.iter().any(|w| w.to_lowercase().starts_with(&typed))
        {
            return column + n;
        }
        // on IOS XE a port named by its type and a number refused whole reads no further into its
        // type: the lab C9300-24T (17.6.4) refused `interface gigabitethernet4/0/1` under the `g`,
        // and at network-advantage under the `i`, past the `g` its `GMPLS` begins
        let alpha = typed
            .split(|c: char| c.is_ascii_digit())
            .next()
            .unwrap_or("");
        if types >= 2
            && !self.device.stack.classic()
            && alpha.len() < typed.len()
            && let Some((full, _, _)) = cli::interface_name(&format!("{alpha}1"), None)
        {
            words.retain(|w| !w.eq_ignore_ascii_case(&full));
        }
        if words.iter().any(|w| could_be(w)) {
            return column + past_the_top(&words, &typed);
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
        if self.vlans_seen.is_none() {
            self.vlans_seen = Some(self.device.vlan_contents());
        }
        let before = self.device.vtp_settings();
        let taken = self.accepted.len();
        let out = self.typed(line);
        self.vtp_after(before);
        // a line taken at the console is the configuration's last change
        if !self.redacted && self.accepted.len() > taken {
            self.device.last_change = Some(self.now());
            self.device.last_change_by = None;
        }
        out
    }

    /// The warning for a line known to crash a switch of this family (`hazards`), as the line reads
    /// in the current mode; none for the text a banner, certificate, key or question takes.
    pub fn warning(&self, line: &str) -> Option<String> {
        if self.asking
            || self.banner.is_some()
            || self.certificate.is_some()
            || self.pubkey.is_some()
        {
            return None;
        }
        let family = self.device.stack.family()?;
        crate::hazards::typed(Some(family), self.level().as_deref(), self.mode(), line)
            .map(|h| h.warning(&self.device.release))
    }

    /// A line entered at the console or in a script: `type_line`, with the warning for a line known
    /// to crash the switch after the whole answer, so the caret stays under the line it marks.
    pub fn enter(&mut self, line: &str) -> (Vec<String>, Result) {
        let warning = self.warning(line);
        let (mut out, r) = self.type_line(line);
        if let Some(w) = warning {
            self.warned += 1;
            out.push(w);
        }
        (out, r)
    }

    fn typed(&mut self, line: &str) -> (Vec<String>, Result) {
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
        if let Some(done) = self.pubkey_data(line) {
            return done;
        }
        let prompt_len = lead.unwrap_or_else(|| self.prompt().chars().count());
        if let Some(at) = self.do_exec_at(line) {
            if line[at..].trim().is_empty() {
                return (
                    vec!["% Incomplete command.".into(), String::new()],
                    Result::Error,
                );
            }
            return self.run_exec(prompt_len, line, at);
        }
        if let Some(done) = self.terminal(line, prompt_len) {
            return done;
        }
        if let Some(done) = self.vlan_past_4094(line) {
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
        let top = self.frames.len() - 1;
        let mut outcome = self.read_at(top, &mode, trimmed);
        // a line a real switch of this platform and release held in this block is taken as it held it,
        // but for one this image's own answers refused in this mode and context: a held line's block
        // kind is any interface's (`ip address` held under a VLAN interface, refused on a switchport)
        let refused_by_held = !matches!(outcome, Outcome::Ok(_))
            && (matches!(self.parse(&mode, trimmed), Outcome::Ok(_))
                || self.level_refusal(top, &mode, trimmed).is_some());
        if !matches!(outcome, Outcome::Ok(_))
            && !refused_by_held
            && let Some(done) = self.held_in_block(full)
        {
            return done;
        }
        // a configuration submode hands a line it does not know to the nearest enclosing mode that
        // takes it (`no ip address` after `vrf forwarding` goes to the interface), global
        // configuration last; a numbered entry stays in its list. A line the submode takes only as an
        // abbreviation goes to an enclosing mode that takes it word for word: the 17.6 book lists
        // `router ospfv3` under config-if, and the lab C9300 (17.6.4) keeps `router ospf 1` typed there
        let mut back_to = None;
        // a line every mode refuses is the submode's refusal and the session stays, but a line an
        // enclosing mode reads past its first word as one that leaves for it (`leaves`) is that
        // mode's refusal, and the session leaves for it
        let mut refused_in: Option<(usize, Outcome)> = None;
        let abbreviated = matches!(&outcome, Outcome::Ok(p) if !whole_words(p));
        // a first word the submode reads only as the start of a longer one (`key` for
        // `key-string`) is an enclosing mode's word where that mode has it whole: the lab C9300-24T
        // and C9200L-48T-4G (17.6.4) refused `key rad123` and `key 0 auth-key` at
        // (config-keychain-key)# and stayed, as the key chain's `key` takes one number, the caret
        // under the space after `key`
        let first_abbreviated = matches!(&outcome, Outcome::Ok(p) if p.items.iter().find_map(|i| match i {
            Item::Kw { kw, exact, .. } if !kw.eq_ignore_ascii_case("no") => Some(!*exact),
            _ => None,
        }) == Some(true));
        let first_word = trimmed.split_whitespace().next().unwrap_or("");
        let mut whole_refusal = None;
        if cli::is_submode(&mode)
            && seq.is_none()
            && (abbreviated || !matches!(outcome, Outcome::Ok(_)))
        {
            let first = cli::tokens(trimmed)
                .first()
                .map_or(0, |t| t.at + t.text.len());
            for i in (0..self.frames.len().saturating_sub(1)).rev() {
                let m = self.frames[i].mode.clone();
                if !cli::is_config(&m) {
                    break;
                }
                let o = self.read_at(i, &m, trimmed);
                if matches!(&o, Outcome::Ok(q) if !abbreviated || whole_words(q)) {
                    back_to = Some(i);
                    mode = m;
                    outcome = o;
                    refused_in = None;
                    break;
                }
                if first_abbreviated
                    && !matches!(o, Outcome::Ok(_))
                    && reach(&o, trimmed) > first
                    && self.engine.grammar.commands.iter().any(|c| {
                        c.modes.contains(&m)
                            && c.name
                                .split_whitespace()
                                .next()
                                .is_some_and(|w| w.eq_ignore_ascii_case(first_word))
                    })
                    && whole_refusal.is_none()
                {
                    whole_refusal = Some(match o {
                        Outcome::Invalid(_) => Outcome::Invalid(first),
                        o => o,
                    });
                    continue;
                }
                if !abbreviated
                    && refused_in.is_none()
                    && reach(&o, trimmed) > first
                    && self.leaves(i, trimmed)
                {
                    refused_in = Some((i, o));
                }
            }
        }
        if let Some((i, o)) = refused_in {
            outcome = o;
            back_to = Some(i);
        }
        if back_to.is_none()
            && let Some(o) = whole_refusal
        {
            outcome = o;
        }
        let p = match outcome {
            Outcome::Ok(p) => p,
            other => {
                // a line real switches held at another level is not taken where this one's leaves
                // its words out
                if self.level_refusal(2, "config", trimmed).is_none()
                    && let Some(done) = self.held_at_top(trimmed)
                {
                    return done;
                }
                self.frames.extend(closed.drain(..).rev());
                if let Some(i) = back_to {
                    self.frames.truncate(i + 1);
                }
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
        // a port named by the start of its number, last on the line, is incomplete (the lab C9300-24T,
        // 17.6.4: `interface gigabitethernet1/1`, whose network module holds Gi1/1/1 to 1/1/4), and
        // so is the same start on a member the stack lacks
        let last = p.items.len().saturating_sub(1);
        let mut partial = false;
        let refused = p.items.iter().enumerate().find_map(|(n, it)| match it {
            Item::Arg {
                name, value, tok, ..
            } if (matches!(
                args.get(name),
                Some(ArgKind::Interface | ArgKind::InterfaceRange)
            ) || matches!(args.get(name), Some(ArgKind::InterfaceOf { of }) if modelled(of)))
                && !on_stack(value, &self.device.stack) =>
            {
                partial = n == last
                    && (starts_a_port(value, &self.device.stack)
                        || short_of_a_port(value, &self.device.stack));
                Some(*tok)
            }
            _ => None,
        });
        // a sampler a port's flow monitor names before it exists, at its name (the lab
        // C9200L-48T-4G, 17.6.4 and 17.15.6; the C9300 refused the monitor first, `offers`); a
        // sampler is held as a global line, or as a block where its command enters a submode
        let refused = refused.or_else(|| {
            let flow = matches!(
                p.items.get(..3),
                Some([Item::Kw { kw: ip, .. }, Item::Kw { kw: flow, .. }, Item::Kw { kw: monitor, .. }])
                    if (ip == "ip" || ip == "ipv6") && flow == "flow" && monitor == "monitor"
            );
            if !(flow && mode.starts_with("config-if") && self.runs_any(crate::context::C9200_LAB)) {
                return None;
            }
            p.items.windows(2).find_map(|w| match w {
                [Item::Kw { kw, .. }, Item::Arg { value, tok, .. }]
                    if kw == "sampler"
                        && !self.device.blocks.iter().any(|b| {
                            b.header.strip_prefix("sampler ") == Some(value.as_str())
                        })
                        && !self.device.global.iter().any(|e| {
                            e.text.trim().strip_prefix("sampler ") == Some(value.as_str())
                        }) =>
                {
                    Some(*tok)
                }
                _ => None,
            })
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
            // a line an enclosing mode read is refused there as one that leaves for it (the lab
            // C9300, 17.6.4: `interface GigabitEthernet1/0/30` in config-if, for (config)#), else
            // it is the submode's own refusal
            match back_to {
                Some(i) if self.leaves(i, trimmed) => self.frames.truncate(i + 1),
                Some(_) => {
                    let own = self.read_at(self.frames.len() - 1, self.mode(), trimmed);
                    if !matches!(own, Outcome::Ok(_)) {
                        return (self.error(prompt_len, trimmed, &own), Result::Error);
                    }
                }
                None => {}
            }
            let stack = &self.device.stack;
            let at = crate::cli::tokens(trimmed).get(tok).map_or(0, |t| {
                t.at + match past_in_range(&t.text, stack) {
                    0 => past_a_name(&trimmed[t.at..], stack),
                    n => n,
                }
            });
            let o = if partial {
                Outcome::Incomplete
            } else {
                Outcome::Invalid(at)
            };
            return (self.error(prompt_len, trimmed, &o), Result::Error);
        }
        // a line the licence level leaves out of the image: what IOS says, and the mode stays
        if let Some(said) = self.unlicensed(&mode, &p) {
            self.frames.extend(closed.drain(..).rev());
            if let Some(i) = back_to {
                self.frames.truncate(i + 1);
            }
            return (vec![said.to_string()], Result::Error);
        }
        // a line the configuration so far leaves IOS unable to take: what IOS says, and the line
        // changes nothing; one an enclosing mode read leaves for it
        if let Some(said) = self.unconfigured(&mode, &p) {
            self.frames.extend(closed.drain(..).rev());
            if let Some(i) = back_to {
                self.frames.truncate(i + 1);
            }
            let out: Vec<String> = said.lines().map(str::to_string).collect();
            return (out, Result::Error);
        }
        // the expression keeps a space typed at the end of the line, as the lab C9300 (17.6.4) does
        let typed = line.trim_end_matches(['\r', '\n']);
        let modifier = match bar.map(|b| crate::filter::parse(&typed[b + 1..], b + 1)) {
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
        // an IPv6 list keeps its entries numbered: one typed without a number takes the highest
        // plus 10 (the lab C9300-24T, 17.6.4, printed ` permit ipv6 2001:DB8::/32 any` and ` deny
        // ipv6 any any`, typed in a new list, as ` sequence 10 permit ...` and ` sequence 20 deny
        // ...`); `no` before an entry removes it
        if back_to.is_none()
            && mode == "config-ipv6-acl"
            && !self.device.stack.classic()
            && let Some(b) = self.frames.last().and_then(|f| f.blocks.first().copied())
        {
            let canonical = p.canonical();
            let text = canonical
                .strip_prefix("no ")
                .unwrap_or(&canonical)
                .to_string();
            if matches!(text.split_whitespace().next(), Some("permit" | "deny")) {
                let numbered = |e: &Entry| -> Option<(u32, String)> {
                    let (n, rest) = e.text.strip_prefix("sequence ")?.split_once(' ')?;
                    Some((n.parse().ok()?, rest.to_string()))
                };
                let lines = &mut self.device.blocks[b].lines;
                if p.negated {
                    lines.retain(|e| numbered(e).is_none_or(|(_, t)| t != text));
                } else if !lines
                    .iter()
                    .any(|e| numbered(e).is_some_and(|(_, t)| t == text))
                {
                    let next = lines
                        .iter()
                        .filter_map(&numbered)
                        .map(|(n, _)| n)
                        .max()
                        .unwrap_or(0)
                        + 10;
                    lines.push(Entry {
                        key: format!("sequence #{next}"),
                        text: format!("sequence {next} {text}"),
                    });
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
        // the lab C2960G (15.0(2)SE11): `hidden` waits for version 3 and keeps the password set
        // before
        let canonical = p.canonical();
        let vtp_password = (mode == "config")
            .then(|| canonical.strip_prefix("vtp password "))
            .flatten();
        if let Some(pw) = vtp_password
            && pw.ends_with(" hidden")
        {
            let (version, _, _) = self.device.vtp_settings();
            if version != "3" {
                return (
                    vec![format!(
                        "Device is operating in VTP version {version}. hidden option is allowed from VTP version 3 only."
                    )],
                    Result::Ok,
                );
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
        let speeds = self.forced_speeds(&p);
        let warned = self.taken_with_warning(&mode, &p);
        let (mut out, r) = self.run(&mode, trimmed, &p);
        if r == Result::Ok
            && !self.redacted
            && let Some(w) = warned
        {
            out.splice(0..0, w.lines().map(str::to_string));
        }
        if r == Result::Ok {
            self.hold_speeds(&p, speeds);
        }
        // at the console an access or voice VLAN the switch lacks is made, and said so, as IOS does;
        // a replayed configuration defines its own
        if r == Result::Ok
            && !self.redacted
            && !p.negated
            && let Some(made) = self.vlan_made(&p.canonical())
        {
            out.insert(0, made);
        }
        // and says what a VTP password line did (on IOS XE inferred)
        if r == Result::Ok && !self.redacted && mode == "config" {
            if let Some(pw) = vtp_password {
                out.insert(0, format!("Setting device VTP password to {pw}"));
            } else if canonical == "no vtp password" {
                out.insert(0, "Clearing device VTP password.".into());
            }
        }
        // a legacy 802.1X line IOS XE takes and calls deprecated once the configuration is
        // new-style (the lab C9300-24T, 17.6.4); a legacy one takes it silently (the lab C9200L)
        if r == Result::Ok
            && !self.redacted
            && !p.negated
            && !self.device.stack.classic()
            && new_style(&self.device)
            && DEPRECATED
                .iter()
                .any(|(m, start)| *m == mode && canonical.starts_with(start))
        {
            let said =
                format!("%Command deprecated ({canonical}) - use access-session instead\n\n");
            out.splice(0..0, said.lines().map(str::to_string));
        }
        // an address typed at the console reads Method `manual` (the lab C9300, 17.6.4)
        if r == Result::Ok
            && !self.redacted
            && !p.negated
            && crate::device::key(&p).starts_with("ip address")
        {
            let typed: Vec<String> = self
                .frames
                .last()
                .map(|f| {
                    f.blocks
                        .iter()
                        .map(|&b| self.device.blocks[b].header.clone())
                        .collect()
                })
                .unwrap_or_default();
            self.device.manual.extend(typed);
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

    /// What IOS XE says for a line its licence level leaves out of the image, where the level is gated
    /// (`license::GATED`; the lab C9300-24T, 17.6.4, at network-essentials): `router bgp` and `router
    /// isis` print `Protocol not in this image`, a VRF but the management port's `% Feature is not
    /// supported`, and a redistribution from IS-IS `% Failed to create routing protocol isis`.
    fn unlicensed(&self, mode: &str, p: &Parse) -> Option<&'static str> {
        if p.negated || !self.engine.held.gated(self.level().as_deref()) {
            return None;
        }
        let canonical = p.canonical();
        let words: Vec<&str> = canonical.split_whitespace().collect();
        let management =
            |name: &str| name == "Mgmt-vrf" && self.device.stack.management().is_some();
        match (mode, words.as_slice()) {
            ("config", ["router", "bgp" | "isis", ..]) => Some("Protocol not in this image"),
            ("config", ["vrf", "definition", name] | ["ip", "vrf", name]) if !management(name) => {
                Some("% Feature is not supported")
            }
            (m, ["redistribute", "isis", ..]) if m.starts_with("config-router") => {
                Some("% Failed to create routing protocol isis")
            }
            _ => None,
        }
    }

    /// What IOS says for a line the configuration so far does not let it take, on IOS XE (the lab
    /// C9300-24T, 17.6.4): a second BGP process (`BGP is already running; AS is 65000`), a named
    /// EIGRP address family's second AS or one another named instance or a classic process runs, on
    /// the C9200 image (the lab C9200L-48T-4G, 17.6.4) a port's flow monitor holding no record and
    /// a monitor's second record, a class a policy map names before its class map exists (`class
    /// map class1 not configured`), a second mDNS controller (`Only one Controller is allowed`,
    /// then a blank line), a VRF the switch does not hold (`NO_VRF`, an OSPF process's `Unknown VRF
    /// specified`, a classic EIGRP family's), a global SXP filter group's filter list not made, a
    /// BGP neighbor's policy outside an address family once the process has one, a template a
    /// neighbor inherits before it exists, a private VLAN association naming a VLAN the switch does
    /// not hold, a secondary type for a VLAN whose SVI is up, a tunnel PIM registers through
    /// (`pim_tunnels`), an object a line names before it exists (a policy map, a first-hop security
    /// policy (`ATTACH`), a NAT pool, an access list, a trustpoint, a regex parameter map, a VFI's
    /// VPN ID, a LISP locator set), a classifier condition while the classifier runs, an IP service
    /// on a LISP instance carrying Ethernet, a LISP PETR's priority beside one without, an MLD SSM
    /// map's static entry before `ipv6 mld ssm-map enable` (`%ssm-map is not enabled!`), an auth
    /// proxy rule a port names before it is defined (the C9200 17.6 image), a subnet another
    /// interface holds, a BGP neighbor line before the neighbor's remote AS, a link-local peer
    /// outside an address family, the switch's own address as a peer, a SPAN line its sessions
    /// refuse (`span_refusal`), an HSRP group's second address family, a setting already in force
    /// or one another excludes (BackboneFast, an NTP trusted key, NSF, an MDT default group, a
    /// tracked object's kind, an FQDN list's name), a value out of range (a template's port
    /// priority, an engine ID, a list number of the wrong kind), an interface whose mode or
    /// features refuse the line (half duplex, `nonegotiate`, network policy, `cts manual`, REP,
    /// Umbrella, DHCPv6 client), and the EXEC shows of state the switch lacks (MST, WCCP, an OSPF
    /// topology, a BGP neighbor).
    /// `no` lines are taken, but for a missing policy map's removal and `no switchport` on a port
    /// under 802.1X port control.
    fn unconfigured(&self, mode: &str, p: &Parse) -> Option<String> {
        if self.device.stack.classic() {
            return None;
        }
        let canonical = p.canonical();
        let words: Vec<&str> = canonical.split_whitespace().collect();
        let named = |prefix: &str, name: &str| {
            self.device
                .blocks
                .iter()
                .any(|b| b.header.strip_prefix(prefix) == Some(name))
        };
        // a policy map an interface names before it exists, its removal too
        if let (
            "config-if",
            ["service-policy", "input" | "output", name]
            | ["no", "service-policy", "input" | "output", name],
        ) = (mode, words.as_slice())
            && !named("policy-map ", name)
        {
            return Some(format!("% policy map {name} not configured"));
        }
        // the block the line is typed into: the router, interface or VLAN of its mode
        let block = self
            .frames
            .iter()
            .rev()
            .find(|f| f.mode == mode)
            .and_then(|f| f.blocks.first())
            .and_then(|&b| self.device.blocks.get(b));
        // a port mode change on a port under 802.1X port control (the lab C9200L-48T-4G, 17.6.4,
        // took `switchport mode trunk` and `switchport mode private-vlan host` there)
        if let (
            "config-if",
            ["no", "switchport"]
            | ["switchport", "mode", "dot1q-tunnel"]
            | ["switchport", "mode", "private-vlan", "promiscuous"],
        ) = (mode, words.as_slice())
            && block.is_some_and(|b| {
                b.lines
                    .iter()
                    .any(|e| e.text.trim().starts_with("authentication port-control "))
            })
        {
            return Some(
                "Command rejected: Conflict with Authentication. \nAuthentication must be disabled \
                 before changing port mode "
                    .into(),
            );
        }
        if p.negated {
            return None;
        }
        let header = block.map_or("", |b| b.header.as_str());
        let holds =
            |text: &str| block.is_some_and(|b| b.lines.iter().any(|e| e.text.trim() == text));
        // the management port's VRF is in the factory default (`startup::factory`)
        let vrf = |name: &str| {
            (name == "Mgmt-vrf" && self.device.stack.management().is_some())
                || self.device.blocks.iter().any(|b| {
                    b.header.strip_prefix("vrf definition ") == Some(name)
                        || b.header.strip_prefix("ip vrf ") == Some(name)
                })
        };
        for (m, start, said) in NO_VRF {
            if (*m == mode || *m == "exec" && mode == "privileged")
                && words.len() > start.len()
                && words[..start.len()] == **start
                && !vrf(words[start.len()])
            {
                return Some(said.replace("{}", words[start.len()]));
            }
        }
        // an OSPF process in a VRF the switch does not hold (the lab C9300-24T and C9200L-48T-4G)
        if let ("config", ["router", "ospf", _, "vrf", name, ..]) = (mode, words.as_slice())
            && !vrf(name)
        {
            return Some("Unknown VRF specified".into());
        }
        // a global SXP filter group before its filter list (the lab C9300-24T and C9200L-48T-4G)
        if let (
            "config",
            [
                "cts",
                "sxp",
                "filter-group",
                "listener" | "speaker",
                "global",
                name,
            ],
        ) = (mode, words.as_slice())
            && !named("cts sxp filter-list ", name)
        {
            return Some(format!("Filter-list {name} not configured"));
        }
        let vlans = || show::defined_vlans(&self.device);
        for (m, feature) in ATTACH {
            if *m == mode
                && words.len() > feature.len() + 1
                && words[..feature.len()] == **feature
                && words[feature.len()] == "attach-policy"
                && !named(
                    &format!("{} policy ", feature.join(" ")),
                    words[feature.len() + 1],
                )
            {
                let name = words[feature.len() + 1];
                return Some(format!("% Policy {name} not defined for the feature\n\n"));
            }
        }
        let global = |prefix: &str| {
            self.device
                .global
                .iter()
                .any(|e| e.text.starts_with(prefix))
                || self
                    .device
                    .blocks
                    .iter()
                    .any(|b| b.header.starts_with(prefix))
        };
        // an access list by number or name, numbered ones also as `access-list <n>` lines
        let acl = |name: &str| {
            global(&format!("access-list {name} "))
                || ["standard", "extended"]
                    .iter()
                    .any(|k| named(&format!("ip access-list {k} "), name))
        };
        let mst = self
            .device
            .global
            .iter()
            .any(|e| e.text == "spanning-tree mode mst");
        // a BGP neighbor given a remote AS, a peer group or a session template in the block
        let peered = |peer: &str| {
            block.is_some_and(|b| {
                b.lines.iter().any(|e| {
                    let t = e.text.trim();
                    ["remote-as ", "peer-group", "inherit peer-session "]
                        .iter()
                        .any(|k| t.starts_with(&format!("neighbor {peer} {k}")))
                })
            })
        };
        match (mode, words.as_slice()) {
            ("config", ["ipv6", "mld", "ssm-map", "static", ..])
                if !global("ipv6 mld ssm-map enable") =>
            {
                return Some("%ssm-map is not enabled!".into());
            }
            // an auth proxy rule a port names before `ip admission name` defines it (the lab
            // C9200L-48T-4G, 17.6.4 and 17.15.6; the lab C9300-24T at 17.6.4 took it silently)
            ("config-if", ["ip", "admission", name])
                if self.runs_any(crate::context::C9200_LAB)
                    && !global(&format!("ip admission name {name} ")) =>
            {
                return Some(format!("%Auth-proxy name {name} is not defined\n\n"));
            }
            (
                "config",
                [
                    "ip",
                    "nat",
                    "inside" | "outside",
                    "source",
                    "list",
                    _,
                    "pool",
                    name,
                    ..,
                ],
            ) if !global(&format!("ip nat pool {name} ")) => {
                return Some(format!("%Pool {name} not configured \n\n"));
            }
            ("config", ["ntp", "access-group", _, name, ..]) if !acl(name) => {
                let is = if name.bytes().all(|b| b.is_ascii_digit()) {
                    "is "
                } else {
                    ""
                };
                return Some(format!("%Access-list {name} {is}not defined"));
            }
            ("config", ["cts", "role-based", "permissions", rest @ ..]) => {
                let names = match rest {
                    ["from", _, "to", _, names @ ..] | ["default", names @ ..] => names,
                    _ => return None,
                };
                let said: Vec<String> = names
                    .iter()
                    .filter(|n| !["ipv4", "ipv6"].contains(n))
                    .filter(|n| {
                        !named("ip access-list role-based ", n)
                            && !named("ipv6 access-list role-based ", n)
                    })
                    .map(|n| format!("% ACL {n} is not defined."))
                    .collect();
                return (!said.is_empty()).then(|| said.join("\n"));
            }
            ("config", ["crypto", "pki", "certificate", "chain", name])
                if !named("crypto pki trustpoint ", name) =>
            {
                return Some("% CA trustpoint for cert chain not known.\n\n".into());
            }
            (_, ["crypto", "pki", "enroll", name]) if !named("crypto pki trustpoint ", name) => {
                return Some(format!("% CA server trustpoint '{name}' is not known.\n\n"));
            }
            ("config-profile", ["local-domain", name])
                if !named("parameter-map type regex ", name) =>
            {
                return Some(format!("% Parameter-map regex {name} does not exist"));
            }
            (
                "config",
                [
                    "device",
                    "classifier",
                    kind @ ("condition" | "device-type"),
                    ..,
                ],
            ) if self.running_has("device classifier") => {
                let (gap, end) = if *kind == "device-type" {
                    (" ", "\n\n")
                } else {
                    ("", "")
                };
                return Some(format!(
                    "% Device classifier is enabled.{gap}Please disable it and execute the command \
                     again.{end}"
                ));
            }
            ("config-router-af", ["neighbor", peer, kw, ..])
                if header.starts_with("router bgp ")
                    && (BGP_POLICY.contains(kw) || *kw == "activate")
                    && !peered(peer) =>
            {
                return Some("% Specify remote-as or peer-group commands first".into());
            }
            ("config-if", ["ip", "address", a, m]) => {
                let (a, m) = (v4(a)?, v4(m)?);
                let vrf_of = |b: &crate::device::Block| {
                    b.lines
                        .iter()
                        .find_map(|e| e.text.trim().strip_prefix("vrf forwarding "))
                        .map(str::to_string)
                };
                let own = block.and_then(vrf_of);
                // the other interface whose subnet overlaps, the lowest subnet as IOS names it
                let other = self
                    .device
                    .blocks
                    .iter()
                    .filter(|b| b.header.starts_with("interface ") && b.header != header)
                    .filter(|b| vrf_of(b) == own)
                    .flat_map(|b| {
                        b.lines
                            .iter()
                            .filter_map(|e| ip_address(&e.text))
                            .map(move |(n, k)| (n & k, k, b))
                    })
                    .filter(|&(n, k, _)| n & (k & m) == a & (k & m))
                    .min_by_key(|&(n, _, _)| n)?;
                let name = other.2.header.trim_start_matches("interface ");
                return Some(format!(
                    "% {} overlaps with {name}",
                    std::net::Ipv4Addr::from(a & m)
                ));
            }
            // an HSRP group with a primary IPv4 address
            ("config-if", ["standby", g, "ip"] | ["standby", g, "ipv6", ..])
                if block.is_some_and(|b| {
                    b.lines
                        .iter()
                        .any(|e| e.text.trim().starts_with(&format!("standby {g} ip ")))
                }) =>
            {
                return Some(if words[2] == "ip" {
                    "% Failed. Unconfigure existing primary vIP address configured for the group."
                        .into()
                } else {
                    "% Group configured for different address family".into()
                });
            }
            ("config", ["spanning-tree", "backbonefast"])
                if self
                    .device
                    .global
                    .iter()
                    .any(|e| e.text == "spanning-tree backbonefast") =>
            {
                return Some("% Backbonefast already enabled\n\n".into());
            }
            ("config-template", ["spanning-tree", "port-priority", n])
                if n.parse::<u32>().is_ok_and(|v| v % 16 != 0) =>
            {
                return Some("% Port Priority in increments of 16 is required\n\n".into());
            }
            // access lists after a prefix list in one sequence: one line for each list named
            (
                "config-route-map",
                [
                    "match",
                    "ip",
                    kind @ ("address" | "route-source"),
                    lists @ ..,
                ],
            ) if lists.first().is_some_and(|l| *l != "prefix-list")
                && block.is_some_and(|b| {
                    b.lines.iter().any(|e| {
                        e.text
                            .trim()
                            .starts_with(&format!("match ip {kind} prefix-list "))
                    })
                }) =>
            {
                return Some(
                    vec![
                        "% prefix-list and access-list can not co-exist in one route-map sequence.";
                        lists.len()
                    ]
                    .join("\n"),
                );
            }
            ("config", ["ipv6", "access-list", "fqdn", name])
                if named("ip access-list fqdn ", name) =>
            {
                return Some("% A(n) IP FQDN access list with this name already exists".into());
            }
            ("config-router", ["nsf", "cisco", ..])
                if header.starts_with("router isis")
                    && !block.is_some_and(|b| {
                        b.lines.iter().any(|e| e.text.trim().starts_with("net "))
                    }) =>
            {
                return Some("%ISIS not running (configure NET)".into());
            }
            ("config-router", ["nsf", "ietf", ..])
                if header.starts_with("router ospf ")
                    && block.is_some_and(|b| {
                        b.lines
                            .iter()
                            .any(|e| e.text.trim().starts_with("nsf cisco"))
                    }) =>
            {
                return Some(
                    "% OSPF: Cannot enable both Cisco NSF and IETF Graceful Restart".into(),
                );
            }
            // a VRF's default MDT group is one group
            ("config-vrf-af", ["mdt", "default", group])
                if let Some(other) = block.and_then(|b| {
                    b.lines
                        .iter()
                        .find_map(|e| e.text.trim().strip_prefix("mdt default "))
                        .filter(|g| g != group)
                }) =>
            {
                let vrf = header.strip_prefix("vrf definition ").unwrap_or(header);
                return Some(format!(
                    "% {other} has already been configured for vrf {vrf}"
                ));
            }
            // a data MDT group comes before the lines that tune it
            ("config-vrf-af", ["mdt", "log-reuse"] | ["mdt", "data", "threshold", ..])
                if !block.is_some_and(|b| {
                    b.lines.iter().any(|e| {
                        e.text
                            .trim()
                            .strip_prefix("mdt data ")
                            .is_some_and(|r| !r.starts_with("threshold"))
                    })
                }) =>
            {
                return Some(if words[1] == "log-reuse" {
                    "% First configure the data MDT group!".into()
                } else {
                    "% MDT data must be configured first".into()
                });
            }
            ("config", ["crypto", "pki", "import", name, "certificate"])
                if !named("crypto pki trustpoint ", name) =>
            {
                return Some(format!("% CA server trustpoint '{name}' is not known.\n\n"));
            }
            ("config", ["snmp-server", "engineID", "local", id]) if id.len() < 10 => {
                return Some("%Engine ID must be 10 hexadecimal characters or more".into());
            }
            // half duplex only where autonegotiation offers no more than 100 Mb/s
            ("config-if", ["duplex", "half"])
                if !block.is_some_and(|b| {
                    b.lines.iter().any(|e| {
                        let speeds: Vec<&str> = e
                            .text
                            .split_whitespace()
                            .skip(1)
                            .filter(|w| *w != "auto")
                            .collect();
                        e.text.trim().starts_with("speed ")
                            && !speeds.is_empty()
                            && speeds.iter().all(|s| matches!(*s, "10" | "100"))
                    })
                }) =>
            {
                return Some(
                    "\n% Duplex cannot be set to half when speed autonegotiation subset contains \
                     1Gbps,2.5Gbps,5Gbps or 10Gbps\n\n"
                        .into(),
                );
            }
            // a port with no access or trunk mode negotiates (`dynamic auto`)
            ("config-if", ["switchport", "nonegotiate"])
                if !holds("switchport mode access") && !holds("switchport mode trunk") =>
            {
                let name = header.strip_prefix("interface ")?;
                return Some(format!(
                    "Command rejected: Conflict between 'nonegotiate' and 'dynamic' status on this \
                     interface: {}",
                    show::short(name)
                ));
            }
            ("config-if", ["network-policy", _]) if holds("switchport mode trunk") => {
                return Some(
                    "Command rejected: Network Policy is not supported on a trunk port".into(),
                );
            }
            ("config-if", ["cts", "manual"]) if holds("dot1x pae authenticator") => {
                let name = header.strip_prefix("interface ")?;
                return Some(format!(
                    "Command rejected ({}): conflict with Dot1x Auth\n\n",
                    show::short(name)
                ));
            }
            ("config-if", ["umbrella", "in", ..]) if holds("umbrella out") => {
                return Some("% 'umbrella out' already configured on interface".into());
            }
            ("config-if", ["ipv6", "address", "dhcp", ..])
                if block.is_some_and(|b| {
                    b.lines
                        .iter()
                        .any(|e| e.text.trim().starts_with("ipv6 dhcp server "))
                }) =>
            {
                return Some("% Interface is in DHCP server mode".into());
            }
            ("config", ["ntp", "trusted-key", key])
                if self
                    .device
                    .global
                    .iter()
                    .any(|e| e.text == format!("ntp trusted-key {key}")) =>
            {
                return Some("%NTP trusted-key is invalid : key already configured ".into());
            }
            // a tracked object keeps its kind: interface, IP route or SLA, list
            ("config", ["track", n, kind, rest @ ..]) => {
                let kind_of = |kind: &str, rest: &[&str]| match kind {
                    "ip" => format!("ip {}", rest.first().copied().unwrap_or("")),
                    k => k.to_string(),
                };
                let mine = kind_of(kind, rest);
                let other = self.device.blocks.iter().any(|b| {
                    let w: Vec<&str> = b.header.split_whitespace().collect();
                    w.len() > 2 && w[0] == "track" && w[1] == *n && kind_of(w[2], &w[3..]) != mine
                });
                if other {
                    return Some(format!(
                        "% Cannot change tracked object {n} - delete old config first"
                    ));
                }
            }
            // a numbered list of the wrong kind: offset lists take standard, MSDP extended
            ("config-router", ["offset-list", n, "in" | "out", ..])
                if n.parse::<u32>()
                    .is_ok_and(|n| !matches!(n, 1..=99 | 1300..=1999)) =>
            {
                return Some("% Invalid access list name.".into());
            }
            ("config", ["ip", "msdp", "redistribute", "list", n, ..])
                if n.parse::<u32>()
                    .is_ok_and(|n| !matches!(n, 100..=199 | 2000..=2699)) =>
            {
                return Some("% Invalid access list name.".into());
            }
            ("config-if", ["rep", "stcn", ..]) if !holds("switchport mode trunk") => {
                return Some(
                    "Command rejected: conflict with the current Switchport Mode\n\n".into(),
                );
            }
            ("exec" | "privileged", ["show", "ip", "ospf", "interface", .., "topology", name])
                if !self.device.blocks.iter().any(|b| {
                    b.header.starts_with("router ospf ")
                        && b.lines.iter().any(|e| {
                            let t = e.text.trim();
                            t == format!("topology {name}")
                                || t.starts_with(&format!("topology {name} "))
                        })
                }) =>
            {
                return Some("%OSPF: Specified topology not found for any OSPF process".into());
            }
            ("exec" | "privileged", ["show", "ip", "bgp", "neighbors", peer, _, ..])
                if peer.parse::<std::net::IpAddr>().is_ok()
                    && !self.device.blocks.iter().any(|b| {
                        b.header.starts_with("router bgp ")
                            && b.lines
                                .iter()
                                .any(|e| e.text.trim().starts_with(&format!("neighbor {peer} ")))
                    }) =>
            {
                return Some("% No such neighbor or address family".into());
            }
            ("exec" | "privileged", ["show", "ip", "wccp", ..]) if !global("ip wccp ") => {
                return Some("% WCCP version 2 is not enabled".into());
            }
            ("exec" | "privileged", ["show", "spanning-tree", "mst", n, ..])
                if n.parse::<u32>().is_ok() && !mst =>
            {
                return Some("% Switch is not in mst mode\n\n".into());
            }
            ("config-vrf" | "config-vrf-af", ["route-target", ..])
                if !block.is_some_and(|b| {
                    b.lines
                        .iter()
                        .any(|e| e.text.trim_start().starts_with("rd "))
                }) =>
            {
                let name = header
                    .strip_prefix("vrf definition ")
                    .or_else(|| header.strip_prefix("ip vrf "))?;
                return Some(format!(
                    "% VRF {name} does not have an RD configured, please configure the RD before \
                     configuring route-target"
                ));
            }
            ("config-vlan-config", ["member", "vfi", name])
                if !self.device.blocks.iter().any(|b| {
                    b.header.strip_prefix("l2vpn vfi context ") == Some(*name)
                        && b.lines
                            .iter()
                            .any(|e| e.text.trim_start().starts_with("vpn id "))
                }) =>
            {
                return Some(format!(
                    "% VFI {name} must be configured with a valid vpn id first."
                ));
            }
            ("config-lisp-inst", ["service", "ipv4" | "ipv6"]) => {
                // the instance's own lines: those under its `instance-id` line, indented further
                let entered = &self.frames.iter().rev().find(|f| f.mode == mode)?.entered;
                let id = entered.split_whitespace().nth(1)?;
                let lines = &block?.lines;
                let indent = |t: &str| t.len() - t.trim_start().len();
                let at = lines
                    .iter()
                    .position(|e| e.text.trim() == format!("instance-id {id}"))?;
                let own = indent(&lines[at].text);
                let ethernet = lines[at + 1..]
                    .iter()
                    .take_while(|e| indent(&e.text) > own)
                    .any(|e| e.text.trim() == "service ethernet");
                if ethernet {
                    return Some(format!(
                        "% Instance ID {id} has 'service ethernet' enabled.\n L2 and L3 services \
                         are exclusive, remove L2 service first."
                    ));
                }
            }
            // a LISP database mapping naming a locator set `router lisp` lacks, and a PETR with a
            // priority beside one without (the lab C9200L-48T-4G, 17.6.4 and 17.15.6; one without
            // beside one with is inferred)
            (
                "config-lisp-inst-srv-eth" | "config-lisp-inst-srv-ipv4",
                ["database-mapping", rest @ ..],
            ) => {
                if let Some(name) = rest.iter().skip_while(|w| **w != "locator-set").nth(1)
                    && !holds(&format!("locator-set {name}"))
                {
                    return Some(format!("% No locator-set named '{name}'."));
                }
            }
            ("config-lisp-srv-ipv4", ["use-petr", _, rest @ ..])
                if block.is_some_and(|b| {
                    b.lines.iter().any(|e| {
                        e.text.trim().strip_prefix("use-petr ").is_some_and(|t| {
                            t.split_whitespace().any(|w| w == "priority") == rest.is_empty()
                        })
                    })
                }) =>
            {
                return Some(
                    "% Cannot configure use-petr entries with and without a priority and weight \
                     specification at the same time."
                        .into(),
                );
            }
            ("config", ["ipv6", "router", "ospf", n]) if named("router ospfv3 ", n) => {
                return Some(format!(
                    "% OSPFv3: Use \"router ospfv3 {n}\" to configure this process\n% Failed to \
                     create routing protocol ospf"
                ));
            }
            ("config", ["snmp-server", "host", _, "vrf", name, ..]) if !vrf(name) => {
                return Some(format!("% VRF name {name} does not exist"));
            }
            // a classic EIGRP process's IPv4 family in a VRF the switch does not hold, the name
            // anywhere in the line (the lab C9200L-48T-4G, 17.15.6, at both levels)
            ("config-router", ["address-family", "ipv4", rest @ ..])
                if header
                    .strip_prefix("router eigrp ")
                    .is_some_and(|n| n.parse::<u32>().is_ok())
                    && rest.contains(&"vrf") =>
            {
                if let Some(name) = rest.iter().skip_while(|w| **w != "vrf").nth(1)
                    && !vrf(name)
                {
                    return Some(format!(
                        "%VRF {name} does not exist or is not enabled for IPv4"
                    ));
                }
            }
            ("config-router", ["address-family", "ipv4" | "ipv6", .., "vrf", name])
                if !vrf(name) =>
            {
                if header.starts_with("router bgp ") {
                    return Some(format!("% VRF {name} does not exist."));
                }
                if header.starts_with("router ospfv3 ") {
                    return Some(format!("% VRF {name} does not exist"));
                }
            }
            // a named EIGRP instance keeps the AS its address family first took, and refuses one
            // another named instance's family runs (the lab C9300-24T and C9200L-48T-4G, 17.6.4)
            ("config-router", ["address-family", ..])
                if header
                    .strip_prefix("router eigrp ")
                    .is_some_and(|n| n.parse::<u32>().is_err()) =>
            {
                if let Some((family, asn)) = eigrp_family(&words) {
                    let held = |b: &crate::device::Block| {
                        b.lines
                            .iter()
                            .filter_map(|e| {
                                eigrp_family(&e.text.split_whitespace().collect::<Vec<_>>())
                            })
                            .find(|(f, _)| *f == family)
                            .map(|(_, n)| n.to_string())
                    };
                    if let Some(old) = block.and_then(held).filter(|old| old != asn) {
                        return Some(format!(
                            "Changing from AS({old}) to AS({asn}) is not allowed"
                        ));
                    }
                    let other = self.device.blocks.iter().find_map(|b| {
                        let name = b.header.strip_prefix("router eigrp ")?;
                        (b.header != header
                            && name.parse::<u32>().is_err()
                            && held(b).is_some_and(|n| n == asn))
                        .then_some(name)
                    });
                    if let Some(name) = other {
                        return Some(format!("%ERROR: AS({asn}) in use by VR({name})"));
                    }
                    // nor the AS a classic `router eigrp <asn>` runs, in the global IPv4 table (the
                    // lab C9200L-48T-4G, 17.15.6)
                    if matches!(family, ("ipv4", _, None))
                        && self
                            .device
                            .blocks
                            .iter()
                            .any(|b| b.header == format!("router eigrp {asn}"))
                    {
                        return Some(format!("%ERROR: AS({asn}) in use by classic router"));
                    }
                }
            }
            // a port's flow monitor holding no record, and a monitor's second record, on the C9200
            // images (the lab C9200L-48T-4G, 17.6.4 and 17.15.6; the C9300 refused both names,
            // `offers`)
            (m, ["ip" | "ipv6", "flow", "monitor", name, ..])
                if m.starts_with("config-if")
                    && self.runs_any(crate::context::C9200_LAB)
                    && self.device.blocks.iter().any(|b| {
                        b.header.strip_prefix("flow monitor ") == Some(*name)
                            && !b.lines.iter().any(|e| e.text.trim().starts_with("record "))
                    }) =>
            {
                return Some(
                    "% Flow Monitor: Failed to add monitor to interface: The monitor does not \
                     have a valid record"
                        .into(),
                );
            }
            ("config-flow-monitor", ["record", name])
                if self.runs_any(crate::context::C9200_LAB)
                    && block.is_some_and(|b| {
                        b.lines.iter().any(|e| {
                            e.text
                                .trim()
                                .strip_prefix("record ")
                                .is_some_and(|held| held != *name)
                        })
                    }) =>
            {
                return Some(
                    "% Flow Monitor: Failed to set record: Already there is an existing record \
                     configured for this monitor"
                        .into(),
                );
            }
            ("config-router", ["neighbor", _, "inherit", "peer-session", name])
                if header.starts_with("router bgp ")
                    && !holds(&format!("template peer-session {name}")) =>
            {
                return Some("%Template not found. Please create a template first.".into());
            }
            ("config-router", ["neighbor", _, kw, rest @ ..])
                if header.starts_with("router bgp ")
                    && (BGP_POLICY.contains(kw)
                        || *kw == "inherit" && rest.first() == Some(&"peer-policy"))
                    && block.is_some_and(|b| {
                        b.lines
                            .iter()
                            .any(|e| e.text.trim_start().starts_with("address-family "))
                    }) =>
            {
                return Some("% Policy commands not allowed without an address family".into());
            }
            // a link-local peer belongs to an address family; the switch's own address is no peer
            ("config-router", ["neighbor", peer, "remote-as", ..])
                if header.starts_with("router bgp ") && link_local(peer) =>
            {
                return Some("% Link local peer outside address-family scope".into());
            }
            ("config-router", ["neighbor", peer, "remote-as", ..])
                if header.starts_with("router bgp ")
                    && self.device.blocks.iter().any(|b| {
                        b.header.starts_with("interface ")
                            && b.lines.iter().any(|e| {
                                e.text
                                    .trim()
                                    .strip_prefix("ip address ")
                                    .and_then(|a| a.split_whitespace().next())
                                    == Some(*peer)
                            })
                    }) =>
            {
                return Some("% Cannot configure the local system as neighbor".into());
            }
            // any other neighbor line before the neighbor has a remote AS or a peer group
            ("config-router", ["neighbor", peer, kw, ..])
                if header.starts_with("router bgp ")
                    && !["remote-as", "peer-group", "inherit"].contains(kw)
                    && !peered(peer) =>
            {
                return Some("% Specify remote-as or peer-group commands first".into());
            }
            ("config", ["monitor", "session", n, rest @ ..]) => {
                // the interfaces the line names, short as the session's lines hold them
                let named: Vec<String> = p
                    .items
                    .iter()
                    .filter_map(|it| match it {
                        crate::cli::Item::Arg { value, .. } => Some(value),
                        _ => None,
                    })
                    .flat_map(|v| v.split(','))
                    .map(|v| v.split(" - ").next().unwrap_or(v).trim())
                    .filter(|v| crate::cli::interface_name(v, None).is_some())
                    .map(show::short)
                    .collect();
                return span_refusal(&self.device, n, rest, &named, &vlans());
            }
            ("config-vlan", ["private-vlan", "association", ..]) => {
                let primary: u32 = header.strip_prefix("vlan ")?.parse().ok()?;
                let list = words[2..].join("");
                let list = ["add", "remove"]
                    .iter()
                    .find_map(|k| list.strip_prefix(k))
                    .unwrap_or(&list);
                return no_private_association(primary, list, &vlans());
            }
            (
                "config-if",
                [
                    "switchport",
                    "private-vlan",
                    "mapping" | "association",
                    "trunk",
                    primary,
                    list @ ..,
                ]
                | [
                    "switchport",
                    "private-vlan",
                    "host-association" | "mapping",
                    primary,
                    list @ ..,
                ],
            ) => {
                let list = list.join("");
                let list = ["add", "remove"]
                    .iter()
                    .find_map(|k| list.strip_prefix(k))
                    .unwrap_or(&list);
                return no_private_association(primary.parse().ok()?, list, &vlans());
            }
            ("config-vlan", ["private-vlan", "isolated" | "community"]) => {
                let id = header.strip_prefix("vlan ")?;
                let svi = self
                    .device
                    .blocks
                    .iter()
                    .find(|b| b.header == format!("interface Vlan{id}"))?;
                let down = svi.lines.iter().any(|e| e.text.trim() == "shutdown");
                return (!down).then(|| {
                    format!(
                        "%Command rejected: invalid private vlan type assignment. VLAN {id}'s \
                         interface is not administratively down and cannot be set to secondary VLAN."
                    )
                });
            }
            ("config", ["interface", ..]) => {
                let pim = pim_tunnels(&self.device);
                for h in headers(p) {
                    let n = h
                        .strip_prefix("interface Tunnel")
                        .and_then(|n| n.parse::<u32>().ok());
                    if let Some(n) = n.filter(|n| pim.contains(n)) {
                        return Some(format!(
                            "%Tunnel{n} used by PIM for Registering, configuration not allowed"
                        ));
                    }
                }
            }
            _ => {}
        }
        match (mode, words.as_slice()) {
            ("config", ["router", "bgp", asn]) => {
                let running = self.device.blocks.iter().find_map(|b| {
                    b.header
                        .strip_prefix("router bgp ")
                        .filter(|n| !n.contains(' '))
                })?;
                (running != *asn).then(|| format!("BGP is already running; AS is {running}"))
            }
            ("config-pmap", ["class", name]) if !name.eq_ignore_ascii_case("class-default") => {
                let known = self.device.blocks.iter().any(|b| {
                    b.header.starts_with("class-map ")
                        && b.header.split_whitespace().last() == Some(*name)
                });
                (!known).then(|| format!("class map {name} not configured"))
            }
            ("config", ["service-export", "mdns-sd", "controller", name]) => {
                let running =
                    self.device.blocks.iter().find_map(|b| {
                        b.header.strip_prefix("service-export mdns-sd controller ")
                    })?;
                (running != *name).then(|| "Only one Controller is allowed\n\n".to_string())
            }
            _ => None,
        }
    }

    /// What IOS XE warns of for a line it takes, judged before the line runs (the lab C9300-24T,
    /// 17.6.4): an MKA policy or a control policy map opened again, an HSRP address outside the
    /// interface's subnets, a PortFast, OSPF reference bandwidth or MACsec cipher change, an MLD
    /// static group without multicast routing, an automatic DHCPv6 pool, `auto qos global compact`,
    /// a default SGT mapping without a default route, a civic location holding no attributes, and
    /// the MST region shown outside MST.
    fn taken_with_warning(&self, mode: &str, p: &Parse) -> Option<String> {
        if p.negated || self.device.stack.classic() {
            return None;
        }
        let canonical = p.canonical();
        let words: Vec<&str> = canonical.split_whitespace().collect();
        let exists = || self.device.blocks.iter().any(|b| b.header == canonical);
        // the interface the line is typed into, and whether it holds the line already
        let block = self
            .frames
            .iter()
            .rev()
            .find(|f| f.mode == mode)
            .and_then(|f| f.blocks.first())
            .and_then(|&b| self.device.blocks.get(b));
        let again = block.is_some_and(|b| b.lines.iter().any(|e| e.text.trim() == canonical));
        match (mode, words.as_slice()) {
            ("config", ["mka", "policy", name]) if exists() => Some(format!(
                "\n% MKA Policy \"{name}\" may have associated active MKA Sessions.\n\n  Changes to \
                 MKA Policy \"{name}\" values\n  will cause all associated active MKA Sessions to be \
                 cleared.\n\n"
            )),
            ("config", ["policy-map", "type", "control", "subscriber", name]) if exists() => {
                Some(format!(
                    "%Warning, modifying policy-map \"{name}\" may impact sessions hosted on ports \
                     for which \"{name}\" has already been applied."
                ))
            }
            ("config-if", ["standby", .., "ip", address]) => {
                let a = v4(address)?;
                let subnets: Vec<(u32, u32)> = block?
                    .lines
                    .iter()
                    .filter_map(|e| ip_address(&e.text))
                    .collect();
                // said when the address changes, not when the line is typed again
                (!again && !subnets.is_empty() && !subnets.iter().any(|&(n, m)| a & m == n & m))
                    .then(|| "% Warning: address is not within a subnet on this interface".into())
            }
            // printed before the region's configuration whatever the mode
            ("exec" | "privileged", ["show", "spanning-tree", "mst", "configuration", ..])
                if !self
                    .device
                    .global
                    .iter()
                    .any(|e| e.text == "spanning-tree mode mst") =>
            {
                Some("% Switch is not in mst mode".into())
            }
            ("config-router", ["auto-cost", "reference-bandwidth", _])
                if !again && block.is_some_and(|b| b.header.starts_with("router ospf ")) =>
            {
                Some(
                    "% OSPF: Reference bandwidth is changed. \n        Please ensure reference \
                     bandwidth is consistent across all routers."
                        .into(),
                )
            }
            ("config-mka-policy", ["macsec-cipher-suite", ..]) if !again => Some(
                "\n% If cipher currently being used is not configured, new SAK will be generated\n\n"
                    .into(),
            ),
            ("config-if", ["ipv6", "mld", "static-group", ..])
                if !self
                    .device
                    .global
                    .iter()
                    .any(|e| e.text.starts_with("ipv6 multicast-routing")) =>
            {
                Some("%Warning: Enable MLD router-side for this command to be effective".into())
            }
            ("config-if", ["ipv6", "dhcp", "server", "automatic", ..]) => {
                Some("% Warning: Pool  not configured globally - configuring anyway.".into())
            }
            // a named pool the configuration lacks (the lab C9200L-48T-4G, 17.6.4)
            ("config-if", ["ipv6", "dhcp", "server", name, ..])
                if !self
                    .device
                    .blocks
                    .iter()
                    .any(|b| b.header == format!("ipv6 dhcp pool {name}")) =>
            {
                Some(format!(
                    "% Warning: Pool {name} not configured globally - configuring anyway."
                ))
            }
            ("config", ["auto", "qos", "global", "compact"]) => Some(
                "%Warning: This command will reset all autoqos generated configurations".into(),
            ),
            // the room left for a civic location's attributes: all of it while it holds none
            ("config", ["location", "civic-location", "identifier", _])
                if !self
                    .device
                    .blocks
                    .iter()
                    .any(|b| b.header == canonical && !b.lines.is_empty()) =>
            {
                Some("% 250 bytes available to configure civic address attributes".into())
            }
            ("config", ["cts", "role-based", "sgt-map", "0.0.0.0/0", ..])
                if !self
                    .device
                    .global
                    .iter()
                    .any(|e| e.text.starts_with("ip route 0.0.0.0 0.0.0.0 ")) =>
            {
                Some(
                    "%Please ensure default route is created using ip route 0.0.0.0 command"
                        .into(),
                )
            }
            // said when PortFast changes, not when the line is typed again
            ("config-if", ["spanning-tree", "portfast", rest @ ..])
                if !again && matches!(rest, [] | ["trunk"]) =>
            {
                let mut said = "%Warning: portfast should only be enabled on ports connected to a \
                                single\n host. Connecting hubs, concentrators, switches, bridges, \
                                etc... to this\n interface  when portfast is enabled, can cause \
                                temporary bridging loops.\n Use with CAUTION\n\n"
                    .to_string();
                if rest.is_empty() {
                    let name = block?.header.strip_prefix("interface ")?;
                    said += &format!(
                        "%Portfast has been configured on {name} but will only\n have effect \
                         when the interface is in a non-trunking mode."
                    );
                }
                Some(said)
            }
            _ => None,
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

    /// A line of the public key being typed after `key-string`: IOS reads every line as the key's
    /// text, `end` and `configure terminal` included, until `exit`, which leaves the key as the
    /// `key-hash` it makes or says it could not read it (the lab C9300, 17.6.4).
    fn pubkey_data(&mut self, line: &str) -> Option<(Vec<String>, Result)> {
        self.pubkey.as_ref()?;
        if self.mode() != "conf-ssh-pubkey-data" {
            self.pubkey = None;
            return None;
        }
        let t = line.trim();
        if t.is_empty() {
            return Some((vec![], Result::Ok));
        }
        let parsed = self.parse("conf-ssh-pubkey-data", t);
        if matches!(&parsed, Outcome::Ok(p) if self.engine.grammar.commands[p.command].name == "exit")
        {
            self.frames.pop();
            // no text keeps no key and says nothing (the lab C9300, 17.6.4)
            let text = self.pubkey.take().unwrap_or_default();
            if text.is_empty() {
                return Some((vec![], Result::Ok));
            }
            let Some(key) = crate::pubkey::key_hash(&text) else {
                return Some((
                    vec!["%SSH: Failed to decode the Key Value".into()],
                    Result::Ok,
                ));
            };
            // the comment is kept whole, though a typed `key-hash` takes one word
            let mode = self.mode().to_string();
            let line = format!("key-hash {} {}", key.kind, key.hex);
            if let Outcome::Ok(mut p) = self.parse(&mode, &line) {
                if let Some(c) = &key.comment
                    && let Some(Item::Arg { value, .. }) = p.items.last_mut()
                {
                    value.push(' ');
                    value.push_str(c);
                }
                self.write(&p);
            }
            return Some((vec![], Result::Ok));
        }
        if let Some(text) = self.pubkey.as_mut() {
            text.push_str(t);
        }
        let block = self.block_header();
        let key = match &parsed {
            Outcome::Ok(p) => crate::device::key(p),
            _ => String::new(),
        };
        self.accepted.push(Accepted {
            block,
            line: t.to_string(),
            key: Some(key),
        });
        Some((vec![], Result::Ok))
    }

    /// The rest of `line` from `at`, as typed, run at the privileged prompt (`do`, `do-exec`); a
    /// refused word's caret stands under it after the configuration prompt and the words before it.
    fn run_exec(&mut self, prompt_len: usize, line: &str, at: usize) -> (Vec<String>, Result) {
        let lead = prompt_len + line[..at].chars().count();
        let saved = std::mem::replace(&mut self.frames, vec![Frame::top("privileged")]);
        self.lead = Some(lead);
        let out = self.typed(&line[at..]);
        self.frames = saved;
        out
    }

    /// Where the command of a configuration line `do-exec <command>` starts, `do-exec` typed whole or
    /// abbreviated (`cli::names_do_exec`): IOS XE 17 and the 2960's 15.0 list `do-exec` in
    /// configuration modes and hide `do`.
    fn do_exec_at(&self, line: &str) -> Option<usize> {
        if !cli::is_config(self.mode()) {
            return None;
        }
        let toks = cli::tokens(line);
        let first = toks.first()?;
        cli::names_do_exec(&first.text).then(|| toks.get(1).map_or(line.len(), |t| t.at))
    }

    fn run(&mut self, mode: &str, line: &str, p: &Parse) -> (Vec<String>, Result) {
        self.raw = line.to_string();
        let cmd = &self.engine.grammar.commands[p.command];
        let name = cmd.name.as_str();
        let mut enters = self.engine.enters(p).map(str::to_string);
        // a subinterface opens its own mode, where `encapsulation dot1Q` runs (the lab C9300-24T,
        // 17.6.4: `interface GigabitEthernet1/0/7.10`)
        if enters.as_deref() == Some("config-if")
            && headers(p)
                .first()
                .is_some_and(|h| h.starts_with("interface ") && h.contains('.'))
        {
            enters = Some("config-subif".into());
        }
        match name {
            "do" => {
                let at = cli::tokens(line).get(1).map_or(line.len(), |t| t.at);
                return self.run_exec(self.prompt().chars().count(), line, at);
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
            // IOS XE says what the smart licensing address means when it is set (the lab C9300,
            // 17.6.4, during a configuration replace)
            "contact-email-addr"
                if !p.negated
                    && !self.device.stack.classic()
                    && p.canonical() == "contact-email-addr sch-smart-licensing@cisco.com" =>
            {
                self.write(p);
                return (
                    vec![crate::show::CALL_HOME_NOTE[1][2..].replace("the email", "The email")],
                    Result::Ok,
                );
            }
            // a hash typed in a pubkey chain prints in upper case, and on IOS XE with a space after
            // it where no comment follows (the lab C9300, 17.6.4); one a key's lines make has none,
            // and classic IOS, unmeasured, is given none
            "key-hash" if !p.negated && cli::is_config(mode) => {
                let mut q = p.clone();
                let spaceless = self.device.stack.classic()
                    || q.items
                        .iter()
                        .any(|i| matches!(i, Item::Arg { name, .. } if name == "key-comment"));
                for i in &mut q.items {
                    if let Item::Arg { name, value, .. } = i
                        && name == "key-name"
                    {
                        *value = value.to_uppercase();
                        if !spaceless {
                            value.push(' ');
                        }
                    }
                }
                self.write(&q);
                return (vec![], Result::Ok);
            }
            "copy running-config startup-config" => {
                self.asking = true;
                return (vec![], Result::Ok);
            }
            // the boot level lives in the license store, not the configuration: typed at the console
            // it changes nothing until saved and booted, and says so (the lab C9300, 17.6.4; other IOS
            // XE releases inferred); a replayed configuration's line is the level the switch booted at
            _ if cli::is_config(mode)
                && !self.redacted
                && !p.negated
                && !self.device.stack.classic()
                && p.canonical().starts_with("license boot level ") =>
            {
                return (
                    vec![
                        "% use 'write' command to make license boot config take effect on next boot"
                            .into(),
                    ],
                    Result::Ok,
                );
            }
            _ => {}
        }
        if !cli::is_config(mode) {
            // EXEC: show commands print; the rest run silently
            return self.said(mode, line, p, show::render(self, name, p, line));
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
            && headers(p).first().is_some_and(|h| {
                self.frames[3]
                    .blocks
                    .first()
                    .is_some_and(|&b| crate::context::reopens(&self.device.blocks[b].header, h))
            })
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
                    // classic IOS takes the MST region back to its defaults first, its header kept, and
                    // drops it at a second `no` (the lab C2960G, 15.0(2)SE11); the
                    // lab C9300 (17.6.4) dropped it at the first
                    if header == "spanning-tree mst configuration"
                        && self.device.stack.classic()
                        && let Some(b) = self.device.blocks.iter_mut().find(|b| b.header == header)
                        && !b.lines.is_empty()
                    {
                        b.lines.clear();
                        continue;
                    }
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
                    entered: line.trim().to_string(),
                });
            }
            Some(sub)
                if cli::is_config(sub)
                    && (top.mode.starts_with("config-if") || top.mode == "config-subif")
                    && top.prefix.is_empty()
                    && self.closes(sub, &headers(p)[0]).is_some() =>
            {
                // a submode of an interface with an `exit-` line opens a section in the interface,
                // whose lines and `exit-` line print two spaces in (the lab C9300-24T, 17.6.4:
                // `  address 10.99.20.254 primary` and `  exit-vrrp` under `vrrp 20 address-family
                // ipv4`, `  exit-vrrp` alone under an empty one)
                let header = headers(p).remove(0);
                let close = self.closes(sub, &header).unwrap_or_default();
                for &b in &top.blocks {
                    let lines = &mut self.device.blocks[b].lines;
                    if !lines.iter().any(|e| e.key == header) {
                        lines.push(Entry {
                            key: header.clone(),
                            text: header.clone(),
                        });
                        lines.push(Entry {
                            key: crate::device::closing(&header),
                            text: format!(" {close}"),
                        });
                    }
                }
                self.frames.push(Frame {
                    mode: sub.into(),
                    blocks: top.blocks.clone(),
                    prefix: " ".into(),
                    section: Some(header),
                    entered: line.trim().to_string(),
                });
            }
            Some(sub)
                if cli::is_config(sub)
                    && (top.mode.starts_with("config-if") || top.mode == "config-subif") =>
            {
                // a submode of an interface writes into the interface itself (`vrf forwarding`)
                self.write(p);
                self.frames.push(Frame {
                    mode: sub.into(),
                    blocks: top.blocks.clone(),
                    prefix: top.prefix.clone(),
                    section: None,
                    entered: line.trim().to_string(),
                });
            }
            Some(sub) if cli::is_config(sub) => {
                // a submode inside a block opens an indented section (address-family), which IOS
                // closes with its `exit-` line however the mode is left (c9200 17.12 capture 1's empty ones)
                let header = headers(p).remove(0);
                let close = self.closes(sub, &header);
                // a public key's lines make the `key-hash` IOS prints in place of `key-string`
                let key_string = sub == "conf-ssh-pubkey-data";
                if key_string {
                    self.pubkey = Some(String::new());
                }
                // a section inside a section is named by the enclosing one too, and goes at its
                // end: an EIGRP address family's `topology base` and its service family's are two
                // (the lab C9300-24T, 17.6.4)
                let key = match &top.section {
                    Some(outer) => format!("{}{header} @{outer}", top.prefix),
                    None => format!("{}{header}", top.prefix),
                };
                for &b in top.blocks.iter().filter(|_| !key_string) {
                    let lines = &mut self.device.blocks[b].lines;
                    if !lines.iter().any(|e| e.key == key) {
                        let mut new = vec![Entry {
                            key: key.clone(),
                            text: format!("{}{header}", top.prefix),
                        }];
                        if let Some(c) = &close {
                            new.push(Entry {
                                key: crate::device::closing(&key),
                                text: format!("{}{c}", top.prefix),
                            });
                        }
                        let at = top.section.as_deref().map_or(lines.len(), |outer| {
                            section_span(lines, outer, &top.prefix).1
                        });
                        lines.splice(at..at, new);
                    }
                }
                self.frames.push(Frame {
                    mode: sub.into(),
                    blocks: top.blocks.clone(),
                    prefix: format!("{} ", top.prefix),
                    section: Some(key),
                    entered: line.trim().to_string(),
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
                ["Loopback", "Null", "Port-channel", "Tunnel", "Vlan"].contains(full)
                    || physical.iter().any(|p| cli::of_type(p, full))
            })
            .map(str::to_string)
            .collect()
    }

    /// A held `?` answer as this session's switch prints it: an interface-type list (one with a
    /// physical-type row) offers the stack's physical types, not the crawled switch's, the logical
    /// ones as held; at `>`, no row for a command `privilege exec level` raised above it.
    fn as_this_switch(&self, mode: &str, text: &str, answer: Vec<String>) -> Vec<String> {
        let rows = crate::live::keywords(&answer);
        let physical = |w: &str| cli::PHYSICAL_TYPES.iter().any(|(t, _)| *t == w);
        let types = rows.iter().any(|w| physical(w));
        let ports = self.device.stack.physical();
        let fitted = |w: &str| ports.iter().any(|p| cli::of_type(p, w));
        let add: Vec<(&str, &str)> = if types {
            cli::PHYSICAL_TYPES
                .iter()
                .filter(|(t, _)| fitted(t) && !rows.iter().any(|w| w == t))
                .copied()
                .collect()
        } else {
            vec![]
        };
        // the words before the row's keyword, each a start of a raised command's word
        let mut before: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
        if !text.is_empty() && !text.ends_with(char::is_whitespace) {
            before.pop();
        }
        let raised = if mode == "exec" {
            self.raised()
        } else {
            vec![]
        };
        let above = |w: &str| {
            raised.iter().any(|r| {
                r.len() == before.len() + 1
                    && r.iter()
                        .zip(&before)
                        .all(|(a, b)| a.starts_with(b.as_str()))
                    && r[before.len()] == w.to_lowercase()
            })
        };
        let drop = |w: &str| (types && physical(w) && !fitted(w)) || above(w);
        if add.is_empty() && !rows.iter().any(|w| drop(w)) {
            return answer;
        }
        crate::live::relaid(&answer, |w| !drop(w), &add)
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
                    ArgKind::Hex { min, max } => format!("<{min:X}-{max:X}>"),
                    ArgKind::Ipv4Prefix => "A.B.C.D/nn".into(),
                    ArgKind::Ipv6Prefix => "X:X:X:X::X/<0-128>".into(),
                    ArgKind::InterfaceOf { of } => {
                        return vec![Offer {
                            shown: of.clone(),
                            help: String::new(),
                            kind: Offered::Type,
                        }];
                    }
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
    /// keywords that word begins. `do-exec ?` is what the crawl held for it, which leaves out the
    /// privileged prompt's commands that change mode.
    pub fn help(&self, text: &str) -> Vec<String> {
        let answer = match self.after_do(text) {
            Some("") => self
                .held_answer(self.mode(), self.context().as_deref(), "do-exec ")
                .unwrap_or_else(|| self.help_in("privileged", Some(""), "")),
            Some(rest) => self.help_in("privileged", Some(""), rest),
            None => self.help_in(self.mode(), self.context().as_deref(), text),
        };
        crate::live::refusal_for(text, answer)
    }

    /// The text after a configuration line's `do` or `do-exec` (whole or abbreviated) and a space,
    /// which EXEC reads: `?` and Tab answer it as the privileged prompt would.
    fn after_do<'t>(&self, text: &'t str) -> Option<&'t str> {
        let toks = cli::tokens(text);
        let first = toks.first()?;
        let spaced = text.len() > first.at + first.text.len();
        let exec = first.text.eq_ignore_ascii_case("do") || cli::names_do_exec(&first.text);
        (cli::is_config(self.mode()) && exec && spaced)
            .then(|| &text[toks.get(1).map_or(text.len(), |t| t.at)..])
    }

    /// Whether a line a submode refused, which the enclosing frame `i` reads past its first word,
    /// leaves for that frame's mode, as the lab C9300 (17.6.4) left: global configuration's
    /// `interface`, `router` and `route-map` (`router rip cisco` in config-router-af, `route-map
    /// ROUTE in` in config-route-map), and an enclosing submode's `address-family` that opened the
    /// submode above it (`address-family ipv4 unicast vrf inside` in config-vrf-af, for
    /// (config-vrf)#). It refused `policy-map class vlan100` in config-pmap, `vlan 10
    /// guest-interface 2` in config-vlan, `ipv6 access-list list2 permit any any` in
    /// config-ipv6-acl and a `police` line in config-pmap-c-police as the submode and stayed.
    fn leaves(&self, i: usize, line: &str) -> bool {
        let Some(w) = line.split_whitespace().next().map(str::to_lowercase) else {
            return false;
        };
        if w.len() < 2 {
            return false;
        }
        if self.frames[i].mode == "config" {
            return ["interface", "router", "route-map"]
                .iter()
                .any(|k| k.starts_with(&w));
        }
        "address-family".starts_with(&w)
            && self
                .frames
                .get(i + 1)
                .and_then(|f| f.entered.split_whitespace().next())
                .is_some_and(|e| "address-family".starts_with(&e.to_lowercase()))
    }

    /// A line read in the mode of frame `i`: as the books read it, but for the earliest refusal of
    /// a word the session's licence level leaves out (`level_refusal`) or one a real switch of this
    /// image refused (`held_refusal`) before the books' own.
    fn read_at(&self, i: usize, mode: &str, line: &str) -> Outcome {
        // what the switch of this image did with this line on Enter at the session's level: its
        // refusal, else its answer, over the books and the level's inferred gate
        if let Some(o) = self.measured_refusal(mode, line) {
            return o;
        }
        if let Some(p) = self.measured(mode, line) {
            let books = self.books(mode, line);
            return if matches!(books, Outcome::Ok(_)) {
                books
            } else {
                Outcome::Ok(p)
            };
        }
        let o = self.read_unmeasured(i, mode, line);
        // a line refused there that a form of the level it is gated against takes, its words
        // carried at both levels by this image's own crawls
        if !matches!(o, Outcome::Ok(_))
            && mode == "privileged"
            && let Some((crate::swept::Carriage::Carried, p)) = self.carriage(line)
        {
            let books = self.books(mode, line);
            return if matches!(books, Outcome::Ok(_)) {
                books
            } else {
                Outcome::Ok(p)
            };
        }
        o
    }

    /// How this image's own crawls at the session's gated level and at the level it is gated
    /// against read a privileged line that a form measured at the level above takes
    /// (`live::Held::carriage`), with that form's parse: the form's keywords whole, its values as
    /// read.
    fn carriage(&self, line: &str) -> Option<(crate::swept::Carriage, Parse)> {
        let level = self.level()?;
        let above = crate::license::gated_by(&level)?;
        let Outcome::Ok(p) = self.engine.parse_measured("privileged", line, above) else {
            return None;
        };
        let words: Vec<String> = p
            .items
            .iter()
            .flat_map(|it| match it {
                Item::Kw { kw, .. } => vec![kw.clone()],
                Item::Arg { value, .. } => value.split_whitespace().map(str::to_string).collect(),
            })
            .collect();
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        let c = self.engine.held.carriage(&level, above, &words)?;
        Some((c, p))
    }

    /// `read_at` past the measured lines: the unlisted words, the books, the licence gate and the
    /// held refusals.
    fn read_unmeasured(&self, i: usize, mode: &str, line: &str) -> Outcome {
        // a word the level takes though no book or answer lists it: its refusal stands
        if let Some(o) = self.unlisted_refusal(i, mode, line) {
            return o;
        }
        let mut o = self.books(mode, line);
        let level = self.level_refusal(i, mode, line);
        // the level's caret inside a word the books refuse at its start stands (`refused_type`)
        if let (Some(Outcome::Invalid(l)), Outcome::Invalid(b)) = (&level, &o)
            && l > b
            && token_at(line, *l) == token_at(line, *b)
        {
            o = Outcome::Invalid(*l);
        }
        // a held `% Unrecognized command` refuses a line the books take; a refusal of theirs keeps
        // its caret
        let taken = matches!(o, Outcome::Ok(_));
        let held = self
            .held_refusal(i, mode, line)
            .filter(|(_, unrecognized)| taken || !unrecognized)
            .map(|(o, _)| o);
        for refused in [level, held].into_iter().flatten() {
            if reach(&refused, line) < reach(&o, line) {
                o = refused;
            }
        }
        o
    }

    /// Where a line is refused for a word the session's licence level leaves out of the image, in the
    /// mode and context of frame `i`, EXEC included: the gate's crawl at the level answered `?` before
    /// the word without it and the crawl at the level above offered it (`live::Held::gate_answers`).
    /// Reading stops at a word neither offers, at a `LINE` row and at a held refusal; a line the
    /// level takes though its crawl offers it no word (`license::taken`) is not read. An interface
    /// type the level lists and refuses (`license::refused_types`) is refused past its longest
    /// start another row begins.
    fn level_refusal(&self, i: usize, mode: &str, line: &str) -> Option<Outcome> {
        let level = self.level();
        if !self.engine.held.gated(level.as_deref()) {
            return None;
        }
        if let Some(o) = self.unlisted_refusal(i, mode, line) {
            return Some(o);
        }
        if let Some(l) = level.as_deref()
            && self.level_takes(i, l, mode, line)
        {
            return None;
        }
        let refused_types = crate::license::refused_types(level.as_deref().unwrap_or(""));
        let context = self.context_at(i)?;
        let toks = cli::tokens(line);
        for (k, t) in toks.iter().enumerate() {
            let text = match k {
                0 => String::new(),
                _ => format!("{} ", &line[..toks[k - 1].at + toks[k - 1].text.len()]),
            };
            let Some((at, above)) =
                self.engine
                    .held
                    .gate_answers(level.as_deref(), mode, Some(&context), &text, |l| {
                        self.running_has(l)
                    })
            else {
                continue;
            };
            if crate::live::is_refusal(at) || crate::live::is_refusal(above) {
                return None;
            }
            if let Some(past) = refused_type(at, &t.text, refused_types) {
                return Some(Outcome::Invalid(t.at + past));
            }
            if type_above_only(at, above, &t.text) {
                return Some(Outcome::Invalid(t.at));
            }
            match (best_fit(at, &t.text), best_fit(above, &t.text)) {
                (Fit::No, Fit::Word | Fit::Rest) => return Some(Outcome::Invalid(t.at)),
                (Fit::Word, _) => {}
                _ => return None,
            }
        }
        None
    }

    /// Whether `level` takes `line` in `mode` on this image though its crawl offers no such line
    /// (`license::taken_on`), read in the context of frame `i`.
    fn level_takes(&self, i: usize, level: &str, mode: &str, line: &str) -> bool {
        let book = crate::grammars::book_of(&self.engine.grammar.platform).unwrap_or("");
        crate::license::taken_on(
            (book, &self.engine.grammar.release),
            level,
            mode,
            line,
            |text| self.keywords_before(i, level, mode, text),
        )
    }

    /// The keywords offered after `text` in `mode` and the context of frame `i` at `level`: the held
    /// answer's, else the books'.
    fn keywords_before(&self, i: usize, level: &str, mode: &str, text: &str) -> Vec<String> {
        let held = self.context_at(i).and_then(|context| {
            self.engine
                .held
                .help_for(Some(level), mode, Some(&context), text, |l| {
                    self.running_has(l)
                })
                .filter(|a| !crate::live::is_refusal(a))
        });
        if let Some(a) = held {
            return crate::live::keywords(a);
        }
        let (next, _) = self.engine.next_at(mode, &cli::tokens(text), Some(level));
        next.into_iter()
            .filter_map(|n| match n {
                Next::Kw { kw, .. } => Some(kw),
                Next::Arg { .. } => None,
            })
            .collect()
    }

    /// A line whose first word the session's level takes in `mode` though no `?` lists it
    /// (`license::unlisted`), refused at the word after it; one whose first word the level refuses
    /// where its crawl judges nothing (`license::left_out`), refused at that word.
    fn unlisted_refusal(&self, i: usize, mode: &str, line: &str) -> Option<Outcome> {
        let level = self.level()?;
        if !self.engine.held.gated(Some(&level)) {
            return None;
        }
        let toks = cli::tokens(line);
        let first = toks.first()?;
        let offered = || self.keywords_before(i, &level, mode, "");
        // a unique abbreviation of the measured word stands for it; one beginning another word too
        // is ambiguous
        match crate::license::unlisted(&level, mode, &first.text, offered) {
            Some(false) => return Some(Outcome::Invalid(toks.get(1)?.at)),
            Some(true) => return Some(Outcome::Ambiguous),
            None => {}
        }
        match crate::license::left_out(&level, mode, &first.text, offered)? {
            false => Some(Outcome::Invalid(first.at)),
            true => Some(Outcome::Ambiguous),
        }
    }

    /// Where a configuration or privileged EXEC line was refused by a real switch of this image,
    /// read against what it answered for `?` after each whole word in the mode and context of frame
    /// `i` (its own answers, never a donor's): a word no row of the answer before it offers is
    /// invalid there, and a line whose answer after its last word holds no `<cr>` is incomplete. A
    /// word with no answer held before it is not read, unless the word before it is a value the
    /// crawl typed otherwise (`Held::crawled_value`), which reads the keyword after it as after the
    /// crawl's value; reading stops at a refusal held for the line so far and at a `LINE` row,
    /// which takes the rest. A privileged line is read past its first word. An interface type is left to the stack (`on_stack`), and on a routed port only the
    /// objects an answer offers are read: a physical port's context holds the crawled switchport's
    /// answers. A line this image's switch refused at the session's level where its crawl offers the
    /// words (`context::REFUSED`), or a unique abbreviation of it, is refused where it printed the
    /// caret; an ambiguous abbreviation is ambiguous. The flag says the refusal is a held
    /// `% Unrecognized command`.
    fn held_refusal(&self, i: usize, mode: &str, line: &str) -> Option<(Outcome, bool)> {
        if !cli::is_config(mode) && mode != "privileged" {
            return None;
        }
        let frame = self.frames.get(i)?;
        let routed = mode.starts_with("config-if")
            && frame.blocks.iter().any(|&b| {
                self.device.blocks[b]
                    .lines
                    .iter()
                    .any(|e| e.text.trim() == "no switchport")
            });
        // the crawled ports had no PoE (the lab C9300-24T's `power ?` lists only
        // efficient-ethernet), so a PoE port's `power` lines are left to the books
        let first = line.split_whitespace().next().unwrap_or("").to_lowercase();
        if first.len() >= 2
            && "power".starts_with(&first)
            && frame.blocks.iter().any(|&b| {
                let header = &self.device.blocks[b].header;
                header
                    .strip_prefix("interface ")
                    .is_some_and(|n| self.device.stack.poe(n).is_some())
            })
        {
            return None;
        }
        let level = self.level();
        let configured = |l: &str| self.running_has(l);
        if let Some(l) = level.as_deref()
            && self.engine.held.crawls.contains_key(l)
            && let Some(book) = crate::grammars::book_of(&self.engine.grammar.platform)
            && let Some(r) = crate::context::refused(
                (book, &self.engine.grammar.release),
                l,
                mode,
                line,
                configured,
                |text| self.keywords_before(i, l, mode, text),
            )
        {
            let o = match r {
                crate::context::Refused::At(at) => Outcome::Invalid(at),
                crate::context::Refused::Ambiguous => Outcome::Ambiguous,
            };
            return Some((o, false));
        }
        // `do` runs an exec line from any mode and `end` leaves any, and no submode's answer
        // need list them
        if ["do", "end"].contains(&first.as_str())
            || crate::context::unjudged(mode, line, configured)
            || level
                .as_deref()
                .is_some_and(|l| self.level_takes(i, l, mode, line))
        {
            return None;
        }
        let context = self.context_at(i)?;
        if mode == "config" && self.provisions_another_family(level.as_deref(), &context, line) {
            return None;
        }
        let held = &self.engine.held;
        let toks = cli::tokens(line);
        let mut before: Vec<&str> = vec![];
        // a privileged line's first word is the books': a crawl that asked `do-exec ?` and not the
        // prompt's own `?` holds an answer that leaves out the commands that change mode
        let first = usize::from(mode == "privileged");
        // a word the switch answered `?` after, where the answer before it said no object of the
        // kind was configured, is a keyword it hides there (`name` after `show flow exporter`)
        let answered_after = |k: usize| {
            let through = format!("{} ", &line[..toks[k].at + toks[k].text.len()]);
            held.help_for(level.as_deref(), mode, Some(&context), &through, configured)
                .is_some_and(|a| !crate::live::is_refusal(a))
        };
        // the kind of object the answer before the last word read listed, and whether that word was
        // the keyword before a name (`name` after `clear flow monitor`) rather than a name
        let mut listed: Option<(&str, bool)> = None;
        for k in first..=toks.len() {
            let after_listed = listed.take();
            let text = match k {
                0 => String::new(),
                _ => format!("{} ", &line[..toks[k - 1].at + toks[k - 1].text.len()]),
            };
            let Some(answer) =
                held.help_for(level.as_deref(), mode, Some(&context), &text, configured)
            else {
                // a value the crawl typed otherwise: the keyword after it is read as after the
                // crawl's own (`spanning-tree vlan 2-4 cost` as `spanning-tree vlan 10 cost`);
                // a number's bounds and the answers further on can depend on the value (`mpls
                // label range 100 1000`; MST 0 takes `root primary diameter`, instance 10 not)
                if k > first
                    && let Some(next) = toks.get(k)
                    && next.text.starts_with(|c: char| c.is_ascii_alphabetic())
                    && let Some(v) = held.crawled_value(
                        level.as_deref(),
                        mode,
                        Some(&context),
                        &text,
                        configured,
                    )
                {
                    let t = &toks[k - 1];
                    let swapped = format!("{}{v}{}", &line[..t.at], &line[t.at + t.text.len()..]);
                    let at = next.at - t.text.len() + v.len();
                    if let Some((Outcome::Invalid(x), u)) = self.held_refusal(i, mode, &swapped)
                        && x == at
                    {
                        return Some((Outcome::Invalid(next.at), u));
                    }
                    return None;
                }
                continue;
            };
            if crate::live::is_refusal(answer) {
                // a refusal after a name, or after the keyword before one, where the answer before
                // listed the objects configured: the crawl's switch held none by that name, and
                // this session's objects decide
                match after_listed {
                    Some((_, false)) => {
                        before.clear();
                        continue;
                    }
                    Some((kind, true)) if self.holds_kind(kind) => {
                        let Some(t) = toks.get(k) else {
                            return Some((Outcome::Incomplete, false));
                        };
                        if !self.offers(kind, &t.text, mode) {
                            return Some((Outcome::Invalid(t.at), false));
                        }
                        listed = Some((kind, false));
                        before.clear();
                        continue;
                    }
                    _ => {}
                }
                // `% Unrecognized command` after the last word as typed (no address placeholder):
                // the switch refused that word
                let last = k.checked_sub(1).map(|k| &toks[k]);
                let unrecognized = held.unrecognized_after(
                    level.as_deref(),
                    mode,
                    Some(&context),
                    &text,
                    configured,
                );
                return last
                    .filter(|t| {
                        !routed
                            && unrecognized
                            && crate::context::values(&t.text.to_lowercase())
                                == t.text.to_lowercase()
                    })
                    .map(|t| (Outcome::Invalid(t.at), true));
            }
            // `No monitors configured`, or the crawl's own `User defined` names: the answer offers
            // the objects configured, and this session's are its own; a keyword it offers beside
            // them (`netflow` after `record`) is read as any other
            let none = crate::live::configured_kind(answer);
            let kind = none.or_else(|| {
                let keyword = &toks.get(k.checked_sub(1)?)?.text;
                crate::live::defined_kind(answer, keyword)
            });
            if let Some(kind) = kind {
                let t = toks.get(k)?;
                let typed = t.text.to_lowercase();
                let keyword = crate::live::answer_rows(answer).iter().any(|&(w, what)| {
                    !what.eq_ignore_ascii_case("user defined")
                        && w.bytes().any(|b| b.is_ascii_lowercase())
                        && w.to_lowercase().starts_with(&typed)
                });
                if !keyword {
                    let named = self.offers(kind, &t.text, mode);
                    if !(none.is_some() && answered_after(k)) && !named {
                        return Some((Outcome::Invalid(t.at), false));
                    }
                    listed = Some((kind, !named));
                    before.clear();
                    continue;
                }
                listed = Some((kind, true));
            }
            if routed {
                before.clear();
                continue;
            }
            let rows = crate::live::answer_rows(answer);
            let Some(t) = toks.get(k) else {
                // an answer that offers again what the one before offered is a list's, which ends
                // anywhere
                let list =
                    !before.is_empty() && before.iter().all(|w| rows.iter().any(|(r, _)| r == w));
                return (!list && !rows.iter().any(|(w, _)| *w == "<cr>"))
                    .then_some((Outcome::Incomplete, false));
            };
            // an answer listing interface types by their names, as IOS capitalizes them
            let types = rows
                .iter()
                .filter(|(w, _)| {
                    cli::interface_types()
                        .iter()
                        .chain(cli::PHYSICAL_TYPES)
                        .any(|(full, _)| *full == *w)
                })
                .count()
                >= 2;
            // a file system's row fits a path after it, and no other word
            let typed = t.text.to_lowercase();
            match rows
                .iter()
                .map(|&(w, what)| match file_system(w) {
                    true => fit(w.starts_with(&typed) || typed.starts_with(w)),
                    false => fits_row(w, what, &t.text, types),
                })
                .max()
            {
                Some(Fit::Rest) => return None,
                Some(Fit::Word) => before = rows.iter().map(|(w, _)| *w).collect(),
                _ if self.names_configured(mode, &toks[..=k]) => before.clear(),
                _ => return Some((Outcome::Invalid(t.at), false)),
            }
        }
        None
    }

    /// Whether the last of a privileged line's words names an object this session configured of a
    /// kind `?` answers name (`live::NAMED_KINDS`), the words between the command and it starting
    /// the kind's keywords (`show flow record R` and a `flow record R`): a crawl asked with none
    /// configured lists no names.
    fn names_configured(&self, mode: &str, toks: &[cli::Tok]) -> bool {
        let Some((name, words)) = toks.split_last() else {
            return false;
        };
        let words = words.get(1..).unwrap_or_default();
        mode == "privileged"
            && !words.is_empty()
            && self.device.blocks.iter().any(|b| {
                let header: Vec<&str> = b.header.split_whitespace().collect();
                let kind = header
                    .split_last()
                    .map(|(_, k)| format!("{} ", k.join(" ")));
                kind.is_some_and(|k| crate::live::NAMED_KINDS.contains(&k.as_str()))
                    && header.len() == words.len() + 1
                    && header.last() == Some(&name.text.as_str())
                    && words
                        .iter()
                        .zip(&header)
                        .all(|(w, h)| h.to_lowercase().starts_with(&w.text.to_lowercase()))
            })
    }

    /// Whether the session runs the image (book, release).
    fn runs(&self, image: (&str, &str)) -> bool {
        crate::grammars::book_of(&self.engine.grammar.platform) == Some(image.0)
            && self.engine.grammar.release == image.1
    }

    /// Whether the session runs one of the images.
    fn runs_any(&self, images: &[(&str, &str)]) -> bool {
        images.iter().any(|&image| self.runs(image))
    }

    /// Whether `line` is `switch <n> provision ...` and the held answer after `provision` lists types
    /// of other families alone than this stack's: the types are those the crawled switch stacks
    /// with (the lab C9200L-48T-4G listed C9200L models alone), so that answer does not judge the
    /// line for a stack of another family.
    fn provisions_another_family(&self, level: Option<&str>, context: &str, line: &str) -> bool {
        let toks = cli::tokens(line);
        let [switch, n, provision, ..] = toks.as_slice() else {
            return false;
        };
        let starts = |t: &cli::Tok, k: &str| {
            t.text.len() >= 2 && k.starts_with(t.text.to_lowercase().as_str())
        };
        if !(starts(switch, "switch")
            && n.text.bytes().all(|b| b.is_ascii_digit())
            && starts(provision, "provision"))
        {
            return false;
        }
        let Some(family) = self.device.stack.family() else {
            return false;
        };
        let through = format!("switch {} provision ", n.text);
        let Some(answer) =
            self.engine
                .held
                .help_for(level, "config", Some(context), &through, |l| {
                    self.running_has(l)
                })
        else {
            return false;
        };
        let listed: Vec<crate::platform::Family> = crate::live::keywords(answer)
            .iter()
            .filter_map(|k| crate::platform::model(k))
            .map(|m| m.family)
            .collect();
        !listed.is_empty() && !listed.contains(&family)
    }

    /// Whether this session holds a block of the kind (`flow monitor `).
    fn holds_kind(&self, kind: &str) -> bool {
        self.device
            .blocks
            .iter()
            .any(|b| b.header.starts_with(kind))
    }

    /// Whether this session holds the block `kind name` where IOS offers it for `mode`. The lab
    /// C9300 (17.6.4) refused an empty record in a flow monitor and a monitor with none on a port,
    /// so on that image a flow record holds a field and a monitor named on an interface a record;
    /// the lab C9200L (17.6.4) read both names and answered with a message.
    fn offers(&self, kind: &str, name: &str, mode: &str) -> bool {
        let measured = self.runs(crate::context::C9300_17_6);
        self.device.blocks.iter().any(|b| {
            let holds = |p: &[&str]| {
                b.lines
                    .iter()
                    .any(|e| p.iter().any(|p| e.text.trim().starts_with(p)))
            };
            b.header.strip_prefix(kind) == Some(name)
                && match kind {
                    "flow record " if measured => holds(&["match ", "collect "]),
                    "flow monitor " if measured && mode.starts_with("config-if") => {
                        holds(&["record "])
                    }
                    _ => true,
                }
        })
    }

    /// The context of frame `i`'s mode (`context`).
    fn context_at(&self, i: usize) -> Option<String> {
        let entered: Vec<&str> = self
            .frames
            .get(3..=i)
            .unwrap_or_default()
            .iter()
            .map(|f| f.entered.as_str())
            .collect();
        if entered.iter().any(|l| l.is_empty()) {
            return None;
        }
        Some(crate::context::of(&entered))
    }

    /// The context the submodes entered give the current mode, as the corpus derives it from a
    /// session's prompts (`context::of`); none when a frame's entering line is unknown.
    fn context(&self) -> Option<String> {
        let entered: Vec<&str> = self
            .frames
            .iter()
            .skip(3)
            .map(|f| f.entered.as_str())
            .collect();
        if entered.iter().any(|l| l.is_empty()) {
            return None;
        }
        Some(crate::context::of(&entered))
    }

    /// Whether the running configuration holds `line` at its top level.
    fn running_has(&self, line: &str) -> bool {
        self.device.global.iter().any(|e| e.text.trim() == line)
    }

    /// The session's license level: its running configuration's `license boot level`, else the
    /// level it booted at (`boot_level`), else its book's default (`license::default_for`); it picks which
    /// crawls answer `?` and which words the level leaves out (`level_refusal`).
    fn level(&self) -> Option<String> {
        let book = crate::grammars::book_of(&self.engine.grammar.platform)?;
        let running = self.device.global.iter().map(|e| e.text.as_str());
        crate::license::stated(running)
            .or_else(|| self.boot_level.clone())
            .or_else(|| crate::license::default_for(book).map(str::to_string))
    }

    /// What a real switch of this image printed for `?` after `text` in `mode` and the current
    /// context, where the session's state leaves that answer true.
    fn held_help(&self, mode: &str, text: &str) -> Option<&Vec<String>> {
        let context = self.context();
        self.engine.held.help_for(
            self.level().as_deref(),
            mode,
            context.as_deref(),
            text,
            |l| self.running_has(l),
        )
    }

    /// What may follow the whole tokens `done` at `mode`, and whether the line is complete there; at
    /// `>`, no command `privilege exec level` raised above it.
    fn offered_at(&self, mode: &str, done: &[cli::Tok]) -> (Vec<Offer>, bool) {
        let (next, mut complete) = self.engine.next_at(mode, done, self.level().as_deref());
        let mut offered = self.offered(&next);
        if mode == "exec" {
            let raised = self.raised();
            let words: Vec<String> = done.iter().map(|t| t.text.to_lowercase()).collect();
            offered.retain(|o| {
                let mut line = words.clone();
                line.push(o.shown.to_lowercase());
                !Self::raised_typed(&raised, mode, &line)
            });
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

    /// What `?` after `text` answers before the books: a refusal after a command raised above `>`'s
    /// level (c9200 17.15 capture 27's `show ?`), else a held answer, abbreviations read as their
    /// keywords; else a keyword-set command after several picks, from what it offered after each
    /// alone; the image's own first, then a donor's. Each is printed for this switch
    /// (`as_this_switch`).
    fn held_answer(&self, mode: &str, context: Option<&str>, text: &str) -> Option<Vec<String>> {
        // the whole words before the one being typed, a raised command among their starts
        let mut done: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
        if !text.ends_with(char::is_whitespace) {
            done.pop();
        }
        if Self::raised_typed(&self.raised(), mode, &done) {
            return Some(vec!["% Unrecognized command".into()]);
        }
        let answer =
            self.engine
                .held
                .answer(self.level().as_deref(), mode, context, text, |l| {
                    self.running_has(l)
                })?;
        Some(self.as_this_switch(mode, text, answer))
    }

    /// `help` at `mode`.
    fn help_in(&self, mode: &str, context: Option<&str>, text: &str) -> Vec<String> {
        if let Some(lines) = self.held_answer(mode, context, text) {
            return lines;
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
            let mut words: BTreeSet<String> = offered
                .into_iter()
                .filter(|o| {
                    o.kind != Offered::Placeholder && o.shown.to_lowercase().starts_with(&typed)
                })
                .map(|o| o.shown)
                .collect();
            if do_exec_begins(mode, done, &typed) {
                words.insert("do-exec".into());
            }
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

    /// The line Tab makes of `text`: its last word as typed, then the rest of the one keyword or
    /// interface type it begins and a space, or none when it begins none or several, a whole keyword
    /// among them (lab C9300-24T 17.6.4: `interface TWE` Tab gives `interface TWEntyFiveGigE `;
    /// `interface Twe1/0/1`, `sh` and `ip` beside `ipv6` change nothing).
    pub fn complete(&self, text: &str) -> Option<String> {
        match self.after_do(text) {
            Some(rest) => {
                let head = &text[..text.len() - rest.len()];
                self.complete_in("privileged", Some(""), rest)
                    .map(|done| format!("{head}{done}"))
            }
            None => self.complete_in(self.mode(), self.context().as_deref(), text),
        }
    }

    /// `complete` at `mode`. The words come from the answer `?` gives for the line before the word,
    /// a held one included (`held_answer`), a refusal offering none; the books' keywords and the
    /// stack's interface types where none is held.
    fn complete_in(&self, mode: &str, context: Option<&str>, text: &str) -> Option<String> {
        if text.is_empty() || text.ends_with(char::is_whitespace) {
            return None;
        }
        let toks = crate::cli::tokens(text);
        let last = toks.last()?;
        let typed = last.text.to_lowercase();
        let head = &text[..last.at];
        let mut kws: Vec<String> = match self.held_answer(mode, context, head) {
            Some(answer) => crate::live::keywords(&answer),
            None => {
                let (offered, _) = self.offered_at(mode, &toks[..toks.len() - 1]);
                offered
                    .into_iter()
                    .filter(|o| o.kind != Offered::Placeholder)
                    .map(|o| o.shown)
                    .collect()
            }
        };
        kws.retain(|k| k.to_lowercase().starts_with(&typed));
        if do_exec_begins(mode, &toks[..toks.len() - 1], &typed)
            && !kws.iter().any(|k| k == "do-exec")
        {
            kws.push("do-exec".into());
        }
        // one keyword the word begins, whole or not: `ip` beside `ipv6` rings the bell (lab C9300-24T
        // 17.6.4), though `?` and Enter take a whole word as itself
        let mut begun: Vec<String> = kws.iter().map(|k| k.to_lowercase()).collect();
        begun.sort_unstable();
        begun.dedup();
        let [_] = begun[..] else {
            return None;
        };
        let pick = kws.first()?;
        let rest = pick.get(last.text.len()..).unwrap_or("");
        Some(format!("{head}{}{rest} ", last.text))
    }

    /// After `privilege exec level <n> <command>` of several words, each shorter start of the command
    /// raised to the same level, over a start set before at another level: the lab C9300-24T
    /// (17.6.4) kept `... 14 show ip route`, `... 14 show ip`, `... 14 show`, and after `... 5 show`,
    /// `... 14 show version` left `... 14 show`. Their order in print is `show::configuration`'s.
    fn raise_parents(&mut self, p: &Parse) {
        let line = p.canonical();
        let words: Vec<&str> = line.split_whitespace().collect();
        let [privilege, exec, level, n, command @ ..] = &words[..] else {
            return;
        };
        let head = [privilege, exec, level].map(|w| w.to_lowercase());
        if p.negated || head != ["privilege", "exec", "level"] || command.len() < 2 {
            return;
        }
        for k in (1..command.len()).rev() {
            let parent = format!("privilege exec level {n} {}", command[..k].join(" "));
            if let Outcome::Ok(q) = self.engine.parse("config", &parent) {
                let kept = self.engine.held.has("", &q.canonical());
                self.device
                    .apply_as(&q, None, kept, self.whole("config", &q));
            }
        }
    }

    /// Writes an accepted configuration line where the current mode keeps it.
    fn write(&mut self, p: &Parse) {
        let top = self
            .frames
            .last()
            .cloned()
            .unwrap_or_else(|| Frame::top("config"));
        // a line a real switch of this platform and release printed is kept as it printed it, as is
        // one the switch whose configuration is replayed printed
        let printed = |d: &Device, b: Option<usize>| {
            let header = b.map_or("", |b| d.blocks[b].header.as_str());
            self.engine
                .held
                .has(&crate::live::kind(header), &p.canonical())
                || d.layout
                    .as_ref()
                    .is_some_and(|l| l.lines.contains(&crate::device::layout_key(&p.canonical())))
        };
        let whole = self.whole(&top.mode, p);
        // `root primary` and `root secondary` store the priority each VLAN takes against the root
        // the switch knows now (the lab C9300, 17.6.4), VLANs taking the same one on one line
        if top.blocks.is_empty()
            && !p.negated
            && let Some((list, primary)) = crate::device::root_request(p)
        {
            let mut by: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
            for v in crate::generate::vlan_ids(&list) {
                let prio =
                    crate::spanning::root_priority(&self.device, self.lab.as_ref(), v, primary);
                by.entry(prio).or_default().insert(v);
            }
            for (prio, vlans) in by {
                let q = crate::device::as_priority(p, &crate::target::ios_list(&vlans), prio);
                self.write(&q);
            }
            return;
        }
        // a VLAN setting is kept per VLAN, VLANs holding one value printed on one line (the lab
        // C2960G, 15.0(2)SE11: `no spanning-tree vlan 10 priority` after `spanning-tree vlan 1,10
        // priority 4096` left VLAN 1 at 4096; the lab C9300 prints VLANs 1 and 10 apart)
        if top.blocks.is_empty()
            && let Some((list, setting, value)) = crate::device::vlan_setting(p)
        {
            let mut by_vlan: BTreeMap<u32, String> = BTreeMap::new();
            self.device.global.retain(|e| {
                let w: Vec<&str> = e.text.split_whitespace().collect();
                match w[..] {
                    ["spanning-tree", "vlan", l, s, val] if s == setting => {
                        for v in crate::generate::vlan_ids(l) {
                            by_vlan.insert(v, val.to_string());
                        }
                        false
                    }
                    _ => true,
                }
            });
            for v in crate::generate::vlan_ids(&list) {
                match &value {
                    Some(x) => by_vlan.insert(v, x.clone()),
                    None => by_vlan.remove(&v),
                };
            }
            let mut by_value: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
            for (v, x) in by_vlan {
                by_value.entry(x).or_default().insert(v);
            }
            let mut groups: Vec<(String, BTreeSet<u32>)> = by_value.into_iter().collect();
            groups.sort_by_key(|(_, vlans)| vlans.first().copied());
            for (x, vlans) in groups {
                let text = format!(
                    "spanning-tree vlan {} {setting} {x}",
                    crate::target::ios_list(&vlans)
                );
                if let Outcome::Ok(q) = self.engine.parse("config", &text) {
                    let kept = self.engine.held.has("", &q.canonical());
                    let whole = self.whole(&top.mode, &q);
                    self.device.apply_as(&q, None, kept, whole);
                }
            }
            return;
        }
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
            self.raise_parents(p);
            return;
        }
        // a member's `channel-group <n>` makes its port-channel interface, as IOS does, with the
        // member's switch port lines (the lab C9300, 17.6.4: Po1 took ` switchport mode trunk`); the
        // interface stays when its members leave
        if !p.negated
            && let Some(&member) = top
                .blocks
                .iter()
                .find(|&&b| self.device.blocks[b].header.starts_with("interface "))
            && let Some(n) = p
                .canonical()
                .strip_prefix("channel-group ")
                .and_then(|r| r.split_whitespace().next())
                .filter(|n| n.chars().all(|c| c.is_ascii_digit()))
        {
            let header = format!("interface Port-channel{n}");
            // a configuration printed by IOS has the port-channel before its members, so replaying
            // one makes nothing here; an empty one takes them too (the lab C2960G, 15.0(2)SE11:
            // its Po1 had no lines)
            let made = !self.device.blocks.iter().any(|b| b.header == header);
            let po = self.device.block(&header);
            if made || self.device.blocks[po].lines.is_empty() {
                let lines: Vec<_> = self.device.blocks[member]
                    .lines
                    .iter()
                    .filter(|e| e.key.starts_with("switchport"))
                    .cloned()
                    .collect();
                self.device.blocks[po].lines = lines;
            }
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
                    if header.starts_with("class-map ")
                        && !self.device.stack.classic()
                        && let Some(t) = match_spacing(&self.device.blocks[b].lines[i].text)
                    {
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
                // IOS activates a neighbor in the IPv4 address family by default and keeps `no
                // neighbor <x> activate` there (the lab C9300-24T, 17.6.4: `  no neighbor
                // 2001:DB8:10::2 activate` under `address-family ipv4`)
                let words: Vec<String> = p
                    .canonical()
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
                let block = &self.device.blocks[b];
                let deactivated = p.negated
                    && block.header.starts_with("router bgp ")
                    && top.section.as_deref().is_some_and(|s| {
                        matches!(
                            s.trim(),
                            "address-family ipv4" | "address-family ipv4 unicast"
                        )
                    })
                    && words.len() == 4
                    && words[1] == "neighbor"
                    && words[3] == "activate"
                    && !block
                        .lines
                        .iter()
                        .any(|e| e.text == "no bgp default ipv4-unicast");
                let prefix = top.prefix.as_str();
                let lines = &mut self.device.blocks[b].lines;
                // within this section only: a policy map's classes each set their own `priority
                // level` (c9300 17.15 capture 1)
                let (start, end) = match top.section.as_deref() {
                    Some(s) => section_span(lines, s, prefix),
                    None => (0, lines.len()),
                };
                // the section's lines as a block's, its indentation off, changed as a block's are
                let strip = |s: String| s.strip_prefix(prefix).map_or(s.clone(), str::to_string);
                let mut section: Vec<Entry> = lines
                    .drain(start..end)
                    .map(|e| Entry {
                        key: strip(e.key),
                        text: strip(e.text),
                    })
                    .collect();
                crate::device::apply_to(
                    &mut section,
                    p,
                    crate::device::key(p),
                    kept || deactivated,
                    whole,
                );
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
            let (o, r) = self.enter(l);
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

/// Where in a refused number the caret stands past its start: under the digit that takes it past the
/// top of the ranges offered there (lab C9300-24T 17.6.4: `terminal length 600` under the last `0`,
/// `spanning-tree vlan 1 priority 70000` under the fifth digit, `privilege exec level 99` under the
/// second `9`); 0 for a number some range takes, one below every range, or a word not a number.
fn past_the_top(offered: &[String], typed: &str) -> usize {
    if typed.is_empty() || !typed.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    let ranges: Vec<(u128, u128)> = offered
        .iter()
        .filter_map(|w| {
            let (a, b) = w.strip_prefix('<')?.strip_suffix('>')?.split_once('-')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        })
        .collect();
    let Some(top) = ranges.iter().map(|r| r.1).max() else {
        return 0;
    };
    let mut value: u128 = 0;
    for (i, d) in typed.bytes().enumerate() {
        value = value * 10 + u128::from(d - b'0');
        if value > top {
            return i;
        }
    }
    0
}

/// Whether `do-exec` is among the words a configuration line's first word, typed as `typed`, begins:
/// `?` lists it and Tab completes it, as IOS does every keyword of the mode.
fn do_exec_begins(mode: &str, done: &[cli::Tok], typed: &str) -> bool {
    cli::is_config(mode) && done.is_empty() && "do-exec".starts_with(typed)
}

/// The blocks a submode-entering line opens: `interface TwentyFiveGigE1/0/1`, `vlan 459`, one per
/// member of an interface range; an interface as IOS names it, its type and number joined
/// (`interface Port-channel11` for `interface port-channel 11`, `interface Vlan20` for
/// `interface vlan 20`).
/// Whether every keyword of a parse was typed whole.
fn whole_words(p: &Parse) -> bool {
    p.items.iter().all(|i| match i {
        Item::Kw { exact, .. } => *exact,
        Item::Arg { .. } => true,
    })
}

/// What IOS XE says for a line naming a VRF the switch does not hold, by its mode and the words
/// before the name (the lab C9300-24T, 17.6.4); `{}` is the name.
const NO_VRF: &[(&str, &[&str], &str)] = &[
    (
        "config-if",
        &["vrf", "forwarding"],
        "% VRF {} not configured",
    ),
    (
        "config-if",
        &["ip", "vrf", "forwarding"],
        "% VRF {} not configured",
    ),
    (
        "config-if",
        &["ip", "helper-address", "vrf"],
        "% Vrf name {} does not exist. Create first",
    ),
    // at network-essentials, after `vrf definition` was refused
    (
        "config",
        &["ip", "route", "vrf"],
        "% vrf {} does not exist. Create first",
    ),
    (
        "config-mon-erspan-src-dst",
        &["vrf"],
        "% VRF {} not configured",
    ),
    (
        "config",
        &["ip", "multicast-routing", "vrf"],
        "% VPN Routing instance {} does not exist.",
    ),
    (
        "config",
        &["ip", "mroute", "vrf"],
        "% VPN Routing instance {} does not exist.",
    ),
    (
        "config",
        &["ip", "pim", "vrf"],
        "% VPN Routing instance {} does not exist.",
    ),
    (
        "config",
        &["ip", "multicast", "vrf"],
        "% VPN Routing instance {} does not exist.",
    ),
    (
        "config",
        &["ipv6", "multicast-routing", "vrf"],
        "%VPN Routing instance {} does not exist. Create first",
    ),
    (
        "config",
        &["ipv6", "pim", "vrf"],
        "%VPN Routing instance {} does not exist. Create first",
    ),
    (
        "config",
        &["ntp", "server", "vrf"],
        "% IP  routing table {} does not exist",
    ),
    (
        "config",
        &["cts", "role-based", "l2-vrf"],
        "%VPN Routing/Forwarding table {} does not exist",
    ),
    (
        "config",
        &["cts", "role-based", "sgt-map", "vrf"],
        "%VPN Routing/Forwarding table {} does not exist",
    ),
    // a LISP instance's IPv4 service (the lab C9200L-48T-4G, 17.15.6)
    (
        "config-lisp-inst-srv-ipv4",
        &["eid-table", "vrf"],
        "% Invalid VRF name!",
    ),
    // `exec`: user and privileged EXEC
    (
        "exec",
        &["show", "ip", "route", "vrf"],
        "% IP routing table vrf {} does not exist",
    ),
    (
        "exec",
        &["show", "ip", "arp", "vrf"],
        "% VRF {} does not exist.",
    ),
    (
        "exec",
        &["show", "ip", "bgp", "vpnv4", "vrf"],
        "% Unknown VRF\n\n",
    ),
    (
        "exec",
        &["show", "ip", "vrf", "interfaces"],
        "% No VRF named {}",
    ),
    (
        "exec",
        &["show", "ip", "vrf", "detail"],
        "% No VRF named {}",
    ),
    (
        "exec",
        &["show", "mpls", "forwarding-table", "vrf"],
        "% IP routing table {} doesn't exist",
    ),
    (
        "exec",
        &["show", "mpls", "static", "binding", "ipv4", "vrf"],
        "% VRF {} does not exist",
    ),
];

/// The 802.1X lines IOS XE takes and calls deprecated for `access-session`, by mode and start (the
/// lab C9300-24T, 17.6.4).
const DEPRECATED: &[(&str, &str)] = &[
    ("config-if", "authentication host-mode "),
    ("config-if", "authentication port-control "),
    ("config-if", "authentication control-direction "),
    ("config-if", "authentication open"),
    ("config", "authentication mac-move "),
];

/// Lines that hold the 802.1X configuration new-style: the lab C9200L-48T-4G (17.6.4), its
/// configuration legacy, asked to convert it before taking each of these.
const NEW_STYLE: &[&str] = &[
    "policy-map type control subscriber ",
    "class-map type control subscriber ",
    "parameter-map type webauth ",
    "parameter-map type subscriber ",
    "autoconf enable",
    "subscriber aging ",
    "access-session host-mode ",
    "access-session closed",
    "access-session port-control ",
    "access-session control-direction ",
];

/// Whether the configuration's 802.1X lines are new-style: a line of `NEW_STYLE` at the top, as a
/// block's header or in a block.
fn new_style(d: &Device) -> bool {
    let style = |t: &str| NEW_STYLE.iter().any(|s| t.starts_with(s));
    d.global.iter().any(|e| style(&e.text))
        || d.blocks
            .iter()
            .any(|b| style(&b.header) || b.lines.iter().any(|e| style(e.text.trim_start())))
}

/// The words after a BGP neighbor's address that set its policy, which an address family holds:
/// what the lab C9300-24T (17.6.4) offered for `neighbor <address> ?` in `address-family ipv4`,
/// `inherit` apart (a peer session is no policy).
const BGP_POLICY: &[&str] = &[
    "activate",
    "additional-paths",
    "advertise",
    "advertise-map",
    "advertisement-interval",
    "aigp",
    "allow-policy",
    "allowas-in",
    "announce",
    "as-override",
    "capability",
    "default-originate",
    "distribute-list",
    "dmzlink-bw",
    "filter-as",
    "filter-list",
    "maximum-prefix",
    "next-hop-self",
    "next-hop-unchanged",
    "prefix-list",
    "remove-private-as",
    "route-map",
    "route-reflector-client",
    "send-community",
    "send-label",
    "slow-peer",
    "soft-reconfiguration",
    "soo",
    "suppress-sr",
    "translate-update",
    "unsuppress-map",
    "weight",
];

/// A SPAN session's lines in the running configuration (`monitor session <n> ...`).
#[derive(Default)]
struct Span {
    sources: Vec<String>,
    local_source: bool,
    destinations: Vec<String>,
    remote_destination: bool,
    filter_vlans: BTreeSet<u32>,
}

/// The running configuration's SPAN sessions by number.
fn spans(d: &Device) -> BTreeMap<String, Span> {
    let mut out: BTreeMap<String, Span> = BTreeMap::new();
    for e in &d.global {
        let Some(rest) = e.text.strip_prefix("monitor session ") else {
            continue;
        };
        let words: Vec<&str> = rest.split_whitespace().collect();
        let Some((n, words)) = words.split_first() else {
            continue;
        };
        let s = out.entry(n.to_string()).or_default();
        match words {
            ["source", "interface", list @ ..] => {
                let list = list.join(" ");
                for name in list.split(',') {
                    let name = name.split(" - ").next().unwrap_or(name);
                    if let Some(name) = name.split_whitespace().next() {
                        s.sources.push(name.to_string());
                    }
                }
                s.local_source = true;
            }
            ["source", "vlan", ..] => s.local_source = true,
            ["destination", "interface", name, ..] => s.destinations.push(name.to_string()),
            ["destination", "remote", "vlan", ..] => s.remote_destination = true,
            ["filter", "vlan", list @ ..] => s.filter_vlans.extend(numbers(&list.join(""))),
            _ => {}
        }
    }
    out
}

/// The numbers of a list such as `1-5,9`.
fn numbers(list: &str) -> Vec<u32> {
    list.split(',')
        .filter_map(|r| match r.split_once('-') {
            Some((a, b)) => {
                Some((a.trim().parse::<u32>().ok()?..=b.trim().parse().ok()?).collect())
            }
            None => r.trim().parse().ok().map(|n| vec![n]),
        })
        .flatten()
        .collect()
}

/// What IOS XE says for a SPAN line its sessions refuse (the lab C9300-24T, 17.6.4): a source that
/// is a destination, a destination that is a source or another session's destination, a remote
/// VLAN in a session of the other kind, a filter VLAN as the remote destination, an access-list
/// filter beside a VLAN filter. `named`: the line's interfaces, short.
fn span_refusal(
    d: &Device,
    n: &str,
    rest: &[&str],
    named: &[String],
    vlans: &BTreeSet<u32>,
) -> Option<String> {
    let all = spans(d);
    let own = all.get(n);
    let others = || all.iter().filter(|(m, _)| m.as_str() != n).map(|(_, s)| s);
    match rest {
        ["source", "interface", ..] => {
            let name = named
                .iter()
                .find(|x| all.values().any(|s| s.destinations.contains(x)))?;
            Some(format!(
                "% Interface(s) {name} already configured as monitor destinations"
            ))
        }
        ["source", "remote", "vlan", ..] => {
            if own.is_some_and(|s| s.remote_destination) {
                Some(
                    "% RSPAN VLAN already exists as destination. The same session cannnot have \
                     RSPAN VLAN both as source and destination"
                        .into(),
                )
            } else if own.is_some_and(|s| s.local_source) {
                Some(format!(
                    "% Cannot add RSPAN VLAN as source for SPAN session {n} as it is not a RSPAN \
                     Destination session"
                ))
            } else {
                None
            }
        }
        ["destination", "interface", _, tail @ ..] => {
            let name = named.first()?;
            if own.is_some_and(|s| s.remote_destination) {
                return Some(format!(
                    "% Cannot add ports as destination for session {n} - a RSPAN Source session"
                ));
            }
            if all.values().any(|s| s.sources.contains(name)) {
                return Some(format!(
                    "% Interface(s) {name} already configured as monitor sources"
                ));
            }
            if !others().any(|s| s.destinations.contains(name)) {
                return None;
            }
            // an ingress VLAN the switch lacks is said first
            let missing = tail
                .iter()
                .position(|w| *w == "vlan")
                .and_then(|i| tail.get(i + 1)?.parse::<u32>().ok())
                .filter(|v| !vlans.contains(v));
            let warning = missing.map_or(String::new(), |v| {
                format!("% Warning: specified default VLAN ({v}) for ingress on dest port does not exist.\n")
            });
            Some(format!(
                "{warning}% Interface(s) {name} already configured as monitor destinations in other \
                 monitor sessions"
            ))
        }
        ["destination", "remote", "vlan", v] => {
            let s = own?;
            if !s.destinations.is_empty() {
                Some(format!(
                    "% Session - {n} is a not a Remote Source session. Can't add RSPAN VLAN as \
                     destination"
                ))
            } else if v.parse::<u32>().is_ok_and(|v| s.filter_vlans.contains(&v)) {
                Some(format!(
                    "% Configuring Destination RSPAN VLANs {v}, but the same vlan is \n% already \
                     configured as filter vlan"
                ))
            } else {
                None
            }
        }
        ["filter", "ip" | "mac", "access-group", ..] => own
            .is_some_and(|s| !s.filter_vlans.is_empty())
            .then(|| "% vlan filter can not be applied with FSPAN".into()),
        _ => None,
    }
}

/// What IOS says when a private VLAN association names a VLAN the switch does not hold, the primary
/// read first, then the secondaries in order (the lab C9300-24T, 17.6.4).
fn no_private_association(primary: u32, list: &str, vlans: &BTreeSet<u32>) -> Option<String> {
    let secondary = crate::generate::vlan_ids(list);
    let first = *secondary.first()?;
    let missing = std::iter::once(primary)
        .chain(secondary.iter().copied())
        .find(|v| !vlans.contains(v))?;
    let pair = if missing == primary { first } else { missing };
    Some(format!(
        "%Command rejected: invalid private vlan association between vlan{primary} and \
         vlan{pair}. VLAN {missing} data is not available."
    ))
}

/// The features whose `attach-policy` names a policy of their own, by the mode it is typed in:
/// `% Policy <name> not defined for the feature` until `<feature> policy <name>` exists.
const ATTACH: &[(&str, &[&str])] = &[
    ("config-if", &["ipv6", "source-guard"]),
    ("config-if", &["ipv6", "destination-guard"]),
    ("config-vlan-config", &["device-tracking"]),
    ("config-vlan-config", &["ipv6", "dhcp", "guard"]),
    ("config-vlan-config", &["ipv6", "nd", "raguard"]),
];

/// Whether a BGP peer is an IPv6 link-local address, its zone after `%` (`fe80::2%Ethernet1/0`).
fn link_local(peer: &str) -> bool {
    let address = peer.split('%').next().unwrap_or(peer);
    address
        .parse::<std::net::Ipv6Addr>()
        .is_ok_and(|a| a.segments()[0] & 0xffc0 == 0xfe80)
}

/// An IPv4 address as a number.
fn v4(text: &str) -> Option<u32> {
    text.parse::<std::net::Ipv4Addr>().ok().map(u32::from)
}

/// The address and mask of an interface's `ip address` line, a secondary's too.
fn ip_address(text: &str) -> Option<(u32, u32)> {
    match text.split_whitespace().collect::<Vec<_>>()[..] {
        ["ip", "address", a, m, ..] => Some((v4(a)?, v4(m)?)),
        _ => None,
    }
}

/// A named EIGRP address family's line (`address-family ipv4 [unicast] [vrf V] autonomous-system
/// N`) as its family (address family, unicast or multicast, VRF) and AS.
type EigrpFamily<'a> = ((&'a str, &'a str, Option<&'a str>), &'a str);
fn eigrp_family<'a>(words: &[&'a str]) -> Option<EigrpFamily<'a>> {
    let ["address-family", afi, rest @ ..] = words else {
        return None;
    };
    let after = |k: &str| {
        let i = rest.iter().position(|w| *w == k)?;
        rest.get(i + 1).copied()
    };
    let cast = match rest.first() {
        Some(&"multicast") => "multicast",
        _ => "unicast",
    };
    Some(((afi, cast, after("vrf")), after("autonomous-system")?))
}

/// The tunnels IOS keeps for PIM registers once multicast routing is on: one for each RP the global
/// configuration names and one more where the switch is an RP itself, numbered from the lowest no
/// tunnel interface holds. The lab C9300-24T (17.6.4), its own RP with Tunnel0 configured, refused
/// Tunnel1 and Tunnel2 and took Tunnel0, 10, 11, 15 and 100.
fn pim_tunnels(d: &Device) -> BTreeSet<u32> {
    if !d
        .global
        .iter()
        .any(|e| e.text.trim_end() == "ip multicast-routing")
    {
        return BTreeSet::new();
    }
    let rps: Vec<&str> = d
        .global
        .iter()
        .filter_map(|e| e.text.strip_prefix("ip pim rp-address "))
        .filter_map(|r| r.split_whitespace().next())
        .collect();
    let own = rps.iter().any(|rp| {
        d.blocks.iter().any(|b| {
            b.header.starts_with("interface ")
                && b.lines.iter().any(|e| {
                    e.text
                        .trim()
                        .strip_prefix("ip address ")
                        .and_then(|a| a.split_whitespace().next())
                        == Some(*rp)
                })
        })
    });
    let used: BTreeSet<u32> = d
        .blocks
        .iter()
        .filter_map(|b| b.header.strip_prefix("interface Tunnel")?.parse().ok())
        .collect();
    (0..)
        .filter(|t| !used.contains(t))
        .take(rps.len() + usize::from(own))
        .collect()
}

/// How far a mode read a line before refusing it: to the refused word, or past its end for a line
/// that was all words of the mode but incomplete. An ambiguous word reads nothing.
fn reach(o: &Outcome, line: &str) -> usize {
    match o {
        Outcome::Ok(_) => usize::MAX,
        Outcome::Invalid(at) => *at,
        Outcome::Incomplete => line.len() + 1,
        Outcome::Ambiguous => 0,
    }
}

/// How a typed word fits a row of a `?` answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    No,
    Word,
    /// a `LINE` row: the rest of the line is its text
    Rest,
}

/// Whether `typed` is a word the `?` row `row` (described `what`) offers: a start of its keyword
/// in any case, a number in its range, an address of its form, any word where it shows another
/// placeholder (`WORD`). A row naming an object the crawled switch had configured (`User defined`)
/// fits any word, since this session's objects are others. An interface named with its number, or a
/// type where the answer lists `types`, fits whatever the row, and the stack judges it: the crawled
/// switch's ports are not this one's.
fn fits_row(row: &str, what: &str, typed: &str, types: bool) -> Fit {
    let t = typed.to_lowercase();
    if row == "<cr>" || row == "|" {
        return Fit::No;
    }
    if row == "LINE" {
        return Fit::Rest;
    }
    if let Some(range) = row.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
        let Some((lo, hi)) = range.split_once('-') else {
            return Fit::Word;
        };
        let (Ok(lo), Ok(hi)) = (lo.parse::<u128>(), hi.parse::<u128>()) else {
            return Fit::Word;
        };
        // a rate with its unit (`1m`), or a list of numbers and ranges (`31-35,41`)
        let number = |v: &str| v.parse::<u128>().is_ok_and(|v| v >= lo && v <= hi);
        let rate = t
            .strip_suffix(['k', 'm', 'g'])
            .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()));
        let list = t.split(',').all(|part| match part.split_once('-') {
            Some((a, b)) => number(a) && number(b),
            None => number(part),
        });
        return fit(rate || list);
    }
    let alpha = t.split(|c: char| c.is_ascii_digit()).next().unwrap_or("");
    if (types || alpha.len() < t.len()) && names_type(alpha) {
        return Fit::Word;
    }
    let what = what.to_lowercase();
    if ["user defined", "defined by user", "vrf name"].contains(&what.as_str()) {
        return Fit::Word;
    }
    // a union of forms (`Hostname or A.B.C.D`) and the mixed-case placeholders
    if what.starts_with("or ")
        || row == "Hostname"
        || row.contains(':')
        || ["Hex-", "Group_", "<", ".", "/"]
            .iter()
            .any(|p| row.starts_with(p))
    {
        return Fit::Word;
    }
    match row {
        "A.B.C.D" => fit(cli::ipv4(typed)),
        "X:X:X:X::X" => fit(typed.contains(':')),
        "H.H.H" => fit(typed.split('.').count() == 3),
        "WORD" | "CHAR" | "MONTH" | "DAY" => Fit::Word,
        _ if ["A.B.C.D", "H.H.", "XX.XX"]
            .iter()
            .any(|p| row.starts_with(p)) =>
        {
            Fit::Word
        }
        // every other word is a keyword (`BDI`, `LISP`, `Light`), typed in any case
        _ => fit(row.to_lowercase().starts_with(&t)),
    }
}

/// Whether a word begins an interface type's name, two letters at least.
fn names_type(word: &str) -> bool {
    word.len() >= 2
        && cli::interface_types()
            .iter()
            .chain(cli::PHYSICAL_TYPES)
            .any(|(full, _)| full.to_lowercase().starts_with(word))
}

fn fit(yes: bool) -> Fit {
    if yes { Fit::Word } else { Fit::No }
}

/// The index of the word of `line` holding byte `at`.
fn token_at(line: &str, at: usize) -> Option<usize> {
    cli::tokens(line)
        .iter()
        .position(|t| at >= t.at && at < t.at + t.text.len())
}

/// Where an interface type `typed` names, in an answer listing interface types, is one the level
/// lists and refuses (`refused`), the characters of it the answer's other rows begin with: the
/// caret stands past them. `template type ?` lists `pseudowire` as a template's type, which the
/// level takes.
fn refused_type(answer: &[String], typed: &str, refused: &[&str]) -> Option<usize> {
    let t = typed.to_lowercase();
    let alpha = t.split(|c: char| c.is_ascii_digit()).next().unwrap_or("");
    let rows = crate::live::answer_rows(answer);
    let listed = |w: &str| rows.iter().any(|(r, _)| r.eq_ignore_ascii_case(w));
    let begins = |w: &str| alpha.len() >= 2 && w.to_lowercase().starts_with(alpha);
    if !refused.iter().any(|r| listed(r) && begins(r)) {
        return None;
    }
    let others: Vec<&str> = rows
        .iter()
        .map(|(w, _)| *w)
        .filter(|w| !refused.iter().any(|r| r.eq_ignore_ascii_case(w)))
        .collect();
    let types = others
        .iter()
        .filter(|w| {
            cli::interface_types()
                .iter()
                .chain(cli::PHYSICAL_TYPES)
                .any(|(full, _)| *full == **w)
        })
        .count();
    if types < 2 {
        return None;
    }
    if others
        .iter()
        .any(|w| begins(w) && names_type(&w.to_lowercase()))
    {
        return None;
    }
    others
        .iter()
        .map(|w| {
            t.chars()
                .zip(w.to_lowercase().chars())
                .take_while(|(a, b)| a == b)
                .count()
        })
        .max()
        .map(|n| n.min(alpha.len().saturating_sub(1)))
}

/// Whether `typed` names an interface type only the level above lists (`above`), beginning with a
/// type the level's answer (`at`) lists: no host name, though `at` offers one. The lab C9300-24T
/// (17.6.4) at network-essentials refused `show ip ospf neighbor Tunnel-tp` under its `-`.
fn type_above_only(at: &[String], above: &[String], typed: &str) -> bool {
    let alpha = typed
        .to_lowercase()
        .split(|c: char| c.is_ascii_digit())
        .next()
        .unwrap_or("")
        .to_string();
    let is_type = |w: &str| {
        cli::interface_types()
            .iter()
            .chain(cli::PHYSICAL_TYPES)
            .any(|(full, _)| *full == w)
    };
    let rows = |a: &[String]| -> Vec<String> {
        crate::live::answer_rows(a)
            .iter()
            .map(|(w, _)| w.to_string())
            .collect()
    };
    let (at, above) = (rows(at), rows(above));
    let begins = |w: &String| w.to_lowercase().starts_with(&alpha);
    !alpha.is_empty()
        && above
            .iter()
            .any(|w| begins(w) && w.starts_with(|c: char| c.is_ascii_uppercase()))
        && !at.iter().any(begins)
        && at
            .iter()
            .any(|w| is_type(w) && alpha.len() > w.len() && alpha.starts_with(&w.to_lowercase()))
}

/// Whether a `?` row names a file system (`flash:`, `usbflash0:`), which a path after it fits.
fn file_system(row: &str) -> bool {
    row.strip_suffix(':').is_some_and(|n| {
        !n.is_empty()
            && n.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

/// How `typed` fits a `?` answer, for the licence gate (`level_refusal`), whose two answers list
/// what each level's image holds: an interface named by its type and number fits a row of that
/// type, where the answer lists types; a file system's row (`flash:`) is the keyword it is, a path
/// after it included; any other word fits as `fits_row` reads it.
fn best_fit(answer: &[String], typed: &str) -> Fit {
    let is_type = |w: &str| {
        cli::interface_types()
            .iter()
            .chain(cli::PHYSICAL_TYPES)
            .any(|(full, _)| *full == w)
    };
    let t = typed.to_lowercase();
    let rows = crate::live::answer_rows(answer);
    let types: Vec<&str> = rows
        .iter()
        .map(|(w, _)| *w)
        .filter(|w| is_type(w))
        .collect();
    let alpha = t.split(|c: char| c.is_ascii_digit()).next().unwrap_or("");
    if !types.is_empty() && names_type(alpha) {
        return fit(types.iter().any(|w| w.to_lowercase().starts_with(alpha)));
    }
    rows.iter()
        .map(|&(w, what)| match file_system(w) {
            true => fit(w.starts_with(&t) || t.starts_with(w)),
            false => fits_row(w, what, typed, types.len() >= 2),
        })
        .max()
        .unwrap_or(Fit::No)
}

/// A class map's `match` on DSCP, precedence or CoS values as IOS XE writes it: each DSCP or
/// precedence value followed by a space, the values a space apart (the lab C9300-24T, 17.6.4: `match
/// dscp cs4  cs5  ef `; 41 17.15 captures `match dscp ef `), a CoS value with a space either side
/// (`match cos  5 `; more than one inferred the same).
fn match_spacing(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let (head, values) = match words.as_slice() {
        ["match", "ip", kind @ ("dscp" | "precedence"), rest @ ..] => {
            (format!("match ip {kind} "), rest)
        }
        ["match", kind @ ("dscp" | "precedence"), rest @ ..] => (format!("match {kind} "), rest),
        ["match", "cos", rest @ ..] if rest.iter().all(|v| v.parse::<u8>().is_ok()) => {
            let spaced: String = rest.iter().map(|v| format!(" {v} ")).collect();
            return (!rest.is_empty()).then(|| format!("match cos {spaced}"));
        }
        _ => return None,
    };
    let spaced: Vec<String> = values.iter().map(|v| format!("{v} ")).collect();
    (!values.is_empty()).then(|| format!("{head}{}", spaced.join(" ")))
}

/// Where the lines of the section keyed `section` lie in a block's lines: after its header, up to
/// the first line less indented than `prefix`, its lines' indent, or its own `exit-` line, which an
/// interface section prints at its lines' depth.
fn section_span(lines: &[Entry], section: &str, prefix: &str) -> (usize, usize) {
    let start = lines
        .iter()
        .rposition(|e| e.key == section)
        .map_or(0, |i| i + 1);
    let depth = |s: &str| s.len() - s.trim_start().len();
    let close = crate::device::closing(section);
    let end = lines[start..]
        .iter()
        .position(|e| depth(&e.text) < prefix.len() || e.key == close)
        .map_or(lines.len(), |i| start + i);
    (start, end)
}

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

/// An interface type vswitch models, whose names the stack decides (`on_stack`).
fn modelled(of: &str) -> bool {
    cli::interface_types()
        .iter()
        .any(|(full, _)| full.eq_ignore_ascii_case(of))
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

/// Where in a refused name typed with its number (`Po10`) the caret goes: past the digits that
/// still name an interface of its type (the lab C2960G, 15.0(2)SE11, refused `Po10` past `Po1`).
/// Other types and platforms are inferred to do the same; any other word, at its start.
fn past_in_range(word: &str, stack: &crate::platform::Stack) -> usize {
    let Some(split) = word.find(|c: char| c.is_ascii_digit()) else {
        return 0;
    };
    let (abbr, digits) = word.split_at(split);
    let types: Vec<(u64, u64)> = crate::cli::interface_types()
        .iter()
        .filter(|(full, _)| {
            !abbr.is_empty() && full.to_lowercase().starts_with(&abbr.to_lowercase())
        })
        .filter_map(|(full, _)| stack.numbered(full))
        .collect();
    let ([(_, hi)], true) = (types.as_slice(), digits.bytes().all(|b| b.is_ascii_digit())) else {
        return 0;
    };
    let fits = (1..=digits.len())
        .take_while(|&k| digits[..k].parse::<u64>().is_ok_and(|v| v <= *hi))
        .last()
        .unwrap_or(0);
    if fits == digits.len() {
        0
    } else {
        abbr.len() + fits
    }
}

/// Where in a refused interface name (`rest`, the line from the name on) the caret goes: past a
/// front-panel port the stack has where the number goes on (`GigabitEthernet1/0/13/0`, at its
/// fourth level), or where the number departs from the management port's (`gigabitethernet 0/1`,
/// at the `1`), as the lab C9200L-48T-4G (17.6.4) put it, or at a level no port could have
/// (`beyond_the_stack`); at the name's start for any other number (`gigabitethernet 1/0/0`,
/// `GigabitEthernet4/0/1`, `GigabitEthernet 0/0/0`). Other IOS XE platforms are inferred to do the
/// same.
fn past_a_name(rest: &str, stack: &crate::platform::Stack) -> usize {
    if stack.classic() {
        return 0;
    }
    let mut words = rest.split_whitespace();
    let Some(first) = words.next() else {
        return 0;
    };
    let Some((full, number, used)) = cli::interface_name(first, words.next()) else {
        return 0;
    };
    let at = match used {
        1 => first.len() - number.len(),
        _ => rest[first.len()..]
            .find(|c: char| !c.is_whitespace())
            .map_or(0, |i| first.len() + i),
    };
    if let Some(i) = number.match_indices('/').map(|(i, _)| i).find(|&i| {
        let name = format!("{full}{}", &number[..i]);
        stack.has(&name) && stack.management() != Some(name.as_str())
    }) {
        return at + i;
    }
    if let Some(own) = stack
        .management()
        .and_then(|m| m.strip_prefix(full.as_str()))
    {
        let same = number
            .bytes()
            .zip(own.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        if same < number.len() && same < own.len() && own[..same].contains('/') {
            return at + same;
        }
    }
    beyond_the_stack(&full, &number, stack).map_or(0, |i| at + i)
}

/// Where in a port's number (`number`, of type `full`) a level no port could have begins: a module
/// no member's ports of the type carry (`gigabitethernet 6/2`, `2/3/0`, the lab C9300-24T and
/// C9200L-48T-4G, 17.6.4), and on a C9200 a member past the stack's most (`9/0/6`; the lab
/// C9300-24T put that caret at the name).
fn beyond_the_stack(full: &str, number: &str, stack: &crate::platform::Stack) -> Option<usize> {
    use crate::platform::Family;
    let family = stack.family().filter(|f| !f.two_part())?;
    let (member, rest) = number.split_once('/')?;
    let module = rest.split('/').next()?;
    let m: usize = member.parse().ok()?;
    if m > family.max_members() {
        return matches!(family, Family::C9200 | Family::C9200L | Family::C9200B).then_some(0);
    }
    let physical = stack.physical();
    let modules: Vec<&str> = physical
        .iter()
        .filter(|p| stack.management() != Some(p.as_str()))
        .filter_map(|p| p.strip_prefix(full)?.split('/').nth(1))
        .collect();
    let numeric = !module.is_empty() && module.bytes().all(|b| b.is_ascii_digit());
    (m > 0 && numeric && !modules.is_empty() && !modules.contains(&module))
        .then_some(member.len() + 1)
}

/// Whether a port name is the start of one the stack has, short of a level of its number
/// (`GigabitEthernet1/1` of `GigabitEthernet1/1/1`).
fn starts_a_port(value: &str, stack: &crate::platform::Stack) -> bool {
    let v = value.trim();
    stack
        .physical()
        .iter()
        .any(|p| p.strip_prefix(v).is_some_and(|rest| rest.starts_with('/')))
}

/// Whether `value` is the start of a port on a member the stack could hold: read on a member it
/// has, it starts one of that member's ports. IOS XE waits for the rest (the lab C9300-24T, 17.6.4,
/// one member: `TenGigabitEthernet 4/1`, `GigabitEthernet 6/1` incomplete; `GigabitEthernet 6/2`,
/// `gigabitethernet 0/1` and every FastEthernet port refused).
fn short_of_a_port(value: &str, stack: &crate::platform::Stack) -> bool {
    let v = value.trim();
    let Some(split) = v.find(|c: char| c.is_ascii_digit()) else {
        return false;
    };
    let (ty, number) = v.split_at(split);
    let Some((full, _)) = cli::PHYSICAL_TYPES
        .iter()
        .find(|(full, _)| full.eq_ignore_ascii_case(ty))
    else {
        return false;
    };
    let (member, rest) = number.split_at(number.find('/').unwrap_or(number.len()));
    let most = stack.family().map_or(0, |f| f.max_members());
    !stack.classic()
        && member
            .parse::<usize>()
            .is_ok_and(|m| (1..=most).contains(&m))
        && stack
            .members
            .iter()
            .any(|m| starts_a_port(&format!("{full}{}{rest}", m.number), stack))
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
                    no_value: false,
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
        // Tab keeps the word as typed (lab C9300-24T 17.6.4: `interface TWE` gives `TWEntyFiveGigE`)
        // and completes only where one keyword begins with it: `IP` and `ip` are one, `ipv6` another
        assert_eq!(s.complete("IP"), None);
        assert_eq!(s.complete("IPV").as_deref(), Some("IPV6 "));
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
    fn the_lines_a_c9200l_held_in_its_configuration_load_on_its_image() {
        // the lab C9200L-48T-4G (17.6.4) held these where its books leave them out or place them
        // in other modes (`data/overrides.json`, platform Catalyst 9200)
        let r = crate::release::Release::parse("17.6").unwrap();
        let e = Engine::new(crate::grammars::grammar("c9200", r).unwrap());
        let stack = crate::platform::parse_stack("C9200L-48T-4G").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.frames.push(Frame::top("config"));
        for (line, mode) in [
            ("ip rcmd rcp-enable", "config"),
            ("ip rcmd rsh-enable", "config"),
            ("ip http client source-interface Vlan1", "config"),
            ("ipv6 nd cache expire 3600", "config"),
            ("ipv6 mld ssm-map enable", "config"),
            ("ipv6 mld ssm-map static ACLA 2001:db8::1", "config"),
            ("fhrp version vrrp v3", "config"),
            ("ip dhcp pool POOLA", "dhcp-config"),
            ("network 192.0.2.0 255.255.255.0", "dhcp-config"),
            ("default-router 192.0.2.1", "dhcp-config"),
            ("dns-server 192.0.2.2 192.0.2.3", "dhcp-config"),
            ("exit", "config"),
            ("device-tracking policy POLA", "config-device-tracking"),
            ("no protocol udp", "config-device-tracking"),
            ("exit", "config"),
            ("object-group network OGA", "config-network-group"),
            ("192.0.2.0 255.255.255.0", "config-network-group"),
            ("exit", "config"),
            ("mac access-list extended MACA", "config-ext-macl"),
            ("permit host 0000.5e00.5301 any", "config-ext-macl"),
            ("exit", "config"),
            ("ipv6 access-list V6A", "config-ipv6-acl"),
            ("sequence 10 permit ipv6 any any", "config-ipv6-acl"),
            ("exit", "config"),
            ("ipv6 nd inspection policy NDA", "config-nd-inspection"),
            ("exit", "config"),
            (
                "class-map type multicast-flows MFA",
                "config-mcast-flows-cmap",
            ),
            ("exit", "config"),
            ("policy-map PMA", "config-pmap"),
            ("class class-default", "config-pmap-c"),
            ("set precedence 2", "config-pmap-c"),
            ("exit", "config-pmap"),
            ("exit", "config"),
            ("interface GigabitEthernet1/0/2", "config-if"),
            ("ip dhcp snooping trust", "config-if"),
            ("ip ospf 1 area 0", "config-if"),
            ("ip dhcp client client-id ascii CLIENTA", "config-if"),
            ("vrrp 1 address-family ipv4", "config-if-vrrp"),
            ("exit-vrrp", "config-if"),
            ("exit", "config"),
            ("router ospf 1", "config-router"),
            ("network 192.0.2.0 0.0.0.255 area 0", "config-router"),
            ("exit", "config"),
            ("router eigrp NAMEA", "config-router"),
            (
                "address-family ipv4 unicast autonomous-system 10",
                "config-router-af",
            ),
            ("eigrp stub connected summary", "config-router-af"),
            ("topology base", "config-router-af-topology"),
            ("exit-af-topology", "config-router-af"),
            ("exit-address-family", "config-router"),
            ("exit", "config"),
            ("router eigrp 30", "config-router"),
            ("eigrp stub receive-only", "config-router"),
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
            assert_eq!(s.mode(), mode, "after {line}");
        }
    }

    #[test]
    fn the_submodes_and_forms_a_c9200l_took_load_on_its_image() {
        // the lab C9200L-48T-4G (17.6.4) entered these submodes and took these forms where its
        // books leave them out, run their keywords together or give them other modes
        // (`data/overrides.json`, platform Catalyst 9200)
        let r = crate::release::Release::parse("17.6").unwrap();
        let e = Engine::new(crate::grammars::grammar("c9200", r).unwrap());
        let stack = crate::platform::parse_stack("C9200L-48T-4G").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.frames.push(Frame::top("config"));
        for (line, mode) in [
            // the switch took the AAA lines below on a base holding `aaa new-model`
            ("aaa new-model", "config"),
            ("coap proxy", "config-coap-proxy"),
            ("exit", "config"),
            ("telemetry ietf subscription 101", "config-mdt-subs"),
            ("exit", "config"),
            ("vlan configuration 10-15", "config-vlan-config"),
            ("ipv6 nd suppress", "config-vlan-config"),
            ("exit", "config"),
            ("ipv6 nd raguard policy POLA", "config-nd-raguard"),
            ("exit", "config"),
            ("key chain CHAINA", "config-keychain"),
            ("key 1", "config-keychain-key"),
            ("exit", "config-keychain"),
            ("exit", "config"),
            ("key chain CHAINB macsec", "config-keychain-macsec"),
            ("key 2000", "config-keychain-macsec-key"),
            (
                "cryptographic-algorithm aes-256-cmac",
                "config-keychain-macsec-key",
            ),
            ("exit", "config-keychain-macsec"),
            ("exit", "config"),
            ("key chain CHAINC tcp", "config-keychain-tcp"),
            ("key 7", "config-keychain-tcp-key"),
            ("send-id 21", "config-keychain-tcp-key"),
            ("recv-id 21", "config-keychain-tcp-key"),
            ("key-string KEYA", "config-keychain-tcp-key"),
            (
                "cryptographic-algorithm hmac-sha-1",
                "config-keychain-tcp-key",
            ),
            ("exit", "config-keychain-tcp"),
            ("exit", "config"),
            ("coap proxy", "config-coap-proxy"),
            ("list ipv4 LISTA", "config-coap-proxy-iplist"),
            ("exit", "config-coap-proxy"),
            ("exit", "config"),
            ("ipv6 dhcp guard policy POLB", "config-dhcp-guard"),
            ("exit", "config"),
            ("ipv6 nd suppress policy POLC", "config-nd-suppress"),
            ("exit", "config"),
            (
                "access-session attributes filter-list list LISTB",
                "config-com-filter-list",
            ),
            ("exit", "config"),
            ("vlan access-map VAMA 10", "config-access-map"),
            ("action forward", "config-access-map"),
            ("exit", "config"),
            ("interface GigabitEthernet1/0/3", "config-if"),
            ("mka policy POLD", "config-if"),
            ("macsec replay-protection window-size 10", "config-if"),
            ("mac-address 0000.5e00.5301", "config-if"),
            ("exit", "config"),
            ("mka policy POLD", "config-mka-policy"),
            ("exit", "config"),
            (
                "snmp-server enable traps call-home message-send-fail",
                "config",
            ),
            ("snmp-server enable traps cef inconsistency", "config"),
            ("snmp-server enable traps license deploy", "config"),
            ("snmp-server enable traps ospf lsa", "config"),
            ("snmp-server enable traps vrfmib vnet-trunk-down", "config"),
            (
                "cts sxp connection peer 192.0.2.1 source 192.0.2.2 password none mode local speaker",
                "config",
            ),
            ("interface Vlan20", "config-if"),
            ("ip ospf transmit-delay 2", "config-if"),
            ("ip ospf priority 5", "config-if"),
            ("ip ospf hello-interval 12", "config-if"),
            ("ip ospf dead-interval 8", "config-if"),
            ("ip ospf authentication-key KEYB", "config-if"),
            ("exit", "config"),
            ("time-range TRA", "config-time-range"),
            ("exit", "config"),
            ("table-map TMA", "config-tablemap"),
            ("exit", "config"),
            ("fallback profile FPA", "config-fallback-profile"),
            ("ip admission RULEA", "config-fallback-profile"),
            ("exit", "config"),
            ("dot1x credentials CREDA", "config-dot1x-creden"),
            ("exit", "config"),
            ("crypto pki certificate map MAPA 10", "ca-certificate-map"),
            ("exit", "config"),
            (
                "device-sensor filter-list cdp list CDPA",
                "config-sensor-cdplist",
            ),
            ("tlv name device-name", "config-sensor-cdplist"),
            ("tlv number 10", "config-sensor-cdplist"),
            ("exit", "config"),
            (
                "device-sensor filter-list lldp list LLDPA",
                "config-sensor-lldplist",
            ),
            ("tlv number 34", "config-sensor-lldplist"),
            ("exit", "config"),
            ("policy-map PMA", "config-pmap"),
            ("class class-default", "config-pmap-c"),
            ("police rate 100 pps", "config-pmap-c-police"),
            ("exit", "config-pmap-c"),
            ("random-detect dscp cs1 percent 10 20", "config-pmap-c"),
            ("exit", "config-pmap"),
            ("exit", "config"),
            ("class-map CMA", "config-cmap"),
            ("match dscp af11 af12", "config-cmap"),
            ("match ip precedence 5 6 7", "config-cmap"),
            ("exit", "config"),
            ("route-map RMA permit 10", "config-route-map"),
            ("set tag 5", "config-route-map"),
            ("set metric 10000 10 255 1 1500", "config-route-map"),
            ("match metric external 500 +- 100", "config-route-map"),
            ("exit", "config"),
            ("radius server RSA", "config-radius-server"),
            ("dscp auth 10 acct 20", "config-radius-server"),
            ("exit", "config"),
            ("interface GigabitEthernet1/0/2", "config-if"),
            ("ip ospf cost 8", "config-if"),
            ("exit", "config"),
            ("monitor session 1 filter vlan 100 - 110", "config"),
            ("snmp-server enable traps bridge newroot", "config"),
            (
                "port-channel load-balance extended dst-ip dst-mac src-ip",
                "config",
            ),
            ("cts role-based sgt-map host 192.0.2.1 sgt 3", "config"),
            ("ip dhcp snooping database tftp://192.0.2.9/FILEA", "config"),
            ("tftp-server flash:FILEB", "config"),
            ("version 2", "config"),
        ] {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Ok, "{line}: {out:?}");
            assert_eq!(s.mode(), mode, "after {line}");
        }
    }

    #[test]
    fn a_c9200_runs_the_lisp_process_at_network_advantage_alone() {
        // the lab C9200L-48T-4G (17.6.4) took `router lisp` and its map-cache limit at
        // network-advantage, from global configuration and as a block after a named EIGRP one,
        // and refused `router lisp` under `lisp` at network-essentials
        let r = crate::release::Release::parse("17.6").unwrap();
        let e = Engine::new(crate::grammars::grammar("c9200", r).unwrap());
        let stack = || crate::platform::parse_stack("C9200L-48T-4G").unwrap();
        let mut s = Session::new(&e, Device::new(stack()));
        s.boot_level = Some("network-advantage".into());
        for (line, mode) in [
            ("enable", "privileged"),
            ("configure terminal", "config"),
            ("router eigrp NAMEA", "config-router"),
            ("router lisp", "config-router-lisp"),
            (
                "ipv4 map-cache-limit 100 warning-threshold 80 reserve-list PLA",
                "config-router-lisp",
            ),
            ("ipv4 map-cache-limit 11264", "config-router-lisp"),
            ("exit", "config"),
        ] {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Ok, "{line}: {out:?}");
            assert_eq!(s.mode(), mode, "after {line}");
        }
        assert_eq!(s.type_line("router lisp").1, Result::Ok);
        let (out, r) = s.type_line("ipv4 map-cache-limit 11265");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(s.mode(), "config-router-lisp");
        let mut s = Session::new(&e, Device::new(stack()));
        s.boot_level = Some("network-essentials".into());
        for line in ["enable", "configure terminal"] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        let (out, r) = s.type_line("router lisp");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch(config)#router ".len());
        assert_eq!(s.mode(), "config");
    }

    #[test]
    fn the_lisp_submodes_a_c9200l_took_load_on_its_image() {
        // the lab C9200L-48T-4G (17.6.4 and 17.15.6) at network-advantage entered these LISP
        // submodes, which its books give no mode, took these lines in them and answered these
        // with what the configuration lacked (`data/overrides.json`, platform Catalyst 9200)
        for release in ["17.6", "17.15"] {
            let r = crate::release::Release::parse(release).unwrap();
            let e = Engine::new(crate::grammars::grammar("c9200", r).unwrap());
            let stack = crate::platform::parse_stack("C9200L-48T-4G").unwrap();
            let mut s = Session::new(&e, Device::new(stack));
            s.boot_level = Some("network-advantage".into());
            let said = |s: &mut Session<'_>, line: &str| -> Vec<String> {
                let (out, r) = s.type_line(line);
                assert_eq!(r, Result::Error, "{line}: {out:?}");
                out
            };
            for (line, mode) in [
                ("enable", "privileged"),
                ("configure terminal", "config"),
                ("router lisp", "config-router-lisp"),
                ("locator-set RLOC", "config-router-lisp-locator-set"),
                (
                    "ipv4-interface Loopback0 priority 10 weight 50",
                    "config-router-lisp-locator-set",
                ),
                (
                    "ipv4-interface Loopback 0 priority 10 weight 50 affinity-id 5 ,10",
                    "config-router-lisp-locator-set",
                ),
                ("auto-discover-rlocs", "config-router-lisp-locator-set"),
                ("exit-locator-set", "config-router-lisp"),
                ("locator default-set RLOC", "config-router-lisp"),
                ("prefix-list wired", "config-router-lisp-prefix-list"),
                ("192.168.0.0/16", "config-router-lisp-prefix-list"),
                ("2001:193:168:1::/64", "config-router-lisp-prefix-list"),
                ("exit-prefix-list", "config-router-lisp"),
                ("service ipv4", "config-lisp-srv-ipv4"),
                ("encapsulation vxlan", "config-lisp-srv-ipv4"),
                (
                    "itr map-resolver 2.1.1.6 prefix-list wired",
                    "config-lisp-srv-ipv4",
                ),
                ("map-cache-limit 200", "config-lisp-srv-ipv4"),
                (
                    "route-export destinations-summary route-tag 10",
                    "config-lisp-srv-ipv4",
                ),
                ("sgt distribution", "config-lisp-srv-ipv4"),
                ("use-petr 10.1.1.1", "config-lisp-srv-ipv4"),
                ("exit-service-ipv4", "config-router-lisp"),
                ("service ethernet", "config-lisp-srv-eth"),
                (
                    "dynamic-eid detection multiple-addr bridged-vm",
                    "config-lisp-srv-eth",
                ),
                ("exit-service-ethernet", "config-router-lisp"),
                ("instance-id 3", "config-lisp-inst"),
                ("dynamic-eid Eng.mod", "config-lisp-inst-dyn-eid"),
                ("exit", "config-lisp-inst"),
                ("service ethernet", "config-lisp-inst-srv-eth"),
                ("broadcast-underlay 225.1.1.1", "config-lisp-inst-srv-eth"),
                ("eid-table vlan 250", "config-lisp-inst-srv-eth"),
                (
                    "database-mapping mac locator-set RLOC",
                    "config-lisp-inst-srv-eth",
                ),
                ("fast-detection", "config-lisp-inst-srv-eth"),
                ("exit-service-ethernet", "config-lisp-inst"),
                // another instance from inside this one, as a line of `router lisp`
                ("instance-id 4", "config-lisp-inst"),
                ("service ipv4", "config-lisp-inst-srv-ipv4"),
                ("eid-table default", "config-lisp-inst-srv-ipv4"),
                ("exit-service-ipv4", "config-lisp-inst"),
                ("router lisp", "config-router-lisp"),
                ("extranet ext1", "config-router-lisp-extranet"),
                (
                    "eid-record-provider instance-id 4097",
                    "config-router-lisp-extranet-eid",
                ),
                ("exit-eid-record-provider", "config-router-lisp-extranet"),
                (
                    "extranet-config-from-transit",
                    "config-router-lisp-extranet",
                ),
                ("exit", "config-router-lisp"),
                ("exit-router-lisp", "config"),
            ] {
                let (out, r) = s.type_line(line);
                assert_eq!(r, Result::Ok, "{release} {line}: {out:?}");
                assert_eq!(s.mode(), mode, "{release} after {line}");
            }
            for line in ["router lisp", "instance-id 3"] {
                s.type_line(line);
            }
            assert_eq!(
                said(&mut s, "service ipv4"),
                [
                    "% Instance ID 3 has 'service ethernet' enabled.",
                    " L2 and L3 services are exclusive, remove L2 service first."
                ]
            );
            // lines the switch refused in an instance, which its services take
            for line in [
                "eid-table default",
                "database-mapping 172.168.0.0/16 locator-set RLOC proxy",
                "sgt 100",
            ] {
                assert!(caret_at(&said(&mut s, line)) > 0, "{line}");
                assert_eq!(s.mode(), "config-lisp-inst");
            }
            s.type_line("service ethernet");
            assert_eq!(
                said(&mut s, "database-mapping mac locator-set rloc2"),
                ["% No locator-set named 'rloc2'."]
            );
            for line in [
                "exit-service-ethernet",
                "exit",
                "instance-id 4",
                "service ipv4",
            ] {
                s.type_line(line);
            }
            assert_eq!(s.mode(), "config-lisp-inst-srv-ipv4");
            assert_eq!(
                said(
                    &mut s,
                    "database-mapping 0.0.0.0/0 locator-set NONE default-etr-route-map abc"
                ),
                ["% No locator-set named 'NONE'."]
            );
            assert_eq!(said(&mut s, "eid-table vrf green"), ["% Invalid VRF name!"]);
            for line in ["exit-service-ipv4", "exit", "service ipv4"] {
                s.type_line(line);
            }
            assert_eq!(s.mode(), "config-lisp-srv-ipv4");
            assert_eq!(
                said(&mut s, "use-petr 10.1.1.1 priority 1 weight 100"),
                [
                    "% Cannot configure use-petr entries with and without a priority and weight \
                     specification at the same time."
                ]
            );
            s.type_line("exit-service-ipv4");
            for line in ["prefix-list NO-MARKETING in", "configure terminal"] {
                said(&mut s, line);
                assert_eq!(s.mode(), "config-router-lisp");
            }
        }
    }

    #[test]
    fn a_provision_answer_judges_a_stack_of_the_crawled_switch_s_family_alone() {
        // the lab C9200L-48T-4G listed C9200L models alone after `switch 1 provision`
        const STEPS: &[(&str, &[&str])] = &[
            ("Switch#configure terminal", ENTERED),
            (
                "Switch(config)#switch 1 provision ?",
                &[
                    "  c9200l-24t-4g  Catalyst 9200L Series Switch with 24G, (4x1G)",
                    "  c9200l-48t-4g  Catalyst 9200L Series Switch with 48G, (4x1G)",
                    "",
                ],
            ),
        ];
        let e = held_engine_of("c9200", "17.15", "network-advantage", STEPS);
        let open = |stack: &str| {
            let stack = crate::platform::parse_stack(stack).unwrap();
            let mut s = Session::new(&e, Device::new(stack));
            s.boot_level = Some("network-advantage".into());
            for line in ["enable", "configure terminal"] {
                assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
            }
            s
        };
        // a stack of another family reads the line as the books do
        let mut s = open("C9200-48T");
        for line in ["switch 1 provision c9200-48t", "switch 1 prov c9200-48t"] {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Ok, "{line}: {out:?}");
        }
        // a stack of the crawled switch's family reads the answer
        let mut s = open("C9200L-48T-4G");
        assert_eq!(
            s.type_line("switch 1 provision c9200l-48t-4g").1,
            Result::Ok
        );
        let (out, r) = s.type_line("switch 1 provision c9200-48t");
        assert_eq!(r, Result::Error, "{out:?}");
    }

    #[test]
    fn a_c9200_takes_the_fqdn_ttl_timeout_factor_at_the_level_whose_answer_offers_fqdn() {
        // the lab C9200L-48T-4G (17.15.6) took `fqdn ttl-timeout-factor 100` in a standard access
        // list at network-advantage, leaving for global configuration, and refused `fqdn` there at
        // network-essentials, whose `?` lists none
        let root = |fqdn: bool| -> Vec<&'static str> {
            let mut rows = vec!["Configure commands:"];
            if fqdn {
                rows.push("  fqdn  fqdn config commands");
            }
            rows.extend([
                "  ip    Global IP configuration subcommands",
                "  no    Negate a command or set its defaults",
                "",
            ]);
            rows
        };
        let (adv, ess) = (root(true), root(false));
        let fqdn: &[&str] = &["  ttl-timeout-factor  time-out factor for ttl", ""];
        let factor: &[&str] = &[
            "  <1-300>   Specify ttl timeout factor",
            "  infinite  fqdn entry stays forever",
            "",
        ];
        type Steps<'a> = Vec<(&'a str, &'a [&'a str])>;
        let levels: [(&str, Steps); 2] = [
            (
                "network-advantage",
                vec![
                    ("Switch#configure terminal", ENTERED),
                    ("Switch(config)# ?", &adv),
                    ("Switch(config)#fqdn ?", fqdn),
                    ("Switch(config)#fqdn ttl-timeout-factor ?", factor),
                    ("Switch(config)#end", &[]),
                ],
            ),
            (
                "network-essentials",
                vec![
                    ("Switch#configure terminal", ENTERED),
                    ("Switch(config)# ?", &ess),
                    ("Switch(config)#end", &[]),
                ],
            ),
        ];
        let table = crate::corpus::HeldTable {
            crawl: levels
                .iter()
                .flat_map(|(level, steps)| {
                    crate::corpus::crawled_help(&transcript(steps))
                        .into_iter()
                        .map(|(m, c, k, a)| (level.to_string(), m, c, k, a))
                })
                .collect(),
            ..Default::default()
        };
        let grammar =
            crate::grammars::grammar("c9200", crate::release::Release::parse("17.15").unwrap())
                .unwrap();
        let c = crate::corpus::Corpus {
            held: vec![("c9200".into(), "17.15".into(), table)],
            ..Default::default()
        };
        let held = crate::live::Held::from_corpus(&c, &grammar.platform, &grammar.release);
        let e = Engine::with_held(grammar, held);
        let open = |level: &str| {
            let stack = crate::platform::parse_stack("C9200L-48T-4G").unwrap();
            let mut s = Session::new(&e, Device::new(stack));
            s.boot_level = Some(level.into());
            for line in [
                "enable",
                "configure terminal",
                "ip access-list standard ACLA",
            ] {
                assert_eq!(s.type_line(line).1, Result::Ok, "{level} {line}");
            }
            s
        };
        let mut s = open("network-advantage");
        assert_eq!(s.type_line("fqdn ttl-timeout-factor 100").1, Result::Ok);
        assert_eq!(s.mode(), "config");
        for line in [
            "fqdn ttl-timeout-factor infinite",
            "fqdn ttl-timeout-factor 300",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        assert_eq!(s.type_line("fqdn ttl-timeout-factor 301").1, Result::Error);
        // its no form names the value: another value changes nothing, the one set removes it
        let (out, r) = s.type_line("no fqdn ttl-timeout-factor");
        assert_eq!(out, ["% Incomplete command.", ""]);
        assert_eq!(r, Result::Error);
        let factor = |s: &Session| -> Vec<String> {
            s.device
                .global
                .iter()
                .filter(|e| e.text.starts_with("fqdn "))
                .map(|e| e.text.clone())
                .collect()
        };
        assert_eq!(factor(&s), ["fqdn ttl-timeout-factor 300"]);
        // `infinite` replaces a number
        for (line, after) in [
            (
                "no fqdn ttl-timeout-factor 200",
                "fqdn ttl-timeout-factor 300",
            ),
            (
                "no fqdn ttl-timeout-factor infinite",
                "fqdn ttl-timeout-factor 300",
            ),
            (
                "fqdn ttl-timeout-factor infinite",
                "fqdn ttl-timeout-factor infinite",
            ),
            (
                "no fqdn ttl-timeout-factor 300",
                "fqdn ttl-timeout-factor infinite",
            ),
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
            assert_eq!(factor(&s), [after], "{line}");
        }
        assert_eq!(
            s.type_line("no fqdn ttl-timeout-factor infinite").1,
            Result::Ok
        );
        assert!(factor(&s).is_empty());
        let mut s = open("network-essentials");
        let (out, r) = s.type_line("fqdn ttl-timeout-factor 100");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch(config-std-nacl)#".len());
        assert_eq!(s.mode(), "config-std-nacl");
    }

    #[test]
    fn a_c9200l_refuses_a_port_mode_change_under_port_control_and_an_auth_proxy_rule_it_lacks() {
        // the lab C9200L-48T-4G (17.6.4)
        let r = crate::release::Release::parse("17.6").unwrap();
        let e = Engine::new(crate::grammars::grammar("c9200", r).unwrap());
        let stack = crate::platform::parse_stack("C9200L-48T-4G").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        let said = |s: &mut Session, line: &str| {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Error, "{line}: {out:?}");
            out.join("\n")
        };
        // port control needs `aaa new-model`, which the switch's base held
        for line in [
            "enable",
            "configure terminal",
            "aaa new-model",
            "interface GigabitEthernet1/0/5",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        // a port without port control takes a mode change
        assert_eq!(s.type_line("switchport mode dot1q-tunnel").1, Result::Ok);
        assert_eq!(
            s.type_line("authentication port-control auto").1,
            Result::Ok
        );
        for line in [
            "no switchport",
            "switchport mode dot1q-tunnel",
            "switchport mode private-vlan promiscuous",
        ] {
            assert_eq!(
                said(&mut s, line),
                "Command rejected: Conflict with Authentication. \nAuthentication must be \
                 disabled before changing port mode ",
                "{line}"
            );
        }
        for line in ["switchport mode trunk", "switchport mode private-vlan host"] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        // an auth proxy rule the configuration does not define, then one it does
        assert_eq!(
            said(&mut s, "ip admission RULEA"),
            "%Auth-proxy name RULEA is not defined\n"
        );
        for line in [
            "exit",
            "ip admission name RULEA proxy http",
            "interface GigabitEthernet1/0/5",
            "ip admission RULEA",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
    }

    #[test]
    fn an_auth_proxy_rule_a_port_names_undefined_is_refused_on_the_lab_c9200_images_alone() {
        let at = |release: &str| {
            let r = crate::release::Release::parse(release).unwrap();
            let e = Engine::new(crate::grammars::grammar("c9200", r).unwrap());
            let stack = crate::platform::parse_stack("C9200L-48T-4G").unwrap();
            let mut s = Session::new(&e, Device::new(stack));
            for line in [
                "enable",
                "configure terminal",
                "interface GigabitEthernet1/0/5",
            ] {
                assert_eq!(s.type_line(line).1, Result::Ok, "{release} {line}");
            }
            let (out, r) = s.type_line("ip admission RULEA");
            (out.join("\n"), r)
        };
        // the lab C9200L-48T-4G, at 17.6.4 and at 17.15.6
        for release in ["17.6", "17.15"] {
            assert_eq!(
                at(release),
                (
                    "%Auth-proxy name RULEA is not defined\n".to_string(),
                    Result::Error
                ),
                "{release}"
            );
        }
        // a release of the book the lab switch did not run takes it as its books do
        assert_eq!(at("17.12"), (String::new(), Result::Ok));
    }

    #[test]
    fn a_certificate_s_hex_is_read_until_quit_and_prints_as_typed() {
        // a 9300 at 17.15, no capture of whose chains vswitch holds
        let r = crate::release::Release::parse("17.15").unwrap();
        let e = Engine::new(crate::grammars::grammar("c9300", r).unwrap());
        let stack = crate::platform::parse_stack("C9300-48P").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.frames.push(Frame::top("config"));
        // a trustpoint's lines in its own mode, before the chain that names it
        for line in [
            "crypto pki trustpoint TP-self-signed-42",
            " enrollment selfsigned",
            " revocation-check none",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        assert_eq!(s.mode(), "ca-trustpoint");
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

    /// `c9300` with no donor answering, so the books answer where the image holds nothing.
    fn c9300_books() -> Engine {
        let r = crate::release::Release::parse("17.15").unwrap();
        let g = crate::grammars::grammar("c9300", r).unwrap();
        let held =
            crate::live::Held::with_donors(crate::corpus::get(), &g.platform, &g.release, &[]);
        Engine::with_held(g, held)
    }

    /// A C9300-48P in global configuration with each line typed, every one taken.
    fn configured<'a>(e: &'a Engine, lines: &[&str]) -> Session<'a> {
        let stack = crate::platform::parse_stack("C9300-48P").unwrap();
        let mut s = Session::new(e, Device::new(stack));
        for line in ["enable", "configure terminal"].iter().chain(lines) {
            // a boot level is the one the switch booted at, as a configuration loaded states it
            let level = line.starts_with("license boot level ");
            s.redacted = level;
            let (out, r) = s.type_line(line);
            s.redacted = false;
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
                "speed 100",
                "duplex half",
                "duplex full",
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
                "neighbor 10.0.0.1 remote-as 65001",
                "address-family ipv4",
                "neighbor 10.0.0.1 activate",
                "exit",
                "address-family ipv6",
                "neighbor 10.0.0.1 activate",
                "end",
                "configure terminal",
                "router bgp 65000",
                "bgp log-neighbor-changes",
                "neighbor 10.0.0.2 remote-as 65002",
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
                " neighbor 10.0.0.1 remote-as 65001",
                " bgp log-neighbor-changes",
                " neighbor 10.0.0.2 remote-as 65002",
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
        // IOS makes the port-channel when its first member joins, with the member's switch port
        // lines, and keeps it
        assert_eq!(
            block_of(&s, "interface Port-channel5"),
            ["interface Port-channel5"]
        );
        let mut t = configured(
            &e,
            &[
                "interface GigabitEthernet1/0/2",
                "switchport mode trunk",
                "description x",
                "channel-group 6 mode on",
            ],
        );
        assert_eq!(
            block_of(&t, "interface Port-channel6"),
            ["interface Port-channel6", " switchport mode trunk"]
        );
        // one that exists keeps its own
        t.type_line("interface GigabitEthernet1/0/3");
        t.type_line("switchport mode access");
        t.type_line("channel-group 6 mode on");
        assert_eq!(
            block_of(&t, "interface Port-channel6"),
            ["interface Port-channel6", " switchport mode trunk"]
        );
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
        // unnamed VLANs print on one line
        assert!(run.contains(&"vlan 10,30,40 ".to_string()), "{run:?}");
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
    fn a_2960g_takes_six_port_channels_and_prints_one_port_s_configuration() {
        let r = crate::release::Release::parse("15.0(2)SE").unwrap();
        let e = Engine::new(crate::grammars::grammar("c2960", r).unwrap());
        let stack = crate::platform::parse_stack("WS-C2960G-24TC-L").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.type_line("enable");
        // refused past the digits that still name a port channel
        let (out, r) = s.type_line("show spanning-tree interface Po10");
        assert_eq!(r, Result::Error);
        assert_eq!(out[0], format!("{}^", " ".repeat(39)));
        s.type_line("configure terminal");
        assert_eq!(s.type_line("interface Port-channel6").1, Result::Ok);
        assert_eq!(s.type_line("interface Port-channel7").1, Result::Error);
        s.type_line("interface GigabitEthernet0/3");
        s.type_line("switchport mode trunk");
        s.type_line("end");
        assert_eq!(
            s.type_line("show running-config interface GigabitEthernet0/3")
                .0,
            [
                "Building configuration...",
                "",
                "Current configuration : 59 bytes",
                "!",
                "interface GigabitEthernet0/3",
                " switchport mode trunk",
                "end",
                "",
            ]
        );
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
    fn a_held_interface_list_offers_the_session_s_physical_types() {
        // the C9300 17.6 crawl's `interface ?` lists Fo, Gi, Te and Twe: on another stack its
        // physical rows follow that stack, the logical ones stay as held
        let e = c9300();
        let at = |spec: &str| {
            let mut s = Session::new(&e, Device::new(crate::platform::parse_stack(spec).unwrap()));
            s.type_line("enable");
            s.type_line("configure terminal");
            s
        };
        let words = |h: &[String]| -> Vec<String> {
            h.iter()
                .filter_map(|l| l.split_whitespace().next().map(str::to_string))
                .collect()
        };
        let phys = |h: &[String]| -> Vec<String> {
            words(h)
                .into_iter()
                .filter(|w| cli::PHYSICAL_TYPES.iter().any(|(t, _)| t == w))
                .collect()
        };
        // a C9300 lists every slot-1 type; the 48UXM's 2.5G ports add a type the crawl lacks
        let mgig = at("C9300-48UXM");
        let h = mgig.help("interface ");
        assert_eq!(
            phys(&h),
            [
                "FortyGigabitEthernet",
                "GigabitEthernet",
                "TenGigabitEthernet",
                "TwentyFiveGigE",
                "TwoGigabitEthernet"
            ],
            "{h:?}"
        );
        // a type no held answer describes prints bare, in ASCII order among the held rows
        assert!(h.contains(&"  TwoGigabitEthernet".to_string()), "{h:?}");
        let w = words(&h);
        let pos = |x: &str| w.iter().position(|y| y == x).unwrap();
        assert!(pos("TwentyFiveGigE") < pos("TwoGigabitEthernet"));
        assert!(pos("TwoGigabitEthernet") < pos("VirtualPortGroup"));
        assert!(
            h.contains(&"  Vlan                  Catalyst Vlans".to_string()),
            "{h:?}"
        );
        // two C9300X-24Y with 100G modules: no 40G row, so the column narrows to AppGigabitEthernet
        let x = at("C9300X-24Y+C9300X-NM-4C,C9300X-24Y+C9300X-NM-4C");
        let h = x.help("interface ");
        assert!(
            !phys(&h).contains(&"FortyGigabitEthernet".to_string()),
            "{h:?}"
        );
        assert!(phys(&h).contains(&"HundredGigE".to_string()), "{h:?}");
        assert!(
            h.contains(&"  Vlan                Catalyst Vlans".to_string()),
            "{h:?}"
        );
        let w = words(&h);
        for logical in [
            "AccessTunnel",
            "Loopback",
            "Port-channel",
            "Vlan",
            "nve",
            "range",
        ] {
            assert!(w.contains(&logical.to_string()), "{logical}: {h:?}");
        }
        // Tab reads the same list
        assert_eq!(mgig.complete("interface tw"), None, "TwentyFive and Two");
        assert_eq!(
            mgig.complete("interface two").as_deref(),
            Some("interface twoGigabitEthernet ")
        );
        assert_eq!(
            mgig.complete("interface fo").as_deref(),
            Some("interface fortyGigabitEthernet ")
        );
        assert_eq!(x.complete("interface fo"), None);
        assert_eq!(
            x.complete("interface hu").as_deref(),
            Some("interface hundredGigE ")
        );
        // an answer with no physical-type row is not a type list and stays as held
        assert_eq!(mgig.help("switchport mode "), x.help("switchport mode "));
    }

    #[test]
    fn a_held_answer_at_the_user_level_drops_rows_a_raised_command_takes() {
        let e = c9300();
        // the parent at level 1 by a line of its own: only `show version` stays raised
        let mut s = configured(
            &e,
            &[
                "privilege exec level 14 show version",
                "privilege exec level 1 show",
            ],
        );
        s.type_line("end");
        let has = |h: &[String], w: &str| h.iter().any(|l| l.split_whitespace().next() == Some(w));
        let at_hash = s.help("show ");
        assert!(
            has(&at_hash, "version") && has(&at_hash, "clock"),
            "{at_hash:?}"
        );
        assert_eq!(s.complete("show vers").as_deref(), Some("show version "));
        s.type_line("disable");
        assert_eq!(s.mode(), "exec");
        for typed in ["show ", "sh ", "SHO  "] {
            let h = s.help(typed);
            assert!(!has(&h, "version"), "{typed:?}: {h:?}");
            assert!(has(&h, "clock"), "{typed:?}: {h:?}");
        }
        assert_eq!(s.complete("show vers"), None);
        assert_eq!(s.complete("sh cloc").as_deref(), Some("sh clock "));
        // a word begun, the books' list as much as a held one, and the raised command's own `?`
        let h = s.help("show v");
        assert!(!has(&h, "version") && has(&h, "vlan"), "{h:?}");
        let books = |s: &Session<'_>, typed: &str| -> Vec<String> {
            s.offered_at("exec", &crate::cli::tokens(typed))
                .0
                .into_iter()
                .map(|o| o.shown)
                .collect()
        };
        let plain = configured(&e, &[]);
        assert!(books(&plain, "show").contains(&"version".to_string()));
        for typed in ["show", "sh"] {
            let b = books(&s, typed);
            assert!(!b.contains(&"version".to_string()), "{typed:?}: {b:?}");
            assert!(b.contains(&"vlan".to_string()), "{typed:?}: {b:?}");
        }
        assert_eq!(s.help("show version "), ["% Unrecognized command"]);
        assert_eq!(s.type_line("show version").1, Result::Error);
    }

    #[test]
    fn a_privilege_level_is_0_to_15_and_a_parent_set_before_takes_the_command_s() {
        let e = c9300();
        let mut s = configured(&e, &[]);
        // lab C9300-24T 17.6.4: `Switch(config)#` and the caret under the second `9`
        let (out, r) = s.type_line("privilege exec level 99 show version");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(out[1], "% Invalid input detected at '^' marker.");
        assert_eq!(caret_at(&out), "Switch(config)#".len() + 22, "{out:?}");
        assert!(shown(&s, &["privilege"]).is_empty());
        assert_eq!(
            s.type_line("privilege exec level 15 show version").1,
            Result::Ok
        );
        // a start raised before is taken to the command's level and printed after it
        let s = configured(
            &e,
            &[
                "privilege exec level 5 show",
                "privilege exec level 14 show version",
            ],
        );
        assert_eq!(
            shown(&s, &["privilege"]),
            [
                "privilege exec level 14 show version",
                "privilege exec level 14 show"
            ]
        );
        // three words: each start, longest first
        let s = configured(&e, &["privilege exec level 14 show ip route"]);
        assert_eq!(
            shown(&s, &["privilege"]),
            [
                "privilege exec level 14 show ip route",
                "privilege exec level 14 show ip",
                "privilege exec level 14 show"
            ]
        );
    }

    #[test]
    fn privilege_lines_print_in_the_parse_tree_s_order_whatever_order_they_were_typed_in() {
        let e = c9300();
        // the captures' order (a 2960 at 15.2(6)E, a 9200 at 17.15), typed backwards
        let captured = [
            "rcommand",
            "connect",
            "telnet",
            "tunnel",
            "access-enable",
            "traceroute",
            "systat",
            "ping",
            "where",
            "resume",
            "name-connection",
            "login",
            "terminal",
            "show",
            "clear",
        ];
        let lines: Vec<String> = captured
            .iter()
            .map(|c| format!("privilege exec level 15 {c}"))
            .collect();
        let backwards: Vec<&str> = lines.iter().rev().map(String::as_str).collect();
        let s = configured(&e, &backwards);
        assert_eq!(shown(&s, &["privilege"]), lines);
        // a sibling no switch has ordered keeps its typed place after the listed ones
        let s = configured(
            &e,
            &[
                "privilege exec level 14 show clock",
                "privilege exec level 14 show version",
                "privilege exec level 15 ping",
            ],
        );
        assert_eq!(
            shown(&s, &["privilege"]),
            [
                "privilege exec level 15 ping",
                "privilege exec level 14 show version",
                "privilege exec level 14 show clock",
                "privilege exec level 14 show"
            ]
        );
    }

    #[test]
    fn a_number_past_its_range_is_refused_under_the_digit_that_passes_the_top() {
        let e = c9300();
        let mut s = configured(&e, &[]);
        // lab C9300-24T 17.6.4: the fifth digit of 70000 (max 61440)
        let (out, r) = s.type_line("spanning-tree vlan 1 priority 70000");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch(config)#".len() + 34, "{out:?}");
        // under the range, and a word, at its start
        let (out, _) = s.type_line("spanning-tree vlan 1 priority abc");
        assert_eq!(caret_at(&out), "Switch(config)#".len() + 30, "{out:?}");
    }

    #[test]
    fn a_vlan_past_4094_is_rejected_as_the_vlan_list_reader_rejects_it() {
        let e = c9300();
        let mut s = configured(&e, &[]);
        // lab C9300-24T 17.6.4: two lines, no caret, the prompt stays global configuration
        for (line, at) in [("vlan 5000", 5), ("vlan 4095", 5), ("vlan 10000", 6)] {
            assert_eq!(
                s.type_line(line),
                (
                    vec![
                        format!(
                            "Command rejected: Bad VLAN list - character #{at} (EOL) delimits a VLAN"
                        ),
                        "number which is out of the range 1..4094.".to_string(),
                    ],
                    Result::Error
                ),
                "{line}"
            );
            assert_eq!(s.mode(), "config");
        }
        assert!(shown(&s, &["vlan 5000", "vlan 4095"]).is_empty());
        assert_eq!(s.type_line("vlan 4094").1, Result::Ok);
        assert_eq!(s.mode(), "config-vlan");
    }

    #[test]
    fn a_privilege_line_takes_a_command_of_several_words_and_raises_its_starts() {
        let e = c9300();
        // lab C9300-24T 17.6.4: the line is taken, and its parent printed after it at its level
        let mut s = configured(&e, &["privilege exec level 14 show version"]);
        assert_eq!(
            shown(&s, &["privilege"]),
            [
                "privilege exec level 14 show version",
                "privilege exec level 14 show"
            ]
        );
        // printed as IOS walks its parse tree: longer forms first, `ip` before `version` (lab
        // C9300-24T 17.6.4)
        let (out, r) = s.type_line("privilege exec level 14 show ip route");
        assert_eq!(r, Result::Ok, "{out:?}");
        assert_eq!(
            shown(&s, &["privilege"]),
            [
                "privilege exec level 14 show ip route",
                "privilege exec level 14 show ip",
                "privilege exec level 14 show version",
                "privilege exec level 14 show"
            ]
        );
        // at `>` the whole path is raised: `show` is neither listed, answered nor completed
        s.type_line("end");
        s.type_line("disable");
        let listed: Vec<String> = s
            .help("")
            .iter()
            .filter_map(|l| l.split_whitespace().next().map(str::to_string))
            .collect();
        assert!(listed.contains(&"enable".to_string()), "{listed:?}");
        assert!(!listed.contains(&"show".to_string()), "{listed:?}");
        assert_eq!(s.help("show "), ["% Unrecognized command"]);
        // a word begun after it is answered the same, abbreviated or not, and completes nothing
        for typed in ["show v", "sh ver", "show ip r"] {
            assert_eq!(s.help(typed), ["% Unrecognized command"], "{typed:?}");
        }
        assert_eq!(s.complete("show vers"), None);
        assert_eq!(s.complete("sh cloc"), None);
        assert_eq!(s.complete("sho"), None);
        assert_eq!(s.type_line("show version").1, Result::Error);
        // the one-word form the captures hold prints alone
        let s = configured(&e, &["privilege exec level 15 show"]);
        assert_eq!(shown(&s, &["privilege"]), ["privilege exec level 15 show"]);
        // a configuration holding the command without its parent raises the parent with it
        let mut s = configured(&e, &[]);
        s.device.global.push(Entry {
            key: "privilege #exec show version".into(),
            text: "privilege exec level 14 show version".into(),
        });
        s.type_line("end");
        s.type_line("disable");
        assert_eq!(s.help("show "), ["% Unrecognized command"]);
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
                " neighbor 10.0.0.1 remote-as 65001",
                " neighbor 10.0.0.2 remote-as 65002",
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
                " neighbor 10.0.0.1 remote-as 65001",
                " neighbor 10.0.0.2 remote-as 65002",
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
        // a number past the range under the digit that takes it past 512 (lab C9300-24T 17.6.4:
        // `terminal length 600` under the last `0`), a word at its start
        for (line, at) in [
            ("terminal length 600", 25),
            ("terminal length abc", 23),
            ("terminal width 600", 24),
            ("terminal width 999", 24),
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
        // `?` and Tab know both words and the range: the books', and where a real switch's answer
        // is held (the C9300 17.6 crawl), its rows with their descriptions
        let books = c9300_books();
        let mut b = Session::new(&books, Device::new(s.device.stack.clone()));
        b.type_line("enable");
        assert!(b.help("terminal ").iter().any(|l| l.trim() == "width"));
        assert_eq!(b.help("terminal width ")[0].trim_end(), "  <0-512>");
        for s in [&s, &b] {
            assert!(
                s.help("terminal ")
                    .iter()
                    .any(|l| l.split_whitespace().next() == Some("width"))
            );
            assert!(s.help("terminal width ")[0].starts_with("  <0-512>"));
            assert!(s.help("terminal length ")[0].starts_with("  <0-512>"));
            assert_eq!(
                s.complete("terminal wi").as_deref(),
                Some("terminal width ")
            );
        }
        let width = format!("  {:<22}  Set width of the display terminal", "width");
        assert!(s.help("terminal ").contains(&width));
        assert_eq!(
            s.help("terminal width ")[0],
            "  <0-512>  Number of characters on a screen line"
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

    /// A crawl's log as a session transcript: each step is the prompt and what was typed, then the
    /// lines IOS printed.
    fn transcript(steps: &[(&str, &[&str])]) -> String {
        steps
            .iter()
            .flat_map(|(typed, out)| std::iter::once(*typed).chain(out.iter().copied()))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    /// The crawl the tests below hold: global configuration's `?` with a row printed twice, each router
    /// protocol and interface family asked under the `(config-router)#` and `(config-if)#` prompts they
    /// share, an address family under its router, and `do-exec show ?`.
    fn crawl() -> String {
        const CONF: &[&str] = &["Enter configuration commands, one per line.  End with CNTL/Z."];
        let sections: &[(&str, &[&str])] = &[
            ("Switch#configure terminal", CONF),
            (
                "Switch(config)# ?",
                &[
                    "Configure commands:",
                    "  aaa        Authentication, Authorization and Accounting.",
                    "  auto       Configure Automation",
                    "  auto       Configure Automation",
                    "  interface  Select an interface to configure",
                    "  license    Configure license features",
                    "  no         Negate a command or set its defaults",
                    "  router     Enable a routing process",
                    "",
                ],
            ),
            (
                "Switch(config)#aaa ?",
                &[
                    "  new-model  Enable NEW access control commands and functions.(Disables OLD commands.)",
                ],
            ),
            (
                "Switch(config)#no aaa ?",
                &[
                    "  new-model  Enable NEW access control commands and functions.(Disables OLD commands.)",
                ],
            ),
            (
                "Switch(config)#default aaa ?",
                &[
                    "  new-model  Enable NEW access control commands and functions.(Disables OLD commands.)",
                ],
            ),
            (
                "Switch(config)#alias ?",
                &[
                    "  exec          Exec (prompt) mode",
                    "  filter        Filter mode",
                    "  filter-group  Filter group mode",
                ],
            ),
            (
                "Switch(config)#snmp-server enable traps stackwise ?",
                &[
                    "  member-upgrade-notification  Enable STACKWISE member to be reloaded for upgrade trap",
                    "  new-master                   Enable STACKWISE new master trap",
                    "  new-member                   Enable STACKWISE stack new member trap",
                    "  port-change                  Enable STACKWISE port change trap",
                    "  ring-redundant               Enable STACKWISE stack ring redundant trap",
                    "  <cr>                         <cr>",
                ],
            ),
            (
                "Switch(config)#snmp-server enable traps stackwise member-upgrade-notification ?",
                &[
                    "  new-master      Enable STACKWISE new master trap",
                    "  new-member      Enable STACKWISE stack new member trap",
                    "  port-change     Enable STACKWISE port change trap",
                    "  ring-redundant  Enable STACKWISE stack ring redundant trap",
                    "  <cr>            <cr>",
                ],
            ),
            (
                "Switch(config)#snmp-server enable traps stackwise new-master ?",
                &[
                    "  member-upgrade-notification  Enable STACKWISE member to be reloaded for upgrade trap",
                    "  new-member                   Enable STACKWISE stack new member trap",
                    "  port-change                  Enable STACKWISE port change trap",
                    "  ring-redundant               Enable STACKWISE stack ring redundant trap",
                    "  <cr>                         <cr>",
                ],
            ),
            (
                "Switch(config)#snmp-server enable traps stackwise port-change ?",
                &[
                    "  member-upgrade-notification  Enable STACKWISE member to be reloaded for upgrade trap",
                    "  new-master                   Enable STACKWISE new master trap",
                    "  new-member                   Enable STACKWISE stack new member trap",
                    "  ring-redundant               Enable STACKWISE stack ring redundant trap",
                    "  <cr>                         <cr>",
                ],
            ),
            (
                // a new row: a subcommand, not a pick of the set
                "Switch(config)#snmp-server enable traps stackwise ring-redundant ?",
                &[
                    "  <1-10>                       Ring priority",
                    "  member-upgrade-notification  Enable STACKWISE member to be reloaded for upgrade trap",
                    "  new-master                   Enable STACKWISE new master trap",
                    "  port-change                  Enable STACKWISE port change trap",
                ],
            ),
            (
                "Switch(config)#snmp-server enable traps stackwise new-master port-change ?",
                &[
                    "  member-upgrade-notification  held after both picks",
                    "  <cr>                         <cr>",
                ],
            ),
            (
                "Switch(config)#do-exec show ?",
                &["  vlan  VTP VLAN status", "  zzz   held only by the crawl"],
            ),
            (
                "Switch(config)#do-exec show frob ?",
                &["  zeta  held by the crawl", "  zeus  held by the crawl"],
            ),
            (
                "Switch(config)#alias filter ?",
                &["% Ambiguous command:  \"alias filter \""],
            ),
            (
                "Switch(config)#do-exec show aliases filter ?",
                &["% Ambiguous command:  \"do-exec show aliases filter \""],
            ),
            (
                "Switch(config)#event manager applet 10 class test ?",
                &["% Unrecognized command"],
            ),
            ("Switch(config)#router ospf 1", &[]),
            (
                "Switch(config-router)# ?",
                &["  area  OSPF area parameters"],
            ),
            (
                "Switch(config-router)#area ?",
                &["  <0-4294967295>  OSPF area ID as a decimal value"],
            ),
            ("Switch(config-router)#exit", &[]),
            ("Switch(config)#router bgp 65000", &[]),
            (
                "Switch(config-router)# ?",
                &[
                    "  neighbor        Specify a neighbor router",
                    "  address-family  Enter Address Family command mode",
                ],
            ),
            ("Switch(config-router)#address-family ipv4", &[]),
            (
                "Switch(config-router-af)# ?",
                &["  network  Specify a network to announce via BGP"],
            ),
            ("Switch(config-router-af)#end", &[]),
            ("Switch#configure terminal", CONF),
            ("Switch(config)#router eigrp 1", &[]),
            (
                "Switch(config-router)# ?",
                &["  metric  Modify metrics and parameters for advertisement"],
            ),
            ("Switch(config-router)#exit", &[]),
            ("Switch(config)#router eigrp NAMED", &[]),
            (
                "Switch(config-router)# ?",
                &["  address-family  Enter Address Family command mode"],
            ),
            ("Switch(config-router)#exit", &[]),
            ("Switch(config)#interface GigabitEthernet1/0/1", &[]),
            (
                "Switch(config-if)# ?",
                &["  switchport  Set switching mode characteristics"],
            ),
            ("Switch(config-if)#interface Vlan10", &[]),
            (
                "Switch(config-if)# ?",
                &["  ip  Interface Internet Protocol config commands"],
            ),
            ("Switch(config-if)#interface Loopback0", &[]),
            (
                "Switch(config-if)# ?",
                &["  ipv6  IPv6 interface subcommands"],
            ),
            ("Switch(config-if)#end", &[]),
        ];
        transcript(sections)
    }

    const ADVANTAGE: &str = "network-advantage";
    const LICENSE: &str = "license boot level network-advantage addon dna-advantage";

    /// A C9300 at 17.6 whose books are set aside for what `crawl()` held, at network-advantage.
    fn crawled() -> Engine {
        let r = crate::release::Release::parse("17.6").unwrap();
        let grammar = crate::grammars::grammar("c9300", r).unwrap();
        let c = crate::corpus::Corpus {
            held: vec![("c9300".into(), "17.6".into(), crawled_table())],
            ..Default::default()
        };
        let held = crate::live::Held::from_corpus(&c, &grammar.platform, &grammar.release);
        Engine::with_held(grammar, held)
    }

    /// `crawl()`'s answers at network-advantage, as the corpus holds them.
    fn crawled_table() -> crate::corpus::HeldTable {
        crate::corpus::HeldTable {
            crawl: crate::corpus::crawled_help(&crawl())
                .into_iter()
                .map(|(m, c, k, a)| (ADVANTAGE.to_string(), m, c, k, a))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_session_s_level_is_its_license_line_else_its_book_s_default() {
        let at = |book: &str, release: &str, lines: &[&str]| {
            let r = crate::release::Release::parse(release).unwrap();
            let e = Engine::new(crate::grammars::grammar(book, r).unwrap());
            let s = configured(&e, lines);
            s.level()
        };
        assert_eq!(
            at("c9300", "17.6", &[]).as_deref(),
            Some("network-advantage")
        );
        assert_eq!(
            at("c9300", "17.6", &[LICENSE]).as_deref(),
            Some("network-advantage")
        );
        assert_eq!(
            at(
                "c9200",
                "17.15",
                &["license boot level network-essentials addon dna-essentials"]
            )
            .as_deref(),
            Some("network-essentials")
        );
        assert_eq!(
            at("c9400", "17.5", &[]).as_deref(),
            Some("network-advantage")
        );
        assert_eq!(
            at("ie3300", "17.6", &[]).as_deref(),
            Some("network-essentials")
        );
        assert_eq!(at("c2960x", "15.2(7)E", &[]).as_deref(), Some("lanbase"));
        assert_eq!(at("c2960", "15.0(2)SE", &[]).as_deref(), Some("lanbase"));
    }

    #[test]
    fn an_image_s_own_crawl_answers_at_its_level_alone() {
        let (e, b) = (crawled(), books_17_6());
        let books = configured(&b, &[]).help("alias ");
        // a fresh session states no level and runs network-advantage, as stated does
        for lines in [&[][..], &[LICENSE][..]] {
            let held = configured(&e, lines).help("alias ");
            assert_eq!(held[0].split_whitespace().next(), Some("exec"), "{lines:?}");
            assert_ne!(held, books);
        }
        // another level: the books
        let essentials = "license boot level network-essentials addon dna-essentials";
        assert_eq!(configured(&e, &[essentials]).help("alias "), books);
        let in_ospf = |lines: &[&str]| {
            let mut all = lines.to_vec();
            all.push("router ospf 1");
            configured(&e, &all).help("")
        };
        assert_eq!(in_ospf(&[]), in_ospf(&[LICENSE]));
        assert_ne!(in_ospf(&[essentials]), in_ospf(&[]));
    }

    /// A C9300 at 17.6 with no crawl, whose sweeps measured on Enter: `show widget` forms (the books
    /// read `widget` as a word and nothing after it) and a `show clock` refusal at
    /// network-advantage, an interface form at network-essentials.
    fn measured() -> Engine {
        let row = |level: &str, line: &str, taken: &str| {
            (
                level.to_string(),
                line.to_string(),
                taken.to_string(),
                String::new(),
            )
        };
        let essentials = "network-essentials";
        let table = crate::corpus::HeldTable {
            swept: vec![
                row(ADVANTAGE, "show widget gadget", ""),
                row(ADVANTAGE, "show widget gadgetry", ""),
                row(ADVANTAGE, "show widget <number:1-10> detail", ""),
                row(ADVANTAGE, "show widget knob extra", ""),
                row(ADVANTAGE, "show widget knobs", ""),
                row(ADVANTAGE, "show clock", "invalid 1 2"),
                row(essentials, "show widget <interface:Vlan>", ""),
            ],
            ..Default::default()
        };
        let r = crate::release::Release::parse("17.6").unwrap();
        let grammar = crate::grammars::grammar("c9300", r).unwrap();
        let c = crate::corpus::Corpus {
            held: vec![("c9300".into(), "17.6".into(), table)],
            ..Default::default()
        };
        let held = crate::live::Held::from_corpus(&c, &grammar.platform, &grammar.release);
        Engine::with_held(grammar, held)
    }

    fn no_model(out: &[String]) -> bool {
        out.first()
            .is_some_and(|l| l.starts_with("% vswitch has no output model for '"))
    }

    /// `measured`'s C9300 with forms whose sweep printed `%` lines: a fixed message, a notice, a
    /// state message naming the line's VRF (in a form of its own, whose crawled answer offers a
    /// keyword, and in one beside the books') and two naming the spanning-tree mode, one of them on
    /// a line the shell warns of.
    fn measured_with_messages() -> Engine {
        let row = |line: &str, said: &str| {
            (
                ADVANTAGE.to_string(),
                line.to_string(),
                String::new(),
                said.to_string(),
            )
        };
        let table = crate::corpus::HeldTable {
            swept: vec![
                row("show widget gadget", "% Error: Unsupported command"),
                row(
                    "show widget knobs",
                    "% Command accepted but obsolete, unreleased or unsupported; see documentation.",
                ),
                row("show widget vrf <word>", "% No VRF named {3}"),
                row("show widget mode", "% Switch is not in mst mode"),
                row("show ip vrf <word>", "% No VRF named {3}"),
                row(
                    "show spanning-tree mst configuration",
                    "% Switch is not in mst mode",
                ),
            ],
            // the crawl's answer after `show widget vrf`, a keyword among its rows
            crawl: vec![(
                ADVANTAGE.to_string(),
                "privileged".to_string(),
                String::new(),
                "show widget vrf ".to_string(),
                vec![
                    "  WORD    VRF name".to_string(),
                    "  select  Select".to_string(),
                ],
            )],
            ..Default::default()
        };
        let r = crate::release::Release::parse("17.6").unwrap();
        let grammar = crate::grammars::grammar("c9300", r).unwrap();
        let c = crate::corpus::Corpus {
            held: vec![("c9300".into(), "17.6".into(), table)],
            ..Default::default()
        };
        let held = crate::live::Held::from_corpus(&c, &grammar.platform, &grammar.release);
        Engine::with_held(grammar, held)
    }

    #[test]
    fn a_measured_form_prints_the_switch_s_message_where_it_stands() {
        let e = measured_with_messages();
        let mut s = configured(&e, &["end"]);
        let typed = |s: &mut Session, line: &str| s.type_line(line);
        assert_eq!(
            typed(&mut s, "sh wid gadg"),
            (vec!["% Error: Unsupported command".into()], Result::Error)
        );
        let (out, r) = typed(&mut s, "show widget knobs");
        assert_eq!(r, Result::Ok);
        assert!(
            out[0].starts_with("% Command accepted but obsolete"),
            "{out:?}"
        );
        assert!(no_model(&out[1..]), "{out:?}");
        // the VRF the line names, as typed
        assert_eq!(
            typed(&mut s, "show widget vrf RED"),
            (vec!["% No VRF named RED".into()], Result::Error)
        );
        assert_eq!(
            typed(&mut s, "show widget mode"),
            (vec!["% Switch is not in mst mode".into()], Result::Error)
        );
        // the shell's warning before the region's configuration stands, its output after it
        let (out, r) = typed(&mut s, "show spanning-tree mst configuration");
        assert_eq!(r, Result::Ok, "{out:?}");
        assert!(
            out[0] == "% Switch is not in mst mode" && out.len() > 1,
            "{out:?}"
        );
        // the feature configured: the line prints what it printed before
        let mut s = configured(
            &e,
            &[
                "vrf definition RED",
                "exit",
                "spanning-tree mode mst",
                "end",
            ],
        );
        for line in ["show widget vrf RED", "show widget mode"] {
            let (out, r) = typed(&mut s, line);
            assert!(r == Result::Ok && no_model(&out), "{line}: {out:?}");
        }
        assert_eq!(
            typed(&mut s, "show widget vrf BLUE"),
            (vec!["% No VRF named BLUE".into()], Result::Error)
        );
        assert_eq!(typed(&mut s, "show widget gadget").1, Result::Error);
        // a word the books or the crawl read as a keyword where it stands is that keyword's line
        for line in ["show ip vrf detail", "show widget vrf select"] {
            let (out, _) = typed(&mut s, line);
            assert!(
                !out.iter().any(|l| l.contains("No VRF named")),
                "{line}: {out:?}"
            );
        }
    }

    #[test]
    fn a_form_the_switch_answered_is_taken_at_its_level_alone() {
        let e = measured();
        let mut adv = configured(&e, &["end"]);
        let mut ess = configured(&e, &[ESSENTIALS, "end"]);
        for line in [
            "show widget gadget",
            "sh wid gadgetr",
            "show widget 10 detail",
            "show widget 1 det",
        ] {
            let (out, r) = adv.type_line(line);
            assert!(r == Result::Ok && no_model(&out), "{line}: {out:?}");
            let (out, r) = ess.type_line(line);
            assert!(r == Result::Error && !no_model(&out), "{line}: {out:?}");
        }
        for line in ["show widget Vlan10", "show widget vl10"] {
            let (out, r) = ess.type_line(line);
            assert!(r == Result::Ok && no_model(&out), "{line}: {out:?}");
            let (_, r) = adv.type_line(line);
            assert_eq!(r, Result::Error, "{line}");
        }
    }

    #[test]
    fn a_form_s_value_is_taken_in_its_range_alone() {
        let e = measured();
        let mut s = configured(&e, &["end"]);
        let (out, r) = s.type_line("show widget 11 detail");
        assert_eq!(r, Result::Error);
        // past the digits the range's top allows, as for a book's range (`past_the_top`)
        assert_eq!(caret_at(&out), "Switch#show widget 1".len());
        let help = s.help("show widget ");
        let first: Vec<&str> = help
            .iter()
            .filter_map(|l| l.split_whitespace().next())
            .collect();
        for w in ["<1-10>", "gadget", "gadgetry", "knob", "knobs"] {
            assert!(first.contains(&w), "{w}: {help:?}");
        }
        assert!(!first.contains(&"Vlan"), "{help:?}");
        assert!(
            s.help("show widget 4 ")[0]
                .trim_start()
                .starts_with("detail")
        );
    }

    #[test]
    fn help_and_tab_offer_a_level_s_forms_where_no_answer_is_held() {
        let e = measured();
        let adv = configured(&e, &["end"]);
        let ess = configured(&e, &[ESSENTIALS, "end"]);
        // a whole keyword among the words begun rings the bell
        assert_eq!(adv.complete("show widget gad"), None);
        assert_eq!(
            adv.complete("show widget gadgetr").as_deref(),
            Some("show widget gadgetry ")
        );
        assert_eq!(ess.complete("show widget gadgetr"), None);
        let help = ess.help("show widget ");
        let first: Vec<&str> = help
            .iter()
            .filter_map(|l| l.split_whitespace().next())
            .collect();
        assert!(first.contains(&"Vlan"), "{help:?}");
        assert!(!first.contains(&"gadget"), "{help:?}");
    }

    #[test]
    fn a_refusal_the_switch_gave_stands_over_the_books_with_its_caret() {
        let e = measured();
        let mut adv = configured(&e, &["end"]);
        let mut ess = configured(&e, &[ESSENTIALS, "end"]);
        // the switch's caret two characters into `clock`, for the line and its abbreviations
        for (line, at) in [("show clock", 7 + 5 + 2), ("sh clock", 7 + 3 + 2)] {
            let (out, r) = adv.type_line(line);
            assert_eq!(r, Result::Error, "{line}: {out:?}");
            assert_eq!(caret_at(&out), at, "{line}");
            assert_eq!(out[1], "% Invalid input detected at '^' marker.");
            let (out, r) = ess.type_line(line);
            assert_eq!(r, Result::Ok, "{line}: {out:?}");
        }
        // the line alone: a longer one is the books', refused at `detail`
        let (out, _) = adv.type_line("show clock detail");
        assert_eq!(caret_at(&out), "Switch#show clock ".len(), "{out:?}");
    }

    #[test]
    fn a_whole_keyword_is_never_the_start_of_a_longer_form() {
        let e = measured();
        let mut s = configured(&e, &["end"]);
        // `knob` is a keyword of `show widget knob extra`, so it is not read as `knobs`
        let (out, _) = s.type_line("show widget knob");
        assert!(!no_model(&out), "{out:?}");
        let (out, r) = s.type_line("show widget knobs");
        assert!(r == Result::Ok && no_model(&out), "{out:?}");
    }

    /// A C9300 17.6 table whose crawls at both levels answer `?` at the privileged prompt, with forms
    /// the sweep measured at network-advantage: network-essentials' `show ?` leaves out `gadget`,
    /// and after `show widget` it takes a word where network-advantage takes a number. Its
    /// essentials sweep refused `show widget detail` under `detail`.
    fn carried_table() -> crate::corpus::HeldTable {
        let essentials = "network-essentials";
        let answer = |rows: &[&str]| rows.iter().map(|r| format!("  {r}")).collect::<Vec<_>>();
        let mut crawl = vec![];
        for level in [ADVANTAGE, essentials] {
            let at = |key: &str, rows: &[&str]| {
                (
                    level.to_string(),
                    "privileged".to_string(),
                    String::new(),
                    key.to_string(),
                    answer(rows),
                )
            };
            let (show, widget): (&[&str], &[&str]) = if level == ADVANTAGE {
                (
                    &["gadget  Gadget", "widget  Widget"],
                    &["<1-10>  Number", "brief  Brief", "detail  Detail"],
                )
            } else {
                (
                    &["widget  Widget"],
                    &["WORD  Name", "brief  Brief", "detail  Detail"],
                )
            };
            crawl.extend([
                at("", &["show  Show running system information"]),
                at("show ", show),
                at("show widget ", widget),
                at("show widget brief ", &["<cr>  <cr>"]),
                at("show widget detail ", &["<cr>  <cr>"]),
            ]);
        }
        let row = |level: &str, line: &str, taken: &str| {
            (
                level.to_string(),
                line.to_string(),
                taken.to_string(),
                String::new(),
            )
        };
        crate::corpus::HeldTable {
            crawl,
            swept: vec![
                row(ADVANTAGE, "show gadget", ""),
                row(ADVANTAGE, "show widget brief", ""),
                row(ADVANTAGE, "show widget detail", ""),
                row(ADVANTAGE, "show widget <number:1-10>", ""),
                row(essentials, "show widget detail", "invalid 2 0"),
            ],
            ..Default::default()
        }
    }

    /// An engine of `book` at `release` on a corpus holding `carried_table` as the C9300 17.6's,
    /// with the donor mappings `enabled` lists.
    fn carried_engine(book: &str, release: &str, enabled: &[(&str, &str, &str)]) -> Engine {
        let r = crate::release::Release::parse(release).unwrap();
        let grammar = crate::grammars::grammar(book, r).unwrap();
        let c = crate::corpus::Corpus {
            held: vec![("c9300".into(), "17.6".into(), carried_table())],
            ..Default::default()
        };
        let held = crate::live::Held::with_donors(&c, &grammar.platform, &grammar.release, enabled);
        Engine::with_held(grammar, held)
    }

    #[test]
    fn a_form_of_the_level_above_is_taken_where_both_crawls_carry_its_words() {
        let e = carried_engine("c9300", "17.6", &[]);
        let mut ess = configured(&e, &[ESSENTIALS, "end"]);
        for line in ["show widget brief", "sh wid br"] {
            let (out, r) = ess.type_line(line);
            assert!(r == Result::Ok && no_model(&out), "{line}: {out:?}");
        }
        // a keyword the level's `?` leaves out stays refused, under it
        let (out, r) = ess.type_line("show gadget");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch#show ".len());
        // a value the level types otherwise: a word there, a number above
        let (out, r) = ess.type_line("show widget 5");
        assert!(r == Result::Error && !no_model(&out), "{out:?}");
        // the level's own measured refusal stands over a carried form
        let (out, r) = ess.type_line("show widget detail");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch#show widget ".len());
        // at the level above, each of them is its form
        let mut adv = configured(&e, &["end"]);
        for line in ["show gadget", "show widget 5", "show widget detail"] {
            let (out, r) = adv.type_line(line);
            assert!(r == Result::Ok && no_model(&out), "{line}: {out:?}");
        }
    }

    #[test]
    fn no_form_is_carried_through_a_donor_or_from_another_release() {
        let essentials = "network-essentials";
        for (book, release) in [("c9200", "17.15"), ("c9300", "17.15")] {
            let on = [(book, release, essentials), (book, release, ADVANTAGE)];
            let e = carried_engine(book, release, &on);
            let mut s = configured(&e, &[ESSENTIALS, "end"]);
            let (out, r) = s.type_line("show widget brief");
            assert!(
                r == Result::Error && !no_model(&out),
                "{book} {release}: {out:?}"
            );
        }
    }

    #[test]
    fn help_and_tab_after_no_offer_no_form() {
        let e = measured();
        let s = configured(&e, &["end"]);
        assert!(s.help("show widget ").iter().any(|l| l.contains("gadget")));
        let after_no = s.help("no show widget ");
        assert!(
            !after_no.iter().any(|l| l.contains("gadget")),
            "{after_no:?}"
        );
        assert_eq!(s.complete("no show widget gadgetr"), None);
        // and Enter reads none there
        let mut s = configured(&e, &["end"]);
        let (out, _) = s.type_line("no show widget gadget");
        assert!(!no_model(&out), "{out:?}");
    }

    /// A C9200 at 17.15 whose donor is `crawl()`'s C9300 17.6 crawl at network-advantage, with that
    /// mapping on where `on`.
    fn lent(on: bool) -> Engine {
        let donor = crawled_table();
        let c = crate::corpus::Corpus {
            held: vec![
                ("c9200".into(), "17.15".into(), Default::default()),
                ("c9300".into(), "17.6".into(), donor),
            ],
            ..Default::default()
        };
        let r = crate::release::Release::parse("17.15").unwrap();
        let grammar = crate::grammars::grammar("c9200", r).unwrap();
        let enabled: &[(&str, &str, &str)] = if on {
            &[("c9200", "17.15", ADVANTAGE)]
        } else {
            &[]
        };
        let held = crate::live::Held::with_donors(&c, &grammar.platform, &grammar.release, enabled);
        Engine::with_held(grammar, held)
    }

    #[test]
    fn a_donor_crawl_answers_a_switch_at_its_level_where_its_mapping_is_on() {
        let (on, off) = (lent(true), lent(false));
        let ask = |e: &Engine, lines: &[&str], typed: &str| configured(e, lines).help(typed);
        let held = ask(&on, &[LICENSE], "alias ");
        assert_eq!(held[0].split_whitespace().next(), Some("exec"));
        assert_ne!(held, ask(&off, &[LICENSE], "alias "));
        // an abbreviation read against the donor's answers
        let aaa = ask(&on, &[LICENSE], "aa ");
        assert_eq!(aaa[0].split_whitespace().next(), Some("new-model"));
        assert_ne!(aaa, ask(&off, &[LICENSE], "aa "));
        // in the donor's context only
        let ospf = ask(&on, &[LICENSE, "router ospf 1"], "");
        assert_eq!(ospf[0].split_whitespace().next(), Some("area"));
        assert_eq!(
            ask(&on, &[LICENSE, "router rip"], ""),
            ask(&off, &[LICENSE, "router rip"], "")
        );
        // a fresh session runs network-advantage; at another level, the books
        assert_eq!(ask(&on, &[], "alias "), held);
        let essentials = ["license boot level network-essentials addon dna-essentials"];
        assert_eq!(
            ask(&on, &essentials, "alias "),
            ask(&off, &essentials, "alias ")
        );
        // a donor's answer makes no line taken: Enter reads the books as before
        for line in ["alias exec x show version", "bogus line"] {
            assert_eq!(
                configured(&on, &[LICENSE]).type_line(line),
                configured(&off, &[LICENSE]).type_line(line),
                "{line}"
            );
        }
    }

    #[test]
    fn a_global_command_typed_whole_in_a_submode_is_not_read_as_a_submode_abbreviation() {
        // the 17.6 book lists `router ospfv3` under config-if; the lab C9300 (17.6.4) printed
        // `router ospf 1` typed after an interface block
        let e = books_17_6();
        let s = configured(
            &e,
            &[
                "interface Loopback0",
                "router ospf 1",
                " network 192.0.2.1 0.0.0.0 area 0",
            ],
        );
        assert_eq!(s.mode(), "config-router");
        let text = crate::show::configuration(&s.device).join("\n");
        assert!(
            text.contains("router ospf 1\n network 192.0.2.1 0.0.0.0 area 0"),
            "{text}"
        );
        assert!(!text.contains("ospfv3"), "{text}");
    }

    #[test]
    fn a_key_typed_after_key_string_prints_as_the_key_hash_it_makes() {
        // the lab C9300 (17.6.4): a key's lines make `key-hash` with the MD5 of its blob and the
        // words after it; a typed hash prints in upper case with a space after it; every line
        // before `exit` is key text; no text keeps no key and says nothing
        let e = books_17_6();
        let typed = format!("ssh-rsa {} two words here", crate::pubkey::LAB_KEY);
        let mut lines = vec!["ip ssh pubkey-chain", "username lab8i", "key-string"];
        lines.extend(
            typed
                .as_bytes()
                .chunks(72)
                .map(|c| std::str::from_utf8(c).unwrap()),
        );
        lines.extend([
            "exit",
            "exit",
            "username lab8d",
            "key-hash ssh-rsa 0123456789abcdef0123456789abcdef",
            "exit",
            "username lab8k",
            "key-string",
        ]);
        let mut s = configured(&e, &lines);
        assert_eq!(s.type_line("exit"), (vec![], Result::Ok));
        for line in ["exit", "username lab8x", "key-string", "end"] {
            s.type_line(line);
        }
        assert_eq!(s.mode(), "conf-ssh-pubkey-data");
        let (out, r) = s.type_line("exit");
        assert_eq!(
            (out, r),
            (
                vec!["%SSH: Failed to decode the Key Value".to_string()],
                Result::Ok
            )
        );
        assert_eq!(s.mode(), "conf-ssh-pubkey-user");
        let text = crate::show::configuration(&s.device).join("\n");
        let want = "ip ssh pubkey-chain\n  username lab8i\n   key-hash ssh-rsa \
            F7D30FD3AC9B60E00F3E2A65685F80D5 two words here\n  username lab8d\n   key-hash \
            ssh-rsa 0123456789ABCDEF0123456789ABCDEF \n  username lab8k\n  username lab8x\n";
        assert!(text.contains(want), "{text}");
        assert!(!text.contains("key-string"), "{text}");
    }

    #[test]
    fn the_smart_licensing_address_prints_with_its_note() {
        // the lab C9300 (17.6.4): the address says what it means when set, and the running
        // configuration prints the note above it and above no other address
        let e = books_17_6();
        let mut s = configured(&e, &["call-home"]);
        let (out, r) = s.type_line("contact-email-addr sch-smart-licensing@cisco.com");
        assert_eq!(r, Result::Ok);
        assert_eq!(
            out,
            vec![
                "The email address configured in Cisco Smart License Portal will be used as \
                 contact email address to send SCH notifications."
                    .to_string()
            ]
        );
        let text = crate::show::configuration(&s.device).join("\n");
        let want = format!(
            "call-home\n {}\n {}\n contact-email-addr sch-smart-licensing@cisco.com\n",
            crate::show::CALL_HOME_NOTE[0],
            crate::show::CALL_HOME_NOTE[1]
        );
        assert!(text.contains(&want), "{text}");
        let (out, _) = s.type_line("contact-email-addr noc@example.com");
        assert!(out.is_empty(), "{out:?}");
        let text = crate::show::configuration(&s.device).join("\n");
        assert!(!text.contains("! If contact"), "{text}");
    }

    #[test]
    fn separators_between_top_level_lines_are_as_many_as_ios_prints() {
        // the lab C9300 (17.6.4): one class-map follows another with no `!` between them, where
        // each block used to close with one
        let e = books_17_6();
        let s = configured(
            &e,
            &[
                "class-map match-any lab-a",
                "description one",
                "exit",
                "class-map match-any lab-b",
                "description two",
            ],
        );
        let text = crate::show::configuration(&s.device).join("\n");
        assert!(
            text.contains("  description one\nclass-map match-any lab-b\n"),
            "{text}"
        );
    }

    /// A C9300 at 17.6 on its books and the captures' held lines, without the crawls' answers.
    fn books_17_6() -> Engine {
        let r = crate::release::Release::parse("17.6").unwrap();
        let grammar = crate::grammars::grammar("c9300", r).unwrap();
        let mut c = crate::corpus::get().clone();
        for (_, _, h) in &mut c.held {
            h.crawl.clear();
        }
        let held = crate::live::Held::from_corpus(&c, &grammar.platform, &grammar.release);
        Engine::with_held(grammar, held)
    }

    #[test]
    fn a_crawl_answers_each_context_under_a_shared_prompt_with_its_own() {
        let e = crawled();
        let ask = |lines: &[&str]| configured(&e, lines).help("");
        let cases: [(&[&str], &str); 8] = [
            (&["router ospf 1"], "area"),
            (&["router ospf 100"], "area"),
            (&["router bgp 65000"], "neighbor"),
            (&["router eigrp 1"], "metric"),
            (&["router eigrp NAMED"], "address-family"),
            (&["router bgp 65000", "address-family ipv4"], "network"),
            (&["interface GigabitEthernet1/0/9"], "switchport"),
            (&["interface Vlan10"], "ip"),
        ];
        for (lines, first) in cases {
            let got = ask(lines);
            assert_eq!(
                got[0].split_whitespace().next(),
                Some(first),
                "{lines:?}: {got:?}"
            );
            // the crawl's own rows, not the books' answer
            assert!(got.len() <= 2, "{lines:?}: {got:?}");
        }
        assert_eq!(
            ask(&["interface Loopback0"]),
            ["  ipv6  IPv6 interface subcommands"]
        );
        // a context the crawl never entered takes none of another's answer: the books answer
        let books = configured(&books_17_6(), &["interface Port-channel1"]).help("");
        assert_eq!(ask(&["interface Port-channel1"]), books);
        assert!(books.len() > 1, "{books:?}");
        let books = configured(&books_17_6(), &["router rip"]).help("");
        assert_eq!(ask(&["router rip"]), books);
    }

    #[test]
    fn the_shell_and_the_corpus_give_a_prompt_the_same_context() {
        let e = books_17_6();
        let stack = crate::platform::parse_stack("C9300-48P").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        let script = [
            "enable",
            "configure terminal",
            "router ospf 1",
            "exit",
            "router bgp 65000",
            "address-family ipv4",
            "exit-address-family",
            "exit",
            "router eigrp 1",
            "exit",
            "router eigrp NAMED",
            "end",
            "configure terminal",
            "interface GigabitEthernet1/0/1",
            "interface Vlan10",
            "interface Loopback0",
            "interface range GigabitEthernet1/0/5 - 6",
            "interface Port-channel1",
            "exit",
            "policy-map TESTPOLICY",
            "class class-default",
            "exit",
            "exit",
            "key chain TESTKEYS",
            "key 1",
            "end",
        ];
        let mut log = vec![];
        let mut shell_context = vec![];
        for line in script {
            log.push(format!("{}{line}", s.prompt()));
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Ok, "{line}: {out:?}");
            log.extend(out);
            log.push(format!("{} ?", s.prompt()));
            log.push(format!("  answer-{}", shell_context.len()));
            shell_context.push(s.context().expect("a context"));
        }
        let (_, ex) = crate::capture::exchanges(&log.join("\n")).unwrap();
        let probes: Vec<_> = ex.iter().filter(|e| e.command.ends_with('?')).collect();
        assert_eq!(probes.len(), script.len());
        for (probe, want) in probes.iter().zip(&shell_context) {
            assert_eq!(probe.context.as_deref(), Some(want.as_str()), "{probe:?}");
        }
        let distinct: std::collections::BTreeSet<_> = shell_context.iter().collect();
        assert!(distinct.len() >= 12, "{distinct:?}");
        assert!(shell_context.iter().all(|c| !c.contains("TEST")));
    }

    #[test]
    fn a_held_crawl_answer_replays_with_its_duplicate_rows_and_spacing() {
        let e = crawled();
        let s = configured(&e, &[]);
        assert_eq!(
            s.help(""),
            [
                "Configure commands:",
                "  aaa        Authentication, Authorization and Accounting.",
                "  auto       Configure Automation",
                "  auto       Configure Automation",
                "  interface  Select an interface to configure",
                "  license    Configure license features",
                "  no         Negate a command or set its defaults",
                "  router     Enable a routing process",
                "",
            ]
        );
    }

    #[test]
    fn a_crawl_answer_asked_where_the_session_is_not_asked_there() {
        let e = crawled();
        // global configuration's own answer is no answer inside a submode the crawl asked in
        let in_ospf = configured(&e, &["router ospf 1"]);
        assert!(
            !in_ospf
                .help("")
                .contains(&"Configure commands:".to_string())
        );
        // a context the session cannot name holds no crawl answer
        let mut unknown = configured(&e, &["router ospf 1"]);
        unknown.frames.last_mut().unwrap().entered.clear();
        let books = configured(&books_17_6(), &["router ospf 1"]).help("");
        assert_eq!(unknown.help(""), books);
    }

    #[test]
    fn a_crawl_answer_that_depends_on_aaa_holds_only_without_it() {
        let e = crawled();
        let mut s = configured(&e, &[]);
        let crawled_answer = vec![
            "  new-model  Enable NEW access control commands and functions.(Disables OLD commands.)"
                .to_string(),
        ];
        assert_eq!(s.help("aaa "), crawled_answer);
        assert_eq!(s.type_line("aaa new-model").1, Result::Ok);
        let books = configured(&books_17_6(), &["aaa new-model"]).help("aaa ");
        assert_ne!(s.help("aaa "), crawled_answer);
        assert_eq!(s.help("aaa "), books);
        assert!(books.len() > 1, "{books:?}");
        // `no aaa ?` and `default aaa ?` offer `new-model` alone only while AAA is off, too
        for typed in ["no aaa ", "default aaa "] {
            let books = configured(&books_17_6(), &["aaa new-model"]).help(typed);
            assert_eq!(s.help(typed), books, "{typed}");
            assert_ne!(books, crawled_answer, "{typed}");
            assert_eq!(configured(&e, &[]).help(typed), crawled_answer, "{typed}");
        }
        // the state follows the configuration
        assert_eq!(s.type_line("no aaa new-model").1, Result::Ok);
        assert_eq!(s.help("aaa "), crawled_answer);
        assert_eq!(s.help("no aaa "), crawled_answer);
    }

    const STACKWISE: &str = "snmp-server enable traps stackwise ";

    #[test]
    fn a_keyword_set_after_several_picks_offers_what_every_pick_left_in_the_set_s_order() {
        let e = crawled();
        let s = configured(&e, &[]);
        let want = [
            "  new-master      Enable STACKWISE new master trap",
            "  new-member      Enable STACKWISE stack new member trap",
            "  ring-redundant  Enable STACKWISE stack ring redundant trap",
            "  <cr>            <cr>",
        ];
        // the column narrows to the longest word left, `<cr>` and its description included
        for picks in [
            "member-upgrade-notification port-change ",
            "port-change member-upgrade-notification ",
            "PORT-CHANGE  member-upgrade-notification ",
            // a pick abbreviated as the answer before it reads it
            "member-upgrade-notification port ",
            "port-c member-up ",
        ] {
            assert_eq!(s.help(&format!("{STACKWISE}{picks}")), want, "{picks}");
        }
    }

    #[test]
    fn an_abbreviated_keyword_takes_the_held_answer_for_it_in_the_session_s_context() {
        let e = crawled();
        let in_ospf = configured(&e, &["router ospf 1"]);
        for typed in ["ar ", "AR  ", "area "] {
            assert_eq!(
                in_ospf.help(typed),
                ["  <0-4294967295>  OSPF area ID as a decimal value"],
                "{typed}"
            );
        }
        // BGP's `?` offers no keyword `ar` begins and its `area ?` is not held: the books answer
        let books = configured(&books_17_6(), &["router bgp 65000"]).help("ar ");
        assert_eq!(configured(&e, &["router bgp 65000"]).help("ar "), books);
        // the start of two keywords is refused, quoting the line typed
        let s = configured(&e, &[]);
        assert_eq!(
            s.help("alias  fil "),
            ["% Ambiguous command:  \"alias  fil \""]
        );
    }

    #[test]
    fn a_held_answer_after_several_picks_wins_over_the_derived_one() {
        let e = crawled();
        let s = configured(&e, &[]);
        assert_eq!(
            s.help(&format!("{STACKWISE}new-master port-change ")),
            [
                "  member-upgrade-notification  held after both picks",
                "  <cr>                         <cr>",
            ]
        );
    }

    #[test]
    fn a_value_a_subcommand_or_an_unheld_pick_leaves_several_picks_to_the_books() {
        let (e, b) = (crawled(), books_17_6());
        let (s, books) = (configured(&e, &[]), configured(&b, &[]));
        for picks in [
            // a value typed where the set offers none ends the picks
            "new-master 5 ",
            "5 new-master port-change ",
            // `ring-redundant ?` offers a row the set does not: not a pick of it
            "new-master ring-redundant ",
            // `new-member ?` was never asked
            "new-master new-member ",
        ] {
            let typed = format!("{STACKWISE}{picks}");
            assert_eq!(s.help(&typed), books.help(&typed), "{picks}");
        }
    }

    #[test]
    fn do_exec_runs_the_rest_of_the_line_at_the_privileged_prompt() {
        let e = c9300();
        let mut s = configured(&e, &["interface Vlan10", "exit"]);
        let by_do = s.type_line("do show running-config | include interface Vlan10");
        assert_eq!(by_do.0, ["interface Vlan10"]);
        assert_eq!(
            s.type_line("do-exec show running-config | include interface Vlan10"),
            by_do
        );
        assert_eq!(
            s.type_line("DO-EXEC  show   vlan   brief"),
            s.type_line("do show vlan brief")
        );
        assert_eq!(s.mode(), "config");
        // nothing but EXEC ran: the configuration holds no line for it
        assert!(
            s.accepted
                .iter()
                .all(|a| !a.line.to_lowercase().starts_with("do"))
        );
        assert_eq!(
            s.type_line("do-exec"),
            (
                vec!["% Incomplete command.".to_string(), String::new()],
                Result::Error
            )
        );
        // a refused word's caret stands after the configuration prompt and `do-exec `
        assert_eq!(caret_at(&s.type_line("do-exec shw run").0), 25);
        assert_eq!(caret_at(&s.type_line("do shw run").0), 20);
        // `?` and Tab read it as EXEC does
        let exec = {
            let mut p = configured(&e, &[]);
            p.type_line("end");
            p.help("show vlan ")
        };
        assert_eq!(s.help("do-exec show vlan "), exec);
        assert_eq!(s.help("do show vlan "), exec);
        assert_eq!(s.help("do-exec show vl"), s.help("do show vl"));
        assert_eq!(s.complete("do-exec sh").as_deref(), Some("do-exec show "));
        // also from a submode
        s.type_line("interface Vlan10");
        assert_eq!(s.help("do-exec show vlan "), exec);
    }

    #[test]
    fn a_crawl_s_do_exec_answer_is_privileged_exec_help() {
        let e = crawled();
        let mut s = configured(&e, &[]);
        let held = vec![
            "  vlan  VTP VLAN status".to_string(),
            "  zzz   held only by the crawl".to_string(),
        ];
        assert_eq!(s.help("do-exec show "), held);
        assert_eq!(s.help("do show "), held);
        s.type_line("end");
        assert_eq!(s.help("show "), held);
        // another context's `do-exec` is the same privileged answer
        s.type_line("configure terminal");
        s.type_line("router ospf 1");
        assert_eq!(s.help("do-exec show "), held);
    }

    #[test]
    fn tab_completes_from_the_answer_that_question_mark_gives() {
        let e = crawled();
        let books = books_17_6();
        let mut s = configured(&e, &[]);
        let plain = configured(&books, &[]);
        // `zzz` is held by the crawl alone: `?` lists it, so Tab takes its unique start
        assert_eq!(plain.complete("do-exec show z"), None);
        for (typed, done) in [
            ("do-exec show z", "do-exec show zzz "),
            ("do show zz", "do show zzz "),
            ("DO-EXEC  show ZZZ", "DO-EXEC  show ZZZ "),
            ("do show ZZ", "do show ZZz "),
        ] {
            assert_eq!(s.complete(typed).as_deref(), Some(done), "{typed}");
        }
        assert_eq!(
            s.complete("do-exec show v").as_deref(),
            Some("do-exec show vlan ")
        );
        // the keywords held are the whole answer: the books' `show s...` words are not offered
        assert_eq!(s.complete("do-exec show s"), None);
        s.type_line("end");
        assert_eq!(s.complete("show z").as_deref(), Some("show zzz "));
        // a start two held keywords share completes nothing, as it does for the books' keywords
        assert_eq!(s.complete("show frob ze"), None);
        assert_eq!(
            s.complete("show frob zet").as_deref(),
            Some("show frob zeta ")
        );
        assert_eq!(
            s.complete("show frob zeu").as_deref(),
            Some("show frob zeus ")
        );
    }

    #[test]
    fn tab_completes_nothing_after_a_held_refusal() {
        let e = crawled();
        let s = configured(&e, &[]);
        let typed = "event manager applet 10 class test ";
        let b = books_17_6();
        let books = configured(&b, &[]);
        let first = crate::live::keywords(&books.help(typed))
            .into_iter()
            .next()
            .expect("the books offer a keyword there");
        let start = &first[..1];
        assert!(books.complete(&format!("{typed}{first}")).is_some());
        assert_eq!(s.help(typed), ["% Unrecognized command"]);
        assert_eq!(s.complete(&format!("{typed}{first}")), None);
        assert_eq!(s.complete(&format!("{typed}{start}")), None);
        // a held ambiguous refusal likewise
        assert_eq!(s.complete("alias filter e"), None);
    }

    #[test]
    fn tab_completes_from_the_books_where_no_answer_is_held_for_the_line_before() {
        let e = crawled();
        let b = books_17_6();
        let (s, books) = (configured(&e, &[]), configured(&b, &[]));
        for typed in ["interface twe", "interface twe1/0/1"] {
            let head = &typed[..=typed.rfind(' ').unwrap()];
            assert_eq!(s.held_answer("config", Some(""), head), None, "{typed}");
            assert_eq!(s.complete(typed), books.complete(typed), "{typed}");
        }
        // a type's start completes; a name with its number is left as typed (lab C9300-24T 17.6.4)
        assert!(books.complete("interface twe").is_some());
        assert_eq!(books.complete("interface twe1/0/1"), None);
        // the first word, where no answer is held for the empty line
        let (mut s, mut books) = (configured(&e, &[]), configured(&b, &[]));
        s.type_line("end");
        books.type_line("end");
        assert_eq!(s.held_answer("privileged", Some(""), ""), None);
        assert_eq!(s.complete("conf").as_deref(), Some("configure "));
        assert_eq!(s.complete("conf"), books.complete("conf"));
        assert_eq!(s.held_answer("privileged", Some(""), "terminal "), None);
        assert_eq!(s.complete("terminal wi"), books.complete("terminal wi"));
        assert!(books.complete("terminal wi").is_some());
    }

    #[test]
    fn a_refusal_the_crawl_held_answers_where_the_books_offer() {
        let e = crawled();
        let s = configured(&e, &[]);
        let typed = "event manager applet 10 class test ";
        let books = configured(&books_17_6(), &[]).help(typed);
        assert!(!crate::live::is_refusal(&books), "{books:?}");
        assert_eq!(s.help(typed), ["% Unrecognized command"]);
        // spaced and cased otherwise, the same line
        assert_eq!(
            s.help("EVENT  manager applet 10   class test "),
            ["% Unrecognized command"]
        );
        // the same line in a submode the crawl did not ask it in is the books'
        let in_ospf = configured(&e, &["router ospf 1"]);
        let books = configured(&books_17_6(), &["router ospf 1"]).help(typed);
        assert_eq!(in_ospf.help(typed), books);
    }

    #[test]
    fn a_held_ambiguous_refusal_quotes_the_line_the_user_typed() {
        let e = crawled();
        let mut s = configured(&e, &[]);
        assert_eq!(
            s.help("alias filter "),
            ["% Ambiguous command:  \"alias filter \""]
        );
        assert_eq!(
            s.help("alias   FILTER "),
            ["% Ambiguous command:  \"alias   FILTER \""]
        );
        // `do-exec`'s answer is privileged EXEC's, quoting the whole line as the C9300 did
        for typed in [
            "do-exec show aliases filter ",
            "do-ex show  aliases filter ",
            "do show aliases filter ",
        ] {
            assert_eq!(
                s.help(typed),
                [format!("% Ambiguous command:  \"{typed}\"")],
                "{typed}"
            );
        }
        s.type_line("end");
        assert_eq!(
            s.help("show aliases filter "),
            ["% Ambiguous command:  \"show aliases filter \""]
        );
    }

    #[test]
    fn a_held_refusal_leaves_an_executed_line_s_caret_where_the_books_put_it() {
        // a crawl that answered every word before its `% Unrecognized command`
        let text = transcript(&[
            (
                "Switch#configure terminal",
                &["Enter configuration commands, one per line.  End with CNTL/Z."],
            ),
            (
                "Switch(config)# ?",
                &["Configure commands:", "  event  Event related commands", ""],
            ),
            (
                "Switch(config)#event ?",
                &["  manager  Event Manager configuration commands"],
            ),
            (
                "Switch(config)#event manager ?",
                &["  applet  Register an Event Manager applet"],
            ),
            (
                "Switch(config)#event manager applet ?",
                &["  WORD  Name of the Event Manager applet"],
            ),
            (
                "Switch(config)#event manager applet 10 ?",
                &["  class  Specify a class for the applet", "  <cr>   <cr>"],
            ),
            (
                "Switch(config)#event manager applet 10 class ?",
                &["  WORD  Class name"],
            ),
            (
                "Switch(config)#event manager applet 10 class test ?",
                &["% Unrecognized command"],
            ),
        ]);
        let table = crate::corpus::HeldTable {
            crawl: crate::corpus::crawled_help(&text)
                .into_iter()
                .map(|(m, c, k, a)| (ADVANTAGE.to_string(), m, c, k, a))
                .collect(),
            ..Default::default()
        };
        let r = crate::release::Release::parse("17.6").unwrap();
        let grammar = crate::grammars::grammar("c9300", r).unwrap();
        let c = crate::corpus::Corpus {
            held: vec![("c9300".into(), "17.6".into(), table)],
            ..Default::default()
        };
        let held = crate::live::Held::from_corpus(&c, &grammar.platform, &grammar.release);
        let crawl = Engine::with_held(grammar, held);
        let books = books_17_6();
        // the books offer `trap` there: the caret stands past `tr`, under the `x`
        for line in [
            "event manager applet 10 class test trx",
            "event  manager   applet 10 class    test trx",
        ] {
            let mut held = configured(&crawl, &[]);
            let mut plain = configured(&books, &[]);
            let (out, r) = held.type_line(line);
            assert_eq!(r, Result::Error, "{line}: {out:?}");
            assert_eq!(out, plain.type_line(line).0, "{line}");
            let x = line.find("trx").unwrap() + 2 + "Switch(config)#".len();
            assert_eq!(caret_at(&out), x, "{line}: {out:?}");
        }
    }

    /// A C9300 at 17.6 holding what the crawl `steps` print, at network-advantage.
    fn held_engine(steps: &[(&str, &[&str])]) -> Engine {
        held_engine_of("c9300", "17.6", ADVANTAGE, steps)
    }

    /// An engine of `book` at `release` holding what the crawl `steps` print, at `level`.
    fn held_engine_of(book: &str, release: &str, level: &str, steps: &[(&str, &[&str])]) -> Engine {
        // a probe is complete once the prompt follows it
        let last = steps
            .last()
            .map_or("Switch#", |(t, _)| &t[..=t.find('#').unwrap()]);
        let closed = format!("{last}end");
        let steps: Vec<(&str, &[&str])> = steps
            .iter()
            .copied()
            .chain([(closed.as_str(), &[][..])])
            .collect();
        let table = crate::corpus::HeldTable {
            crawl: crate::corpus::crawled_help(&transcript(&steps))
                .into_iter()
                .map(|(m, c, k, a)| (level.to_string(), m, c, k, a))
                .collect(),
            ..Default::default()
        };
        let r = crate::release::Release::parse(release).unwrap();
        let grammar = crate::grammars::grammar(book, r).unwrap();
        let c = crate::corpus::Corpus {
            held: vec![(book.into(), release.into(), table)],
            ..Default::default()
        };
        let held = crate::live::Held::from_corpus(&c, &grammar.platform, &grammar.release);
        Engine::with_held(grammar, held)
    }

    const ENTERED: &[&str] = &["Enter configuration commands, one per line.  End with CNTL/Z."];

    #[test]
    fn a_privileged_line_is_judged_by_the_crawl_past_its_first_word() {
        // a crawl that asked `do-exec ?` alone: its answer, which lists no `configure`, stands for
        // the privileged prompt's
        let e = held_engine(&[
            ("Switch#configure terminal", ENTERED),
            (
                "Switch(config)#do-exec ?",
                &["  show  Show running system information", ""],
            ),
            (
                "Switch(config)#do-exec show ?",
                &["  vlan  VTP VLAN status", ""],
            ),
            (
                "Switch(config)#do-exec show vlan ?",
                &["  brief  VTP all VLAN status in brief", "  <cr>   <cr>", ""],
            ),
        ]);
        let books = books_17_6();
        let mut plain = configured(&books, &["end"]);
        let mut s = configured(&e, &["end"]);
        assert_eq!(s.mode(), "privileged");
        // the books take `show clock`, which the crawl's `show ?` leaves out
        assert_eq!(plain.type_line("show clock").1, Result::Ok);
        let (out, r) = s.type_line("show clock");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch#show ".len());
        assert_eq!(s.type_line("show vlan brief").1, Result::Ok);
        assert_eq!(s.type_line("configure terminal").1, Result::Ok);
        assert_eq!(s.mode(), "config");
    }

    #[test]
    fn a_held_unrecognized_command_refuses_a_line_the_books_take_at_its_last_word() {
        let e = held_engine(&[
            ("Switch#configure terminal", ENTERED),
            (
                "Switch(config)#do-exec ?",
                &["  show  Show running system information", ""],
            ),
            (
                "Switch(config)#do-exec show ?",
                &["  vlan  VTP VLAN status", ""],
            ),
            (
                "Switch(config)#do-exec show vlan ?",
                &["  id  VTP VLAN status by VLAN id", ""],
            ),
            (
                "Switch(config)#do-exec show vlan id ?",
                &["  WORD  VLAN id", ""],
            ),
            (
                "Switch(config)#do-exec show vlan id 1 ?",
                &["% Unrecognized command"],
            ),
        ]);
        let books = books_17_6();
        let line = "show vlan id 1";
        assert_eq!(configured(&books, &["end"]).type_line(line).1, Result::Ok);
        let mut s = configured(&e, &["end"]);
        let (out, r) = s.type_line(line);
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch#show vlan id ".len());
        // the words before it abbreviated
        let (out, r) = s.type_line("sh vl id 1");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch#sh vl id ".len());
        // a value the crawl typed other than this line types it is not judged
        assert_eq!(s.type_line("show vlan id 2").1, Result::Ok);
    }

    #[test]
    fn the_privileged_prompt_s_own_answer_places_the_caret_under_its_first_word() {
        // `do-exec ?` asked before the prompt's own `?`, which lists `configure` beside `connect`
        let e = held_engine(&[
            ("Switch#configure terminal", ENTERED),
            (
                "Switch(config)#do-exec ?",
                &[
                    "  clear    Reset functions",
                    "  connect  Open a terminal connection",
                    "",
                ],
            ),
            ("Switch(config)#end", &[]),
            (
                "Switch# ?",
                &[
                    "Exec commands:",
                    "  clear      Reset functions",
                    "  configure  Enter configuration mode",
                    "  connect    Open a terminal connection",
                    "",
                ],
            ),
        ]);
        let mut s = configured(&e, &["end"]);
        let (out, r) = s.type_line("configuration terminal");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch#configur".len());
        let words = |help: Vec<String>| -> Vec<String> {
            help.iter()
                .filter_map(|l| l.strip_prefix("  "))
                .filter_map(|l| l.split_whitespace().next())
                .map(str::to_string)
                .collect()
        };
        assert!(words(s.help("")).contains(&"configure".to_string()));
        // `do-exec ?` answers what the crawl's `do-exec ?` printed
        assert_eq!(s.type_line("configure terminal").1, Result::Ok);
        for typed in ["do-exec ", "do "] {
            assert_eq!(words(s.help(typed)), ["clear", "connect"], "{typed}");
        }
    }

    #[test]
    fn a_refusal_after_a_listed_name_leaves_the_name_to_the_session() {
        // the crawl's switch held one monitor, refused `name` with none by that name, and refused
        // a name it did not hold
        let e = held_engine(&[
            ("Switch#clear ?", &["  flow  Flow information", ""]),
            ("Switch#clear flow ?", &["  monitor  Flow monitor", ""]),
            (
                "Switch#clear flow monitor ?",
                &["  MONA  User defined", "  name  Name of a flow monitor", ""],
            ),
            (
                "Switch#clear flow monitor name ?",
                &["% Unrecognized command"],
            ),
            (
                "Switch#clear flow monitor MONB ?",
                &["% Unrecognized command"],
            ),
        ]);
        let mut s = configured(&e, &["flow monitor MONB", "exit", "end"]);
        for line in [
            "clear flow monitor name MONB",
            "clear flow monitor name MONB statistics",
            "clear flow monitor MONB statistics",
        ] {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Ok, "{line}: {out:?}");
        }
        // a name this session does not hold
        let (out, r) = s.type_line("clear flow monitor name MONC");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch#clear flow monitor name ".len());
        // with no monitor, as the crawl's switch answered `name`
        let mut s = configured(&e, &["end"]);
        let (out, r) = s.type_line("clear flow monitor name MONB");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch#clear flow monitor ".len());
    }

    #[test]
    fn a_flow_object_s_contents_are_judged_only_on_the_image_that_measured_them() {
        const STEPS: &[(&str, &[&str])] = &[
            ("Switch#configure terminal", ENTERED),
            ("Switch(config)#interface GigabitEthernet1/0/1", &[]),
            (
                "Switch(config-if)#ip flow monitor ?",
                &["No monitors configured", "  <cr>  <cr>", ""],
            ),
            ("Switch(config-if)#exit", &[]),
            ("Switch(config)#flow monitor MONA", &[]),
            (
                "Switch(config-flow-monitor)#record ?",
                &[
                    "  RECA     User defined",
                    "  netflow  Traditional NetFlow collection schemes",
                    "",
                ],
            ),
        ];
        let essentials = "network-essentials";
        let at = |e: &Engine, lines: &[&str]| -> Vec<Result> {
            let stack = crate::platform::parse_stack("C9200L-48T-4G").unwrap();
            let mut s = Session::new(e, Device::new(stack));
            s.boot_level = Some(essentials.into());
            for line in ["enable", "configure terminal"].iter().chain(&[
                "flow record RECB",
                "exit",
                "flow monitor MONB",
                "exit",
                "flow monitor MONC",
                "exit",
            ]) {
                assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
            }
            lines.iter().map(|l| s.type_line(l).1).collect()
        };
        // an empty record named in a monitor, and a monitor with no record named on a port
        let lines = [
            "flow monitor MONB",
            "record RECB",
            "exit",
            "interface GigabitEthernet1/0/1",
            "ip flow monitor MONC input",
        ];
        let measured = held_engine_of("c9300", "17.6", essentials, STEPS);
        assert_eq!(
            at(&measured, &lines),
            [
                Result::Ok,
                Result::Error,
                Result::Ok,
                Result::Ok,
                Result::Error
            ]
        );
        // the C9200L took the empty record and answered the monitor without one with a message
        let other = held_engine_of("c9200", "17.6", essentials, STEPS);
        assert_eq!(
            at(&other, &lines),
            [
                Result::Ok,
                Result::Ok,
                Result::Ok,
                Result::Ok,
                Result::Error
            ]
        );
        // a name neither holds is refused on both
        for e in [&measured, &other] {
            assert_eq!(
                at(e, &["flow monitor MONB", "record RECC"]),
                [Result::Ok, Result::Error]
            );
        }
    }

    #[test]
    fn the_c9200_answers_a_monitor_without_a_record_and_refuses_a_sampler_it_lacks() {
        const STEPS: &[(&str, &[&str])] = &[
            ("Switch#configure terminal", ENTERED),
            ("Switch(config)#interface GigabitEthernet1/0/1", &[]),
            (
                "Switch(config-if)#ip flow monitor ?",
                &["No monitors configured", "  <cr>  <cr>", ""],
            ),
            ("Switch(config-if)#exit", &[]),
        ];
        // the lab C9200L-48T-4G answered alike at 17.6.4 and 17.15.6
        for release in ["17.6", "17.15"] {
            measured_flow_monitor(&held_engine_of(
                "c9200",
                release,
                "network-essentials",
                STEPS,
            ));
        }
        // a release of the book the lab switch did not run reads them as its books do
        let e = held_engine_of("c9200", "17.12", "network-essentials", STEPS);
        let mut s = flow_session(&e);
        for line in [
            "ip flow monitor MONA input",
            "exit",
            "flow monitor MONA",
            "record RECA",
            "record RECB",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "17.12 {line}");
        }
    }

    /// A C9200L session holding records RECA and RECB, monitor MONA without one and sampler SAMA,
    /// at a port.
    fn flow_session(e: &Engine) -> Session<'_> {
        let stack = crate::platform::parse_stack("C9200L-48T-4G").unwrap();
        let mut s = Session::new(e, Device::new(stack));
        s.boot_level = Some("network-essentials".into());
        for line in [
            "enable",
            "configure terminal",
            "flow record RECA",
            "exit",
            "flow record RECB",
            "exit",
            "flow monitor MONA",
            "exit",
            "sampler SAMA",
            "interface GigabitEthernet1/0/1",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        s
    }

    fn measured_flow_monitor(e: &Engine) {
        let mut s = flow_session(e);
        let (out, r) = s.type_line("ip flow monitor MONA input");
        assert_eq!(
            (out, r),
            (
                vec![
                    "% Flow Monitor: Failed to add monitor to interface: The monitor does not have \
                     a valid record"
                        .to_string()
                ],
                Result::Error
            )
        );
        let (out, r) = s.type_line("ip flow monitor MONA sampler SAMB input");
        assert_eq!(r, Result::Error);
        assert_eq!(
            caret_at(&out),
            "Switch(config-if)#ip flow monitor MONA sampler ".len()
        );
        for line in ["exit", "flow monitor MONA", "record RECA", "record RECA"] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        let (out, r) = s.type_line("record RECB");
        assert_eq!(
            (out, r),
            (
                vec![
                    "% Flow Monitor: Failed to set record: Already there is an existing record \
                     configured for this monitor"
                        .to_string()
                ],
                Result::Error
            )
        );
        for line in [
            "exit",
            "interface GigabitEthernet1/0/1",
            "ip flow monitor MONA input",
            "ipv6 flow monitor MONA sampler SAMA output",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
    }

    #[test]
    fn a_file_system_row_fits_a_path_after_it_and_no_other_word() {
        let e = held_engine(&[
            ("Switch#configure terminal", ENTERED),
            (
                "Switch(config)#do-exec show ?",
                &[
                    "  flash:  display information about flash: file system",
                    "  vlan    VTP VLAN status",
                    "",
                ],
            ),
        ]);
        let mut s = configured(&e, &["end"]);
        let (out, r) = s.type_line("show clock");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch#show ".len());
        assert_eq!(s.type_line("show flash:").1, Result::Ok);
    }

    #[test]
    fn an_object_the_session_configured_fits_where_the_crawl_listed_none() {
        let e = held_engine(&[
            ("Switch#configure terminal", ENTERED),
            (
                "Switch(config)#do-exec show ?",
                &["  flow  Flow information", ""],
            ),
            (
                "Switch(config)#do-exec show flow ?",
                &["  record  Flow record", ""],
            ),
            (
                "Switch(config)#do-exec show flow record ?",
                &[
                    "  netflow  Traditional NetFlow collection schemes",
                    "  <cr>     <cr>",
                    "",
                ],
            ),
        ]);
        let mut s = configured(&e, &["end"]);
        let (out, r) = s.type_line("show flow record R1");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch#show flow record ".len());
        let mut s = configured(&e, &["flow record R1", "match ipv4 source address", "end"]);
        assert_eq!(s.type_line("show flow record R1").1, Result::Ok);
    }

    #[test]
    fn a_keyword_answered_after_a_configured_objects_answer_is_read_on() {
        // `?` after `show flow exporter` lists the exporters configured, none, and hides `name`
        let e = held_engine(&[
            ("Switch#configure terminal", ENTERED),
            (
                "Switch(config)#do-exec show ?",
                &["  flow  Flow information", ""],
            ),
            (
                "Switch(config)#do-exec show flow ?",
                &["  exporter  Flow Exporter", ""],
            ),
            (
                "Switch(config)#do-exec show flow exporter ?",
                &["No exporters configured", "  <cr>  <cr>", ""],
            ),
            (
                "Switch(config)#do-exec show flow exporter name ?",
                &["No exporters configured", ""],
            ),
        ]);
        let mut s = configured(&e, &["flow exporter E1", "end"]);
        assert_eq!(s.type_line("show flow exporter name E1").1, Result::Ok);
        let (out, r) = s.type_line("show flow exporter E2");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch#show flow exporter ".len());
    }

    /// The crawl's `spanning-tree vlan` answers, asked after `10` and, where `others` is set, after
    /// `20` with `cost` offered too.
    fn spanning_crawl(others: bool) -> Engine {
        let mut steps: Vec<(&str, &[&str])> = vec![
            ("Switch#configure terminal", ENTERED),
            (
                "Switch(config)# ?",
                &[
                    "Configure commands:",
                    "  mpls           Configure MPLS parameters",
                    "  spanning-tree  Spanning Tree Subsystem",
                    "",
                ],
            ),
            (
                "Switch(config)#spanning-tree ?",
                &["  vlan  VLAN Switch Spanning Tree", ""],
            ),
            (
                "Switch(config)#spanning-tree vlan ?",
                &["  WORD  vlan range, example: 1,3-5,7,9-11", ""],
            ),
            (
                "Switch(config)#spanning-tree vlan 10 ?",
                &[
                    "  priority  Set the bridge priority for the spanning tree",
                    "  root      Configure switch as root",
                    "  <cr>      <cr>",
                    "",
                ],
            ),
            (
                "Switch(config)#spanning-tree vlan 10 priority ?",
                &["  <0-61440>  bridge priority in increments of 4096", ""],
            ),
            (
                "Switch(config)#spanning-tree vlan 10 root ?",
                &[
                    "  primary  Configure this switch as primary root for this spanning tree",
                    "",
                ],
            ),
            (
                "Switch(config)#mpls label range ?",
                &[
                    "  <16-1048575>  Minimum label value for dynamic label range",
                    "",
                ],
            ),
            (
                "Switch(config)#mpls label range 16 ?",
                &[
                    "  <16-1048575>  Maximum label value for dynamic label range",
                    "",
                ],
            ),
            (
                "Switch(config)#mpls label range 16 16 ?",
                &[
                    "  static  Specify block of labels for static bindings",
                    "  <cr>    <cr>",
                    "",
                ],
            ),
            (
                "Switch(config)#mpls label range 16 16 static ?",
                &["  <17-1048575>  Upper minimum static label value", ""],
            ),
            (
                "Switch(config)#spanning-tree vlan 10 priority 4096 ?",
                &["  <cr>  <cr>", ""],
            ),
        ];
        if others {
            steps.push((
                "Switch(config)#spanning-tree vlan 20 ?",
                &[
                    "  cost      Change an interface's spanning tree path cost",
                    "  priority  Set the bridge priority for the spanning tree",
                    "  root      Configure switch as root",
                    "  <cr>      <cr>",
                    "",
                ],
            ));
        }
        held_engine(&steps)
    }

    #[test]
    fn a_value_the_crawl_typed_otherwise_reads_the_keyword_after_it_as_after_the_crawl_s() {
        let books = books_17_6();
        let e = spanning_crawl(false);
        let mut s = configured(&e, &[]);
        for line in [
            "spanning-tree vlan 2-4 cost 30",
            "spanning-tree vlan 3,5-6 cost 30",
            "span vlan 7 cost 30",
        ] {
            assert_eq!(
                configured(&books, &[]).type_line(line).1,
                Result::Ok,
                "{line}"
            );
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Error, "{line}: {out:?}");
            let at = "Switch(config)#".len() + line.find("cost").unwrap();
            assert_eq!(caret_at(&out), at, "{line}");
        }
        assert_eq!(
            s.type_line("spanning-tree vlan 2-4 priority 4096").1,
            Result::Ok
        );
        // only the word after the value: the answers further on can depend on it
        let deeper = "spanning-tree vlan 2-4 root secondary";
        assert_eq!(configured(&books, &[]).type_line(deeper).1, Result::Ok);
        assert_eq!(s.type_line(deeper).1, Result::Ok);
        let (out, r) = s.type_line("spanning-tree vlan 10 root secondary");
        assert_eq!(r, Result::Error, "{out:?}");
        // nor a number after it, whose bounds can follow the value (the crawl's `16 16 static`
        // offers 17 and up)
        let label = "mpls label range 100 1000 static 16 99";
        assert_eq!(configured(&books, &[]).type_line(label).1, Result::Ok);
        assert_eq!(s.type_line(label).1, Result::Ok);
        // a number of a `<lo-hi>` row stands for another of the row
        let e = held_engine(&[
            ("Switch#configure terminal", ENTERED),
            ("Switch(config)#interface GigabitEthernet1/0/1", &[]),
            (
                "Switch(config-if)#l2protocol-tunnel drop-threshold ?",
                &[
                    "  <1-4096>  Drop threshold in packets per second",
                    "  cdp       Cisco Discovery Protocol",
                    "",
                ],
            ),
            (
                "Switch(config-if)#l2protocol-tunnel drop-threshold 1 ?",
                &["  <cr>  <cr>", ""],
            ),
        ]);
        let line = "l2protocol-tunnel drop-threshold 100 cdp";
        let mut plain = configured(&books, &["interface GigabitEthernet1/0/1"]);
        assert_eq!(plain.type_line(line).1, Result::Ok);
        let mut s = configured(&e, &["interface GigabitEthernet1/0/1"]);
        let (out, r) = s.type_line(line);
        assert_eq!(r, Result::Error, "{out:?}");
        let at = "Switch(config-if)#".len() + line.find("cdp").unwrap();
        assert_eq!(caret_at(&out), at);
        // the crawl's values answered unlike each other: no value stands for the others
        let others = spanning_crawl(true);
        let mut s = configured(&others, &[]);
        assert_eq!(s.type_line("spanning-tree vlan 2-4 cost 30").1, Result::Ok);
    }

    #[test]
    fn a_routed_port_reads_only_the_objects_an_answer_offers() {
        let e = held_engine(&[
            ("Switch#configure terminal", ENTERED),
            ("Switch(config)#interface GigabitEthernet1/0/1", &[]),
            (
                "Switch(config-if)# ?",
                &[
                    "Interface configuration commands:",
                    "  ip  Interface Internet Protocol config commands",
                    "  no  Negate a command or set its defaults",
                    "",
                ],
            ),
            (
                "Switch(config-if)#ip ?",
                &["  flow  NetFlow Related commands", ""],
            ),
            (
                "Switch(config-if)#ip flow ?",
                &["  monitor  Apply a Flow Monitor", ""],
            ),
            (
                "Switch(config-if)#ip flow monitor ?",
                &["No monitors configured", "  <cr>  <cr>", ""],
            ),
            ("Switch(config-if)#end", &[]),
        ]);
        let mut s = configured(
            &e,
            &[
                "flow monitor M1",
                "exit",
                "interface GigabitEthernet1/0/1",
                "no switchport",
            ],
        );
        // `M1` holds no record: refused on a routed port as on a switchport
        let (out, r) = s.type_line("ip flow monitor M1 input");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch(config-if)#ip flow monitor ".len());
        // a word the switchport's answers leave out is the books' there
        assert_eq!(
            s.type_line("ip address 10.0.0.1 255.255.255.0").1,
            Result::Ok
        );
    }

    #[test]
    fn class_does_not_begin_class_map_on_the_image_whose_switch_refused_it() {
        const STEPS: &[(&str, &[&str])] = &[
            ("Switch#configure terminal", ENTERED),
            (
                "Switch(config)# ?",
                &[
                    "Configure commands:",
                    "  class-map  Configure CPL Class Map",
                    "  clock      Configure time-of-day clock",
                    "",
                ],
            ),
        ];
        // the lab C9200L-48T-4G refused it alike at both levels, at 17.6.4 and at 17.15.6
        for (release, level) in ["17.6", "17.15"]
            .into_iter()
            .flat_map(|r| ["network-essentials", "network-advantage"].map(|l| (r, l)))
        {
            let c9200 = held_engine_of("c9200", release, level, STEPS);
            let stack = crate::platform::parse_stack("C9200L-48T-4G").unwrap();
            let mut s = Session::new(&c9200, Device::new(stack));
            s.boot_level = Some(level.into());
            for line in ["enable", "configure terminal"] {
                assert_eq!(s.type_line(line).1, Result::Ok, "{release} {level} {line}");
            }
            let (out, r) = s.type_line("class C1");
            assert_eq!(r, Result::Error, "{release} {level} {out:?}");
            assert_eq!(
                caret_at(&out),
                "Switch(config)#class".len(),
                "{release} {level}"
            );
        }
        let e = held_engine(STEPS);
        let mut s = configured(&e, &[]);
        let (out, r) = s.type_line("class C1");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch(config)#class".len());
        assert_eq!(s.mode(), "config");
        assert_eq!(s.type_line("class-map C1").1, Result::Ok);
        // typed in a class map, which hands it to global configuration
        assert_eq!(s.type_line("class C2").1, Result::Error);
        assert_eq!(s.mode(), "config-cmap");
        // an image with no crawl reads it as the books do
        let books = books_17_6();
        let mut plain = configured(&books, &[]);
        assert_eq!(plain.type_line("class C1").1, Result::Ok);
        assert_eq!(plain.mode(), "config-cmap");
    }

    #[test]
    fn an_image_takes_its_measured_start_though_its_own_crawl_offers_no_first_word() {
        const STEPS: &[(&str, &[&str])] = &[
            ("Switch#configure terminal", ENTERED),
            (
                "Switch(config)# ?",
                &[
                    "Configure commands:",
                    "  l2protocol-tunnel  Configure l2protocol-tunnel parameters",
                    "  lacp               LACP configuration",
                    "",
                ],
            ),
            (
                "Switch(config)#l2protocol-tunnel ?",
                &["  cos     Class of Service", "  global  Global settings"],
            ),
        ];
        let open = |book: &str, release: &str, level: &str, stack: &str| {
            let e = held_engine_of(book, release, level, STEPS);
            let stack = crate::platform::parse_stack(stack).unwrap();
            let mut s = Session::new(&e, Device::new(stack));
            s.boot_level = Some(level.into());
            for line in ["enable", "configure terminal"] {
                assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
            }
            s.type_line("l2 traceroute")
        };
        // the lab C9200L-48T-4G took it at network-advantage, its `?` listing no `l2`, at 17.6.4
        // and at 17.15.6
        for release in ["17.6", "17.15"] {
            let (out, r) = open("c9200", release, "network-advantage", "C9200L-48T-4G");
            assert_eq!(r, Result::Ok, "{release} {out:?}");
        }
        // another image's crawl at that level still reads `l2` as the keyword it offers
        let (out, r) = open("c9300", "17.6", "network-advantage", "C9300-24T");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(caret_at(&out), "Switch(config)#l2 ".len());
    }

    #[test]
    fn ptp_mode_and_transport_are_refused_under_the_dot1as_profile() {
        let e = held_engine(&[
            ("Switch#configure terminal", ENTERED),
            (
                "Switch(config)# ?",
                &["Configure commands:", "  ptp  Precision Time Protocol", ""],
            ),
            (
                "Switch(config)#ptp ?",
                &[
                    "  mode       Change PTP clock mode",
                    "  priority1  Set priority1 for the clock",
                    "  profile    Change PTP clock profile",
                    "  transport  Underlying Network Transport protocol",
                    "",
                ],
            ),
        ]);
        let lines = ["ptp mode e2etransparent", "ptp transport ipv4 udp"];
        let mut s = configured(&e, &[]);
        for line in lines {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        let mut s = configured(&e, &["ptp profile dot1as"]);
        for line in lines {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Error, "{line}: {out:?}");
            assert_eq!(caret_at(&out), "Switch(config)#ptp ".len(), "{line}");
        }
        assert_eq!(s.type_line("ptp priority1 120").1, Result::Ok);
    }

    /// The crawl steps the dot1as `ptp` refusals are read against.
    const PTP: &[(&str, &[&str])] = &[
        ("Switch#configure terminal", ENTERED),
        (
            "Switch(config)# ?",
            &[
                "Configure commands:",
                "  license  Configure license features",
                "  ptp      Precision Time Protocol",
                "",
            ],
        ),
        (
            "Switch(config)#ptp ?",
            &[
                "  mode       Change PTP clock mode",
                "  priority1  Set priority1 for the clock",
                "  profile    Change PTP clock profile",
                "  transport  Underlying Network Transport protocol",
                "",
            ],
        ),
        (
            "Switch(config)#ptp mode ?",
            &[
                "  boundary        Boundary Clock mode",
                "  e2etransparent  End to End Transparent Clock mode",
                "  p2ptransparent  Peer to Peer Transparent Clock mode",
                "",
            ],
        ),
    ];

    /// A measured refusal holds on the image and at the levels it was measured on: elsewhere a line
    /// reads as the image's books and crawl read it.
    #[test]
    fn a_measured_refusal_holds_only_on_the_image_and_levels_it_was_measured_on() {
        // a C2960 at 15.0(2)SE with its own crawl at lanbase, against the same image with none
        let classic = held_engine_of(
            "c2960",
            "15.0(2)SE",
            "lanbase",
            &[
                ("Switch#configure terminal", ENTERED),
                (
                    "Switch(config)# ?",
                    &[
                        "Configure commands:",
                        "  class-map  Configure Class Map",
                        "  clock      Configure time-of-day clock",
                        "",
                    ],
                ),
            ],
        );
        let r = crate::release::Release::parse("15.0(2)SE").unwrap();
        let grammar = crate::grammars::grammar("c2960", r).unwrap();
        let held = crate::live::Held::from_corpus(
            &crate::corpus::Corpus::default(),
            &grammar.platform,
            &grammar.release,
        );
        let books = Engine::with_held(grammar, held);
        let typed = |e: &Engine, line: &str| {
            let stack = crate::platform::parse_stack("WS-C2960G-24TC-L").unwrap();
            let mut s = Session::new(e, Device::new(stack));
            s.type_line("enable");
            s.type_line("configure terminal");
            let out = s.type_line(line);
            (out, s.mode().to_string())
        };
        assert_eq!(typed(&classic, "class C1"), typed(&books, "class C1"));
        // the dot1as `ptp` lines were measured at network-advantage alone
        let essentials = held_engine_of("c9300", "17.6", "network-essentials", PTP);
        let books = books_17_6();
        let at_essentials = |e| {
            let mut s = configured(e, &[]);
            s.boot_level = Some("network-essentials".into());
            assert_eq!(s.type_line("ptp profile dot1as").1, Result::Ok);
            s
        };
        let mut s = at_essentials(&essentials);
        let mut plain = at_essentials(&books);
        for line in ["ptp mode e2etransparent", "ptp transport ipv4 udp"] {
            assert_eq!(s.type_line(line), plain.type_line(line), "{line}");
        }
        let advantage = held_engine(PTP);
        let mut s = configured(&advantage, &["ptp profile dot1as"]);
        assert_eq!(s.type_line("ptp mode e2etransparent").1, Result::Error);
    }

    /// A measured refusal holds for the line's unique abbreviations, each word read against the
    /// keywords offered before it and the refused line's own; an abbreviation that begins two is
    /// ambiguous.
    #[test]
    fn a_measured_refusal_holds_for_a_unique_abbreviation_and_an_ambiguous_one_is_ambiguous() {
        let ptp = held_engine(PTP);
        let mut s = configured(&ptp, &["ptp profile dot1as"]);
        for line in [
            "ptp m e2etransparent",
            "pt mo e2e",
            "PTP MODE E2ETRANSPARENT",
        ] {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Error, "{line}: {out:?}");
            let at = line.find(' ').unwrap() + 1 + "Switch(config)#".len();
            assert_eq!(caret_at(&out), at, "{line}: {out:?}");
        }
        assert_eq!(s.type_line("ptp mode p2ptransparent").1, Result::Error);
        assert_eq!(s.type_line("ptp priority1 120").1, Result::Ok);
        let e = held_engine(&[
            ("Switch#configure terminal", ENTERED),
            (
                "Switch(config)# ?",
                &[
                    "Configure commands:",
                    "  class-map  Configure CPL Class Map",
                    "  clock      Configure time-of-day clock",
                    "",
                ],
            ),
        ]);
        let mut s = configured(&e, &[]);
        // `clas` begins the refused `class` and `class-map`
        assert_eq!(
            s.type_line("clas C1"),
            (
                vec!["% Ambiguous command:  \"clas C1\"".to_string()],
                Result::Error
            )
        );
        assert_eq!(s.type_line("class-m C1").1, Result::Ok);
        assert_eq!(s.mode(), "config-cmap");
    }

    #[test]
    fn an_mld_ssm_map_entry_is_global_and_wants_ssm_mapping_enabled() {
        let books = books_17_6();
        let line = "ipv6 mld ssm-map static SSM1 2001:DB8:1::1";
        let mut s = configured(&books, &["interface Vlan30"]);
        let (out, r) = s.type_line(line);
        assert_eq!(
            (out, r),
            (vec!["%ssm-map is not enabled!".to_string()], Result::Error)
        );
        assert_eq!(s.mode(), "config");
        assert_eq!(s.type_line("ipv6 mld ssm-map enable").1, Result::Ok);
        assert_eq!(s.type_line(line), (vec![], Result::Ok));
    }

    #[test]
    fn a_civic_location_enters_its_mode_with_the_room_it_has() {
        let books = books_17_6();
        let mut s = configured(&books, &[]);
        let (out, r) = s.type_line("location civic-location identifier 1");
        let room = "% 250 bytes available to configure civic address attributes";
        assert_eq!((out, r), (vec![room.to_string()], Result::Ok));
        assert_eq!(s.mode(), "config-civic");
        for line in ["number 3550", "state CA", "country US", "no state"] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        assert_eq!(s.type_line("number").1, Result::Error);
        assert_eq!(s.mode(), "config-civic");
        assert_eq!(s.type_line("exit").1, Result::Ok);
        // a location holding attributes has less room, by a count not measured
        let (out, _) = s.type_line("location civic-location identifier 1");
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(s.mode(), "config-civic");
    }

    #[test]
    fn a_port_s_start_on_a_member_the_stack_lacks_is_incomplete() {
        let books = books_17_6();
        let mut s = configured(&books, &[]);
        for line in [
            "interface TenGigabitEthernet 4/1",
            "interface GigabitEthernet 6/1",
        ] {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Error, "{line}");
            assert_eq!(out, ["% Incomplete command.", ""], "{line}");
        }
        // refused: the whole number of a port on a member the stack lacks, a module no member's
        // ports carry, member 0 past the management port's, and a type the stack lacks
        for line in [
            "interface GigabitEthernet4/0/1",
            "interface GigabitEthernet 6/2",
            "interface gigabitethernet 0/1",
            "interface FastEthernet 2/1",
        ] {
            let (out, _) = s.type_line(line);
            assert!(
                out.iter().any(|l| l.starts_with("% Invalid input")),
                "{line}: {out:?}"
            );
        }
        let mut s = configured(&books, &["interface Vlan1"]);
        let (out, _) = s.type_line("ip unnumbered gigabitethernet 3/1");
        assert_eq!(out, ["% Incomplete command.", ""]);
    }

    #[test]
    fn a_refused_port_s_caret_goes_past_a_name_the_stack_has_or_the_management_port_s_number() {
        let stack = crate::platform::parse_stack("C9200L-48T-4G").unwrap();
        for (rest, at) in [
            // the management port's number, departed from
            ("gigabitethernet 0/1", 18),
            ("gigabitethernet0/1", 17),
            ("GigabitEthernet 0/2/0", 18),
            // a port the stack has, gone on past
            ("GigabitEthernet1/0/13/0", 21),
            // any other number, at the name's start
            ("gigabitethernet 1/0/0", 0),
            ("GigabitEthernet4/0/1", 0),
            // the management port's whole number, gone on past
            ("GigabitEthernet 0/0/0", 0),
            // a module no member's ports carry, at the module; a member past the most, at it
            ("gigabitethernet 6/2", 18),
            ("gigabitethernet 2/3/0", 18),
            ("gigabitethernet 9/0/6", 16),
            ("gigabitethernet 5/0/2", 0),
            ("TenGigabitEthernet 6/2", 0),
        ] {
            assert_eq!(past_a_name(rest, &stack), at, "{rest}");
        }
        // the lab C9300-24T put a member past the most at the name
        let stack = crate::platform::parse_stack("C9300-24T").unwrap();
        for (rest, at) in [("gigabitethernet 9/0/6", 0), ("gigabitethernet 6/2", 18)] {
            assert_eq!(past_a_name(rest, &stack), at, "{rest}");
        }
    }

    /// The shell's contexts for `script` typed from privileged EXEC, and the corpus's for the session
    /// log the same typing makes, a `?` probe after each line.
    fn contexts_both_ways(e: &Engine, script: &[&str]) -> (Vec<String>, Vec<Option<String>>) {
        let stack = crate::platform::parse_stack("C9300-48P").unwrap();
        let mut s = Session::new(e, Device::new(stack));
        s.type_line("enable");
        let mut log = vec![];
        let mut shell = vec![];
        for line in script {
            log.push(format!("{}{line}", s.prompt()));
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Ok, "{line}: {out:?}");
            log.extend(out);
            log.push(format!("{} ?", s.prompt()));
            log.push("  answer".into());
            shell.push(s.context().expect("a context"));
        }
        log.push(format!("{}end", s.prompt()));
        let (_, ex) = crate::capture::exchanges(&log.join("\n")).unwrap();
        let corpus = ex
            .iter()
            .filter(|e| e.command.ends_with('?'))
            .map(|e| e.context.clone())
            .collect();
        (shell, corpus)
    }

    #[test]
    fn blocks_of_one_kind_typed_back_to_back_take_the_same_context_in_the_shell_and_the_corpus() {
        let script = [
            "configure terminal",
            "vrf definition TESTV",
            "exit",
            "policy-map TESTA",
            "policy-map TESTB",
            "class class-default",
            "policy-map TESTC",
            "route-map TESTA permit 10",
            "route-map TESTB deny 20",
            "ip access-list extended TESTA",
            "ip access-list extended TESTB",
            "ip access-list standard TESTC",
            "class-map match-any TESTA",
            "class-map match-all TESTB",
            "interface Vlan10",
            "ip vrf forwarding TESTV",
            "end",
        ];
        let (shell, corpus) = contexts_both_ways(&books_17_6(), &script);
        let want = [
            "",
            "vrf definition «word»",
            "",
            "policy-map «word»",
            "policy-map «word»",
            "policy-map «word» > class «word»",
            "policy-map «word»",
            "route-map «word» permit «number»",
            "route-map «word» deny «number»",
            "ip access-list extended «word»",
            "ip access-list extended «word»",
            "ip access-list standard «word»",
            "class-map match-any «word»",
            "class-map match-all «word»",
            "interface vlan",
            "interface vlan",
            "",
        ];
        assert_eq!(shell, want);
        let corpus: Vec<Option<&str>> = corpus.iter().map(|c| c.as_deref()).collect();
        let shell: Vec<Option<&str>> = shell.iter().map(|c| Some(c.as_str())).collect();
        assert_eq!(corpus, shell);
    }

    #[test]
    fn do_exec_abbreviated_runs_answers_and_completes_as_do_exec() {
        let e = c9300();
        let mut s = configured(&e, &["interface Vlan10", "exit"]);
        let whole = s.type_line("do-exec show running-config | include interface Vlan10");
        assert_eq!(whole.0, ["interface Vlan10"]);
        for typed in ["do-", "do-e", "DO-EX", "do-exe"] {
            assert_eq!(
                s.type_line(&format!(
                    "{typed} show running-config | include interface Vlan10"
                )),
                whole,
                "{typed}"
            );
            assert_eq!(
                s.help(&format!("{typed} show vlan ")),
                s.help("do-exec show vlan ")
            );
            assert_eq!(
                s.complete(&format!("{typed} sh")).as_deref(),
                Some(format!("{typed} show ").as_str())
            );
            // the word as typed, then the rest of `do-exec` (`interface TWE` Tab gives `TWEntyFiveGigE`)
            let rest = &"do-exec"[typed.len()..];
            assert_eq!(
                s.complete(typed),
                Some(format!("{typed}{rest} ")),
                "{typed}"
            );
            assert_eq!(
                s.type_line(typed),
                (
                    vec!["% Incomplete command.".to_string(), String::new()],
                    Result::Error
                )
            );
        }
        assert_eq!(s.mode(), "config");
        // `?` right after the word lists `do-exec` among the words it begins
        assert_eq!(s.help("do-"), ["do-exec", ""]);
        assert!(
            s.help("do")[0].split("  ").any(|w| w == "do-exec"),
            "{:?}",
            s.help("do")
        );
        // `do` begins `do-exec` and `dot1x` (lab C9300-24T 17.6.4: Tab rings the bell), and runs as
        // its own command; `d` begins too many words to be either
        assert_eq!(s.complete("do"), None);
        assert_eq!(s.complete("do-e").as_deref(), Some("do-exec "));
        assert_eq!(
            s.type_line("do show running-config | include interface Vlan10"),
            whole
        );
        assert_ne!(s.type_line("d show running-config").1, Result::Ok);
        // a caret after an abbreviated `do-exec` stands under the refused word
        assert_eq!(caret_at(&s.type_line("do-ex shw run").0), 23);
    }

    /// Whether the books' keywords could stand in for a held answer's where none is held: over the
    /// crawl logs under `VSWITCH_CRAWLS`, each held answer in global configuration and privileged EXEC
    /// with no context, each keyword row cut to its shortest unique start among the answer's keyword
    /// rows, read against the keywords the books offer after the same line (`context::reads`).
    #[test]
    #[ignore = "an audit: reads the crawl logs VSWITCH_CRAWLS names"]
    fn the_books_keywords_against_the_held_ones() {
        use crate::context::{Reads, reads};
        let Ok(dir) = std::env::var("VSWITCH_CRAWLS") else {
            return;
        };
        for f in crate::corpus::Source::read_crawls(std::path::Path::new(&dir)) {
            let (book, stack) = match f.book.as_str() {
                "c2960" => ("c2960", "WS-C2960X-48FPD-L"),
                _ => ("c9300", "C9300-48P"),
            };
            let r = crate::release::Release::parse(&f.release).unwrap();
            let e = Engine::new(crate::grammars::grammar(book, r).unwrap());
            let mut s = Session::new(
                &e,
                Device::new(crate::platform::parse_stack(stack).unwrap()),
            );
            s.type_line("enable");
            let mut tally: BTreeMap<&str, (usize, Vec<String>)> = BTreeMap::new();
            let mut seen = BTreeSet::new();
            for (m, c, k, a) in crate::corpus::crawled_help(&f.text) {
                if !c.is_empty()
                    || !matches!(m.as_str(), "config" | "privileged")
                    || !k.ends_with(' ') && !k.is_empty()
                    || !seen.insert((m.clone(), k.clone()))
                {
                    continue;
                }
                let held: Vec<String> = crate::live::keywords(&a);
                let toks = cli::tokens(&k);
                let books: Vec<String> = s
                    .offered_at(&m, &toks)
                    .0
                    .into_iter()
                    .filter(|o| o.kind != Offered::Placeholder)
                    .map(|o| o.shown)
                    .collect();
                let held_ref: Vec<&str> = held.iter().map(String::as_str).collect();
                let books_ref: Vec<&str> = books.iter().map(String::as_str).collect();
                for w in &held {
                    let Some(p) = (1..=w.len())
                        .filter(|&n| w.is_char_boundary(n))
                        .map(|n| &w[..n])
                        .find(|p| matches!(reads(p, &held_ref), Reads::One(_)) || *p == w.as_str())
                    else {
                        continue;
                    };
                    let class = match reads(p, &books_ref) {
                        Reads::One(x) if x.eq_ignore_ascii_case(w) => "the same keyword",
                        Reads::Exact if p.eq_ignore_ascii_case(w) => "the same keyword",
                        Reads::One(_) | Reads::Exact => "another keyword",
                        Reads::Several => "ambiguous in the books",
                        Reads::None => "no keyword in the books",
                    };
                    let t = tally.entry(class).or_default();
                    t.0 += 1;
                    if t.1.len() < 4 && class != "the same keyword" {
                        t.1.push(format!("{m} {k:?} {p:?} for {w:?}"));
                    }
                }
            }
            println!("{}-{}", f.book, f.release);
            for (class, (n, ex)) in tally {
                println!("  {class}: {n}");
                for x in ex {
                    println!("    {x}");
                }
            }
        }
    }

    /// A hardware stack for an audit's session of each book.
    fn audit_stack(book: &str) -> crate::platform::Stack {
        let named = match book {
            "c2960" => "WS-C2960G-8TC-L",
            "c2960x" => "WS-C2960X-48FPD-L",
            "c9200" => "C9200-48P",
            "c9400" => "1=C9400-LC-48T,3=C9400-SUP-1XL",
            _ => "C9300-48P",
        };
        crate::platform::parse_stack(named).unwrap()
    }

    /// A session of `e` in `mode` (EXEC or privileged EXEC) at `level`, stated by a license line
    /// where the book has no default of that level.
    fn audit_session<'a>(e: &'a Engine, book: &str, level: &str, mode: &str) -> Session<'a> {
        let mut s = Session::new(e, Device::new(audit_stack(book)));
        s.type_line("enable");
        if crate::license::default_for(book) != Some(level) {
            for line in [
                "configure terminal",
                &format!(
                    "license boot level {level} addon dna-{}",
                    level.trim_start_matches("network-")
                ),
                "end",
            ] {
                // the level the switch booted at, as a configuration loaded states it
                s.redacted = line.starts_with("license ");
                let (out, r) = s.type_line(line);
                s.redacted = false;
                assert_eq!(r, Result::Ok, "{book} {line}: {out:?}");
            }
        }
        if mode == "exec" {
            s.type_line("disable");
        }
        assert_eq!(s.mode(), mode);
        s
    }

    /// The committed corpus with the crawls under `VSWITCH_CRAWLS` compiled into it.
    fn with_crawls(dir: &str) -> crate::corpus::Corpus {
        let mut c = crate::corpus::get().clone();
        let src = crate::corpus::Source {
            crawls: crate::corpus::Source::read_crawls(std::path::Path::new(dir)),
            ..Default::default()
        };
        for f in &src.crawls {
            println!(
                "crawl {}-{}/{} at {}: {} bytes",
                f.book,
                f.release,
                f.folder,
                f.level,
                f.text.len()
            );
        }
        for (b, r, t) in crate::corpus::build(&src).held {
            let mut keys: BTreeMap<&str, usize> = BTreeMap::new();
            for e in &t.crawl {
                *keys.entry(e.0.as_str()).or_default() += 1;
            }
            println!("crawl {b}-{r} keys held by level: {keys:?}");
            match c.held.iter_mut().find(|(cb, cr, _)| *cb == b && *cr == r) {
                Some((.., held)) => held.crawl = t.crawl,
                None => c.held.push((b, r, t)),
            }
        }
        c
    }

    /// An answer's rows, each its word and description with continuation lines joined and spacing
    /// collapsed, as a console of any width prints them; headers and blank lines left out.
    fn audit_rows(answer: &[String]) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for l in answer {
            let indent = l.len() - l.trim_start_matches(' ').len();
            let words = l.split_whitespace().collect::<Vec<_>>().join(" ");
            if words.is_empty() || indent < 2 {
                if indent == 0 && !words.is_empty() && !words.ends_with(':') {
                    out.push(words);
                }
                continue;
            }
            match out.last_mut() {
                Some(last) if indent > 2 => {
                    last.push(' ');
                    last.push_str(&words);
                }
                _ => out.push(words),
            }
        }
        out
    }

    /// Keywords both answers offer, and how many each offers.
    fn overlap(truth: &[String], got: &[String]) -> (usize, usize, usize) {
        let t: BTreeSet<String> = crate::live::keywords(truth).into_iter().collect();
        let g: BTreeSet<String> = crate::live::keywords(got).into_iter().collect();
        (t.intersection(&g).count(), t.len(), g.len())
    }

    /// Masked truth: each `?` answer the committed corpus holds from an image's captures, asked of the
    /// full resolver with that image's capture answers set aside and its donor mapping on at the
    /// image's level (the captures' `license boot level`, else the book's default), and of the books
    /// alone; with the crawls under `VSWITCH_CRAWLS`. A Catalyst 9000 image at another level is also
    /// asked at network-advantage, as a level mismatch.
    #[test]
    #[ignore = "an audit: reads the crawl logs VSWITCH_CRAWLS names"]
    fn donor_answers_against_the_captures_answers() {
        let Ok(dir) = std::env::var("VSWITCH_CRAWLS") else {
            return;
        };
        let c = with_crawls(&dir);
        for (book, release, table) in &crate::corpus::get().held {
            if table.help.is_empty() {
                continue;
            }
            let top = table
                .lines
                .iter()
                .filter(|(k, _)| k.is_empty())
                .map(|(_, l)| l.as_str());
            let Some(level) = crate::license::of(book, top) else {
                println!("{book} {release}: no level");
                continue;
            };
            let r = crate::release::Release::parse(release).unwrap();
            let grammar = crate::grammars::grammar(book, r).unwrap();
            let mut books_held =
                crate::live::Held::from_corpus(&c, &grammar.platform, &grammar.release);
            books_held.help.clear();
            books_held.crawls.clear();
            let books = Engine::with_held(grammar.clone(), books_held);
            let mut runs = vec![(level.clone(), "")];
            let mismatch = "network-advantage";
            let crawled = |b: &str, r: &str, l: &str| {
                c.held
                    .iter()
                    .any(|(cb, cr, t)| cb == b && cr == r && t.crawl.iter().any(|e| e.0 == l))
            };
            let lent = crate::license::donor(book, release, &level)
                .is_some_and(|d| crawled(d.0, d.1, d.2));
            if !lent && !crawled(book, release, &level) {
                println!("{book} {release} at {level}: no donor at this level");
            }
            if level != mismatch && crate::license::donor(book, release, mismatch).is_some() {
                runs.push((mismatch.to_string(), " (level mismatch)"));
            }
            for (run_level, label) in runs {
                let on = [(book.as_str(), release.as_str(), run_level.as_str())];
                let mut held =
                    crate::live::Held::with_donors(&c, &grammar.platform, &grammar.release, &on);
                held.help.clear();
                let lent = Engine::with_held(grammar.clone(), held);
                let donor = crate::license::donor(book, release, &run_level)
                    .map_or("none".to_string(), |d| format!("{}-{} {}", d.0, d.1, d.2));
                let (mut exact, mut b_exact, mut keys) = (0, 0, 0);
                let (mut row_hit, mut b_row_hit, mut row_n) = (0, 0, 0);
                let (mut hit, mut t_n, mut g_n) = (0, 0, 0);
                let (mut b_hit, mut b_t, mut b_g) = (0, 0, 0);
                let mut tiers: BTreeMap<&str, usize> = BTreeMap::new();
                let mut misses = vec![];
                let mut differs = vec![];
                for (mode, key, truth) in &table.help {
                    let s = audit_session(&lent, book, &run_level, mode);
                    let b = audit_session(&books, book, &run_level, mode);
                    let got = s.help(key);
                    let plain = b.help(key);
                    let tier =
                        match lent
                            .held
                            .answered(Some(&run_level), mode, Some(""), key, |_| false)
                        {
                            Some((crate::live::Tier::Own, _)) => "own crawl",
                            Some((crate::live::Tier::Donor, a)) if crate::live::is_refusal(&a) => {
                                misses.push(format!("{mode} {key:?}: donor refuses {:?}", a[0]));
                                "donor"
                            }
                            Some((crate::live::Tier::Donor, _)) => "donor",
                            None => {
                                misses.push(format!("{mode} {key:?}: no donor answer"));
                                "books"
                            }
                        };
                    *tiers.entry(tier).or_default() += 1;
                    keys += 1;
                    let (want, rows) = (audit_rows(truth), audit_rows(&got));
                    let b_rows = audit_rows(&plain);
                    exact += usize::from(rows == want);
                    b_exact += usize::from(b_rows == want);
                    row_hit += want.iter().filter(|r| rows.contains(r)).count();
                    b_row_hit += want.iter().filter(|r| b_rows.contains(r)).count();
                    row_n += want.len();
                    if rows != want && differs.len() < 3 {
                        let at = rows.iter().zip(&want).position(|(g, t)| g != t);
                        differs.push(format!(
                            "{mode} {key:?}: {} rows vs {}, first difference {:?}",
                            rows.len(),
                            want.len(),
                            at.map(|i| (&rows[i], &want[i]))
                        ));
                    }
                    let (h, t, g) = overlap(truth, &got);
                    (hit, t_n, g_n) = (hit + h, t_n + t, g_n + g);
                    let (h, t, g) = overlap(truth, &plain);
                    (b_hit, b_t, b_g) = (b_hit + h, b_t + t, b_g + g);
                }
                let pct = |a: usize, b: usize| {
                    if b == 0 {
                        "-".to_string()
                    } else {
                        format!("{:.0}%", 100.0 * a as f64 / b as f64)
                    }
                };
                println!(
                    "{book} {release} at {run_level}{label}, context '': donor {donor}; {keys} keys, answered by {tiers:?}"
                );
                println!(
                    "  resolver: {exact} of {keys} answers row for row, {row_hit} of {row_n} rows; keyword recall {} precision {} ({hit} of {t_n} truth, {g_n} offered)",
                    pct(hit, t_n),
                    pct(hit, g_n)
                );
                println!(
                    "  books:    {b_exact} of {keys} answers row for row, {b_row_hit} of {row_n} rows; keyword recall {} precision {} ({b_hit} of {b_t} truth, {b_g} offered)",
                    pct(b_hit, b_t),
                    pct(b_hit, b_g)
                );
                for m in misses.iter().chain(&differs) {
                    println!("    {m}");
                }
            }
        }
    }

    /// Enter on a donor's own image: each keyword its crawl offers at the root of global
    /// configuration and privileged EXEC, typed alone there through the shell, counted by whether the
    /// shell takes it, answers `% Incomplete command` or refuses it with `% Invalid input`.
    #[test]
    #[ignore = "an audit: reads the crawl logs VSWITCH_CRAWLS names"]
    fn a_donor_s_root_keywords_typed_on_its_own_image() {
        let Ok(dir) = std::env::var("VSWITCH_CRAWLS") else {
            return;
        };
        let donors: BTreeSet<crate::license::Image> =
            crate::license::DONORS.iter().map(|d| d.2).collect();
        for f in crate::corpus::Source::read_crawls(std::path::Path::new(&dir)) {
            if !donors.contains(&(f.book.as_str(), f.release.as_str(), f.level.as_str())) {
                continue;
            }
            let r = crate::release::Release::parse(&f.release).unwrap();
            let e = Engine::new(crate::grammars::grammar(&f.book, r).unwrap());
            let stack = match f.book.as_str() {
                "c2960" => "WS-C2960G-8TC-L",
                _ => "C9300-24T",
            };
            let mut roots: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for (m, c, k, a) in crate::corpus::crawled_help(&f.text) {
                if c.is_empty() && k.is_empty() && matches!(m.as_str(), "config" | "privileged") {
                    roots.entry(m).or_insert_with(|| crate::live::keywords(&a));
                }
            }
            for (mode, words) in roots {
                let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
                let mut refused = vec![];
                for w in &words {
                    let mut s = Session::new(
                        &e,
                        Device::new(crate::platform::parse_stack(stack).unwrap()),
                    );
                    s.type_line("enable");
                    if mode == "config" {
                        s.type_line("configure terminal");
                    }
                    let (out, r) = s.type_line(w);
                    let has = |p: &str| out.iter().any(|l| l.contains(p));
                    let class = if has("% Invalid input") {
                        refused.push(w.clone());
                        "refused, % Invalid input"
                    } else if has("% Incomplete command") {
                        "% Incomplete command"
                    } else if r == Result::Ok {
                        "taken"
                    } else {
                        "other error"
                    };
                    *tally.entry(class).or_default() += 1;
                }
                println!(
                    "{}-{} {} {mode}: {} root keywords: {tally:?}",
                    f.book,
                    f.release,
                    f.level,
                    words.len()
                );
                println!("  first refused: {:?}", &refused[..refused.len().min(30)]);
            }
        }
    }

    #[test]
    fn a_line_known_to_crash_the_switch_is_warned_of_after_its_answer() {
        let script = "enable\nsh ipv6 pim gi\nshow ip pim gir\nshow ipv6 pim gir?\nconfigure terminal\ndo show platform software tdl-database content bp config nodes 0 | include x\nshow ipv6 pim gir\nend\n";
        let at = |book: &str, model: &str, release: &str| {
            let r = crate::release::Release::parse(release).unwrap();
            let e = Engine::new(crate::grammars::grammar(book, r).unwrap());
            let stack = crate::platform::parse_stack(model).unwrap();
            let mut s = Session::new(&e, Device::new(stack));
            let (transcript, refused) = s.script(script);
            (transcript, refused, s.warned)
        };
        let warning = "% vswitch: this command crashes the switch (IOSd segfault and reload on a Catalyst 9300 at IOS XE 17.6.4)";
        let (t, refused, warned) = at("c9300", "C9300-24T", "17.6");
        // the caret line follows the typed line and marks its word; the warning follows the `%`
        // message and its blank line, before the next prompt
        for (typed, word) in [
            ("Switch#sh ipv6 pim gi", "gi"),
            (
                "Switch(config)#do show platform software tdl-database content bp config nodes 0 | include x",
                "tdl-database",
            ),
        ] {
            let i = t.iter().position(|l| l == typed).unwrap();
            assert_eq!(t[i + 1].trim(), "^", "{t:#?}");
            assert_eq!(t[i + 1].find('^'), typed.find(word), "{t:#?}");
            assert_eq!(t[i + 2], "% Invalid input detected at '^' marker.");
            assert_eq!(t[i + 3], "");
            assert_eq!(t[i + 4], warning, "{t:#?}");
            assert!(t[i + 5].starts_with("Switch"), "{t:#?}");
        }
        // `?`, the forms that answer, and a bare line in configuration mode are not warned of
        assert_eq!(
            t.iter()
                .filter(|l| l.starts_with("% vswitch: this command"))
                .count(),
            2,
            "{t:#?}"
        );
        assert_eq!(warned, 2);
        // without its warnings the session is line for line what it is without the check, refusals
        // included
        let plain = {
            let r = crate::release::Release::parse("17.6").unwrap();
            let e = Engine::new(crate::grammars::grammar("c9300", r).unwrap());
            let stack = crate::platform::parse_stack("C9300-24T").unwrap();
            let mut s = Session::new(&e, Device::new(stack));
            let (mut out, mut n) = (vec![], 0);
            for l in script.lines() {
                out.push(format!("{}{l}", s.prompt()));
                if let Some(before) = l.strip_suffix('?') {
                    out.extend(s.help(before));
                    continue;
                }
                let (o, r) = s.type_line(l);
                n += usize::from(r == Result::Error);
                out.extend(o);
            }
            (out, n)
        };
        let unwarned: Vec<String> = t.iter().filter(|l| *l != warning).cloned().collect();
        assert_eq!((unwarned, refused), plain);
        // on another release of the family, what it did at the release measured
        let (t, _, _) = at("c9300", "C9300-24T", "17.15");
        assert!(t.contains(&"% vswitch: this command crashed a Catalyst 9300 at IOS XE 17.6.4 (IOSd segfault and reload)".to_string()), "{t:#?}");
        // on another family, nothing
        for (book, model) in [("c9200", "C9200-24T"), ("c2960", "WS-C2960G-24TC-L")] {
            let release = crate::grammars::default_release(book).to_string();
            let (t, _, warned) = at(book, model, &release);
            assert!(
                !t.iter().any(|l| l.contains("% vswitch: this command")),
                "{model}: {t:#?}"
            );
            assert_eq!(warned, 0);
        }
    }

    #[test]
    fn a_9200_transceiver_max_power_form_is_warned_of_on_the_interface_measured() {
        let script = "show interfaces Tunnel0 transceiver max-power\nsh int gi0/0 trans max-power violation\nshow interfaces GigabitEthernet1/0/1 transceiver max-power\nshow interfaces GigabitEthernet1/0/1 transceiver max-power violation\nshow ipv6 pim gir\n";
        let at = |book: &str, model: &str, release: &str| {
            let r = crate::release::Release::parse(release).unwrap();
            let e = Engine::new(crate::grammars::grammar(book, r).unwrap());
            let stack = crate::platform::parse_stack(model).unwrap();
            let mut s = Session::new(&e, Device::new(stack));
            s.type_line("enable");
            let (transcript, _) = s.script(script);
            (transcript, s.warned)
        };
        let warnings = |t: &[String]| -> Vec<String> {
            t.iter()
                .filter(|l| l.starts_with("% vswitch: this command"))
                .cloned()
                .collect()
        };
        let (t, warned) = at("c9200", "C9200L-48T-4G", "17.15");
        let crashes = "% vswitch: this command crashes the switch (IOSd segfault and reload on a Catalyst 9200L at IOS XE 17.15.6)";
        assert_eq!(warnings(&t), [crashes, crashes], "{t:#?}");
        assert_eq!(warned, 2);
        // each warning follows its own line's answer, before the next prompt
        for typed in [
            "Switch#show interfaces Tunnel0 transceiver max-power",
            "Switch#sh int gi0/0 trans max-power violation",
        ] {
            let i = t.iter().position(|l| l == typed).unwrap();
            let w = i + t[i..].iter().position(|l| l == crashes).unwrap();
            assert!(
                t[i + 1..w].iter().all(|l| !l.starts_with("Switch")),
                "{t:#?}"
            );
            assert!(t[w + 1].starts_with("Switch"), "{t:#?}");
        }
        // at 17.6, what it did at 17.15.6; on a 9300, its own hazard alone
        let (t, _) = at("c9200", "C9200L-48T-4G", "17.6");
        assert_eq!(
            warnings(&t),
            ["% vswitch: this command crashed a Catalyst 9200L at IOS XE 17.15.6 (IOSd segfault and reload)";
                2]
        );
        let (t, _) = at("c9300", "C9300-24T", "17.15");
        assert_eq!(
            warnings(&t),
            [
                "% vswitch: this command crashed a Catalyst 9300 at IOS XE 17.6.4 (IOSd segfault and reload)"
            ]
        );
    }

    /// A C9300 at 17.6 on its books and every answer the corpus holds.
    fn c9300_17_6() -> Engine {
        let r = crate::release::Release::parse("17.6").unwrap();
        Engine::new(crate::grammars::grammar("c9300", r).unwrap())
    }

    /// A C9300-24T with a 4x1G uplink module in global configuration, each line typed whatever
    /// its answer.
    fn typed_24t<'a>(e: &'a Engine, lines: &[&str]) -> Session<'a> {
        let stack = crate::platform::parse_stack("C9300-24T+C9300-NM-4G").unwrap();
        let mut s = Session::new(e, Device::new(stack));
        for line in ["enable", "configure terminal"].iter().chain(lines) {
            s.type_line(line);
        }
        s
    }

    #[test]
    fn a_9200_cts_environment_data_line_is_warned_of_at_network_essentials_alone() {
        let r = crate::release::Release::parse("17.15").unwrap();
        let e = Engine::new(crate::grammars::grammar("c9200", r).unwrap());
        let at = |level: &str| {
            let stack = crate::platform::parse_stack("C9200L-48T-4G").unwrap();
            let mut s = Session::new(&e, Device::new(stack));
            s.boot_level = Some(level.into());
            s.type_line("enable");
            let (t, _) = s.script("configure terminal\ncts environment-data enable\nend\n");
            (t, s.warned)
        };
        let (t, warned) = at("network-essentials");
        assert_eq!(warned, 1, "{t:#?}");
        let crashes = "% vswitch: this command crashes the switch (IOSd segfault and reload on a Catalyst 9200L at IOS XE 17.15.6 network-essentials)";
        assert!(t.iter().any(|l| l == crashes), "{t:#?}");
        // network-advantage took it
        let (t, warned) = at("network-advantage");
        assert_eq!(warned, 0, "{t:#?}");
    }
    #[test]
    fn a_refused_line_leaves_a_submode_only_where_global_configuration_reads_past_its_first_word() {
        let e = c9300_17_6();
        let cases: [(&[&str], &str); 3] = [
            (
                &["interface GigabitEthernet1/0/1"],
                "interface GigabitEthernet1/0/30",
            ),
            (&["router ospf 1"], "router ospf"),
            (
                &["router bgp 65000", "address-family ipv4"],
                "router rip cisco",
            ),
        ];
        for (opened, line) in cases {
            let mut s = typed_24t(&e, opened);
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Error, "{line}: {out:?}");
            assert_eq!(s.mode(), "config", "{line}: {out:?}");
        }
        // an address family closes back to the mode whose `address-family` opened it
        let mut s = typed_24t(&e, &["vrf definition V", "address-family ipv4"]);
        assert_eq!(s.mode(), "config-vrf-af");
        assert_eq!(s.type_line("address family ipv4").1, Result::Error);
        assert_eq!(s.mode(), "config-vrf");
        // any other refused line is the submode's error, and the session stays there
        let cases: [(&[&str], &str, &str); 2] = [
            (&["policy-map P"], "policy-map class vlan100", "config-pmap"),
            (&["vlan 10"], "vlan 4094 guest-interface 0", "config-vlan"),
        ];
        for (opened, line, mode) in cases {
            let mut s = typed_24t(&e, opened);
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Error, "{line}: {out:?}");
            assert_eq!(s.mode(), mode, "{line}: {out:?}");
        }
    }

    #[test]
    fn a_port_named_by_the_start_of_its_number_is_incomplete_and_nve_is_1_to_32() {
        let e = c9300_17_6();
        let mut s = typed_24t(&e, &[]);
        let (out, r) = s.type_line("interface gigabitethernet1/1");
        assert_eq!(
            (out[0].as_str(), r),
            ("% Incomplete command.", Result::Error)
        );
        assert_eq!(s.mode(), "config");
        assert_eq!(s.type_line("interface nve1").1, Result::Ok);
        assert_eq!(s.mode(), "config-if");
        s.type_line("exit");
        assert_eq!(s.type_line("interface nve 33").1, Result::Error);
    }

    #[test]
    fn a_named_eigrp_family_keeps_its_as_and_refuses_one_another_instance_runs() {
        let e = c9300_17_6();
        let mut s = typed_24t(
            &e,
            &[
                "router eigrp EIGA",
                "address-family ipv4 autonomous-system 10",
                "exit-address-family",
            ],
        );
        assert_eq!(s.mode(), "config-router");
        let (out, r) = s.type_line("address-family ipv4 autonomous-system 20");
        assert_eq!(out, ["Changing from AS(10) to AS(20) is not allowed"]);
        assert_eq!((r, s.mode()), (Result::Error, "config-router"));
        // the AS it holds, `unicast` written or not
        assert_eq!(
            s.type_line("address-family ipv4 unicast autonomous-system 10")
                .1,
            Result::Ok
        );
        assert_eq!(s.mode(), "config-router-af");
        for line in ["exit-address-family", "exit", "router eigrp EIGB"] {
            s.type_line(line);
        }
        let (out, r) = s.type_line("address-family ipv4 unicast autonomous-system 10");
        assert_eq!(out, ["%ERROR: AS(10) in use by VR(EIGA)"]);
        assert_eq!((r, s.mode()), (Result::Error, "config-router"));
        assert_eq!(
            s.type_line("address-family ipv4 autonomous-system 30").1,
            Result::Ok
        );
        assert_eq!(s.mode(), "config-router-af");
    }

    #[test]
    fn the_cef_table_holds_a_running_subnet_and_an_installed_route_alone() {
        // the lab C9200L-48T-4G (17.6.4): a down SVI's subnet was not in it, a Null0 route's was
        let e = c9300_17_6();
        let s = typed_24t(
            &e,
            &[
                "interface Vlan10",
                "ip address 10.1.1.2 255.255.255.0",
                "interface Loopback0",
                "ip address 198.51.100.1 255.255.255.255",
                "exit",
                "ip route 203.0.113.0 255.255.255.0 Null0",
                "ip route 198.18.0.0 255.254.0.0 192.0.2.1",
            ],
        );
        let cef = show::cef_prefixes(&s.device);
        let has = |a: [u8; 4], n: u8| cef.contains(&(a.into(), n));
        assert!(has([0, 0, 0, 0], 0) && has([224, 0, 0, 0], 4));
        assert!(has([203, 0, 113, 0], 24) && has([198, 51, 100, 1], 32));
        assert!(!has([10, 1, 1, 0], 24) && !has([10, 1, 1, 2], 32));
        // a next hop no running subnet reaches
        assert!(!has([198, 18, 0, 0], 15));
    }

    #[test]
    fn a_named_eigrp_family_refuses_the_as_a_classic_process_runs() {
        // the lab C9200L-48T-4G (17.15.6) on a base holding `router eigrp 1`
        let r = crate::release::Release::parse("17.15").unwrap();
        let e = Engine::new(crate::grammars::grammar("c9200", r).unwrap());
        let stack = crate::platform::parse_stack("C9200L-48T-4G").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.frames.push(Frame::top("config"));
        for line in ["router eigrp 1", "exit", "router eigrp virtual-name"] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        let (out, r) = s.type_line("address-family ipv4 autonomous-system 1");
        assert_eq!(out, ["%ERROR: AS(1) in use by classic router"]);
        assert_eq!((r, s.mode()), (Result::Error, "config-router"));
        assert_eq!(
            s.type_line("address-family ipv4 autonomous-system 4453").1,
            Result::Ok
        );
        assert_eq!(s.mode(), "config-router-af");
        // the classic process refuses a family in a VRF the switch does not hold, by name
        // wherever the line holds it, and takes one in a VRF it holds
        for line in ["exit-address-family", "exit", "router eigrp 1"] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        for (line, vrf) in [
            ("address-family ipv4 vrf RED autonomous-system 45000", "RED"),
            ("address-family ipv4 unicast vrf inside", "inside"),
        ] {
            let (out, r) = s.type_line(line);
            assert_eq!(
                out,
                [format!(
                    "%VRF {vrf} does not exist or is not enabled for IPv4"
                )]
            );
            assert_eq!((r, s.mode()), (Result::Error, "config-router"), "{line}");
        }
        for line in [
            "exit",
            "vrf definition RED",
            "address-family ipv4",
            "exit-address-family",
            "exit",
            "router eigrp 1",
            "address-family ipv4 vrf RED autonomous-system 45000",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        assert_eq!(s.mode(), "config-router-af");
    }

    #[test]
    fn a_second_bgp_process_an_unknown_class_and_a_second_controller_are_refused_by_message() {
        let e = c9300_17_6();
        let mut s = typed_24t(&e, &["router bgp 65000", "exit"]);
        let (out, r) = s.type_line("router bgp 65001");
        assert_eq!(
            (out, r),
            (
                vec!["BGP is already running; AS is 65000".to_string()],
                Result::Error
            )
        );
        assert_eq!(s.mode(), "config");
        assert_eq!(s.type_line("router bgp 65000").1, Result::Ok);
        let mut s = typed_24t(&e, &["class-map C2", "exit", "policy-map P"]);
        let (out, r) = s.type_line("class C1");
        assert_eq!(
            (out, r),
            (
                vec!["class map C1 not configured".to_string()],
                Result::Error
            )
        );
        assert_eq!(s.mode(), "config-pmap");
        for taken in ["class C2", "exit", "class class-default"] {
            assert_eq!(s.type_line(taken).1, Result::Ok, "{taken}");
        }
        let mut s = typed_24t(&e, &["service-export mdns-sd controller ONE", "exit"]);
        let (out, r) = s.type_line("service-export mdns-sd controller TWO");
        assert_eq!(out[0], "Only one Controller is allowed");
        assert_eq!((r, s.mode()), (Result::Error, "config"));
        assert_eq!(
            s.type_line("service-export mdns-sd controller ONE").1,
            Result::Ok
        );
    }

    #[test]
    fn a_line_the_books_take_is_refused_where_the_switch_s_answers_offer_no_such_word() {
        let e = c9300_17_6();
        // the crawled ports were switchports: a routed port's lines are left to the books
        let mut s = typed_24t(&e, &["interface GigabitEthernet1/0/2"]);
        let (out, r) = s.type_line("ip address 10.0.0.1 255.255.255.0");
        assert_eq!(r, Result::Error, "{out:?}");
        assert_eq!(s.type_line("no switchport").1, Result::Ok);
        assert_eq!(
            s.type_line("ip address 10.0.0.1 255.255.255.0").1,
            Result::Ok
        );
        // a policy map's own mode offers no `set`; its class does
        let mut s = typed_24t(&e, &["policy-map P"]);
        assert_eq!(s.type_line("set dscp 10").1, Result::Error);
        assert_eq!(s.type_line("class class-default").1, Result::Ok);
        assert_eq!(s.type_line("set dscp 10").1, Result::Ok);
        // a type the answer does not list, though rows in capitals (`BDI`) stand beside it
        let mut s = typed_24t(&e, &[]);
        assert_eq!(s.type_line("interface ethernet 0").1, Result::Error);
        assert_eq!(s.type_line("interface vlan 10").1, Result::Ok);
        // a number in a rate or a list fits the row's range
        let mut s = typed_24t(&e, &["policy-map P", "class class-default"]);
        assert_eq!(s.type_line("police 1m").1, Result::Ok);
        let mut s = typed_24t(&e, &["interface GigabitEthernet1/0/2"]);
        assert_eq!(
            s.type_line("switchport trunk allowed vlan 31-35,41").1,
            Result::Ok
        );
    }

    #[test]
    fn a_line_the_crawl_could_not_judge_is_left_to_the_books() {
        let e = c9300_17_6();
        // the crawl ran without `aaa new-model` and `fhrp version vrrp v3`
        let cases: [(&[&str], &str, &str); 3] = [
            (&[], "radius server R1", "aaa new-model"),
            (
                &["line con 0"],
                "login authentication CONSOLE",
                "aaa new-model",
            ),
            (
                &["interface Vlan10"],
                "vrrp 1 address-family ipv4",
                "fhrp version vrrp v3",
            ),
        ];
        for (opened, line, state) in cases {
            let mut s = typed_24t(&e, opened);
            assert_eq!(s.type_line(line).1, Result::Error, "{line}");
            let mut lines = vec![state];
            lines.extend(opened);
            let mut s = typed_24t(&e, &lines);
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Ok, "{line} under {state}: {out:?}");
        }
        // an answer that lists the monitors configured, none when it was crawled: this session's,
        // a monitor on a port once it holds a record
        let mut s = typed_24t(
            &e,
            &[
                "flow monitor MON1",
                "exit",
                "interface GigabitEthernet1/0/1",
            ],
        );
        assert_eq!(s.type_line("ip flow monitor MON1 input").1, Result::Error);
        assert_eq!(s.type_line("ip flow monitor MON2 input").1, Result::Error);
        let mut s = typed_24t(
            &e,
            &[
                "flow record REC1",
                "match ipv4 source address",
                "exit",
                "flow record REC2",
                "exit",
                "flow exporter EXP1",
                "exit",
                "flow monitor MON1",
            ],
        );
        // the crawl's own names (`User defined`) stand for this session's
        for (line, r) in [
            ("record REC2", Result::Error),
            ("record REC3", Result::Error),
            ("exporter EXP2", Result::Error),
            ("exporter EXP1", Result::Ok),
            ("record REC1", Result::Ok),
            ("exit", Result::Ok),
            ("interface GigabitEthernet1/0/1", Result::Ok),
            ("ip flow monitor MON1 input", Result::Ok),
        ] {
            assert_eq!(s.type_line(line).1, r, "{line}");
        }
        // a sub-interface's encapsulation, which the crawled one had none of
        let mut s = typed_24t(
            &e,
            &[
                "interface GigabitEthernet1/0/7",
                "no switchport",
                "interface GigabitEthernet1/0/7.10",
                "encapsulation dot1Q 10",
            ],
        );
        let (out, r) = s.type_line("xconnect 10.0.0.2 200 encapsulation mpls");
        assert_eq!(r, Result::Ok, "{out:?}");
        // a port's `ptp ?` offers `enable` alone; the switch takes `ptp sync` all the same
        let mut s = typed_24t(&e, &["interface GigabitEthernet1/0/3"]);
        let (out, r) = s.type_line("ptp sync interval 1");
        assert_eq!(r, Result::Ok, "{out:?}");
        assert_eq!(s.type_line("ptp announce interval 1").1, Result::Error);
    }

    #[test]
    fn a_line_the_configuration_so_far_refuses_says_what_ios_xe_says() {
        let e = c9300_17_6();
        let said = |s: &mut Session, line: &str| {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Error, "{line}: {out:?}");
            out.join("\n")
        };
        // a VRF the switch does not hold, each command family in its own words
        let mut s = typed_24t(&e, &[]);
        for (line, want) in [
            (
                "ip multicast-routing vrf V1",
                "% VPN Routing instance V1 does not exist.",
            ),
            (
                "ip pim vrf V1 rp-address 10.0.0.1",
                "% VPN Routing instance V1 does not exist.",
            ),
            (
                "ipv6 multicast-routing vrf V1",
                "%VPN Routing instance V1 does not exist. Create first",
            ),
            (
                "snmp-server host 10.0.0.1 vrf V1 public",
                "% VRF name V1 does not exist",
            ),
            (
                "ntp server vrf V1 10.0.0.1",
                "% IP  routing table V1 does not exist",
            ),
        ] {
            assert_eq!(said(&mut s, line), want);
        }
        let mut s = typed_24t(&e, &["interface Vlan10"]);
        assert_eq!(said(&mut s, "vrf forwarding V1"), "% VRF V1 not configured");
        let mut s = typed_24t(&e, &["vrf definition V1", "exit", "interface Vlan10"]);
        assert_eq!(s.type_line("vrf forwarding V1").1, Result::Ok);
        // an OSPF process in a VRF, from a process's mode too, which hands the line up
        let mut s = typed_24t(&e, &["router ospf 1"]);
        assert_eq!(
            said(&mut s, "router ospf 12 vrf V2"),
            "Unknown VRF specified"
        );
        assert_eq!(s.mode(), "config");
        let mut s = typed_24t(&e, &["vrf definition V2", "exit"]);
        assert_eq!(s.type_line("router ospf 12 vrf V2").1, Result::Ok);
        assert_eq!(s.mode(), "config-router");
        // a global SXP filter group before its filter list, from a filter group's mode too
        let mut s = typed_24t(&e, &["cts sxp filter-group listener FGA"]);
        assert_eq!(
            said(&mut s, "cts sxp filter-group listener global FLB"),
            "Filter-list FLB not configured"
        );
        assert_eq!(s.mode(), "config");
        let mut s = typed_24t(&e, &["cts sxp filter-list FLB", "exit"]);
        assert_eq!(
            s.type_line("cts sxp filter-group speaker global FLB").1,
            Result::Ok
        );
        let mut s = typed_24t(&e, &["router bgp 65000"]);
        assert_eq!(
            said(&mut s, "address-family ipv4 vrf V1"),
            "% VRF V1 does not exist."
        );
        assert_eq!(s.mode(), "config-router");
        // a neighbor's policy, once the process has an address family; a template not made
        let mut s = typed_24t(
            &e,
            &["router bgp 65000", "neighbor 10.0.0.2 remote-as 65001"],
        );
        assert_eq!(s.type_line("neighbor 10.0.0.2 activate").1, Result::Ok);
        s.type_line("address-family ipv6");
        s.type_line("exit-address-family");
        assert_eq!(
            said(&mut s, "neighbor 10.0.0.2 send-community both"),
            "% Policy commands not allowed without an address family"
        );
        assert_eq!(
            said(&mut s, "neighbor 10.0.0.2 inherit peer-session S1"),
            "%Template not found. Please create a template first."
        );
        assert_eq!(
            s.type_line("neighbor 10.0.0.2 description uplink").1,
            Result::Ok
        );
        // a private VLAN association naming a VLAN the switch lacks, the primary read first
        let mut s = typed_24t(&e, &["vlan 20"]);
        assert_eq!(
            said(&mut s, "private-vlan association 501-503"),
            "%Command rejected: invalid private vlan association between vlan20 and vlan501. \
             VLAN 501 data is not available."
        );
        let mut s = typed_24t(&e, &["vlan 20", "exit", "interface GigabitEthernet1/0/5"]);
        assert_eq!(
            said(&mut s, "switchport private-vlan host-association 3 20"),
            "%Command rejected: invalid private vlan association between vlan3 and vlan20. \
             VLAN 3 data is not available."
        );
        // a secondary VLAN's SVI must be down
        let mut s = typed_24t(&e, &["vlan 30", "exit", "interface Vlan30", "vlan 30"]);
        assert_eq!(
            said(&mut s, "private-vlan isolated"),
            "%Command rejected: invalid private vlan type assignment. VLAN 30's interface is not \
             administratively down and cannot be set to secondary VLAN."
        );
        // the tunnels PIM registers through, past those configured
        let mut s = typed_24t(
            &e,
            &[
                "ip multicast-routing",
                "interface Loopback0",
                "ip address 10.0.0.1 255.255.255.255",
                "interface Tunnel0",
                "exit",
                "ip pim rp-address 10.0.0.1",
            ],
        );
        for n in [1, 2] {
            assert_eq!(
                said(&mut s, &format!("interface tunnel {n}")),
                format!("%Tunnel{n} used by PIM for Registering, configuration not allowed")
            );
            assert_eq!(s.mode(), "config");
        }
        assert_eq!(s.type_line("interface tunnel 3").1, Result::Ok);
        // a policy an interface or VLAN names before it exists, its removal too
        let mut s = typed_24t(&e, &["interface GigabitEthernet1/0/3"]);
        for line in ["service-policy input PM1", "no service-policy input PM1"] {
            assert_eq!(said(&mut s, line), "% policy map PM1 not configured");
        }
        for (line, policy) in [
            ("ipv6 source-guard attach-policy SG1", "SG1"),
            ("ipv6 destination-guard attach-policy DG2", "DG2"),
        ] {
            assert_eq!(
                said(&mut s, line),
                format!("% Policy {policy} not defined for the feature\n")
            );
        }
        let mut s = typed_24t(&e, &["vlan configuration 10"]);
        assert_eq!(
            said(&mut s, "device-tracking attach-policy DT1"),
            "% Policy DT1 not defined for the feature\n"
        );
        // a route target before the VRF's route distinguisher
        let mut s = typed_24t(&e, &["vrf definition V1"]);
        assert_eq!(
            said(&mut s, "route-target both 1:1"),
            "% VRF V1 does not have an RD configured, please configure the RD before configuring \
             route-target"
        );
        assert_eq!(s.type_line("rd 1:1").1, Result::Ok);
        assert_eq!(s.type_line("route-target both 1:1").1, Result::Ok);
        // an OSPFv3 process opened the old way
        let mut s = typed_24t(&e, &["router ospfv3 1", "exit"]);
        assert_eq!(
            said(&mut s, "ipv6 router ospf 1"),
            "% OSPFv3: Use \"router ospfv3 1\" to configure this process\n% Failed to create \
             routing protocol ospf"
        );
        assert_eq!(s.mode(), "config");
        // a VFI named before its VPN ID
        let mut s = typed_24t(
            &e,
            &["l2vpn vfi context VF1", "exit", "vlan configuration 10"],
        );
        assert_eq!(
            said(&mut s, "member vfi VF1"),
            "% VFI VF1 must be configured with a valid vpn id first."
        );
        let mut s = typed_24t(
            &e,
            &[
                "l2vpn vfi context VF1",
                "vpn id 10",
                "exit",
                "vlan configuration 10",
            ],
        );
        assert_eq!(s.type_line("member vfi VF1").1, Result::Ok);
        // an IP service on a LISP instance that carries Ethernet
        let mut s = typed_24t(
            &e,
            &[
                "router lisp",
                "instance-id 3",
                "service ethernet",
                "exit-service-ethernet",
            ],
        );
        assert_eq!(
            said(&mut s, "service ipv4"),
            "% Instance ID 3 has 'service ethernet' enabled.\n L2 and L3 services are exclusive, \
             remove L2 service first."
        );
        assert_eq!(s.mode(), "config-lisp-inst");
        let mut s = typed_24t(&e, &["router lisp", "instance-id 4"]);
        assert_eq!(s.type_line("service ipv4").1, Result::Ok);
        assert_eq!(s.mode(), "config-lisp-inst-srv-ipv4");
        // objects a line names before they exist: a NAT pool, access lists, role-based lists, a
        // trustpoint, a regex parameter map, a first-hop security policy
        let mut s = typed_24t(&e, &[]);
        for (line, want) in [
            (
                "ip nat inside source list 1 pool P1",
                "%Pool P1 not configured \n",
            ),
            ("ntp access-group peer 99", "%Access-list 99 is not defined"),
            (
                "ntp access-group serve A1 kod",
                "%Access-list A1 not defined",
            ),
            (
                "cts role-based permissions from 3 to 5 R3 R5",
                "% ACL R3 is not defined.\n% ACL R5 is not defined.",
            ),
            (
                "crypto pki certificate chain T1",
                "% CA trustpoint for cert chain not known.\n",
            ),
            (
                "crypto pki enroll T1",
                "% CA server trustpoint 'T1' is not known.\n",
            ),
        ] {
            assert_eq!(said(&mut s, line), want);
            assert_eq!(s.mode(), "config");
        }
        let mut s = typed_24t(
            &e,
            &[
                "ip nat pool P1 10.0.0.1 10.0.0.9 netmask 255.255.255.0",
                "access-list 99 permit any",
            ],
        );
        for line in [
            "ip nat inside source list 1 pool P1",
            "ntp access-group peer 99",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        let mut s = typed_24t(&e, &["parameter-map type umbrella global"]);
        assert_eq!(
            said(&mut s, "local-domain RX1"),
            "% Parameter-map regex RX1 does not exist"
        );
        let mut s = typed_24t(
            &e,
            &[
                "parameter-map type regex RX1",
                "exit",
                "parameter-map type umbrella global",
            ],
        );
        assert_eq!(s.type_line("local-domain RX1").1, Result::Ok);
        let mut s = typed_24t(&e, &["vlan configuration 10"]);
        for line in [
            "ipv6 nd raguard attach-policy RG1",
            "ipv6 dhcp guard attach-policy DG1",
        ] {
            let policy = line.split_whitespace().last().unwrap_or("");
            assert_eq!(
                said(&mut s, line),
                format!("% Policy {policy} not defined for the feature\n")
            );
        }
        // a classifier condition while the classifier runs
        let mut s = typed_24t(&e, &["device classifier"]);
        assert_eq!(
            said(&mut s, "device classifier condition C1"),
            "% Device classifier is enabled.Please disable it and execute the command again."
        );
        assert_eq!(
            said(&mut s, "device classifier device-type T1"),
            "% Device classifier is enabled. Please disable it and execute the command again.\n"
        );
        // a subnet another interface holds, named by the lowest overlapping subnet
        let mut s = typed_24t(
            &e,
            &[
                "interface Vlan30",
                "ip address 10.30.0.1 255.255.255.0",
                "interface Vlan20",
                "ip address 10.20.0.1 255.255.255.0",
                "interface Vlan21",
            ],
        );
        assert_eq!(
            said(&mut s, "ip address 10.20.0.2 255.0.0.0"),
            "% 10.0.0.0 overlaps with Vlan20"
        );
        assert_eq!(
            s.type_line("ip address 10.21.0.1 255.255.255.0").1,
            Result::Ok
        );
        // a BGP neighbor activated before its remote AS
        let mut s = typed_24t(
            &e,
            &[
                "router bgp 65000",
                "neighbor 10.0.0.2 remote-as 65001",
                "address-family ipv4",
            ],
        );
        assert_eq!(s.type_line("neighbor 10.0.0.2 activate").1, Result::Ok);
        assert_eq!(
            said(&mut s, "neighbor 10.0.0.3 activate"),
            "% Specify remote-as or peer-group commands first"
        );
        // in the router's own mode: any neighbor line but its remote AS, peer group or template
        let mut s = typed_24t(
            &e,
            &[
                "interface Vlan20",
                "ip address 10.20.0.1 255.255.255.0",
                "exit",
                "router bgp 65000",
                "neighbor 10.0.0.2 remote-as 65001",
                "neighbor G1 peer-group",
            ],
        );
        for line in [
            "neighbor 10.0.0.3 fall-over bfd",
            "neighbor 10.0.0.4 bmp-activate server 1",
        ] {
            assert_eq!(
                said(&mut s, line),
                "% Specify remote-as or peer-group commands first"
            );
        }
        for line in [
            "neighbor 10.0.0.2 fall-over bfd",
            "neighbor G1 description core",
        ] {
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        assert_eq!(
            said(&mut s, "neighbor 10.20.0.1 remote-as 2"),
            "% Cannot configure the local system as neighbor"
        );
        assert_eq!(
            said(&mut s, "neighbor fe80::2%Ethernet1/0 remote-as 65400"),
            "% Link local peer outside address-family scope"
        );
    }

    #[test]
    fn a_show_naming_a_vrf_the_switch_lacks_says_what_ios_xe_says() {
        let e = c9300_17_6();
        let stack = crate::platform::parse_stack("C9300-24T").unwrap();
        let mut s = Session::new(&e, Device::new(stack));
        s.type_line("enable");
        for (line, want) in [
            (
                "show ip route vrf V9",
                vec!["% IP routing table vrf V9 does not exist"],
            ),
            ("show ip arp vrf V9", vec!["% VRF V9 does not exist."]),
            ("show ip bgp vpnv4 vrf V9", vec!["% Unknown VRF", ""]),
            ("show ip vrf detail V9", vec!["% No VRF named V9"]),
            (
                "show mpls forwarding-table vrf V9",
                vec!["% IP routing table V9 doesn't exist"],
            ),
        ] {
            let (out, r) = s.type_line(line);
            assert_eq!(
                (out, r),
                (want.iter().map(|w| w.to_string()).collect(), Result::Error),
                "{line}"
            );
        }
        // the management VRF is the switch's own
        let (out, _) = s.type_line("show ip route vrf Mgmt-vrf");
        assert!(!out.iter().any(|l| l.contains("does not exist")), "{out:?}");
    }

    #[test]
    fn a_span_line_its_sessions_refuse_says_what_ios_xe_says() {
        let e = c9300_17_6();
        let said = |s: &mut Session, line: &str| {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Error, "{line}: {out:?}");
            out.join("\n")
        };
        let mut s = typed_24t(
            &e,
            &[
                "monitor session 1 source interface GigabitEthernet1/0/5",
                "monitor session 1 destination interface GigabitEthernet1/0/6",
                "monitor session 2 filter vlan 1 - 5 , 9",
            ],
        );
        for (line, want) in [
            (
                "monitor session 2 destination interface GigabitEthernet1/0/6 ingress vlan 77",
                "% Warning: specified default VLAN (77) for ingress on dest port does not exist.\n\
                 % Interface(s) Gi1/0/6 already configured as monitor destinations in other \
                 monitor sessions",
            ),
            (
                "monitor session 2 source interface GigabitEthernet1/0/6 rx",
                "% Interface(s) Gi1/0/6 already configured as monitor destinations",
            ),
            (
                "monitor session 2 destination interface GigabitEthernet1/0/5",
                "% Interface(s) Gi1/0/5 already configured as monitor sources",
            ),
            (
                "monitor session 1 destination remote vlan 700",
                "% Session - 1 is a not a Remote Source session. Can't add RSPAN VLAN as \
                 destination",
            ),
            (
                "monitor session 1 source remote vlan 701",
                "% Cannot add RSPAN VLAN as source for SPAN session 1 as it is not a RSPAN \
                 Destination session",
            ),
            (
                "monitor session 2 destination remote vlan 5",
                "% Configuring Destination RSPAN VLANs 5, but the same vlan is \n% already \
                 configured as filter vlan",
            ),
            (
                "monitor session 2 filter ip access-group 7",
                "% vlan filter can not be applied with FSPAN",
            ),
        ] {
            assert_eq!(said(&mut s, line), want);
        }
        // a remote VLAN makes session 2 a remote source: ports cannot be its destination
        assert_eq!(
            s.type_line("monitor session 2 destination remote vlan 702")
                .1,
            Result::Ok
        );
        assert_eq!(
            said(&mut s, "monitor session 2 source remote vlan 703"),
            "% RSPAN VLAN already exists as destination. The same session cannnot have RSPAN VLAN \
             both as source and destination"
        );
        assert_eq!(
            said(
                &mut s,
                "monitor session 2 destination interface GigabitEthernet1/0/7"
            ),
            "% Cannot add ports as destination for session 2 - a RSPAN Source session"
        );
        // a source may feed several sessions
        assert_eq!(
            s.type_line("monitor session 2 source interface GigabitEthernet1/0/5")
                .1,
            Result::Ok
        );
    }

    #[test]
    fn portfast_warns_when_it_changes_and_says_where_it_takes_effect() {
        let e = c9300_17_6();
        let mut s = typed_24t(&e, &["interface GigabitEthernet1/0/12"]);
        let warning = [
            "%Warning: portfast should only be enabled on ports connected to a single",
            " host. Connecting hubs, concentrators, switches, bridges, etc... to this",
            " interface  when portfast is enabled, can cause temporary bridging loops.",
            " Use with CAUTION",
            "",
        ];
        let (out, r) = s.type_line("spanning-tree portfast trunk");
        assert_eq!((out, r), (warning.map(String::from).to_vec(), Result::Ok));
        let (out, r) = s.type_line("spanning-tree portfast");
        let mut want: Vec<String> = warning.map(String::from).to_vec();
        want.push("%Portfast has been configured on GigabitEthernet1/0/12 but will only".into());
        want.push(" have effect when the interface is in a non-trunking mode.".into());
        assert_eq!((out, r), (want, Result::Ok));
        // typed again it is no change
        let (out, r) = s.type_line("spanning-tree portfast");
        assert_eq!((out.len(), r), (0, Result::Ok));
        assert_eq!(
            shown(&s, &["spanning-tree portfast"]),
            [" spanning-tree portfast"]
        );
        let (out, _) = s.type_line("spanning-tree portfast trunk");
        assert_eq!(out.len(), warning.len());
    }

    #[test]
    fn mka_mld_and_dhcpv6_server_lines_warn_as_ios_xe_does() {
        let e = c9300_17_6();
        let mut s = typed_24t(&e, &["mka policy P1"]);
        let (out, r) = s.type_line("macsec-cipher-suite gcm-aes-256");
        assert_eq!(r, Result::Ok);
        assert_eq!(
            out,
            [
                "",
                "% If cipher currently being used is not configured, new SAK will be generated",
                "",
            ]
        );
        assert_eq!(s.type_line("macsec-cipher-suite gcm-aes-256").0.len(), 0);
        let mut s = typed_24t(&e, &["interface Vlan20"]);
        let (out, r) = s.type_line("ipv6 mld static-group ff04::10 include 100::1");
        assert_eq!(
            (out, r),
            (
                vec![
                    "%Warning: Enable MLD router-side for this command to be effective".to_string()
                ],
                Result::Ok
            )
        );
        let (out, r) = s.type_line("ipv6 dhcp server automatic");
        assert_eq!(
            (out, r),
            (
                vec!["% Warning: Pool  not configured globally - configuring anyway.".to_string()],
                Result::Ok
            )
        );
        let (out, r) = s.type_line("ipv6 dhcp server POOLA");
        assert_eq!(
            (out, r),
            (
                vec![
                    "% Warning: Pool POOLA not configured globally - configuring anyway."
                        .to_string()
                ],
                Result::Ok
            )
        );
        let mut s = typed_24t(&e, &["ipv6 dhcp pool POOLB", "exit", "interface Vlan20"]);
        let (out, r) = s.type_line("ipv6 dhcp server POOLB");
        assert_eq!((out.len(), r), (0, Result::Ok), "{out:?}");
        let mut s = typed_24t(&e, &["ipv6 multicast-routing", "interface Vlan20"]);
        let (out, r) = s.type_line("ipv6 mld static-group ff04::10 include 100::1");
        assert_eq!((out.len(), r), (0, Result::Ok));
    }

    #[test]
    fn auto_qos_and_a_default_sgt_mapping_warn_as_ios_xe_does() {
        let e = c9300_17_6();
        let mut s = typed_24t(&e, &[]);
        let (out, r) = s.type_line("auto qos global compact");
        assert_eq!(r, Result::Ok);
        assert_eq!(
            out,
            ["%Warning: This command will reset all autoqos generated configurations"]
        );
        let (out, r) = s.type_line("cts role-based sgt-map 0.0.0.0/0 sgt 3");
        assert_eq!(r, Result::Ok);
        assert_eq!(
            out,
            ["%Please ensure default route is created using ip route 0.0.0.0 command"]
        );
        let mut s = typed_24t(&e, &["ip route 0.0.0.0 0.0.0.0 10.0.0.1"]);
        let (out, r) = s.type_line("cts role-based sgt-map 0.0.0.0/0 sgt 3");
        assert_eq!((out.len(), r), (0, Result::Ok));
    }

    #[test]
    fn an_ospf_reference_bandwidth_change_warns_once() {
        let e = c9300_17_6();
        let mut s = typed_24t(&e, &["router ospf 1"]);
        let (out, r) = s.type_line("auto-cost reference-bandwidth 5");
        assert_eq!(r, Result::Ok);
        assert_eq!(
            out,
            [
                "% OSPF: Reference bandwidth is changed. ",
                "        Please ensure reference bandwidth is consistent across all routers.",
            ]
        );
        let (out, r) = s.type_line("auto-cost reference-bandwidth 5");
        assert_eq!((out.len(), r), (0, Result::Ok));
    }

    #[test]
    fn a_line_needing_configuration_first_says_what_ios_xe_says() {
        let e = c9300_17_6();
        let refused = |opened: &[&str], line: &str, said: &[&str]| {
            let mut s = typed_24t(&e, opened);
            let (out, r) = s.type_line(line);
            assert_eq!(
                (out, r),
                (
                    said.iter().map(|l| l.to_string()).collect::<Vec<_>>(),
                    Result::Error
                ),
                "{line}"
            );
        };
        refused(
            &["ip access-list fqdn F1", "exit"],
            "ipv6 access-list fqdn F1",
            &["% A(n) IP FQDN access list with this name already exists"],
        );
        refused(
            &["router isis T1"],
            "nsf cisco",
            &["%ISIS not running (configure NET)"],
        );
        refused(
            &["router ospf 1", "nsf cisco enforce global"],
            "nsf ietf restart-interval 60",
            &["% OSPF: Cannot enable both Cisco NSF and IETF Graceful Restart"],
        );
        let af = ["vrf definition V1", "address-family ipv4"];
        refused(
            &[af[0], af[1], "mdt default 226.10.10.10"],
            "mdt default 232.1.1.1",
            &["% 226.10.10.10 has already been configured for vrf V1"],
        );
        refused(
            &af,
            "mdt log-reuse",
            &["% First configure the data MDT group!"],
        );
        refused(
            &af,
            "mdt data threshold 50",
            &["% MDT data must be configured first"],
        );
        refused(
            &[],
            "crypto pki import T1 certificate",
            &["% CA server trustpoint 'T1' is not known.", ""],
        );
        refused(
            &[],
            "ip mroute vrf V1 224.100.0.5 255.255.255.255 fallback-lookup vrf V2",
            &["% VPN Routing instance V1 does not exist."],
        );
        refused(
            &["end"],
            "show ip wccp summary",
            &["% WCCP version 2 is not enabled"],
        );
        refused(
            &["end"],
            "show spanning-tree mst 3 detail",
            &["% Switch is not in mst mode", ""],
        );
        refused(
            &["router ospf 1", "end"],
            "show ip ospf interface brief topology VOICE",
            &["%OSPF: Specified topology not found for any OSPF process"],
        );
        refused(
            &[
                "router bgp 65000",
                "neighbor 10.0.0.1 remote-as 65001",
                "end",
            ],
            "show ip bgp neighbors 10.0.0.2 policy",
            &["% No such neighbor or address family"],
        );
        let mut s = typed_24t(
            &e,
            &[
                "router bgp 65000",
                "neighbor 10.0.0.1 remote-as 65001",
                "end",
            ],
        );
        let (out, _) = s.type_line("show ip bgp neighbors 10.0.0.1 policy");
        assert_ne!(
            out.first().map(String::as_str),
            Some("% No such neighbor or address family")
        );
        refused(
            &[],
            "snmp-server engineID local 1234",
            &["%Engine ID must be 10 hexadecimal characters or more"],
        );
        refused(
            &["interface GigabitEthernet1/0/1"],
            "duplex half",
            &[
                "",
                "% Duplex cannot be set to half when speed autonegotiation subset contains \
                 1Gbps,2.5Gbps,5Gbps or 10Gbps",
                "",
            ],
        );
        refused(
            &["interface GigabitEthernet1/0/3"],
            "switchport nonegotiate",
            &[
                "Command rejected: Conflict between 'nonegotiate' and 'dynamic' status on this \
                 interface: Gi1/0/3",
            ],
        );
        refused(
            &["interface GigabitEthernet1/0/4", "switchport mode trunk"],
            "network-policy 60",
            &["Command rejected: Network Policy is not supported on a trunk port"],
        );
        refused(
            &[
                "aaa new-model",
                "interface GigabitEthernet1/0/5",
                "switchport mode access",
                "dot1x pae authenticator",
            ],
            "cts manual",
            &["Command rejected (Gi1/0/5): conflict with Dot1x Auth", ""],
        );
        refused(
            &["interface GigabitEthernet1/0/6", "umbrella out"],
            "umbrella in T1",
            &["% 'umbrella out' already configured on interface"],
        );
        refused(
            &["interface Vlan20", "ipv6 dhcp server automatic"],
            "ipv6 address dhcp rapid-commit",
            &["% Interface is in DHCP server mode"],
        );
        refused(
            &["ntp trusted-key 1"],
            "ntp trusted-key 1",
            &["%NTP trusted-key is invalid : key already configured "],
        );
        refused(
            &["track 2 interface GigabitEthernet1/0/3 line-protocol"],
            "track 2 ip sla 123 state",
            &["% Cannot change tracked object 2 - delete old config first"],
        );
        refused(
            &["router rip"],
            "offset-list 103 in 10",
            &["% Invalid access list name."],
        );
        refused(
            &[],
            "ip msdp redistribute list 21",
            &["% Invalid access list name."],
        );
        refused(
            &["interface GigabitEthernet1/0/7", "switchport mode access"],
            "rep stcn stp",
            &[
                "Command rejected: conflict with the current Switchport Mode",
                "",
            ],
        );
        // the same lines once the configuration has what they need
        let mut s = typed_24t(&e, &["router rip"]);
        assert_eq!(s.type_line("offset-list 21 out 10").1, Result::Ok);
        assert_eq!(s.type_line("ip msdp redistribute list 100").1, Result::Ok);
        let mut s = typed_24t(
            &e,
            &["track 2 interface GigabitEthernet1/0/3 line-protocol"],
        );
        assert_eq!(
            s.type_line("track 2 interface GigabitEthernet1/0/1 ip routing")
                .1,
            Result::Ok
        );
        let mut s = typed_24t(&e, &["interface GigabitEthernet1/0/1", "speed 100"]);
        assert_eq!(s.type_line("duplex half").1, Result::Ok);
        let mut s = typed_24t(
            &e,
            &["interface GigabitEthernet1/0/3", "switchport mode access"],
        );
        assert_eq!(s.type_line("switchport nonegotiate").1, Result::Ok);
        assert_eq!(s.type_line("network-policy 60").1, Result::Ok);
        assert_eq!(
            s.type_line("snmp-server engineID local 0123456789").1,
            Result::Ok
        );
        let mut s = typed_24t(&e, &[af[0], af[1], "mdt default 226.10.10.10"]);
        assert_eq!(s.type_line("mdt default 226.10.10.10").1, Result::Ok);
        let mut s = typed_24t(&e, &["router isis T1", "net 49.0001.0000.0000.0001.00"]);
        assert_eq!(s.type_line("nsf cisco").1, Result::Ok);
        let mut s = typed_24t(&e, &["router ospf 1"]);
        assert_eq!(s.type_line("nsf ietf restart-interval 60").1, Result::Ok);
        let mut s = typed_24t(&e, &["spanning-tree mode mst", "end"]);
        let (out, _) = s.type_line("show spanning-tree mst 3 detail");
        assert_ne!(
            out.first().map(String::as_str),
            Some("% Switch is not in mst mode")
        );
        let (out, _) = s.type_line("show spanning-tree mst configuration");
        assert_ne!(
            out.first().map(String::as_str),
            Some("% Switch is not in mst mode")
        );
        let mut s = typed_24t(&e, &["end"]);
        let (out, r) = s.type_line("show spanning-tree mst configuration");
        assert_eq!(r, Result::Ok);
        assert_eq!(
            out.first().map(String::as_str),
            Some("% Switch is not in mst mode")
        );
        assert!(out.len() > 1, "{out:?}");
    }

    #[test]
    fn a_line_its_configuration_already_answers_says_what_ios_xe_says() {
        let e = c9300_17_6();
        let mut s = typed_24t(&e, &["spanning-tree backbonefast"]);
        let (out, r) = s.type_line("spanning-tree backbonefast");
        assert_eq!(
            (out, r),
            (
                vec!["% Backbonefast already enabled".to_string(), String::new()],
                Result::Error
            )
        );
        let mut s = typed_24t(&e, &["template T1"]);
        let (out, r) = s.type_line("spanning-tree port-priority 20");
        assert_eq!(
            (out, r),
            (
                vec![
                    "% Port Priority in increments of 16 is required".to_string(),
                    String::new()
                ],
                Result::Error
            )
        );
        assert_eq!(s.type_line("spanning-tree port-priority 32").1, Result::Ok);
        let co_exist = "% prefix-list and access-list can not co-exist in one route-map sequence.";
        let mut s = typed_24t(&e, &["route-map RM1 permit 10"]);
        // with no prefix list the sequence takes access lists
        assert_eq!(s.type_line("match ip address 5").1, Result::Ok);
        let mut s = typed_24t(
            &e,
            &["route-map RM1 permit 10", "match ip address prefix-list P1"],
        );
        let (out, r) = s.type_line("match ip address 5 80");
        assert_eq!((out, r), (vec![co_exist.to_string(); 2], Result::Error));
        let (out, r) = s.type_line("match ip address 2");
        assert_eq!((out, r), (vec![co_exist.to_string()], Result::Error));
        // a route-source access list is checked against a route-source prefix list
        assert_eq!(s.type_line("match ip route-source 10").1, Result::Ok);
    }

    #[test]
    fn a_line_taken_with_a_warning_says_it_as_ios_xe_does() {
        let e = c9300_17_6();
        let mut s = typed_24t(&e, &["policy-map type control subscriber PC1", "exit"]);
        let (out, r) = s.type_line("policy-map type control subscriber PC1");
        assert_eq!(r, Result::Ok);
        assert_eq!(
            out[0],
            "%Warning, modifying policy-map \"PC1\" may impact sessions hosted on ports for which \
             \"PC1\" has already been applied."
        );
        let mut s = typed_24t(
            &e,
            &["interface Vlan20", "ip address 10.20.0.1 255.255.255.0"],
        );
        let (out, r) = s.type_line("standby 1 ip 10.1.1.254");
        assert_eq!(r, Result::Ok);
        assert_eq!(
            out,
            ["% Warning: address is not within a subnet on this interface"]
        );
        let (out, r) = s.type_line("standby 1 ip 10.1.1.254");
        assert_eq!((out.len(), r), (0, Result::Ok));
        let (out, r) = s.type_line("standby 2 ip 10.20.0.254");
        assert_eq!((out.len(), r), (0, Result::Ok));
        // each group keeps its own address: group 1's, typed again after group 2's, is no change
        let (out, r) = s.type_line("standby 1 ip 10.1.1.254");
        assert_eq!((out.len(), r), (0, Result::Ok));
        assert_eq!(
            shown(&s, &["standby"]),
            [" standby 1 ip 10.1.1.254", " standby 2 ip 10.20.0.254"]
        );
        // a group with an IPv4 address refuses a bare `ip` and any IPv6 line
        let (out, r) = s.type_line("standby 1 ip");
        assert_eq!(
            (out, r),
            (
                vec![
                    "% Failed. Unconfigure existing primary vIP address configured for the group."
                        .to_string()
                ],
                Result::Error
            )
        );
        let (out, r) = s.type_line("standby 2 ipv6 autoconfig");
        assert_eq!(
            (out, r),
            (
                vec!["% Group configured for different address family".to_string()],
                Result::Error
            )
        );
        // a group with no address yet takes both
        assert_eq!(s.type_line("standby 3 ipv6 autoconfig").1, Result::Ok);
        assert_eq!(s.type_line("standby 4 ip").1, Result::Ok);
        let mut s = typed_24t(&e, &["mka policy 2", "exit"]);
        let (out, r) = s.type_line("mka policy 2");
        assert_eq!(r, Result::Ok);
        assert_eq!(
            out,
            [
                "",
                "% MKA Policy \"2\" may have associated active MKA Sessions.",
                "",
                "  Changes to MKA Policy \"2\" values",
                "  will cause all associated active MKA Sessions to be cleared.",
                ""
            ]
        );
        assert_eq!(s.mode(), "config-mka-policy");
        // legacy 802.1X configuration takes the line silently, new-style calls it deprecated
        let mut s = typed_24t(&e, &["aaa new-model", "interface GigabitEthernet1/0/4"]);
        let (out, r) = s.type_line("authentication port-control auto");
        assert_eq!((out, r), (vec![], Result::Ok));
        let mut s = typed_24t(
            &e,
            &[
                "aaa new-model",
                "policy-map type control subscriber PC",
                "exit",
                "interface GigabitEthernet1/0/4",
            ],
        );
        let (out, r) = s.type_line("authentication port-control auto");
        assert_eq!(r, Result::Ok);
        assert_eq!(
            out,
            [
                "%Command deprecated (authentication port-control auto) - use access-session instead",
                ""
            ]
        );
        assert_eq!(
            s.type_line("authentication periodic").0,
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_first_word_a_submode_only_abbreviates_is_the_enclosing_mode_s_whole_word() {
        let e = c9300_17_6();
        // the key chain's `key` takes a number; the key's own `key-string` is not read
        let mut s = typed_24t(&e, &["key chain K1", "key 1"]);
        for line in ["key rad123", "key 0 auth-key"] {
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Error);
            // under the space after `key` (the lab C9300-24T and C9200L-48T-4G, 17.6.4)
            assert_eq!(
                caret_at(&out),
                "Switch(config-keychain-key)#key".len(),
                "{line}"
            );
            assert_eq!(s.mode(), "config-keychain-key");
        }
        assert_eq!(s.type_line("key 2").1, Result::Ok);
        assert_eq!(s.type_line("key-str secret").1, Result::Ok);
        // a mode further out that takes the line whole takes it
        assert_eq!(s.type_line("key chain K2").1, Result::Ok);
        assert_eq!(s.mode(), "config-keychain");
    }

    #[test]
    fn a_vlan_list_written_with_spaces_is_one_list() {
        let e = c9300_17_6();
        let mut s = typed_24t(&e, &[]);
        let (out, r) = s.type_line("monitor session 2 filter vlan 1 - 5 , 9");
        assert_eq!(r, Result::Ok, "{out:?}");
        // a comma kept against the VLAN before it is no such list
        assert_eq!(
            s.type_line("vlan configuration 101-110, 200").1,
            Result::Error
        );
    }

    #[test]
    fn the_lab_s_submodes_take_what_it_took_there() {
        let e = c9300_17_6();
        let cases: [(&[&str], &str, &str); 8] = [
            (&["ipv6 router ospf 1"], "router-id 10.0.0.1", "config-rtr"),
            (
                &["crypto ipsec transform-set T1 esp-aes esp-sha-hmac"],
                "mode tunnel",
                "cfg-crypto-trans",
            ),
            (
                &["vlan configuration 10"],
                "member evpn-instance 1 vni 10000",
                "config-vlan-config",
            ),
            (
                &["vlan configuration 10"],
                "mdns-sd gateway",
                "config-vlan-mdns-sd",
            ),
            (
                &["stealthwatch-cloud-monitor"],
                "service-key abc",
                "config-swc-monitor",
            ),
            (&["router lisp"], "instance-id 3", "config-lisp-inst"),
            (
                &["key chain KC macsec"],
                "key 2000",
                "config-keychain-macsec-key",
            ),
            (
                &["policy-map P", "class class-default"],
                "police cir 1000000 pir 2000000",
                "config-pmap-c-police",
            ),
        ];
        for (opened, line, mode) in cases {
            let mut s = typed_24t(&e, opened);
            let (out, r) = s.type_line(line);
            assert_eq!(r, Result::Ok, "{line}: {out:?}");
            assert_eq!(s.mode(), mode, "{line}");
        }
        // a monitor named as the guide's placeholder is a name
        let mut s = typed_24t(&e, &[]);
        assert_eq!(s.type_line("flow monitor M").1, Result::Ok);
        assert_eq!(s.mode(), "config-flow-monitor");
        // a VLAN's own mode takes no mDNS gateway: global configuration does
        let mut s = typed_24t(&e, &["vlan 10"]);
        assert_eq!(s.type_line("mdns-sd gateway").1, Result::Ok);
        assert_eq!(s.mode(), "config-mdns-sd");
    }

    /// The C9300 17.6 books with the crawls the corpus ships, network-essentials' among them.
    fn shipped_17_6() -> Engine {
        let r = crate::release::Release::parse("17.6").unwrap();
        Engine::new(crate::grammars::grammar("c9300", r).unwrap())
    }

    const ESSENTIALS: &str = "license boot level network-essentials addon dna-essentials";

    /// IOS's `% Invalid input` under a line typed at `prompt`, its caret `column` past the prompt.
    fn invalid_under(prompt: &str, column: usize) -> Vec<String> {
        vec![
            format!("{}^", " ".repeat(prompt.chars().count() + column)),
            "% Invalid input detected at '^' marker.".into(),
            String::new(),
        ]
    }

    #[test]
    fn network_essentials_refuses_what_network_advantage_alone_offers_as_the_lab_switch_did() {
        // the lab C9300-24T (17.6.4): each refused at network-essentials under the caret, the mode
        // kept, and taken at network-advantage
        let e = shipped_17_6();
        for (line, column) in [
            ("avb", 1),
            ("service-export mdns-sd controller c1", 8),
            ("monitor session 2 type erspan-source", 18),
            ("interface pseudowire 100", 11),
        ] {
            let mut s = configured(&e, &[ESSENTIALS]);
            let refused = (invalid_under("Switch(config)#", column), Result::Error);
            assert_eq!(s.type_line(line), refused, "{line}");
            assert_eq!(s.mode(), "config", "{line}");
            let mut s = configured(&e, &[LICENSE]);
            assert_eq!(s.type_line(line).1, Result::Ok, "{line}");
        }
        // `l2 traceroute` taken though `?` offers no `l2`, `l2 vfi` refused at the `vfi`
        let mut s = configured(&e, &[ESSENTIALS]);
        assert_eq!(s.type_line("l2 traceroute").1, Result::Ok);
        let mut s = configured(&e, &[ESSENTIALS]);
        assert_eq!(
            s.type_line("l2 vfi v2 autodiscovery").0,
            invalid_under("Switch(config)#", 3)
        );
        // `net` taken first though `?` lists it nowhere, the NET after it refused
        let mut s = configured(&e, &[ESSENTIALS]);
        assert_eq!(
            s.type_line("net 47.0004.004d.0001.0001.0c11.1111.00").0,
            invalid_under("Switch(config)#", 4)
        );
        // `xconnect` on a dot1Q sub-interface, refused at the word
        let subif = [
            ESSENTIALS,
            "interface GigabitEthernet1/0/7.10",
            "encapsulation dot1Q 10",
        ];
        let mut s = configured(&e, &subif);
        assert_eq!(
            s.type_line("xconnect 10.0.0.2 200 encapsulation mpls").0,
            invalid_under("Switch(config-subif)#", 0)
        );
        // an EXEC line naming an interface type the level does not list, under the `-`
        let mut s = configured(&e, &[ESSENTIALS]);
        s.type_line("end");
        assert_eq!(
            s.type_line("show ip ospf neighbor Tunnel-tp").0,
            invalid_under("Switch#", 28)
        );
    }

    #[test]
    fn a_protocol_or_vrf_the_level_leaves_out_says_what_the_lab_switch_said() {
        let e = shipped_17_6();
        let said = |s: &mut Session, line: &str| s.type_line(line);
        let mut s = configured(&e, &[ESSENTIALS]);
        for line in ["router bgp 65000", "router isis"] {
            let (out, r) = said(&mut s, line);
            assert_eq!(r, Result::Error, "{line}");
            assert!(
                out.iter().any(|l| l == "Protocol not in this image"),
                "{line}: {out:?}"
            );
            assert_eq!(s.mode(), "config", "{line}");
        }
        let (out, r) = said(&mut s, "vrf definition blue");
        assert_eq!(r, Result::Error);
        assert!(
            out.iter().any(|l| l == "% Feature is not supported"),
            "{out:?}"
        );
        // the management port's VRF is taken, and at network-advantage every one
        assert_eq!(said(&mut s, "vrf definition Mgmt-vrf").1, Result::Ok);
        let mut s = configured(&e, &[LICENSE]);
        for line in ["router bgp 65000", "exit", "vrf definition blue"] {
            assert_eq!(said(&mut s, line).1, Result::Ok, "{line}");
        }
    }

    #[test]
    fn a_held_answer_s_heading_offers_the_caret_no_word() {
        // the lab C9300-24T (17.6.4) at both levels: `configure terminal` in global configuration
        // under the `e` its `configuration` leaves; at network-essentials, `avb` in a VLAN under the
        // `v` its `are` leaves, where the heading `VLAN configuration commands:` read as a
        // placeholder put it under the `a`
        let e = shipped_17_6();
        for level in [LICENSE, ESSENTIALS] {
            let mut s = configured(&e, &[level]);
            let refused = s.type_line("configure terminal").0;
            assert_eq!(refused, invalid_under("Switch(config)#", 8), "{level}");
        }
        let mut s = configured(&e, &[ESSENTIALS, "vlan 360"]);
        assert_eq!(
            s.type_line("avb").0,
            invalid_under("Switch(config-vlan)#", 1)
        );
        assert_eq!(s.mode(), "config-vlan");
    }

    #[test]
    fn the_gate_reads_a_typed_word_against_each_level_s_answer() {
        let answer = |rows: &[&str]| -> Vec<String> {
            std::iter::once("Interface types:".to_string())
                .chain(rows.iter().map(|r| format!("  {r}")))
                .collect()
        };
        let types = answer(&[
            "Auto-Template     Auto-Template interface",
            "GigabitEthernet   Gigabit Ethernet IEEE 802.3z",
            "Loopback          Loopback interface",
            "Port-channel      Ethernet Channel of interfaces",
            "Tunnel            Tunnel interface",
            "Vlan              Catalyst Vlans",
            "pseudowire        Pseudowire interface",
        ]);
        let refused = ["pseudowire", "Auto-Template"];
        // past the characters another row begins with: `P` of `Port-channel`, `A` of nothing else
        assert_eq!(refused_type(&types, "pseudowire100", &refused), Some(1));
        assert_eq!(refused_type(&types, "Auto-Template1", &refused), Some(0));
        assert_eq!(refused_type(&types, "Loopback0", &refused), None);
        assert_eq!(refused_type(&types, "p1", &refused), None);
        // a template's type is no interface type
        let template = answer(&[
            "ethernet  Ethernet template",
            "pseudowire  Pseudowire template",
        ]);
        assert_eq!(refused_type(&template, "pseudowire", &refused), None);

        let at = answer(&[
            "Hostname  Host name",
            "Tunnel  Tunnel interface",
            "Vlan  Catalyst Vlans",
        ]);
        let above = answer(&[
            "Hostname  Host name",
            "Tunnel  Tunnel interface",
            "Tunnel-tp  MPLS Transport Profile interface",
            "Vlan  Catalyst Vlans",
        ]);
        assert!(type_above_only(&at, &above, "Tunnel-tp"));
        assert!(!type_above_only(&at, &above, "Tunnel5"));
        assert!(!type_above_only(&at, &above, "host1"));
        assert!(!type_above_only(&above, &above, "Tunnel-tp"));

        assert_eq!(best_fit(&types, "Gi1/0/1"), Fit::Word);
        assert_eq!(
            best_fit(
                &answer(&["Vlan  Catalyst Vlans", "Loopback  Loopback"]),
                "Gi1/0/1"
            ),
            Fit::No
        );
        let files = answer(&["flash:  Directory or file name", "<cr>  <cr>"]);
        assert_eq!(best_fit(&files, "flash:a.txt"), Fit::Word);
        assert_eq!(best_fit(&files, "usbflash0:"), Fit::No);
        let numbers = answer(&["<1-66>  SPAN session number", "LINE  Text"]);
        assert_eq!(best_fit(&numbers[..2], "2"), Fit::Word);
        assert_eq!(best_fit(&numbers[..2], "99"), Fit::No);
        assert_eq!(best_fit(&numbers, "anything"), Fit::Rest);

        assert_eq!(token_at("ip  address 10.0.0.1", 4), Some(1));
        assert_eq!(token_at("ip  address 10.0.0.1", 2), None);
        assert_eq!(token_at("ip  address 10.0.0.1", 13), Some(2));
    }

    #[test]
    fn a_port_the_stack_lacks_is_refused_past_the_start_another_row_shares() {
        // the lab C9300-24T (17.6.4), one member: `interface gigabitethernet4/0/1` under the `g` at
        // network-essentials and under the `i` at network-advantage, whose `GMPLS` begins `g`;
        // `interface nve1` at network-essentials past the `n` of `Null`, `PROTECTION_GROUP` listed
        // beside it
        let e = shipped_17_6();
        let typed = |level: &str, line: &str| {
            let stack = crate::platform::parse_stack("C9300-24T+C9300-NM-4G").unwrap();
            let mut s = Session::new(&e, Device::new(stack));
            for (l, boot) in [
                ("enable", false),
                ("configure terminal", false),
                (level, true),
            ] {
                s.redacted = boot;
                s.type_line(l);
            }
            s.redacted = false;
            s.type_line(line).0
        };
        let at = |column| invalid_under("Switch(config)#", column);
        assert_eq!(typed(ESSENTIALS, "interface gigabitethernet4/0/1"), at(10));
        assert_eq!(typed(LICENSE, "interface gigabitethernet4/0/1"), at(11));
        assert_eq!(typed(ESSENTIALS, "interface nve1"), at(11));
        assert_eq!(typed(LICENSE, "interface nve1"), vec![] as Vec<String>);
    }
}
