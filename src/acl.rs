//! Named IPv4 access lists as IOS keeps them, from two lab sessions of 2026-10-07 (c9300 17.6 capture 1, a
//! C9300-24T at 17.6.4, and the lab 2960G at 15.0(2)SE11; `data/live/*/*-session-acl.log`). Every
//! entry has a sequence number: one typed without a number takes the highest plus 10, a number in use
//! is refused, and an entry identical to one held adds nothing. A remark takes the number of the entry
//! it comes before (the 17.15 captures print `20 remark ...` above `20 permit ...`). IOS XE prints
//! the numbers in the running configuration; classic IOS prints the entries without them, a standard
//! list's host entries first and its actions padded to `permit`'s width.

use crate::device::Entry;

/// What an access list holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Standard,
    Extended,
}

/// The kind and name of the access list a block header opens (`ip access-list standard VSWITCH-STD`).
pub fn of(header: &str) -> Option<(Kind, &str)> {
    let words: Vec<&str> = header.split_whitespace().collect();
    if words.len() != 4
        || !words[0].eq_ignore_ascii_case("ip")
        || !words[1].eq_ignore_ascii_case("access-list")
    {
        return None;
    }
    let kind = if words[2].eq_ignore_ascii_case("standard") {
        Kind::Standard
    } else if words[2].eq_ignore_ascii_case("extended") {
        Kind::Extended
    } else {
        return None;
    };
    let name = header.trim_end().rsplit(' ').next()?;
    Some((kind, name))
}

/// Whether a configuration mode is an IPv4 access list's.
pub fn in_mode(mode: &str) -> bool {
    mode == "config-std-nacl" || mode == "config-ext-nacl"
}

/// The sequence number a line starts with and the entry after it (`15 permit ip host 10.0.0.3 any`).
pub fn numbered(line: &str) -> Option<(u32, &str)> {
    let line = line.trim_start();
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let rest = &line[digits..];
    if !rest.starts_with(char::is_whitespace) || rest.trim().is_empty() {
        return None;
    }
    let n: u32 = line[..digits].parse().ok()?;
    (1..=2_147_483_647)
        .contains(&n)
        .then_some((n, rest.trim_start()))
}

/// The number `no <number>` removes.
pub fn unnumbered(line: &str) -> Option<u32> {
    let mut words = line.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("no") {
        return None;
    }
    let n: u32 = words.next()?.parse().ok()?;
    (words.next().is_none() && (1..=2_147_483_647).contains(&n)).then_some(n)
}

