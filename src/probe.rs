//! A test script typed on a real lab switch (`vswitch probe --lab`): each line's answer and the
//! running configuration IOS keeps, compared with vswitch's at the switch's release and hardware. The
//! switch's configuration is copied to flash first and put back with `configure replace` after. For a
//! private lab reservation only: a script with a line that could cut the session is refused before
//! anything is sent, and nothing is typed unless the copy was made.

use std::collections::BTreeSet;

use crate::capture::{self, Exchange};
use crate::{check, cli, hazards, platform, shell, show};

pub const ARCHIVE: &str = "flash:vswitch-probe-before.cfg";

/// Commands a probe never sends: the session's login and path, and the switch's identity. A line is
/// read as IOS reads it: each keyword by any prefix, in its `no` and `default` forms too.
const UNSAFE: &[&str] = &[
    "aaa",
    "username",
    "enable secret",
    "enable password",
    "line",
    "ip ssh",
    "crypto",
    "ip route",
    "ip default-gateway",
    "ip http",
    "hostname",
    "boot",
    "reload",
    "license",
    "archive",
    "event manager",
    "tacacs",
    "radius",
    "ip domain",
    "ntp",
    "snmp-server",
    "logging host",
    "vrf definition",
    "management",
    "netconf",
    "restconf",
    "gnxi",
    "write",
    "copy",
    "erase",
    "delete",
    "format",
    "do",
    // runs EXEC as `do` does, typed whole or as any start of three characters or more
    "do-exec",
];

/// Commands whose `no` or `default` form cuts the session though the command does not.
const UNSAFE_UNDONE: &[&str] = &["ip routing"];

/// Lines that leave configuration mode, or EXEC commands, which the lines after a top-level `exit`
/// would run as: `exit` from global configuration drops to EXEC, where `clear line` or `logout` end the
/// session. IOS leaves a submode by itself when a line names a global command.
const LEAVES: &[&str] = &["exit", "logout", "quit", "disable", "clear"];

/// A line's words as IOS reads them: lowercase, split on any space or tab, a byte order mark left out.
fn words(line: &str) -> Vec<String> {
    line.split(|c: char| c.is_whitespace() || c == '\u{feff}')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Whether the words begin with the command, each keyword whole or abbreviated. Any prefix counts,
/// whatever other keyword it might also name: the guard refuses what IOS might read as the command.
fn names(words: &[String], command: &str) -> bool {
    let keywords: Vec<&str> = command.split(' ').collect();
    words.len() >= keywords.len()
        && words
            .iter()
            .zip(&keywords)
            .all(|(w, k)| k.starts_with(w.as_str()))
}

/// A script's lines a probe types, numbered from 1: blank lines and comments left out, and nothing from
/// the first line IOS could read as `end`, after which lines would run at the EXEC prompt.
fn typed(script: &str) -> impl Iterator<Item = (usize, &str)> {
    script
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l))
        .filter(|(_, l)| {
            let w = words(l);
            w.first().is_some_and(|f| !f.starts_with('!'))
        })
        .take_while(|(_, l)| {
            !words(l)
                .first()
                .is_some_and(|f| "end".starts_with(f.as_str()))
        })
}

/// A configuration made into a probe script for a lab switch of `family`: it stops at `end`, and
/// drops each line `refused` would refuse with the lines under it. Returns the script and each dropped
/// line with its reason.
pub fn adapt(
    config: &str,
    management: &[String],
    family: Option<platform::Family>,
) -> (String, Vec<(usize, String)>) {
    let mut out = String::new();
    let mut dropped = vec![];
    let mut dropping_block = false;
    for (n, line) in typed(config) {
        let top = !line.starts_with(char::is_whitespace);
        if !top && dropping_block {
            dropped.push((n, "under a dropped block".to_string()));
            continue;
        }
        if let Some(why) = unsafe_line(line, management, family) {
            dropping_block = top;
            dropped.push((n, why));
            continue;
        }
        if top {
            dropping_block = false;
        }
        out.push_str(line);
        out.push('\n');
    }
    (out, dropped)
}

/// `Twe1/0/` as IOS names the type: `TwentyFiveGigE1/0/`.
fn full_prefix(p: &str) -> String {
    let split = p.find(|c: char| c.is_ascii_digit()).unwrap_or(p.len());
    let (ty, num) = p.split_at(split);
    crate::cli::interface_name(&format!("{ty}0"), None)
        .map(|(full, _, _)| format!("{full}{num}"))
        .unwrap_or_else(|| p.to_string())
}

/// A line with each interface name whose full form starts with a map's `from` renamed to its `to`.
fn rename(line: &str, maps: &[(String, String)]) -> String {
    if maps.is_empty() {
        return line.to_string();
    }
    let indent = &line[..line.len() - line.trim_start().len()];
    let words: Vec<String> = line
        .split_whitespace()
        .map(|w| {
            let full = crate::cli::interface_name(w, None).map(|(ty, num, _)| format!("{ty}{num}"));
            match full {
                Some(f) => maps
                    .iter()
                    .find(|(from, _)| f.starts_with(from.as_str()))
                    .map_or(w.to_string(), |(from, to)| {
                        format!("{to}{}", &f[from.len()..])
                    }),
                None => w.to_string(),
            }
        })
        .collect();
    format!("{indent}{}", words.join(" "))
}

/// A script with its ports renamed by prefix (`--map Twe1/0/=Gi1/0/`) onto ports the lab switch has,
/// line for line. The renamed lines are the ones checked and typed.
pub fn renamed(script: &str, maps: &[(String, String)]) -> String {
    let maps: Vec<(String, String)> = maps
        .iter()
        .map(|(a, b)| (full_prefix(a), full_prefix(b)))
        .collect();
    script.lines().map(|l| rename(l, &maps) + "\n").collect()
}