fn seq_of(e: &Entry) -> u32 {
    e.key
        .strip_prefix("seq ")
        .and_then(|k| k.split(' ').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn is_remark(e: &Entry) -> bool {
    e.key.ends_with(" remark")
}

fn entry(seq: u32, text: String, remark: bool) -> Entry {
    let key = if remark {
        format!("seq {seq} remark")
    } else {
        format!("seq {seq}")
    };
    Entry { key, text }
}

/// An entry as IOS keeps it: one space between words, and a standard list's `host` dropped
/// (`permit host 10.0.0.7` is kept as `permit 10.0.0.7`).
fn stored(kind: Kind, text: &str) -> String {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    let action = words
        .first()
        .is_some_and(|w| w.eq_ignore_ascii_case("permit") || w.eq_ignore_ascii_case("deny"));
    if kind == Kind::Standard && action && words.len() >= 3 && words[1].eq_ignore_ascii_case("host")
    {
        words.remove(1);
    }
    words.join(" ")
}

fn remark_text(text: &str) -> bool {
    text.split_whitespace()
        .next()
        .is_some_and(|w| w.eq_ignore_ascii_case("remark"))
}

/// The number the next entry typed without one takes: a remark's that waits for its entry, else the
/// highest entry's plus 10.
pub fn next(lines: &[Entry]) -> u32 {
    let top = |remarks: bool| {
        lines
            .iter()
            .filter(|e| is_remark(e) == remarks)
            .map(seq_of)
            .max()
    };
    match (top(false), top(true)) {
        (Some(e), Some(r)) if r > e => r,
        (None, Some(r)) => r,
        (Some(e), _) => e + 10,
        (None, None) => 10,
    }
}

fn sort(lines: &mut [Entry]) {
    lines.sort_by_key(|e| (seq_of(e), !is_remark(e)));
}

/// Adds an entry typed in the list's submode, numbered `seq` or after the others. Returns the error
/// IOS prints for a number in use; an entry identical to one held adds nothing.
pub fn add(lines: &mut Vec<Entry>, kind: Kind, seq: Option<u32>, text: &str) -> Result<(), String> {
    let text = stored(kind, text);
    let remark = remark_text(&text);
    if !remark
        && lines
            .iter()
            .any(|e| !is_remark(e) && e.text.eq_ignore_ascii_case(&text))
    {
        return Ok(());
    }
    let n = match seq {
        Some(n) if !remark && lines.iter().any(|e| !is_remark(e) && seq_of(e) == n) => {
            return Err("% Duplicate sequence number.".into());
        }
        Some(n) => n,
        None => next(lines),
    };
    lines.push(entry(n, text, remark));
    sort(lines);
    Ok(())
}

/// `no <number>`: the entry of that number goes, with its remarks.
pub fn remove_number(lines: &mut Vec<Entry>, seq: u32) {
    lines.retain(|e| seq_of(e) != seq);
}

/// `no <entry>`: the entry with that text goes.
pub fn remove(lines: &mut Vec<Entry>, kind: Kind, text: &str) {
    let text = stored(kind, text);
    let remark = remark_text(&text);
    lines.retain(|e| !(is_remark(e) == remark && e.text.eq_ignore_ascii_case(&text)));
}

/// `ip access-list resequence <name> <start> <step>`: the entries renumbered in order, each remark
/// with the entry after it, a remark after the last entry with the number after it.
pub fn resequence(lines: &mut [Entry], start: u32, step: u32) {
    let mut n = start;
    let mut pending: Vec<usize> = vec![];
    for i in 0..lines.len() {
        if is_remark(&lines[i]) {
            pending.push(i);
            continue;
        }
        for r in pending.drain(..) {
            lines[r].key = format!("seq {n} remark");
        }
        lines[i].key = format!("seq {n}");
        n = n.saturating_add(step);
    }
    for r in pending {
        lines[r].key = format!("seq {n} remark");
    }
}

/// A standard entry that names one host: `permit 10.0.0.9`.
fn host(e: &Entry) -> bool {
    let words: Vec<&str> = e.text.split_whitespace().collect();
    !is_remark(e) && words.len() == 2 && words[1].parse::<std::net::Ipv4Addr>().is_ok()
}

/// The order classic IOS lists entries in: a standard list's host entries first.
fn listed(kind: Kind, lines: &[Entry], classic: bool) -> Vec<&Entry> {
    let mut out: Vec<&Entry> = lines.iter().collect();
    if classic && kind == Kind::Standard {
        out.sort_by_key(|e| !host(e));
    }
    out
}

/// A standard entry's action padded to `permit`'s width (`deny   any`: the 2960G's session, and the
/// lab C9300-24T's `show ip access-lists` at 17.6.4; other IOS XE releases inferred to pad too).
fn padded(kind: Kind, text: &str) -> String {
    match text.split_once(' ') {
        Some((action, rest))
            if kind == Kind::Standard && (action == "permit" || action == "deny") =>
        {
            format!("{action:<6} {rest}")
        }
        _ => text.to_string(),
    }
}

/// An IOS XE 17.6 running configuration's entry: `deny` padded to `permit`'s width in either kind of
/// list (the lab C9300-24T, 17.6.4: ` 20 deny   any log`, ` 30 deny   ip any any log`); 17.15 pads
/// none (ten C9200 and C9300 17.15 captures print ` 30 deny any log`).
fn padded_running(text: &str, pad: bool) -> String {
    match text.split_once(' ') {
        Some(("deny", rest)) if pad => format!("deny   {rest}"),
        _ => text.to_string(),
    }
}

/// The IPv4 access lists IOS XE makes itself, each with its entries as `show ip access-lists`
/// prints them: every one a Catalyst 9300 at 17.6 lists from its factory default on, none of them in
/// the running configuration (the lab C9300-24T, 17.6.4, at factory and with http, web
/// authentication, 802.1X and device tracking configured and removed).
pub const SYSTEM_V4: &[(&str, &[&str])] = &[
    (
        "IP-Adm-V4-Int-ACL-global",
        &[
            "10 permit tcp any any eq www",
            "20 permit tcp any any eq 443",
        ],
    ),
    ("implicit_deny", &["10 deny ip any any"]),
    ("implicit_permit", &["10 permit ip any any"]),
    ("meraki-fqdn-dns", &[]),
    (
        "preauth_v4",
        &[
            "10 permit udp any any eq domain",
            "20 permit tcp any any eq domain",
            "30 permit udp any eq bootps any",
            "40 permit udp any any eq bootpc",
            "50 permit udp any eq bootpc any",
            "60 deny ip any any",
        ],
    ),
];

/// The IPv6 access lists IOS XE makes itself, as `show ipv6 access-list` prints their entries (as
/// `SYSTEM_V4`).
pub const SYSTEM_V6: &[(&str, &[&str])] = &[
    ("implicit_deny_v6", &["deny ipv6 any any sequence 10"]),
    ("implicit_permit_v6", &["permit ipv6 any any sequence 10"]),
    (
        "preauth_v6",
        &[
            "permit udp any any eq domain sequence 10",
            "permit tcp any any eq domain sequence 20",
            "permit icmp any any nd-ns sequence 30",
            "permit icmp any any nd-na sequence 40",
            "permit icmp any any router-solicitation sequence 50",
            "permit icmp any any router-advertisement sequence 60",
            "permit icmp any any redirect sequence 70",
            "permit udp any eq 547 any eq 546 sequence 80",
            "permit udp any eq 546 any eq 547 sequence 90",
            "deny ipv6 any any sequence 100",
        ],
    ),
];

/// The access lists the 2960-X image makes itself, as `SYSTEM_V4` and `SYSTEM_V6`: in every
/// `show access-lists` the lab WS-C2960X-24PS-L printed at 15.2(2)E5 and 15.2(7)E0a, with no 802.1X
/// or AAA configured; `show` titles the preauth lists `(per-user)`.
pub const SYSTEM_2960X_V4: &[(&str, &[&str])] = &[
    (
        "CISCO-CWA-URL-REDIRECT-ACL",
        &[
            "100 deny udp any any eq domain",
            "101 deny tcp any any eq domain",
            "102 deny udp any eq bootps any",
            "103 deny udp any any eq bootpc",
            "104 deny udp any eq bootpc any",
            "105 permit tcp any any eq www",
        ],
    ),
    (
        "preauth_ipv4_acl",
        &[
            "10 permit udp any any eq domain",
            "20 permit tcp any any eq domain",
            "30 permit udp any eq bootps any",
            "40 permit udp any any eq bootpc",
            "50 permit udp any eq bootpc any",
            "60 deny ip any any",
        ],
    ),
];

/// See `SYSTEM_2960X_V4`.
pub const SYSTEM_2960X_V6: &[(&str, &[&str])] = &[(
    "preauth_ipv6_acl",
    &[
        "permit udp any any eq domain sequence 10",
        "permit tcp any any eq domain sequence 20",
        "permit icmp any any nd-ns sequence 30",
        "permit icmp any any nd-na sequence 40",
        "permit icmp any any router-solicitation sequence 50",
        "permit icmp any any router-advertisement sequence 60",
        "permit icmp any any redirect sequence 70",
        "permit udp any eq 547 any eq 546 sequence 80",
        "permit udp any eq 546 any eq 547 sequence 90",
        "deny ipv6 any any sequence 100",
    ],
)];

/// An IPv6 list's entries as `show ipv6 access-list` prints them: the sequence number after the
/// entry (the lab C9300-24T, 17.6.4: ` sequence 10 permit ipv6 2001:DB8::/32 any` prints `permit ipv6
/// 2001:DB8::/32 any sequence 10`); an entry typed with none takes the highest plus 10 (inferred),
/// a remark none.
pub fn show_v6(lines: &[Entry]) -> Vec<String> {
    let mut last = 0;
    let mut out = vec![];
    for e in lines {
        let (seq, text) = match e
            .text
            .strip_prefix("sequence ")
            .and_then(|r| r.split_once(' '))
        {
            Some((n, rest)) => match n.parse::<u32>() {
                Ok(n) => (n, rest),
                Err(_) => continue,
            },
            None => (last + 10, e.text.as_str()),
        };
        if text.starts_with("remark") {
            continue;
        }
        last = last.max(seq);
        out.push(format!("    {text} sequence {seq}"));
    }
    out
}

/// The list's lines as `show running-config` prints them under its header; `pad`: the release pads
/// a `deny` (`padded_running`).
pub fn running(kind: Kind, lines: &[Entry], classic: bool, pad: bool) -> Vec<String> {
    if classic {
        return listed(kind, lines, classic)
            .into_iter()
            .map(|e| format!(" {}", padded(kind, &e.text)))
            .collect();
    }
    // a line kept as a capture printed it carries its own number
    lines
        .iter()
        .map(|e| match e.key.starts_with("seq ") {
            true => format!(" {} {}", seq_of(e), padded_running(&e.text, pad)),
            false => format!(" {}", padded_running(&e.text, pad)),
        })
        .collect()
}

/// The list as `show ip access-lists` prints it: its title, then each entry four spaces in with its
/// number, remarks left out, a standard wildcard written out (`10.1.0.0, wildcard bits 0.0.255.255`).
pub fn show(kind: Kind, name: &str, lines: &[Entry], classic: bool) -> Vec<String> {
    let title = match kind {
        Kind::Standard => "Standard",
        Kind::Extended => "Extended",
    };
    let mut out = vec![format!("{title} IP access list {name}")];
    for e in listed(kind, lines, classic) {
        if is_remark(e) {
            continue;
        }
        let mut text = padded(kind, &e.text);
        if kind == Kind::Standard {
            let words: Vec<&str> = e.text.split_whitespace().collect();
            let address = |w: &&str| w.parse::<std::net::Ipv4Addr>().is_ok();
            if words.len() >= 3 && address(&words[1]) && address(&words[2]) {
                let head = padded(kind, &format!("{} {}", words[0], words[1]));
                let tail = words[3..]
                    .iter()
                    .map(|w| format!(" {w}"))
                    .collect::<String>();
                text = format!("{head}, wildcard bits {}{tail}", words[2]);
            }
        }
        out.push(format!("    {} {}", seq_of(e), text));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_typed_at_the_console_number_print_and_resequence_as_ios_does() {
        let r = crate::release::Release::parse("17.15").unwrap();
        let e = crate::cli::Engine::new(crate::grammars::grammar("c9300", r).unwrap());
        let stack = crate::platform::parse_stack("C9300-24T").unwrap();
        let mut s = crate::shell::Session::new(&e, crate::device::Device::new(stack));
        for l in ["enable", "configure terminal", "ip access-list extended T1"] {
            s.type_line(l);
        }
        let mut typed = |l: &str| s.type_line(l);
        assert_eq!(
            typed("permit ip host 10.0.0.1 any").1,
            crate::shell::Result::Ok
        );
        assert_eq!(
            typed("permit ip host 10.0.0.2 any").1,
            crate::shell::Result::Ok
        );
        assert_eq!(
            typed("15 permit ip host 10.0.0.3 any").1,
            crate::shell::Result::Ok
        );
        // a number in use is refused; an entry held already adds nothing
        assert_eq!(
            typed("10 permit ip host 10.0.0.5 any"),
            (
                vec!["% Duplicate sequence number.".to_string(), String::new()],
                crate::shell::Result::Error
            )
        );
        assert_eq!(
            typed("permit ip host 10.0.0.1 any").1,
            crate::shell::Result::Ok
        );
        // a refused entry's caret stands under the line as typed, its number included
        let (out, _) = typed("25 permitt ip any any");
        let caret = out.iter().find(|l| l.trim() == "^").expect("a caret");
        assert_eq!(caret.len() - 1, "Switch(config-ext-nacl)#25 permit".len());
        assert_eq!(typed("no 15").1, crate::shell::Result::Ok);
        assert_eq!(
            typed("no permit ip host 10.0.0.2 any").1,
            crate::shell::Result::Ok
        );
        assert_eq!(typed("deny ip any any").1, crate::shell::Result::Ok);
        typed("end");
        assert_eq!(
            s.type_line("show ip access-lists T1").0,
            [
                "Extended IP access list T1",
                "    10 permit ip host 10.0.0.1 any",
                "    20 deny ip any any",
            ]
        );
        s.type_line("configure terminal");
        assert_eq!(
            s.type_line("ip access-list resequence T1 100 5").1,
            crate::shell::Result::Ok
        );
        s.type_line("end");
        let run = crate::show::running_config(&s.device);
        let at = run
            .iter()
            .position(|l| l == "ip access-list extended T1")
            .unwrap();
        assert_eq!(
            run[at + 1..at + 3],
            [" 100 permit ip host 10.0.0.1 any", " 105 deny ip any any"]
        );
        // the resequence is no line of the configuration
        assert!(!run.iter().any(|l| l.contains("resequence")));
        assert!(s.type_line("show ip access-lists NONE").0.is_empty());
    }

    #[cfg(all(test, captures))]
    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/acl-tests.rs"));

    fn typed(kind: Kind, lines: &[&str]) -> (Vec<Entry>, Vec<String>) {
        let mut held = vec![];
        let mut errors = vec![];
        for l in lines {
            if let Some(n) = unnumbered(l) {
                remove_number(&mut held, n);
            } else if let Some(rest) = l.strip_prefix("no ") {
                remove(&mut held, kind, rest);
            } else {
                let (seq, text) = numbered(l).map_or((None, *l), |(n, t)| (Some(n), t));
                if let Err(e) = add(&mut held, kind, seq, text) {
                    errors.push(e);
                }
            }
        }
        (held, errors)
    }

    #[test]
    fn a_header_names_its_list() {
        assert_eq!(
            of("ip access-list extended VSWITCH-TEST"),
            Some((Kind::Extended, "VSWITCH-TEST"))
        );
        assert_eq!(
            of("ip access-list standard 11"),
            Some((Kind::Standard, "11"))
        );
        assert_eq!(of("ip access-list role-based x"), None);
        assert_eq!(of("ip access-list extended"), None);
        assert_eq!(
            numbered("15 permit ip host 10.0.0.3 any"),
            Some((15, "permit ip host 10.0.0.3 any"))
        );
        assert_eq!(numbered("permit ip any any"), None);
        assert_eq!(numbered("15"), None);
        assert_eq!(numbered("0 permit ip any any"), None);
        assert_eq!(unnumbered("no 15"), Some(15));
        assert_eq!(unnumbered("no permit ip any any"), None);
    }

    #[test]
    fn entries_number_as_the_lab_switches_numbered_them() {
        // c9300 17.6 capture 1 at 17.6.4 and the 2960G at 15.0(2)SE11 alike
        let (held, errors) = typed(
            Kind::Extended,
            &[
                "permit ip host 10.0.0.1 any",
                "permit ip host 10.0.0.2 any",
                "15 permit ip host 10.0.0.3 any",
                "permit ip host 10.0.0.4 any",
                "10 permit ip host 10.0.0.5 any",
                "permit ip host 10.0.0.1 any",
                "remark a note",
            ],
        );
        assert_eq!(errors, ["% Duplicate sequence number."]);
        assert_eq!(
            show(Kind::Extended, "VSWITCH-TEST", &held, true),
            [
                "Extended IP access list VSWITCH-TEST",
                "    10 permit ip host 10.0.0.1 any",
                "    15 permit ip host 10.0.0.3 any",
                "    20 permit ip host 10.0.0.2 any",
                "    30 permit ip host 10.0.0.4 any",
            ]
        );
        assert_eq!(
            running(Kind::Extended, &held, true, false),
            [
                " permit ip host 10.0.0.1 any",
                " permit ip host 10.0.0.3 any",
                " permit ip host 10.0.0.2 any",
                " permit ip host 10.0.0.4 any",
                " remark a note",
            ]
        );
        let mut held = held;
        remove_number(&mut held, 15);
        remove(&mut held, Kind::Extended, "permit ip host 10.0.0.4 any");
        resequence(&mut held, 100, 5);
        assert_eq!(
            show(Kind::Extended, "VSWITCH-TEST", &held, true)[1..],
            [
                "    100 permit ip host 10.0.0.1 any",
                "    105 permit ip host 10.0.0.2 any"
            ]
        );
        assert_eq!(
            running(Kind::Extended, &held, true, false),
            [
                " permit ip host 10.0.0.1 any",
                " permit ip host 10.0.0.2 any",
                " remark a note"
            ]
        );
        // IOS XE prints each entry's number in the running configuration (c9300 17.6 capture 1)
        let (xe, _) = typed(
            Kind::Extended,
            &[
                "permit ip host 10.0.0.1 any",
                "permit ip host 10.0.0.2 any",
                "15 permit ip host 10.0.0.3 any",
            ],
        );
        assert_eq!(
            running(Kind::Extended, &xe, false, false),
            [
                " 10 permit ip host 10.0.0.1 any",
                " 15 permit ip host 10.0.0.3 any",
                " 20 permit ip host 10.0.0.2 any"
            ]
        );
    }

    #[test]
    fn a_standard_list_prints_its_wildcards_hosts_and_actions_as_ios_does() {
        let lines = [
            "permit 10.0.0.9",
            "permit 10.0.0.1",
            "permit 10.1.0.0 0.0.255.255",
            "permit host 10.0.0.7",
            "deny any",
        ];
        let (held, _) = typed(Kind::Standard, &lines);
        // the 2960G: host entries first, `host` dropped, `deny` padded
        assert_eq!(
            show(Kind::Standard, "VSWITCH-STD", &held, true),
            [
                "Standard IP access list VSWITCH-STD",
                "    10 permit 10.0.0.9",
                "    20 permit 10.0.0.1",
                "    40 permit 10.0.0.7",
                "    30 permit 10.1.0.0, wildcard bits 0.0.255.255",
                "    50 deny   any",
            ]
        );
        assert_eq!(
            running(Kind::Standard, &held, true, false),
            [
                " permit 10.0.0.9",
                " permit 10.0.0.1",
                " permit 10.0.0.7",
                " permit 10.1.0.0 0.0.255.255",
                " deny   any"
            ]
        );
        // c9300 17.6 capture 1 at 17.6.4
        let (xe, _) = typed(Kind::Standard, &lines[..3]);
        assert_eq!(
            show(Kind::Standard, "VSWITCH-STD", &xe, false),
            [
                "Standard IP access list VSWITCH-STD",
                "    10 permit 10.0.0.9",
                "    20 permit 10.0.0.1",
                "    30 permit 10.1.0.0, wildcard bits 0.0.255.255",
            ]
        );
        assert_eq!(
            running(Kind::Standard, &xe, false, false),
            [
                " 10 permit 10.0.0.9",
                " 20 permit 10.0.0.1",
                " 30 permit 10.1.0.0 0.0.255.255"
            ]
        );
    }

    #[test]
    fn a_remark_takes_the_number_of_the_entry_after_it() {
        // c9200 17.15 capture 22 at 17.15 prints `20 remark -MGMT_HOSTS-` above `20 permit 192.0.2.147`
        let (held, _) = typed(
            Kind::Standard,
            &[
                "permit 192.0.2.148",
                "remark -MGMT_HOSTS-",
                "permit 192.0.2.147",
                "deny any log",
            ],
        );
        assert_eq!(
            running(Kind::Standard, &held, false, false),
            [
                " 10 permit 192.0.2.148",
                " 20 remark -MGMT_HOSTS-",
                " 20 permit 192.0.2.147",
                " 30 deny any log"
            ]
        );
        // the same lines typed as the capture holds them
        let (again, errors) = typed(
            Kind::Standard,
            &[
                "10 permit 192.0.2.148",
                "20 remark -MGMT_HOSTS-",
                "20 permit 192.0.2.147",
                "30 deny any log",
            ],
        );
        assert!(errors.is_empty());
        assert_eq!(
            running(Kind::Standard, &again, false, false),
            running(Kind::Standard, &held, false, false)
        );
        assert_eq!(next(&again), 40);
    }
}