/// The lines of a script a probe refuses on a switch of `family`, with the reason: a command known to
/// crash it, a command of `UNSAFE` (or its `no` or `default` form), one that leaves configuration
/// mode, or one naming the management interface or its VRF.
pub fn refused(
    script: &str,
    management: &[String],
    family: Option<platform::Family>,
) -> Vec<(usize, String)> {
    typed(script)
        .filter_map(|(n, line)| unsafe_line(line, management, family).map(|why| (n, why)))
        .collect()
}

/// Why one script line could crash the switch, cut the session or change the switch's identity; none
/// when it cannot.
fn unsafe_line(
    line: &str,
    management: &[String],
    family: Option<platform::Family>,
) -> Option<String> {
    // a crash ends the session before `configure replace` puts the configuration back. With the
    // family not known, a command known to crash any family is refused. The line is read bare too,
    // though IOS takes EXEC in configuration mode only after `do`.
    if let Some(h) = hazards::find(family, line).or_else(|| hazards::typed(family, "config", line))
    {
        return Some(format!(
            "`{}` {}: a crash ends the session and skips the `configure replace` rollback",
            h.command,
            h.crashed()
        ));
    }
    if line.chars().any(|c| c.is_control() && c != '\t') {
        return Some("holds a control character, which the terminal acts on".into());
    }
    let w = words(line);
    // a `no` or `default` form resets what the command sets
    let bare = match w.first() {
        Some(f) if "no".starts_with(f.as_str()) || "default".starts_with(f.as_str()) => &w[1..],
        _ => &w[..],
    };
    if let Some(c) = UNSAFE.iter().find(|c| names(&w, c) || names(bare, c)) {
        return Some(format!(
            "`{c}` can cut the session or change the switch's identity"
        ));
    }
    if bare.len() < w.len()
        && let Some(c) = UNSAFE_UNDONE.iter().find(|c| names(bare, c))
    {
        return Some(format!("turning `{c}` off can cut the session"));
    }
    if let Some(c) = LEAVES.iter().find(|c| names(&w, c)) {
        return Some(format!(
            "`{c}` can leave configuration mode, and the lines after it would run at the EXEC prompt"
        ));
    }
    if names(bare, "interface range macro") {
        return Some("an interface range macro, whose ports the probe does not read".into());
    }
    if line.to_lowercase().contains("mgmt-vrf") || names_one(line, management) {
        return Some("names the management interface or its VRF".into());
    }
    None
}

/// A line's words for reading interface names: commas apart, and a hyphen after a port number apart,
/// so `Gi1/0/1-4,Gi1/0/9` reads as `Gi1/0/1 - 4 , Gi1/0/9`.
fn name_tokens(line: &str) -> Vec<String> {
    let mut out = vec![];
    for w in line
        .split(|c: char| c.is_whitespace() || c == '\u{feff}')
        .filter(|w| !w.is_empty())
    {
        let mut cur = String::new();
        for c in w.chars() {
            if c == ','
                || (c == '-' && (cur.is_empty() || cur.ends_with(|d: char| d.is_ascii_digit())))
            {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                out.push(c.to_string());
            } else {
                cur.push(c);
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

/// An interface number apart at its last port: `1/0/3` is (`1/0/`, 3); a number not ending in a port
/// (`1/0/1.100`, a subinterface) has none.
fn split_port(num: &str) -> (&str, Option<u32>) {
    let at = num.rfind('/').map_or(0, |i| i + 1);
    match num[at..].parse() {
        Ok(p) => (&num[..at], Some(p)),
        Err(_) => (num, None),
    }
}

/// Whether a line names one of the interfaces, a `<first> - <last>` span or comma list expanded:
/// `interface Gi0/0` and `GigabitEthernet 0/0` name GigabitEthernet0/0, `interface range Gi0/1 - 4`
/// names Gi0/3, `vlan 10,54` names Vlan54, and `Vlan10` does not name Vlan1.
fn names_one(line: &str, interfaces: &[String]) -> bool {
    // each interface as its type, its number up to the last port, and the port
    let known: Vec<(String, String, Option<u32>)> = interfaces
        .iter()
        .filter_map(|m| {
            let (ty, num, _) = cli::interface_name(m, None)?;
            let (slot, port) = split_port(&num);
            Some((ty, slot.to_string(), port))
        })
        .collect();
    let within = |ty: &str, slot: &str, first: Option<u32>, last: Option<u32>| {
        known.iter().any(|(t, s, p)| {
            t.eq_ignore_ascii_case(ty)
                && s == slot
                && match (p, first, last) {
                    (Some(p), Some(a), Some(b)) => (a..=b).contains(p),
                    (None, None, None) => true,
                    _ => false,
                }
        })
    };
    let t = name_tokens(line);
    let digits = |i: usize| -> Option<u32> { t.get(i)?.parse().ok() };
    let mut i = 0;
    while i < t.len() {
        if interfaces.iter().any(|m| m.eq_ignore_ascii_case(&t[i])) {
            return true;
        }
        let Some((ty, num, used)) = cli::interface_name(&t[i], t.get(i + 1).map(String::as_str))
        else {
            i += 1;
            continue;
        };
        let (slot, first) = split_port(&num);
        let mut j = i + used;
        let mut last = first;
        if t.get(j).is_some_and(|x| x == "-")
            && let Some(l) = digits(j + 1)
        {
            last = Some(l);
            j += 2;
        }
        if within(&ty, slot, first, last) {
            return true;
        }
        // a comma list goes on with bare numbers of the same type: `vlan 10,54`
        while t.get(j).is_some_and(|x| x == ",")
            && let Some(a) = digits(j + 1)
        {
            j += 2;
            let mut b = a;
            if t.get(j).is_some_and(|x| x == "-")
                && let Some(l) = digits(j + 1)
            {
                b = l;
                j += 2;
            }
            if within(&ty, slot, Some(a), Some(b)) {
                return true;
            }
        }
        i = j.max(i + 1);
    }
    false
}

/// The host a target names: `lab@192.0.2.60`, `ssh://lab@sw1:22` and `lab@[2001:db8::1]:22` name
/// 192.0.2.60, sw1 and 2001:db8::1.
pub fn host_of(target: &str) -> &str {
    let t = target.strip_prefix("ssh://").unwrap_or(target);
    let t = t.rsplit('@').next().unwrap_or(t);
    if let Some(r) = t.strip_prefix('[') {
        return r.split(']').next().unwrap_or(r);
    }
    // `host:port`; an IPv6 address holds more than one colon
    match t.split_once(':') {
        Some((h, p)) if !p.contains(':') => h,
        _ => t,
    }
}

/// The addresses a target reaches: its host's own, else those its name resolves to; none for a name
/// that resolves to nothing, such as an ssh configuration alias.
pub fn addresses(target: &str) -> Vec<String> {
    use std::net::ToSocketAddrs;
    let host = host_of(target);
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return vec![ip.to_string()];
    }
    let mut out: Vec<String> = (host, 22)
        .to_socket_addrs()
        .map(|a| a.map(|s| s.ip().to_string()).collect())
        .unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

/// The ports `show interfaces status` lists, each with its Status and Vlan columns, read at the
/// header's columns since a port's Name may hold spaces; a column that cannot be read is `?`.
fn status_rows(status: &[String]) -> Vec<(String, String, String)> {
    let Some(h) = status
        .iter()
        .position(|l| l.starts_with("Port") && l.contains("Status") && l.contains("Vlan"))
    else {
        return vec![];
    };
    let header = &status[h];
    let (s, v) = (
        header.find("Status").unwrap_or(0),
        header.find("Vlan").unwrap_or(0),
    );
    let d = header.find("Duplex").unwrap_or(v + 11);
    status[h + 1..]
        .iter()
        .filter(|l| l.starts_with(|c: char| !c.is_whitespace()))
        .filter_map(|l| {
            let port = l.split_whitespace().next()?.to_string();
            let column = |a: usize, b: usize| {
                l.get(a..b.min(l.len()))
                    .map_or("?".to_string(), |c| c.trim().to_string())
            };
            Some((port, column(s, v), column(v, d)))
        })
        .collect()
}

/// The ports a configuration bundles into port-channel `channel`.
fn members(config: &str, channel: &str) -> Vec<String> {
    let mut out = vec![];
    let mut interface = None;
    for l in config.lines() {
        if let Some(name) = l.strip_prefix("interface ") {
            interface = Some(name.trim().to_string());
        } else if !l.starts_with(' ') {
            interface = None;
        } else if let Some(i) = &interface
            && l.split_whitespace().take(2).eq(["channel-group", channel])
        {
            out.push(i.clone());
        }
    }
    out
}

/// The interfaces a probe leaves alone: the one holding an address the session reached (`show ip
/// interface brief`), GigabitEthernet0/0, and the ports that carry the session to it. An SVI's are the
/// connected ports `show interfaces status` shows in its VLAN or trunking; a port-channel's are its
/// members. An error when no interface holds the address, since the session's path is then unknown.
pub fn management(
    ip_brief: &[String],
    status: &[String],
    config: &str,
    addresses: &[String],
) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = ip_brief
        .iter()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let (name, address) = (w.next()?, w.next()?);
            addresses
                .iter()
                .any(|a| a == address)
                .then(|| name.to_string())
        })
        .collect();
    if names.is_empty() {
        return Err(format!(
            "no interface of the switch holds {} (`show ip interface brief`), so the interface \
             carrying the session is unknown: nothing was sent. Give the switch's address as the \
             target.",
            if addresses.is_empty() {
                "the address the target names".to_string()
            } else {
                addresses.join(" or ")
            }
        ));
    }
    let rows = status_rows(status);
    for svi in names.clone() {
        let Some(vlan) = svi.strip_prefix("Vlan") else {
            continue;
        };
        if rows.is_empty() {
            return Err(format!(
                "`show interfaces status` listed no ports, so the ports carrying {svi} are unknown: \
                 nothing was sent"
            ));
        }
        names.extend(
            rows.iter()
                .filter(|(_, st, v)| {
                    st == "?" || v == "?" || st == "connected" && (v == vlan || v == "trunk")
                })
                .map(|(port, _, _)| port.clone()),
        );
    }
    let channels: Vec<String> = names
        .iter()
        .filter_map(|n| {
            let (ty, num, _) = cli::interface_name(n, None)?;
            (ty == "Port-channel").then_some(num)
        })
        .collect();
    for c in channels {
        names.extend(members(config, &c));
    }
    names.push("GigabitEthernet0/0".into());
    // a port named short (`Gi0/1`, as `show interfaces status` names it) or in full
    let more: Vec<String> = names
        .iter()
        .flat_map(|n| {
            let full = cli::interface_name(n, None).map(|(ty, num, _)| format!("{ty}{num}"));
            [Some(show::short(n)), full].into_iter().flatten()
        })
        .collect();
    names.extend(more);
    names.sort();
    names.dedup();
    Ok(names)
}

/// The copy session's commands: the running configuration copied to flash, each copy prompt answered
/// with Enter.
pub fn copy_commands() -> Vec<String> {
    vec![
        "terminal length 0".into(),
        format!("copy running-config {ARCHIVE}"),
        String::new(),
        String::new(),
    ]
}

/// Whether the copy session made the copy: IOS says `<n> bytes copied`. Otherwise the switch's error,
/// since a script typed with no copy to put back would leave the switch changed.
pub fn copied(transcript: &str) -> Result<(), String> {
    if transcript.lines().any(|l| l.contains("bytes copied")) {
        return Ok(());
    }
    let why = transcript
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with('%'))
        .unwrap_or("no `bytes copied` in its answer");
    Err(format!(
        "copy running-config {ARCHIVE}: {why}. Nothing was typed."
    ))
}

/// The probe session's commands, the copy made: type the script in configuration mode, read what IOS
/// kept, and put the configuration back.
pub fn commands(script: &str) -> Vec<String> {
    let mut c: Vec<String> = vec!["terminal length 0".into(), "configure terminal".into()];
    c.extend(typed(script).map(|(_, l)| l.trim_end().trim_start_matches('\u{feff}').to_string()));
    c.push("end".into());
    c.push("show running-config".into());
    c.push(format!("configure replace {ARCHIVE} force"));
    c
}

/// A probe that typed its script: the switch as it stood, the lines left out, and what the switch
/// printed.
pub struct Run {
    pub host: String,
    pub version: String,
    /// the hardware as `--stack` takes it
    pub spec: String,
    /// the running configuration before the probe
    pub before: String,
    pub dropped: Vec<(usize, String)>,
    /// the copy session and the script session, as the switch printed them
    pub log: String,
    pub exchanges: Vec<Exchange>,
}

/// What a probe did: refused the script, typing nothing, or typed it.
pub enum Probed {
    Refused(Vec<(usize, String)>),
    Ran(Box<Run>),
}

/// A probe's sessions, each one call of `session`: the switch read (release, hardware, the interfaces
/// carrying the session, configuration), the script's ports renamed by `maps` and the renamed lines
/// checked, the configuration copied to flash, then the script typed. Nothing is typed when a line is
/// refused (unless `skip`, which leaves it out), when no interface holds an address the target reaches,
/// or when the copy was not made.
pub fn sessions(
    script: &str,
    maps: &[(String, String)],
    addresses: &[String],
    skip: bool,
    mut session: impl FnMut(&[&str]) -> Result<String, String>,
) -> Result<Probed, String> {
    let first = session(&[
        "terminal length 0",
        "show version",
        "show inventory",
        "show ip interface brief",
        "show interfaces status",
        "show running-config",
    ])?;
    let (host, ex) = capture::exchanges(&first).ok_or("no prompt in the session")?;
    let answer = |c: &str| {
        ex.iter()
            .find(|e| e.command == c)
            .map(|e| e.output.clone())
            .unwrap_or_default()
    };
    let version =
        capture::release_of(&answer("show version")).ok_or("`show version` named no release")?;
    let (spec, unknown) = capture::stack_of(&answer("show inventory"));
    let spec = spec.ok_or(format!(
        "hardware not in vswitch's catalog: {}",
        unknown.join(", ")
    ))?;
    let before = capture::configuration(&answer("show running-config"));
    let management = management(
        &answer("show ip interface brief"),
        &answer("show interfaces status"),
        &before,
        addresses,
    )?;
    let family = platform::parse_stack(&spec).ok().and_then(|s| s.family());
    let script = renamed(script, maps);
    let bad = refused(&script, &management, family);
    if !bad.is_empty() && !skip {
        return Ok(Probed::Refused(bad));
    }
    let (script, dropped) = adapt(&script, &management, family);
    let copy = session(
        &copy_commands()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    )?;
    copied(&copy)?;
    let transcript = session(
        &commands(&script)
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    )?;
    let (_, exchanges) = capture::exchanges(&transcript).ok_or("no prompt in the probe session")?;
    Ok(Probed::Ran(Box::new(Run {
        host,
        version,
        spec,
        before,
        dropped,
        log: format!("{copy}{transcript}"),
        exchanges,
    })))
}

/// The release with its operating system, as a capture names it: `IOS 15.0(2)SE11`, `IOS XE 17.15.1`.
pub fn named(version: &str) -> String {
    let classic =
        crate::release::Release::parse(version).is_some_and(crate::release::Release::classic);
    format!("{} {version}", if classic { "IOS" } else { "IOS XE" })
}

/// The folder a probe's files go in, beside the switch's capture: `<book>-<train>`, as `vswitch
/// capture` names it (15.0(2)SE11 runs the 15.0(2)SE train).
pub fn folder(book: &str, version: &str) -> String {
    match crate::release::Release::parse(version) {
        Some(train) => format!("{book}-{train}"),
        None => format!("{book}-{version}"),
    }
}

/// A configuration as `block :: line` entries, top-level lines under "". IOS prints every interface,
/// so an interface's header is not an entry; its lines are.
fn entries(config: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut block = String::new();
    for l in config.lines() {
        if l.trim().is_empty() || l.starts_with('!') || l == "end" {
            continue;
        }
        if l.starts_with(' ') {
            out.insert(format!("{block} :: {}", l.trim()));
        } else {
            block = l.trim().to_string();
            if !block.starts_with("interface ") {
                out.insert(format!(" :: {block}"));
            }
        }
    }
    out
}

/// What a probe found.
#[derive(Debug, Default)]
pub struct Report {
    pub lines: usize,
    /// lines the switch took that vswitch refuses, with vswitch's error
    pub vswitch_refuses: Vec<(String, String)>,
    /// lines the switch refused that vswitch takes, with the switch's error
    pub vswitch_takes: Vec<(String, String)>,
    /// configuration entries IOS kept that vswitch's running configuration lacks, and the reverse
    pub only_ios: Vec<String>,
    pub only_vswitch: Vec<String>,
    /// the configuration IOS printed after the script, before the replace
    pub after: String,
    /// whether `configure replace` said `Rollback Done`, the switch back as it was
    pub restored: bool,
}

impl Report {
    /// The switch and vswitch agreed on every line and entry, and the switch is back as it was.
    pub fn clean(&self) -> bool {
        self.restored
            && self.vswitch_refuses.is_empty()
            && self.vswitch_takes.is_empty()
            && self.only_ios.is_empty()
            && self.only_vswitch.is_empty()
    }
}

/// Each script line the switch answered, with its error when it printed one.
pub fn answers(probe: &[Exchange]) -> Vec<(String, Option<String>)> {
    let start = probe
        .iter()
        .position(|e| e.command == "configure terminal")
        .map_or(0, |i| i + 1);
    probe[start..]
        .iter()
        .take_while(|e| e.command != "end")
        .map(|e| {
            let err = e.output.iter().find(|l| l.trim_start().starts_with('%'));
            (e.command.clone(), err.map(|l| l.trim().to_string()))
        })
        .collect()
}

/// Compares the switch's answers and saved configuration with vswitch's: the switch's configuration
/// before the probe (`before`) typed in first, then the script, at the switch's release and hardware.
pub fn compare(
    engine: &cli::Engine,
    stack: platform::Stack,
    before: &str,
    probe: &[Exchange],
) -> Report {
    let mut r = Report::default();
    let (mut s, _) = check::type_config(engine, stack, before);
    let base = entries(&capture::configuration(&show::running_config(&s.device)));
    for (line, err) in answers(probe) {
        r.lines += 1;
        let (out, result) = s.type_line(&line);
        let mine = out.iter().rev().find(|l| l.starts_with('%')).cloned();
        match (err, result) {
            (None, shell::Result::Error) => {
                r.vswitch_refuses.push((line, mine.unwrap_or_default()))
            }
            (Some(e), shell::Result::Ok) => r.vswitch_takes.push((line, e)),
            _ => {}
        }
    }
    s.type_line("end");
    let after_v = entries(&capture::configuration(&show::running_config(&s.device)));
    let after = probe
        .iter()
        .find(|e| e.command == "show running-config")
        .map(|e| capture::configuration(&e.output))
        .unwrap_or_default();
    let ios_before = entries(before);
    let ios_after = entries(&after);
    let ios_new: BTreeSet<&String> = ios_after.difference(&ios_before).collect();
    let mine_new: BTreeSet<&String> = after_v.difference(&base).collect();
    r.only_ios = ios_new
        .difference(&mine_new)
        .map(|s| s.to_string())
        .collect();
    r.only_vswitch = mine_new
        .difference(&ios_new)
        .map(|s| s.to_string())
        .collect();
    r.after = after;
    r.restored = probe
        .iter()
        .find(|e| e.command.starts_with("configure replace"))
        .is_some_and(|e| e.output.iter().any(|l| l.contains("Rollback Done")));
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammars;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    /// `show interfaces status` as the lab 2960G answered it: its uplink connected in VLAN 54.
    const STATUS: &str = "\nPort      Name               Status       Vlan       Duplex  Speed Type
Gi0/1                        connected    54         a-full a-1000 10/100/1000BaseTX
Gi0/2     to the lab bench   notconnect   54           auto   auto 10/100/1000BaseTX
Gi0/7                        connected    1          a-full a-1000 10/100/1000BaseTX
Gi0/8                        connected    trunk      a-full a-1000 10/100/1000BaseTX";

    fn reached(ip_brief: &str, address: &str) -> Result<Vec<String>, String> {
        management(&lines(ip_brief), &lines(STATUS), "", &[address.to_string()])
    }

    #[test]
    fn a_script_that_could_cut_the_session_is_refused_whole() {
        let mgmt = reached(
            "Interface              IP-Address      OK? Method Status                Protocol\nGigabitEthernet0/0     10.10.20.181    YES NVRAM  up                    up\nVlan1                  unassigned      YES NVRAM  administratively down down",
            "10.10.20.181",
        )
        .unwrap();
        assert!(mgmt.contains(&"Gi0/0".to_string()));
        // reached on an SVI, the SVI is management; a neighbouring VLAN interface is not
        let vlan = reached(
            "Vlan1                  10.10.20.181    YES NVRAM  up                    up",
            "10.10.20.181",
        )
        .unwrap();
        assert!(vlan.contains(&"Vlan1".to_string()) && vlan.contains(&"Vl1".to_string()));
        assert_eq!(
            refused("interface Vlan1\n shutdown\n", &vlan, None).len(),
            1
        );
        assert!(
            refused(
                "interface Vlan10\n shutdown\ninterface vlan 100\n",
                &vlan,
                None
            )
            .is_empty()
        );
        assert_eq!(
            refused("interface GigabitEthernet 0/0\n", &mgmt, None).len(),
            1
        );
        let script = "vlan 77\n name test\ninterface Gi0/0\n shutdown\nno username developer\nline vty 0 4\ninterface TwentyFiveGigE1/0/1\n switchport mode trunk\n";
        assert_eq!(
            refused(script, &mgmt, None)
                .iter()
                .map(|(n, _)| *n)
                .collect::<Vec<_>>(),
            [3, 5, 6]
        );
        assert!(
            refused(
                "vlan 77\ninterface Vlan77\n ip address 10.7.7.1 255.255.255.0\n",
                &mgmt,
                None
            )
            .is_empty()
        );
        // the configuration goes to flash in a session of its own, and back after the script
        assert_eq!(copy_commands()[1], format!("copy running-config {ARCHIVE}"));
        let c = commands("vlan 77\n! a note\n name test\n");
        assert_eq!(
            c[1..5],
            ["configure terminal", "vlan 77", " name test", "end"]
        );
        assert_eq!(
            c.last().unwrap(),
            &format!("configure replace {ARCHIVE} force")
        );
    }

    #[test]
    fn the_session_s_interface_is_found_by_its_address_or_nothing_is_sent() {
        let brief = "Interface              IP-Address      OK? Method Status                Protocol\nVlan1                  unassigned      YES NVRAM  up                    down\nVlan54                 192.0.2.60     YES NVRAM  up                    up";
        // a host name or an ssh alias is not an address the switch holds
        assert!(reached(brief, "c2960g.lab").is_err());
        let names = management(
            &lines(brief),
            &lines(STATUS),
            "",
            &addresses("ssh://lab@192.0.2.60:22"),
        )
        .unwrap();
        // the SVI, and the connected ports in its VLAN or trunking; not a port down or in another
        // VLAN
        for name in ["Vlan54", "Gi0/1", "GigabitEthernet0/1", "Gi0/8"] {
            assert!(names.contains(&name.to_string()), "{name}: {names:?}");
        }
        for name in ["Gi0/2", "Gi0/7"] {
            assert!(!names.contains(&name.to_string()), "{name}: {names:?}");
        }
        assert_eq!(
            refused("interface GigabitEthernet0/1\n shutdown\n", &names, None).len(),
            1
        );
        assert_eq!(
            refused("interface range Gi0/1 - 8\n", &names, None).len(),
            1
        );
        assert!(refused("interface Gi0/2\n shutdown\n", &names, None).is_empty());
        // with no ports listed, the ports carrying the SVI are unknown
        assert!(management(&lines(brief), &[], "", &["192.0.2.60".into()]).is_err());
        // a routed port-channel's members carry its traffic
        let po = management(
            &lines("Port-channel1          10.0.0.2        YES NVRAM  up                    up"),
            &[],
            "interface GigabitEthernet1/0/1\n channel-group 1 mode active\ninterface GigabitEthernet1/0/2\n channel-group 2 mode active\n",
            &["10.0.0.2".into()],
        )
        .unwrap();
        assert!(po.contains(&"GigabitEthernet1/0/1".to_string()));
        assert!(!po.contains(&"GigabitEthernet1/0/2".to_string()));
        assert_eq!(host_of("lab@[2001:db8::1]:22"), "2001:db8::1");
        assert!(addresses("lab@localhost").contains(&"127.0.0.1".to_string()));
    }

    #[test]
    fn a_mapped_port_is_checked_by_the_name_it_is_typed_as() {
        let mgmt = reached(
            "GigabitEthernet0/3     192.0.2.60     YES NVRAM  up                    up",
            "192.0.2.60",
        )
        .unwrap();
        let script = renamed(
            "interface GigabitEthernet1/0/3\n shutdown\n",
            &[("Gi1/0/".to_string(), "Gi0/".to_string())],
        );
        assert_eq!(script, "interface GigabitEthernet0/3\n shutdown\n");
        assert_eq!(refused(&script, &mgmt, None).len(), 1);
        let (sent, dropped) = adapt(&script, &mgmt, None);
        assert!(
            sent.is_empty() && dropped.len() == 2,
            "{sent:?} {dropped:?}"
        );
    }

    #[test]
    fn nothing_is_typed_unless_the_copy_was_made() {
        let failed = "C2960G#terminal length 0\nC2960G#copy running-config flash:vswitch-probe-before.cfg\nDestination filename [vswitch-probe-before.cfg]? \n%Error copying flash:vswitch-probe-before.cfg (No space left on device)\nC2960G#\nC2960G#\nC2960G#exit\n";
        let e = copied(failed).unwrap_err();
        assert!(e.contains("No space left on device"), "{e}");
        let made = "C2960G#copy running-config flash:vswitch-probe-before.cfg\nDestination filename [vswitch-probe-before.cfg]? \n2367 bytes copied in 0.532 secs (4449 bytes/sec)\nC2960G#\n";
        assert!(copied(made).is_ok());
        // the script session neither copies nor types without its configuration mode
        let c = commands("vlan 77\n");
        assert!(!c.iter().any(|l| l.starts_with("copy")));
    }

    #[test]
    fn a_probe_whose_replace_did_not_finish_is_not_clean() {
        let engine = cli::Engine::new(grammars::grammar("c9300", crate::release::DEFAULT).unwrap());
        let stack = platform::parse_stack("C9300X-24Y").unwrap();
        let before = "!\nversion 17.15\nhostname lab1\nvtp mode transparent\n!\nend\n";
        let session = |replace: &str| {
            format!(
                "lab1#configure terminal\nlab1(config)#vlan 77\nlab1(config-vlan)#end\nlab1#show running-config\n!\nversion 17.15\nhostname lab1\nvtp mode transparent\nvlan 77\n!\nend\n\nlab1#configure replace flash:vswitch-probe-before.cfg force\n{replace}lab1#\n"
            )
        };
        let report = |text: &str| {
            let (_, ex) = capture::exchanges(text).unwrap();
            compare(&engine, stack.clone(), before, &ex)
        };
        let done = report(&session("Total number of passes: 1\nRollback Done\n\n"));
        assert!(done.restored && done.clean(), "{done:?}");
        let stuck = report(&session("%Error opening flash:vswitch-probe-before.cfg\n"));
        assert!(!stuck.restored && !stuck.clean());
    }

    #[cfg(all(test, captures))]
    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/probe-tests.rs"));

    #[test]
    fn a_classic_ios_probe_is_named_and_filed_as_its_capture_is() {
        assert_eq!(named("15.0(2)SE11"), "IOS 15.0(2)SE11");
        assert_eq!(named("17.15.1"), "IOS XE 17.15.1");
        assert_eq!(folder("c2960", "15.0(2)SE11"), "c2960-15.0(2)SE");
        assert_eq!(folder("c9300", "17.15.1"), "c9300-17.15");
    }

    /// The lab 2960G's management names: reached on Vlan54.
    fn vlan54() -> Vec<String> {
        ["GigabitEthernet0/0", "Gi0/0", "Vlan54", "Vl54"]
            .map(String::from)
            .to_vec()
    }

    #[test]
    fn an_abbreviated_spaced_or_default_form_of_a_refused_command_is_refused() {
        let script = "ip default-g 10.0.0.1\nno ip default-g\nlin vty 0 4\n access-class 1 in\nhostn x\nusern bob privilege 15 secret 0 $9$INVENTED\nboo system flash:x.bin\nno  ip default-gateway\nip  route 0.0.0.0 0.0.0.0 Null0\ndefault ip default-gateway\ndefault line vty 0 4\ndefault aaa new-model\nhostname\tx\nenable sec 0 $9$INVENTED\nsnmp com x RO\n\u{feff}aaa new-model\n";
        let lines: Vec<usize> = refused(script, &vlan54(), None)
            .iter()
            .map(|r| r.0)
            .collect();
        assert_eq!(lines, [1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
        // `ip routing` turns routing on, and a VLAN or interface beside the management one is free
        assert!(
            refused(
                "vlan 77\n name test\nip routing\ninterface Vlan77\n ip address 10.7.7.1 255.255.255.0\n",
                &vlan54(),
                None
            )
            .is_empty()
        );
    }

    #[test]
    fn do_exec_whole_or_abbreviated_is_refused_as_do_is() {
        let script = "do reload\ndo-exec reload\ndo-ex reload\nDO-E show version\ndo- write\ndot1x system-auth-control\n";
        let r: Vec<usize> = refused(script, &vlan54(), None)
            .iter()
            .map(|r| r.0)
            .collect();
        assert_eq!(r, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn turning_routing_off_is_refused() {
        let r = refused(
            "no ip routing\ndefault ip routing\nip routing\n",
            &vlan54(),
            None,
        );
        assert_eq!(r.iter().map(|r| r.0).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn a_line_that_leaves_configuration_mode_is_refused() {
        // what follows a top-level `exit` runs at the EXEC prompt, where nothing is refused
        let script = "vlan 77\n name test\nexit\nexit\nclear line vty 0\nlogout\nquit\ndisable\nvlan 78\n\u{1a}\n";
        let r: Vec<usize> = refused(script, &vlan54(), None)
            .iter()
            .map(|r| r.0)
            .collect();
        assert_eq!(r, [3, 4, 5, 6, 7, 8, 10]);
    }

    #[test]
    fn an_interface_range_or_vlan_list_spanning_the_management_interface_is_refused() {
        let mgmt = ["GigabitEthernet0/0", "Gi0/0", "GigabitEthernet0/3", "Gi0/3"]
            .map(String::from)
            .to_vec();
        for line in [
            "interface range GigabitEthernet0/1 - 8",
            "interface range Gi0/2 -4",
            "interface range Gi0/2-4",
            "interface range Gi0/1 - 2 , Gi0/3",
            "int ran gi0/1-4",
            "interface range macro uplinks",
        ] {
            assert_eq!(refused(line, &mgmt, None).len(), 1, "{line}");
        }
        assert!(refused("interface range GigabitEthernet0/4 - 8\n", &mgmt, None).is_empty());
        for line in [
            "vlan 10,54",
            "vlan 50-60",
            "no vlan 54",
            "interface range vlan 50 - 60",
        ] {
            assert_eq!(refused(line, &vlan54(), None).len(), 1, "{line}");
        }
        assert!(refused("vlan 10,55-60\n", &vlan54(), None).is_empty());
    }

    #[test]
    fn a_configuration_becomes_a_probe_script_for_the_lab_switch() {
        let config = "hostname x\nvtp mode transparent\nvlan 10\n name users\ninterface GigabitEthernet0/0\n vrf forwarding Mgmt-vrf\n ip address 10.0.0.1 255.255.255.0\ninterface range TwentyFiveGigE1/0/1 - 2 , Twe1/0/5\n switchport mode trunk\nline vty 0 4\n transport input ssh\nip route vrf Mgmt-vrf 0.0.0.0 0.0.0.0 10.0.0.254\ninterface Port-channel40\n description core\nend\nwrite memory\n";
        let mgmt = reached(
            "GigabitEthernet0/0     10.10.20.181    YES NVRAM  up                    up",
            "10.10.20.181",
        )
        .unwrap();
        let (script, dropped) = adapt(
            &renamed(config, &[("Twe1/0/".to_string(), "Gi1/0/".to_string())]),
            &mgmt,
            None,
        );
        assert_eq!(
            script,
            "vtp mode transparent\nvlan 10\n name users\ninterface range GigabitEthernet1/0/1 - 2 , GigabitEthernet1/0/5\n switchport mode trunk\ninterface Port-channel40\n description core\n"
        );
        // the hostname, the management port and its lines, the VTY block, the management route
        assert_eq!(
            dropped.iter().map(|d| d.0).collect::<Vec<_>>(),
            [1, 5, 6, 7, 10, 11, 12]
        );
        assert!(refused(&script, &mgmt, None).is_empty());
        assert_eq!(
            refused("write memory\ndo copy run start\n", &mgmt, None).len(),
            2
        );
    }

    #[test]
    fn a_command_known_to_crash_the_switch_s_family_is_refused() {
        use platform::Family;
        let script = "vlan 77\nshow ipv6 pim gir\ndo sh plat soft tdl-database content bp config nodes 0\ndo show ip pim gir\n";
        let on = |family| refused(script, &vlan54(), family);
        let numbers = |r: &[(usize, String)]| r.iter().map(|r| r.0).collect::<Vec<_>>();
        let c9300 = on(Some(Family::C9300));
        assert_eq!(numbers(&c9300), [2, 3, 4]);
        for (_, why) in &c9300[..2] {
            assert!(
                why.contains("crashed a Catalyst 9300 at IOS XE 17.6.4")
                    && why.contains("skips the `configure replace` rollback"),
                "{why}"
            );
        }
        // `do show ip pim gir` answers on the switch; the line is refused for its `do`
        assert!(!c9300[2].1.contains("crash"), "{}", c9300[2].1);
        // another family's switch refuses only the `do` lines, for their `do`
        for family in [Family::C9200, Family::C2960] {
            let r = on(Some(family));
            assert_eq!(numbers(&r), [3, 4], "{family:?}");
            assert!(r.iter().all(|(_, why)| !why.contains("crash")), "{r:?}");
        }
        // a family not known refuses what crashes any
        assert_eq!(numbers(&on(None)), [2, 3, 4]);
        let (sent, dropped) = adapt(script, &vlan54(), Some(Family::C9300));
        assert_eq!(
            (sent.as_str(), numbers(&dropped)),
            ("vlan 77\n", vec![2, 3, 4])
        );
    }

    #[test]
    fn a_9200_transceiver_max_power_form_is_refused_on_the_interface_measured() {
        use platform::Family;
        let script = "show interfaces Tunnel0 transceiver max-power\nshow interfaces GigabitEthernet1/0/1 transceiver max-power\nsh int tu 0 trans max-power violation\nshow ipv6 pim gir\n";
        let on = |family| refused(script, &vlan54(), family);
        let numbers = |r: &[(usize, String)]| r.iter().map(|r| r.0).collect::<Vec<_>>();
        for family in [Family::C9200L, Family::C9200] {
            let r = on(Some(family));
            assert_eq!(numbers(&r), [1, 3], "{family:?}");
            for (_, why) in &r {
                assert!(
                    why.starts_with("`show interfaces Tunnel0 transceiver max-power")
                        && why.contains("crashed a Catalyst 9200L at IOS XE 17.15.6")
                        && why.contains("skips the `configure replace` rollback"),
                    "{why}"
                );
            }
        }
        // the 9300 refuses its own hazard alone
        assert_eq!(numbers(&on(Some(Family::C9300))), [4]);
        // a family not known refuses what crashes any
        assert_eq!(numbers(&on(None)), [1, 3, 4]);
    }

    #[test]
    fn a_script_with_a_command_known_to_crash_the_switch_sends_nothing() {
        let first = "Switch#terminal length 0\nSwitch#show version\nCisco IOS XE Software, Version 17.06.04\nSwitch#show inventory\nNAME: \"Switch 1\", DESCR: \"C9300-24T\"\nPID: C9300-24T         , VID: V01\nSwitch#show ip interface brief\nInterface              IP-Address      OK? Method Status                Protocol\nGigabitEthernet0/0     192.0.2.60      YES NVRAM  up                    up\nSwitch#show interfaces status\nSwitch#show running-config\n!\nversion 17.6\nhostname Switch\n!\nend\n\nSwitch#exit\n";
        let mut asked = 0;
        let probed = sessions(
            "vlan 77\n name test\nshow ipv6 pim gir\n",
            &[],
            &["192.0.2.60".into()],
            false,
            |_| {
                asked += 1;
                Ok(first.to_string())
            },
        )
        .unwrap();
        let Probed::Refused(bad) = probed else {
            panic!("the script was typed");
        };
        assert_eq!(asked, 1);
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].0, 3);
        assert!(
            bad[0].1.contains("`show ipv6 pim gir` crashed"),
            "{}",
            bad[0].1
        );
    }

    #[test]
    fn the_switch_s_answers_and_saved_lines_are_compared_with_vswitch_s() {
        let transcript = "lab1#terminal length 0\nlab1#configure terminal\nEnter configuration commands, one per line.  End with CNTL/Z.\nlab1(config)#vlan 77\nlab1(config-vlan)# name test\nlab1(config-vlan)#fancy-feature enable\nlab1(config)#frobnicate\n            ^\n% Invalid input detected at '^' marker.\n\nlab1(config)#spanning-tree mode mst\n                      ^\n% Invalid input detected at '^' marker.\n\nlab1(config)#interface TwentyFiveGigE1/0/1\nlab1(config-if)# switchport mode trunk\nlab1(config-if)#end\nlab1#show running-config\nBuilding configuration...\n\nCurrent configuration : 200 bytes\n!\nversion 17.15\nhostname lab1\nfancy-feature enable\nvtp mode transparent\nvlan 77\n name test\n!\ninterface TwentyFiveGigE1/0/1\n switchport mode trunk\n!\nend\n\nlab1#configure replace flash:vswitch-probe-before.cfg force\n";
        let (_, ex) = capture::exchanges(transcript).unwrap();
        let a = answers(&ex);
        assert_eq!(a.len(), 7);
        assert_eq!(
            a[3].1.as_deref(),
            Some("% Invalid input detected at '^' marker.")
        );
        let engine = cli::Engine::new(grammars::grammar("c9300", crate::release::DEFAULT).unwrap());
        let r = compare(
            &engine,
            platform::parse_stack("C9300X-24Y").unwrap(),
            "!\nversion 17.15\nhostname lab1\nvtp mode transparent\n!\nend\n",
            &ex,
        );
        assert_eq!(r.lines, 7);
        // IOS took a line the books do not have; it refused one vswitch refuses too
        assert_eq!(
            r.vswitch_refuses
                .iter()
                .map(|x| x.0.as_str())
                .collect::<Vec<_>>(),
            ["fancy-feature enable"]
        );
        // the switch refused a line vswitch takes, and vswitch keeps it
        assert_eq!(
            r.vswitch_takes
                .iter()
                .map(|x| x.0.as_str())
                .collect::<Vec<_>>(),
            ["spanning-tree mode mst"]
        );
        assert_eq!(r.only_ios, [" :: fancy-feature enable"]);
        assert_eq!(r.only_vswitch, [" :: spanning-tree mode mst"]);
    }
}
