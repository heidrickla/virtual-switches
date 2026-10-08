//! A real switch read over SSH (`vswitch capture`): its release, its hardware, its running
//! configuration and the show output vswitch models, swept of secrets and written as
//! `<book>-<release>/<host>-running-config.txt` and `<host>-show.log`, the layout `data/live` holds.
//! Only show commands are sent. The session authenticates by key alone (`BatchMode=yes`), so it never
//! waits at a password prompt.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a capture asks, in order. `terminal width` is set twice so `show vlan brief` shows how the
/// switch wraps at a known width; the `| include` line ends in a space, to see whether IOS keeps it in
/// the expression. `?` follows show commands only: IOS then runs the line it redisplays, and a show
/// command changes nothing.
pub const COMMANDS: &[&str] = &[
    "terminal length 0",
    "terminal width 80",
    "show ?",
    "show vtp ?",
    "show vlan ?",
    "show interfaces ?",
    "show etherchannel ?",
    "show spanning-tree ?",
    "show power inline ?",
    "show ip ?",
    "show version",
    "show inventory",
    "show running-config",
    "show interfaces status",
    "show vlan brief",
    "show etherchannel summary",
    "show interfaces trunk",
    "show ip interface brief",
    "show power inline",
    "show vtp status",
    "terminal width 132",
    "show vlan brief",
    "show running-config | include hostname ",
];

/// One command, the mode its prompt showed as vswitch names modes (`exec` for `>`, `privileged` for
/// `#`, `config-if`), and the lines the switch printed after it.
#[derive(Debug, PartialEq)]
pub struct Exchange {
    pub mode: String,
    pub command: String,
    pub output: Vec<String>,
}

/// Whether a typed command is `full` as IOS reads it: the same words, each typed one a prefix of its
/// own, the first two letters or more and the rest three or more (`sh run` is `show running-config`).
pub fn is_command(typed: &str, full: &str) -> bool {
    let typed: Vec<String> = typed.split_whitespace().map(str::to_lowercase).collect();
    let full: Vec<&str> = full.split_whitespace().collect();
    typed.len() == full.len()
        && typed.iter().zip(&full).enumerate().all(|(i, (t, f))| {
            t.len() >= f.len().min(if i == 0 { 2 } else { 3 }) && f.starts_with(t.as_str())
        })
}

/// The delimiter a configuration's `banner` line opens text with, where the text runs past the line:
/// `^C` as a running configuration prints it, else the first character after the banner's kind.
pub fn banner_opens(line: &str) -> Option<String> {
    const KINDS: &[&str] = &[
        "motd",
        "login",
        "exec",
        "incoming",
        "slip-ppp",
        "prompt-timeout",
        "config-save",
    ];
    let (kind, body) = line.strip_prefix("banner ")?.split_once(' ')?;
    if !KINDS.contains(&kind) {
        return None;
    }
    let body = body.trim_start();
    let d = if body.starts_with("^C") {
        "^C".to_string()
    } else {
        body.chars().next()?.to_string()
    };
    (!body[d.len()..].contains(d.as_str())).then_some(d)
}

/// A session split at its prompts: the switch's hostname and each command it answered. A prompt
/// matches the hostname in any case: a folder's files of one switch may spell it apart.
pub fn exchanges(transcript: &str) -> Option<(String, Vec<Exchange>)> {
    let lines: Vec<&str> = transcript
        .lines()
        .map(|l| l.trim_end_matches('\r').trim_start_matches('\u{feff}'))
        .collect();
    // the hostname: the first line that is a word followed by `#` or `>`
    let host = lines.iter().find_map(|l| {
        let end = l.find(['#', '>'])?;
        let h = &l[..end];
        (!h.is_empty()
            && h.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))
        .then(|| h.to_string())
    })?;
    // `host>` (exec), `host#` (privileged) or a configuration prompt `host(config-if)#`
    let prompt = |l: &str| -> Option<(String, String)> {
        let rest = l
            .get(..host.len())
            .filter(|h| h.eq_ignore_ascii_case(&host))
            .map(|_| &l[host.len()..])?;
        let (mode, rest) = match rest.strip_prefix('(') {
            Some(r) => {
                let close = r.find(')')?;
                (r[..close].to_string(), r[close + 1..].strip_prefix('#')?)
            }
            None => match (rest.strip_prefix('#'), rest.strip_prefix('>')) {
                (Some(r), _) => ("privileged".to_string(), r),
                (None, Some(r)) => ("exec".to_string(), r),
                _ => return None,
            },
        };
        Some((mode, rest.trim_end().to_string()))
    };
    let mut out: Vec<Exchange> = vec![];
    for l in lines {
        match prompt(l) {
            Some((mode, command)) => out.push(Exchange {
                mode,
                command,
                output: vec![],
            }),
            None => {
                if let Some(e) = out.last_mut() {
                    e.output.push(l.to_string());
                }
            }
        }
    }
    Some((host, out))
}

/// The release `show version` names: `Cisco IOS XE Software, Version 17.15.01` is 17.15.1; classic IOS's
/// `Cisco IOS Software, C2960X Software (C2960X-UNIVERSALK9-M), Version 15.2(7)E8, RELEASE SOFTWARE` is
/// 15.2(7)E8.
pub fn release_of(show_version: &[String]) -> Option<String> {
    show_version.iter().find_map(|l| {
        // classic IOS, and IOS XE 16's `Cisco IOS Software [Denali], ..., Version 16.3.9,`
        if l.starts_with("Cisco IOS Software")
            && !l.contains("[IOSXE]")
            && let Some((_, after)) = l.split_once(", Version ")
        {
            return after.split([',', ' ']).next().map(str::to_string);
        }
        let v = l
            .strip_prefix("Cisco IOS XE Software, Version ")?
            .split_whitespace()
            .next()?;
        let parts: Vec<String> = v
            .split('.')
            .map(|p| -> Option<String> {
                let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
                let rest = &p[digits.len()..];
                Some(format!("{}{rest}", digits.parse::<u32>().ok()?))
            })
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();
        (parts.len() >= 2).then(|| parts.join("."))
    })
}

/// A part number as vswitch names it: its model's or module's name, or that name with a licence or
/// variant suffix.
fn known_part(pid: &str) -> Option<&'static str> {
    crate::platform::model(pid)
        .map(|m| m.name)
        .or_else(|| crate::platform::module(pid).map(|m| m.name))
        .or_else(|| {
            let (head, _) = pid.rsplit_once('-')?;
            crate::platform::model(head)
                .map(|m| m.name)
                .or_else(|| crate::platform::module(head).map(|m| m.name))
        })
}

/// The stack `show version` names, for a session with no `show inventory`: its `Switch Ports Model`
/// table's members, else its `Model number` line; no modules, which only the inventory lists. The
/// part numbers vswitch has no model for come back apart.
pub fn stack_of_version(show_version: &[String]) -> (Option<String>, Vec<String>) {
    let mut models: Vec<(u32, String)> = vec![];
    let mut table = false;
    for l in show_version {
        if l.starts_with("Switch Ports Model") {
            table = true;
            continue;
        }
        if table {
            if l.trim().is_empty() {
                table = false;
                continue;
            }
            let w: Vec<&str> = l.trim_start_matches('*').split_whitespace().collect();
            if let [n, ports, model, ..] = w[..]
                && let Ok(n) = n.parse::<u32>()
                && ports.parse::<u32>().is_ok()
            {
                models.push((n, model.to_string()));
            }
        }
    }
    if models.is_empty() {
        models.extend(show_version.iter().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim().eq_ignore_ascii_case("model number")).then(|| (1, v.trim().to_string()))
        }));
    }
    let mut stack = vec![];
    let mut unknown = vec![];
    for (n, m) in models {
        match known_part(&m) {
            Some(k) if crate::platform::model(k).is_some() => stack.push((n, k.to_string())),
            _ => unknown.push(m),
        }
    }
    let spec = (unknown.is_empty() && !stack.is_empty()).then(|| numbered(stack, false));
    (spec, unknown)
}

/// Members as `--stack` notation, by number: a number written where it is not the member's place
/// (a lone switch numbered 2), and always for a 9400's slots.
fn numbered(mut members: Vec<(u32, String)>, slots: bool) -> String {
    members.sort_by_key(|(n, _)| *n);
    members
        .iter()
        .enumerate()
        .map(|(i, (n, m))| {
            if slots || *n != i as u32 + 1 {
                format!("{n}={m}")
            } else {
                m.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The stack `show inventory` lists, as `--stack` notation: each `Switch N` and its uplink module, or
/// each 9400 `Slot N` card and supervisor; and the part numbers vswitch has no model for. A member or
/// card vswitch has no model for leaves no stack: the rest is not the switch.
pub fn stack_of(show_inventory: &[String]) -> (Option<String>, Vec<String>) {
    let mut entries: Vec<(String, String)> = vec![];
    let mut name = String::new();
    for l in show_inventory {
        let l = l.trim();
        if let Some(rest) = l.strip_prefix("NAME:") {
            name = rest
                .split(',')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(['"', '“', '”'])
                .to_string();
        } else if let Some(rest) = l.strip_prefix("PID:") {
            let pid = rest.split(',').next().unwrap_or("").trim().to_string();
            entries.push((name.clone(), pid));
        }
    }
    let known = known_part;
    let mut members: Vec<(u32, String)> = vec![];
    let mut slots = false;
    let mut unknown = vec![];
    let number = |s: &str| {
        s.split_whitespace()
            .next()
            .and_then(|n| n.parse::<u32>().ok())
    };
    for (name, pid) in &entries {
        // classic IOS names a 2960-X member by its number alone: `NAME: "1"`
        let member = name.strip_prefix("Switch ").or_else(|| {
            (!name.is_empty() && name.chars().all(|c| c.is_ascii_digit())).then_some(name.as_str())
        });
        if let Some(rest) = member {
            let Some(n) = number(rest) else { continue };
            match known(pid) {
                Some(m) if crate::platform::module(m).is_some() => {
                    if let Some(member) = members.iter_mut().find(|(k, _)| *k == n) {
                        member.1.push('+');
                        member.1.push_str(m);
                    }
                }
                Some(m) if !rest.contains("Power") && !rest.contains("Fan") => {
                    members.push((n, m.to_string()));
                }
                Some(_) => {}
                None if rest.trim() == n.to_string() => unknown.push(pid.clone()),
                None => {}
            }
        } else if let Some(rest) = name.strip_prefix("Slot ") {
            let Some(n) = number(rest) else { continue };
            slots = true;
            match known(pid) {
                Some(m) => members.push((n, m.to_string())),
                None if rest.contains("Linecard") || rest.contains("Supervisor") => {
                    unknown.push(pid.clone())
                }
                None => {}
            }
        }
    }
    // a switch that names itself `Chassis` alone (an IE-3300) is one member
    if members.is_empty()
        && let Some((_, pid)) = entries.iter().find(|(n, _)| n == "Chassis")
    {
        match known(pid) {
            Some(m) if crate::platform::model(m).is_some() => members.push((1, m.to_string())),
            _ => unknown.push(pid.clone()),
        }
    }
    let spec = (unknown.is_empty() && !members.is_empty()).then(|| numbered(members, slots));
    (spec, unknown)
}

/// A switch's capture: what it is and the two files kept of it.
#[derive(Debug)]
pub struct Capture {
    pub host: String,
    /// the release `show version` names (`17.15.1`)
    pub version: Option<String>,
    pub stack: Option<String>,
    /// part numbers `show inventory` lists that vswitch has no model for
    pub unknown: Vec<String>,
    pub book: &'static str,
    pub running: String,
    pub log: String,
    pub swept: String,
}

impl Capture {
    /// The train the switch runs: `show version`'s, else the configuration's `version` line where it
    /// names one (IOS XE's; classic IOS's `version 15.2` names none).
    pub fn train(&self) -> Option<crate::release::Release> {
        use crate::release::{Release, version_line};
        self.version
            .as_deref()
            .and_then(Release::parse)
            .or_else(|| {
                version_line(&self.running)
                    .and_then(Release::parse)
                    .filter(|r| r.major() >= 16)
            })
    }
}

/// The configuration `show running-config` printed: from its first line (`!` or `version`) through
/// `end`, a banner's text read as text (a line of it may read `end`); empty when it printed none.
pub fn configuration(output: &[String]) -> String {
    let Some(start) = output
        .iter()
        .position(|l| l.starts_with('!') || l.starts_with("version "))
    else {
        return String::new();
    };
    let mut banner: Option<String> = None;
    let mut end = output.len();
    for (i, l) in output.iter().enumerate().skip(start) {
        match &banner {
            Some(d) => {
                if l.contains(d.as_str()) {
                    banner = None;
                }
            }
            None if l == "end" => {
                end = i + 1;
                break;
            }
            None => banner = banner_opens(l),
        }
    }
    output[start..end].join("\n") + "\n"
}

/// The `show tech` sections a capture keeps: the output vswitch models or reads, and the neighbours.
const TECH_KEPT: &[&str] = &[
    "show version",
    "show inventory",
    "show running-config",
    "show interfaces status",
    "show interfaces trunk",
    "show interfaces switchport",
    "show vlan",
    "show vlan brief",
    "show etherchannel summary",
    "show power inline",
    "show vtp status",
    "show ip interface brief",
    "show cdp neighbor detail",
    "show cdp neighbors detail",
    "show lldp neighbors",
];

/// Whether a typed command is `show tech-support` (`show tech`, `sh tech`).
fn is_tech(command: &str) -> bool {
    let w: Vec<String> = command.split_whitespace().map(str::to_lowercase).collect();
    w.len() >= 2
        && is_command(&w[0], "show")
        && w[1].len() >= 4
        && "tech-support".starts_with(&w[1])
}

/// A backup tool's output as a session transcript: a prompt alone on its line joined to the command
/// on the next, and `show tech` split into its sections (`------------------ show clock
/// ------------------`), those in `TECH_KEPT` as commands of their own and the rest left out.
pub fn normalize(text: &str) -> String {
    let text = text.trim_start_matches('\u{feff}');
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    let prompt = |l: &str| {
        let l = l.trim_end();
        let h = l.strip_suffix('#').or_else(|| l.strip_suffix('>'))?;
        (!h.is_empty()
            && h.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))
        .then(|| l.to_string())
    };
    // a prompt with its command on the same line, as a terminal logs it (`SW1#show tech-support`):
    // the prompt and the command, at the session's own hostname once one is known
    let typed = |l: &str, at: &Option<String>| -> Option<(String, String)> {
        let end = l.find(['#', '>'])?;
        let (h, rest) = l.split_at(end + 1);
        let host = &h[..end];
        let known = match at {
            Some(p) => p[..p.len() - 1].eq_ignore_ascii_case(host),
            None => {
                !host.is_empty()
                    && host
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            }
        };
        (known && !rest.trim().is_empty()).then(|| (h.to_string(), rest.trim().to_string()))
    };
    let section = |l: &str| {
        let inner = l
            .trim()
            .strip_prefix("----------")?
            .strip_suffix("----------")?;
        // a rule of dashes inside a section's output (`show vlan`'s under `Remote SPAN VLANs`)
        // names no command
        Some(inner.trim_matches('-').trim().to_string()).filter(|c| !c.is_empty())
    };
    let mut out: Vec<String> = vec![];
    let (mut at, mut tech, mut skip) = (None::<String>, false, false);
    // show tech frames a section: two blank lines after its header, one before the next
    let mut lead = 0;
    let close = |out: &mut Vec<String>, tech: bool, skip: bool| {
        if tech && !skip && out.last().is_some_and(|l| l.trim().is_empty()) {
            out.pop();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if let Some(p) = prompt(l) {
            let next = lines.get(i + 1).copied().unwrap_or("");
            close(&mut out, tech, skip);
            skip = false;
            let joined = !next.trim().is_empty()
                && prompt(next).is_none()
                && typed(next, &Some(p.clone())).is_none();
            tech = joined && is_tech(next);
            if joined {
                out.push(format!("{p}{}", next.trim()));
                at = Some(p);
                i += 2;
                continue;
            }
            out.push(p.clone());
            at = Some(p);
        } else if let Some((p, command)) = typed(l, &at) {
            close(&mut out, tech, skip);
            (tech, skip, lead) = (is_tech(&command), false, 0);
            out.push(l.to_string());
            at = Some(p);
        } else if let (true, Some(cmd)) = (tech, section(l)) {
            close(&mut out, tech, skip);
            skip = !TECH_KEPT.contains(&cmd.as_str());
            lead = 2;
            if let (false, Some(p)) = (skip, &at) {
                out.push(format!("{p}{cmd}"));
            }
        } else if lead > 0 && tech && l.trim().is_empty() {
            lead -= 1;
        } else if !skip {
            lead = 0;
            out.push(l.to_string());
        }
        i += 1;
    }
    out.join("\n") + "\n"
}

/// A transcript with a configuration file appended as its `show running-config` (a backup tool's
/// `Config.Current.Running.<host>.txt`), at the transcript's first prompt.
pub fn with_config(transcript: &str, config: &str) -> Option<String> {
    let (host, _) = exchanges(transcript)?;
    Some(format!(
        "{transcript}\n{host}#show running-config\n{config}\n"
    ))
}

/// A file a capture reads: a session or a configuration, and the switch it names.
#[derive(Debug, PartialEq)]
pub struct Input {
    pub host: String,
    pub config: bool,
    pub text: String,
}

/// What a file holds: a session (lines at a prompt), a configuration printed before its first prompt
/// (a log begun after the prompt: `sh run`, then the answer), or a configuration alone (a backup
/// tool's `Config.Current.Running.<host>.txt`), each with the switch it names: the prompt's host,
/// else the configuration's `hostname`.
pub fn inputs(text: &str) -> Vec<Input> {
    let text = unwrap(&normalize(text.trim_start_matches('\u{feff}')));
    let configured = |t: &str| {
        t.lines()
            .any(|l| l.starts_with("version "))
            .then(|| {
                t.lines()
                    .find_map(|l| l.strip_prefix("hostname "))
                    .map(|h| h.trim().to_string())
            })
            .flatten()
    };
    let mut out = vec![];
    match exchanges(&text) {
        Some((host, ex)) if !ex.is_empty() => {
            let lines: Vec<&str> = text.lines().collect();
            let first = lines
                .iter()
                .position(|l| {
                    l.strip_prefix(host.as_str())
                        .is_some_and(|r| r.starts_with(['#', '>', '(']))
                })
                .unwrap_or(0);
            let before = lines[..first].join("\n");
            if let Some(h) = configured(&before) {
                out.push(Input {
                    host: h,
                    config: true,
                    text: before,
                });
            }
            out.push(Input {
                host,
                config: false,
                text,
            });
        }
        _ => {
            if let Some(host) = configured(&text) {
                out.push(Input {
                    host,
                    config: true,
                    text,
                });
            }
        }
    }
    out
}

/// The configurations a switch was captured with under `out`, one per release folder, its name in any
/// case.
pub fn captured(out: &Path, host: &str) -> Vec<PathBuf> {
    let want = format!("{}-running-config.txt", host.to_lowercase());
    let mut found: Vec<PathBuf> = std::fs::read_dir(out)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|d| d.path().is_dir())
        .flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().flatten())
        .map(|f| f.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().to_lowercase() == want)
        })
        .collect();
    found.sort();
    found
}

/// The train a capture's folder names: `c9300-17.15`, `ws-c3850-48p-16.3.9`.
fn folder_train(kept: &Path) -> Option<crate::release::Release> {
    let folder = kept.parent()?.file_name()?.to_string_lossy().to_string();
    crate::release::Release::parse(folder.rsplit_once('-')?.1)
}

/// The kept capture a new capture of a switch captured already goes into: the one in the folder the
/// new capture would be written to, else, where its files name no hardware vswitch models, the one at
/// the train they name. None where the switch has no capture at that release (an upgrade, a capture
/// of its own); an error where its files name no release, so none can be matched.
pub fn kept_at(
    c: &Capture,
    kept: &[PathBuf],
    out: &Path,
    pending: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    let train = c.train().ok_or(
        "its files name no release (no `show version`, and no train on the `version` line), so \
         they are not matched to a capture",
    )?;
    if let Ok(dir) = dir_of(c, out, pending)
        && let Some(k) = kept.iter().find(|k| k.parent() == Some(dir.as_path()))
    {
        return Ok(Some(k.clone()));
    }
    let named = c.version.is_some() && c.stack.is_some();
    Ok(kept
        .iter()
        .find(|k| !named && folder_train(k) == Some(train))
        .cloned())
}

/// A switch a folder names, and its capture or why there is none.
pub type FolderCapture = (String, Result<Capture, String>);

/// The captures a folder's files make: grouped by the switch each names (in any case), a switch's
/// sessions one after another and its configuration as their last `show running-config`.
pub fn from_folder(dir: &Path) -> Result<Vec<FolderCapture>, String> {
    let files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    from_files(files)
}

/// A file's text: UTF-16 by its byte order mark (Windows PowerShell 5.1's redirection writes it),
/// else UTF-8, a byte that is not read as the replacement character.
pub fn text_of(bytes: &[u8]) -> String {
    let wide = |be: bool| {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| {
                if be {
                    u16::from_be_bytes(c)
                } else {
                    u16::from_le_bytes(c)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xff, 0xfe, ..] => wide(false),
        [0xfe, 0xff, ..] => wide(true),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// The captures some files make, as `from_folder` reads a folder's.
pub fn from_files(mut files: Vec<PathBuf>) -> Result<Vec<FolderCapture>, String> {
    files.sort();
    let mut hosts: std::collections::BTreeMap<String, (String, Vec<String>, Option<String>)> =
        Default::default();
    for f in files {
        let bytes = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        for i in inputs(&text_of(&bytes)) {
            let e = hosts
                .entry(i.host.to_lowercase())
                .or_insert_with(|| (i.host.clone(), vec![], None));
            if i.config {
                e.2 = Some(i.text);
            } else {
                // the switch's own prompt names it, as the first session spells it: the joined
                // sessions are read at that prompt
                if e.1.is_empty() {
                    e.0 = i.host;
                }
                e.1.push(i.text);
            }
        }
    }
    Ok(hosts
        .into_values()
        .map(|(host, sessions, config)| {
            let mut t = sessions.join("\n");
            // a session's own configuration goes with its own show output (a backup tool's file is
            // of another moment: a port it shuts was up at the show tech); the file stands where the
            // sessions printed none
            let printed_one = from_transcript(&t).is_ok_and(|c| !c.running.is_empty());
            if let Some(cfg) = config.filter(|_| !printed_one) {
                t = if sessions.is_empty() {
                    format!("{host}#show running-config\n{cfg}\n")
                } else {
                    with_config(&t, &cfg).unwrap_or(t)
                };
            }
            let c = from_transcript(&t);
            (host, c)
        })
        .collect())
}

/// A transcript whose terminal cut each line every 80 characters and padded the row out to the
/// window's width (c9200 17.15 capture 27's, 237), the lines joined back. The width is the narrowest padded row;
/// a row padded one further began with a space.
pub fn unwrap(transcript: &str) -> String {
    const CUT: usize = 80;
    let padded = |l: &str| -> Option<usize> {
        let rest = l.get(CUT..)?;
        let gap = rest.len() - rest.trim_start_matches(' ').len();
        (gap >= 20 && gap < rest.len()).then_some(CUT + gap)
    };
    let Some(width) = transcript.lines().filter_map(padded).min() else {
        return transcript.to_string();
    };
    let pad = " ".repeat(width - CUT);
    let mut out = String::with_capacity(transcript.len());
    for line in transcript.split_inclusive('\n') {
        let (body, end) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        let mut joined = String::new();
        let mut rest = body;
        while rest.len() > CUT
            && rest.is_char_boundary(CUT)
            && rest[CUT..].starts_with(&pad)
            && rest.len() > width
        {
            joined.push_str(&rest[..CUT]);
            rest = &rest[width..];
        }
        joined.push_str(rest);
        out.push_str(&joined);
        out.push_str(end);
    }
    out
}

/// The values a sweep took out of a text, each one word: what stood where the placeholder stands on
/// a line it changed, between the words kept around it.
fn taken(before: &str, after: &str) -> Vec<String> {
    let mut out = vec![];
    for (b, a) in before.lines().zip(after.lines()).filter(|(b, a)| b != a) {
        let b: Vec<&str> = b.split_whitespace().collect();
        let a: Vec<&str> = a.split_whitespace().collect();
        let mut j = 0;
        for (i, w) in a.iter().enumerate() {
            if *w != crate::sweep::REMOVED || b.get(j) == Some(&crate::sweep::REMOVED) {
                j += 1;
                continue;
            }
            // the value runs to the next word kept, else to the line's end
            let end = a
                .get(i + 1)
                .and_then(|next| (j + 1..b.len()).find(|&k| b[k] == *next))
                .unwrap_or(b.len());
            if end == j + 1 {
                out.push(b[j].to_string());
            }
            j = end;
        }
    }
    out
}

/// A text with each word that is one of `values` (or is one with punctuation around it) replaced by
/// the sweep's placeholder, its spacing kept; and how many lines changed.
fn strip_values(text: &str, values: &std::collections::BTreeSet<String>) -> (String, usize) {
    if values.is_empty() {
        return (text.to_string(), 0);
    }
    let mut n = 0;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let mut rest = line;
        let mut changed = false;
        while !rest.is_empty() {
            let start = rest
                .find(|c: char| !c.is_whitespace())
                .unwrap_or(rest.len());
            out.push_str(&rest[..start]);
            rest = &rest[start..];
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let word = &rest[..end];
            let core = word.trim_matches(|c: char| ",;:()\"'".contains(c));
            if values.contains(word) {
                out.push_str(crate::sweep::REMOVED);
                changed = true;
            } else if values.contains(core) {
                let at = word.find(core).unwrap_or(0);
                out.push_str(&word[..at]);
                out.push_str(crate::sweep::REMOVED);
                out.push_str(&word[at + core.len()..]);
                changed = true;
            } else {
                out.push_str(word);
            }
            rest = &rest[end..];
        }
        n += usize::from(changed);
    }
    (out, n)
}

/// A capture from a session transcript: the running configuration from `show running-config`, the
/// rest as the session log, both swept of secrets. The configuration is empty where the session
/// printed none: `write` refuses such a capture, and `merge` takes its log.
pub fn from_transcript(transcript: &str) -> Result<Capture, String> {
    let transcript = &unwrap(transcript);
    let (host, ex) = exchanges(transcript).ok_or("no prompt in the transcript")?;
    // a command typed again after `enable` answers the second time: the last answer that says
    // something; IOS takes a command abbreviated (`sh inv`)
    let answer = |c: &str| {
        ex.iter()
            .rev()
            .find(|e| is_command(&e.command, c) && !e.output.iter().all(|l| l.trim().is_empty()))
            .map(|e| &e.output)
    };
    // every `show version` answer, newest first: a `show tech` holds more than one
    let versions = || {
        ex.iter()
            .rev()
            .filter(|e| is_command(&e.command, "show version"))
            .map(|e| &e.output)
    };
    let version = versions().find_map(|o| release_of(o));
    // the inventory, else what `show version` names (a backup tool asks only `show version`)
    let from_version = || {
        versions()
            .map(|o| stack_of_version(o))
            .find(|(s, u)| s.is_some() || !u.is_empty())
            .unwrap_or((None, vec![]))
    };
    let (stack, unknown) = match answer("show inventory").map(|o| stack_of(o)) {
        Some((None, u)) if u.is_empty() => from_version(),
        Some(found) => found,
        None => from_version(),
    };
    // `show running-config`, else whatever printed a configuration (a command the switch completed),
    // a saved one last (`show startup-config`, `show config`): it lacks what was not saved
    let printed_config = |e: &Exchange| {
        e.output.iter().any(|l| l.starts_with("version "))
            && e.output.iter().any(|l| l.starts_with("hostname "))
    };
    let saved = |e: &Exchange| {
        let w: Vec<&str> = e.command.split_whitespace().collect();
        w.len() >= 2
            && is_command(w[0], "show")
            && ["startup-config", "configuration", "archive"]
                .iter()
                .any(|f| is_command(w[1], f))
    };
    let running = |e: &Exchange| is_command(&e.command, "show running-config");
    let config = ex
        .iter()
        .rev()
        .filter(|e| running(e))
        .chain(ex.iter().rev().filter(|e| printed_config(e) && !saved(e)))
        .chain(ex.iter().rev().filter(|e| printed_config(e) && saved(e)))
        .map(|e| configuration(&e.output))
        .find(|c| !c.is_empty())
        .unwrap_or_default();
    let family = stack
        .as_deref()
        .and_then(|s| crate::platform::parse_stack(s).ok())
        .and_then(|s| s.family());
    let book = match family {
        Some(f) => crate::grammars::book(Some(f)),
        // a part vswitch does not model still names its series
        None if unknown.iter().any(|p| p.starts_with("C9200")) => "c9200",
        None if unknown.iter().any(|p| p.starts_with("C9400")) => "c9400",
        None => "c9300",
    };
    let mut log = String::new();
    for e in ex.iter().filter(|e| !running(e) && !printed_config(e)) {
        // each prompt as the switch printed it: live help answers by mode
        let prompt = match e.mode.as_str() {
            "exec" => format!("{host}>"),
            "privileged" => format!("{host}#"),
            m => format!("{host}({m})#"),
        };
        log.push_str(&format!("{prompt}{}\n", e.command));
        for l in &e.output {
            log.push_str(l);
            log.push('\n');
        }
    }
    let (raw_config, raw_log) = (config, log);
    let config = crate::sweep::sweep(&raw_config);
    let log = crate::sweep::sweep(&raw_log);
    // a value one sweep took goes wherever else it stands: a filtered `show running-config` prints a
    // server's `key` without the block that makes it one
    let values: std::collections::BTreeSet<String> = taken(&raw_config, &config.text)
        .into_iter()
        .chain(taken(&raw_log, &log.text))
        .filter(|v| v.len() >= 4)
        .collect();
    let (config_text, _) = strip_values(&config.text, &values);
    let (log_text, crossed) = strip_values(&log.text, &values);
    let (running, a) = unserialed(&config_text);
    let (log_text, b) = unserialed(&log_text);
    let crossed = if crossed > 0 {
        format!(" and {crossed} line(s) holding a value swept from another line")
    } else {
        String::new()
    };
    let swept = format!(
        "running configuration: {}; session log: {}{crossed}; {} serial number(s) removed",
        config.summary(),
        log.summary(),
        a + b
    );
    Ok(Capture {
        host,
        version,
        stack,
        unknown,
        book,
        running,
        log: log_text,
        swept,
    })
}

/// A text with its serial numbers removed: `show version`'s serial lines and processor board ID,
/// `show inventory`'s `SN:`, a configuration's `license udi ... sn`; and how many went.
pub fn unserialed(text: &str) -> (String, usize) {
    let rules = [
        (
            r"(?m)^(.*[Ss]erial [Nn](?:UMBER|umber|um)\s*:\s*)(?:[A-Za-z0-9-]+)(\s*)$",
            "${1}<removed>${2}",
        ),
        (r"(SN:\s*)[A-Za-z0-9]{6,}", "${1}<removed>"),
        (r"(?m)^(Processor board ID )\S+", "${1}<removed>"),
        (r"(?m)^(\s*license udi pid \S+ sn )\S+", "${1}<removed>"),
    ];
    let mut t = text.to_string();
    let mut n = 0;
    for (pattern, to) in rules {
        let re = regex::Regex::new(pattern).expect("a serial rule");
        n += re.find_iter(&t).count();
        t = re.replace_all(&t, to).into_owned();
    }
    (t, n)
}

/// Where `write` puts a capture (below).
pub fn dir_of(c: &Capture, out: &Path, pending: Option<&Path>) -> Result<PathBuf, String> {
    // the train the release runs: 17.15.1 is 17.15, 15.2(7)E0a is 15.2(7)E
    let train = c
        .version
        .as_deref()
        .and_then(crate::release::Release::parse);
    Ok(match (train, &c.stack) {
        (Some(train), Some(_)) => out.join(format!("{}-{train}", c.book)),
        _ => {
            let pending = pending.ok_or(
                "`show version` and `show inventory` name no release and hardware vswitch models: \
                 give --pending <dir> for the capture",
            )?;
            let model = c
                .unknown
                .first()
                .map_or("unidentified".to_string(), |m| m.to_lowercase());
            let release = c
                .version
                .clone()
                .or_else(|| crate::release::version_line(&c.running).map(str::to_string))
                .unwrap_or_else(|| "unknown".into());
            pending.join(format!("{model}-{release}"))
        }
    })
}

/// What a capture adds to the one kept of the same switch at `kept` (its running configuration's
/// path): the commands the new session answers that the kept log does not, appended to the kept log,
/// whose own bytes stay as they are. A command both answer keeps the kept answer, which goes with the
/// kept configuration, and the kept configuration stays. Returns the commands added and whether the
/// configurations differ (never where the new files print none).
pub fn merge(kept: &Path, c: &Capture) -> Result<(Vec<String>, bool), String> {
    let name = kept
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let stem = name.strip_suffix("-running-config.txt").unwrap_or(&name);
    let log_path = kept.with_file_name(format!("{stem}-show.log"));
    let bytes = match std::fs::read(&log_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(e) => return Err(format!("{}: {e}", log_path.display())),
    };
    // a log another tool kept may not be UTF-8: it is read for its commands and appended to whole
    let kept_log = String::from_utf8_lossy(&bytes);
    let mut log = String::new();
    let (host, had) = exchanges(&kept_log).unwrap_or((stem.to_string(), vec![]));
    let mut asked: std::collections::BTreeSet<String> =
        had.iter().map(|e| e.command.to_lowercase()).collect();
    let mut added = vec![];
    for e in exchanges(&c.log).map(|(_, ex)| ex).unwrap_or_default() {
        let command = e.command.trim();
        if command.is_empty()
            || e.output.iter().all(|l| l.trim().is_empty())
            || !asked.insert(command.to_lowercase())
        {
            continue;
        }
        let prompt = match e.mode.as_str() {
            "exec" => format!("{host}>"),
            "privileged" => format!("{host}#"),
            m => format!("{host}({m})#"),
        };
        log.push_str(&format!("{prompt}{command}\n"));
        for l in &e.output {
            log.push_str(l);
            log.push('\n');
        }
        added.push(command.to_string());
    }
    if !added.is_empty() {
        let mut whole = bytes;
        if whole.last().is_some_and(|b| *b != b'\n') {
            whole.push(b'\n');
        }
        whole.extend_from_slice(log.as_bytes());
        std::fs::write(&log_path, whole).map_err(|e| format!("{}: {e}", log_path.display()))?;
    }
    let kept_config = std::fs::read(kept)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let differs = !c.running.trim().is_empty() && kept_config.trim_end() != c.running.trim_end();
    Ok((added, differs))
}

/// Writes a capture under `out/<book>-<train>/`, as `data/live` keeps the cores'. A switch whose
/// `show version` and `show inventory` do not name a release and hardware vswitch models goes under
/// `pending/<part>-<release>/` instead (`unidentified` and the configuration's `version` where they
/// say nothing), kept for the platforms to come and never taken as live evidence.
pub fn write(c: &Capture, out: &Path, pending: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    if c.running.trim().is_empty() {
        return Err("the transcript prints no configuration".into());
    }
    let dir = dir_of(c, out, pending)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut written = vec![];
    for (name, text) in [
        (format!("{}-running-config.txt", c.host), &c.running),
        (format!("{}-show.log", c.host), &c.log),
    ] {
        let p = dir.join(name);
        std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
        written.push(p);
    }
    Ok(written)
}

/// The `ssh` arguments of a session: a terminal for IOS's prompts, and the key as the only way in
/// (`BatchMode=yes` fails the session where a password would be asked).
pub fn ssh_args(target: &str, key: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = [
        "-tt",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=20",
        "-o",
        "StrictHostKeyChecking=accept-new",
    ]
    .map(String::from)
    .to_vec();
    if let Some(k) = key {
        a.extend(["-i", k, "-o", "IdentitiesOnly=yes"].map(String::from));
    }
    a.push(target.to_string());
    a
}

/// Runs the commands in one SSH session and returns what the switch printed.
pub fn session(target: &str, key: Option<&str>, commands: &[&str]) -> Result<String, String> {
    let ssh = std::env::var("VSWITCH_SSH").unwrap_or_else(|_| "ssh".into());
    let mut cmd = Command::new(&ssh);
    cmd.args(ssh_args(target, key))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("{ssh}: {e}"))?;
    {
        let mut stdin = child.stdin.take().ok_or("ssh: no stdin")?;
        for c in commands.iter().chain(&["exit"]) {
            stdin
                .write_all(format!("{c}\n").as_bytes())
                .map_err(|e| format!("ssh: {e}"))?;
        }
    }
    let out = child.wait_with_output().map_err(|e| format!("ssh: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() && !text.contains('#') {
        return Err(format!(
            "ssh {target}: {} {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn a_capture_sends_show_commands_and_asks_help_after_paging_is_off() {
        let paging = COMMANDS
            .iter()
            .position(|c| *c == "terminal length 0")
            .unwrap();
        for (i, c) in COMMANDS.iter().enumerate() {
            assert!(c.starts_with("show ") || c.starts_with("terminal "), "{c}");
            if c.ends_with('?') {
                assert!(c.starts_with("show ") && i > paging, "{c}");
            }
        }
    }

    #[test]
    fn a_session_takes_the_key_and_never_a_password_prompt() {
        let a = ssh_args("developer@10.10.20.181", Some("k"));
        assert_eq!(a[0], "-tt");
        assert!(a.windows(2).any(|w| w == ["-o", "BatchMode=yes"]));
        assert!(a.windows(2).any(|w| w == ["-i", "k"]));
        assert_eq!(a.last().unwrap(), "developer@10.10.20.181");
    }

    #[cfg(all(test, captures))]
    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/capture-tests.rs"));

    #[test]
    fn show_version_and_show_inventory_name_the_release_and_the_stack() {
        assert_eq!(
            release_of(&lines(
                "\nCisco IOS XE Software, Version 17.15.01\nCisco IOS Software [IOSXE], Catalyst L3 Switch Software"
            ))
            .as_deref(),
            Some("17.15.1")
        );
        assert_eq!(
            release_of(&lines("Cisco IOS XE Software, Version 16.09.04")).as_deref(),
            Some("16.9.4")
        );
        assert_eq!(
            release_of(&lines(
                "Cisco IOS Software, C2960X Software (C2960X-UNIVERSALK9-M), Version 15.2(7)E8, RELEASE SOFTWARE (fc1)"
            ))
            .as_deref(),
            Some("15.2(7)E8")
        );
        let inventory = lines(
            r#"NAME: "c93xx Stack", DESCR: "c93xx Stack"
PID: C9300-48UXM       , VID: V02  , SN: FCW0000A000

NAME: "Switch 1", DESCR: "C9300-48UXM"
PID: C9300-48UXM       , VID: V02  , SN: FCW0000A000

NAME: "Switch 1 - Power Supply A", DESCR: "Switch 1 - Power Supply A"
PID: PWR-C1-1100WAC-P  , VID: V01  , SN: ART0000A000

NAME: "Switch 1 FRU Uplink Module 1", DESCR: "8x10G Uplink Module"
PID: C9300-NM-8X       , VID: V01  , SN: FOC0000A000

NAME: "Switch 2", DESCR: "C9300-24P"
PID: C9300-24P-A       , VID: V01  , SN: FCW0000A001

NAME: "TenGigabitEthernet1/1/1", DESCR: "SFP-10GBase-SR"
PID: SFP-10G-SR        , VID: V03  , SN: AVD0000A000"#,
        );
        assert_eq!(
            stack_of(&inventory),
            (
                Some("C9300-48UXM+C9300-NM-8X,C9300-24P".to_string()),
                vec![]
            )
        );
        let chassis = lines(
            r#"NAME: "Chassis", DESCR: "Cisco Catalyst 9400 Series 7 Slot Chassis"
PID: C9407R            , VID: V01  , SN: FXS0000A000
NAME: "Slot 1 Linecard", DESCR: "Cisco Catalyst 9400 Series 48-Port UPOE 10/100/1000 (RJ-45)"
PID: C9400-LC-48U      , VID: V01  , SN: JAE0000A000
NAME: "Slot 3 Supervisor", DESCR: "Cisco Catalyst 9400 Series Supervisor 1XL Module"
PID: C9400-SUP-1XL     , VID: V02  , SN: JAE0000A001"#,
        );
        assert_eq!(
            stack_of(&chassis).0.as_deref(),
            Some("1=C9400-LC-48U,3=C9400-SUP-1XL")
        );
        // a model vswitch does not have is named, not guessed
        let cx = lines(
            "NAME: \"Switch 1\", DESCR: \"C9200CX-12P-2X2G\"\nPID: C9200CX-12P-2X2G  , VID: V01",
        );
        assert_eq!(stack_of(&cx), (None, vec!["C9200CX-12P-2X2G".to_string()]));
    }

    #[test]
    fn a_capture_keeps_the_configuration_and_the_session_swept() {
        let transcript = "lab1#terminal length 0\r\nlab1#show version\r\nCisco IOS XE Software, Version 17.15.01\r\nlab1#show inventory\r\nNAME: \"Switch 1\", DESCR: \"C9300-24P\"\r\nPID: C9300-24P-E , VID: V01\r\nlab1#show running-config\r\nBuilding configuration...\r\n\r\nCurrent configuration : 120 bytes\r\n!\r\nversion 17.15\r\nhostname lab1\r\nenable secret 9 $9$abcdefghijklmn$opqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ012345\r\nend\r\n\r\nlab1#show vlan brief\r\n\r\nVLAN Name Status Ports\r\nlab1#exit\r\n";
        let c = from_transcript(transcript).unwrap();
        assert_eq!(
            (
                c.host.as_str(),
                c.version.as_deref(),
                c.stack.as_deref(),
                c.book
            ),
            ("lab1", Some("17.15.1"), Some("C9300-24P"), "c9300")
        );
        assert!(c.running.starts_with("!\nversion 17.15\nhostname lab1\n"));
        assert!(c.running.ends_with("end\n"));
        assert!(!c.running.contains("$9$"), "{}", c.running);
        assert!(c.log.contains("lab1#show vlan brief\n"));
        assert!(!c.log.contains("Building configuration"));
        let dir = std::env::temp_dir().join(format!("vswitch-capture-{}", std::process::id()));
        let files = write(&c, &dir, None).unwrap();
        assert_eq!(
            files
                .iter()
                .map(|p| p
                    .strip_prefix(&dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"))
                .collect::<Vec<_>>(),
            [
                "c9300-17.15/lab1-running-config.txt",
                "c9300-17.15/lab1-show.log"
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_terminal_s_wrap_padding_is_joined_back() {
        // c9200 17.15 capture 27's rows: cut at 80, padded to its window's 237 columns; the second row's cut
        // fell on a space
        let row =
            "Gi1/0/1      OFFICEPC50         connected    65           full    100 10/100/100";
        let sfp =
            "Gi1/1/1      UPLINK-DS1A        connected    65           full   1000 1000BaseLX";
        let pad = " ".repeat(157);
        let t = format!("sw#show int status\n{row}{pad}0BaseTX\n{sfp}{pad} SFP\nsw#\n");
        assert_eq!(
            unwrap(&t),
            format!("sw#show int status\n{row}0BaseTX\n{sfp} SFP\nsw#\n")
        );
        // a line cut twice
        let long = "x".repeat(200);
        let cut = format!(
            "{}{pad}{}{pad}{}\n",
            &long[..80],
            &long[80..160],
            &long[160..]
        );
        assert_eq!(unwrap(&cut), format!("{long}\n"));
        // nothing padded: nothing changes
        assert_eq!(unwrap("sw#show clock\n12:00\n"), "sw#show clock\n12:00\n");
    }

    #[test]
    fn a_classic_session_answers_after_enable_and_names_its_member_by_number() {
        // a 2960-X session: show running-config refused at `>`, typed again after `enable`
        let transcript = "sw4>terminal length 0\r\nsw4>show version\r\nCisco IOS Software, C2960X Software (C2960X-UNIVERSALK9-M), Version 15.2(7)E0a, RELEASE SOFTWARE (fc1)\r\nsw4>show inventory\r\nNAME: \"1\", DESCR: \"WS-C2960X-24PS-L\"\r\nPID: WS-C2960X-24PS-L  , VID: V04  , SN: X\r\n\r\nNAME: \"GigabitEthernet1/0/26\", DESCR: \"1000BaseLX SFP\"\r\nPID: GLC-LH-SMD          , VID: V02  , SN: Y\r\nsw4>show running-config\r\n                  ^\r\n% Invalid input detected at '^' marker.\r\n\r\nsw4>enable\r\nPassword:\r\nsw4#show running-config\r\nBuilding configuration...\r\n\r\n!\r\nversion 15.2\r\nhostname sw4\r\nend\r\n\r\nsw4#exit\r\n";
        let c = from_transcript(transcript).unwrap();
        assert_eq!(
            (c.version.as_deref(), c.stack.as_deref(), c.book),
            (Some("15.2(7)E0a"), Some("WS-C2960X-24PS-L"), "c2960x")
        );
        assert!(c.running.starts_with("!\nversion 15.2\nhostname sw4\n"));
        // the log keeps each prompt: what was typed before `enable` was typed at `>`
        assert!(c.log.starts_with("sw4>terminal length 0\n"), "{}", c.log);
        assert!(c.log.contains("\nsw4>enable\n"));
        assert!(c.log.contains("\nsw4#exit\n"));
        let dir = std::env::temp_dir().join(format!("vswitch-classic-{}", std::process::id()));
        let files = write(&c, &dir, None).unwrap();
        assert!(
            files[0].ends_with("c2960x-15.2(7)E/sw4-running-config.txt"),
            "{files:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
        // a session whose every run printed nothing has no configuration, and is not written
        let refused =
            "sw4>show running-config\r\n% Invalid input detected at '^' marker.\r\nsw4>exit\r\n";
        let c = from_transcript(refused).unwrap();
        assert!(c.running.is_empty());
        assert!(write(&c, &dir, None).is_err());
        assert!(!dir.exists());
    }

    const VERSION_2960X: &str = "Cisco IOS Software, C2960X Software (C2960X-UNIVERSALK9-M), Version 15.2(7)E0a, RELEASE SOFTWARE (fc1)\nProcessor board ID FOC1234X5YZ\nSystem serial number            : FOC1234X5YZ\n\nSwitch Ports Model                     SW Version            SW Image\n------ ----- -----                     ----------            ----------\n*    1 28    WS-C2960X-24PS-L          15.2(7)E0a            C2960X-UNIVERSALK9-M\n\n";

    /// A scratch directory under the system temp directory, removed when dropped.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!("vswitch-cap-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Scratch(d)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_backup_tool_s_output_and_show_tech_read_as_a_session() {
        // the prompt alone, the command on the next line
        let tool = format!("terminal length 0\nSW1#\nshow version\n{VERSION_2960X}SW1#\n");
        let t = normalize(&tool);
        assert!(
            t.contains("SW1#show version\nCisco IOS Software, C2960X"),
            "{t}"
        );
        // show tech's sections: the kept ones as commands, the rest left out, each without the two
        // blank lines after its header and the one before the next (c9300 17.15 capture 1's framing)
        let tech = "SW1#\nshow tech\n\n------------------ show clock ------------------\n\n\n10:55 CST\n\n------------------ show interfaces status ------------------\n\n\n\nPort      Name\nGi1/0/1\n\n------------------ show interfaces trunk ------------------\n\n\n\n------------------ show controllers ------------------\n\n\nasic\n\nSW1#\n";
        let t = normalize(tech);
        assert!(
            t.contains("SW1#show interfaces status\n\nPort      Name\nGi1/0/1\nSW1#show interfaces trunk\nSW1#"),
            "{t}"
        );
        assert!(!t.contains("10:55") && !t.contains("asic"), "{t}");
        // a rule of dashes in a kept section's output is output
        let rule = "-".repeat(78);
        let vlan = format!(
            "SW1#\nshow tech\n\n------------------ show vlan ------------------\n\n\nRemote SPAN VLANs\n{rule}\n\n\nPrimary Secondary Type\n\n------------------ show clock ------------------\n\n\n10:55\n\nSW1#\n"
        );
        let t = normalize(&vlan);
        assert!(
            t.contains(&format!(
                "SW1#show vlan\nRemote SPAN VLANs\n{rule}\n\n\nPrimary Secondary Type\nSW1#"
            )),
            "{t}"
        );
    }

    #[test]
    fn each_file_is_read_for_what_it_holds() {
        let config = "!\nversion 15.2\nhostname SW1\n!\nend\n";
        // a configuration alone
        let i = inputs(config);
        assert_eq!((i.len(), i[0].host.as_str(), i[0].config), (1, "SW1", true));
        // a log begun after the prompt: `sh run`, then the configuration, then a session
        let log =
            format!("sh run\nBuilding configuration...\n{config}sw1#show vlan brief\nVLAN Name\n");
        let i = inputs(&log);
        assert_eq!(i.len(), 2, "{i:?}");
        assert!(i[0].config && i[0].host == "SW1");
        assert!(!i[1].config && i[1].host == "sw1");
        // a session
        assert!(!inputs("sw1#show clock\n10:00\n")[0].config);
        // nothing to read
        assert!(inputs("just text\n").is_empty());
        // a configuration `show config` printed, where no `show running-config` was typed
        let c = from_transcript(&format!(
            "sw1#show config\nUsing 100 out of 65536 bytes\n{config}"
        ))
        .unwrap();
        assert!(c.running.contains("hostname SW1"));
    }

    #[test]
    fn a_folder_s_files_group_by_the_switch_they_name() {
        let d = Scratch::new("folder");
        // the backup tool's file names spell the switch apart from its prompt, and its prompt in
        // another case than the configuration's hostname
        std::fs::write(
            d.0.join("sw1.DeviceOutput.txt"),
            format!("terminal length 0\nSW1#\nshow version\n{VERSION_2960X}SW1#\n"),
        )
        .unwrap();
        std::fs::write(
            d.0.join("Config.Current.Running.sw-one.txt"),
            "!\nversion 15.2\nhostname sw1\ninterface GigabitEthernet1/0/1\n description x\n!\nend\n",
        )
        .unwrap();
        // a session with its own configuration keeps it over the backup tool's file
        std::fs::write(
            d.0.join("sw2.DeviceOutput.txt"),
            "SW2#\nshow running-config\n!\nversion 15.2\nhostname SW2\ninterface GigabitEthernet1/0/1\n shutdown\n!\nend\nSW2#\n",
        )
        .unwrap();
        std::fs::write(
            d.0.join("Config.Current.Running.SW2.txt"),
            "!\nversion 15.2\nhostname SW2\n!\nend\n",
        )
        .unwrap();
        let caps = from_folder(&d.0).unwrap();
        assert_eq!(
            caps.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
            ["SW1", "SW2"]
        );
        let mut caps = caps.into_iter().map(|c| c.1.unwrap());
        let c = caps.next().unwrap();
        assert_eq!(
            (c.host.as_str(), c.version.as_deref(), c.stack.as_deref()),
            ("SW1", Some("15.2(7)E0a"), Some("WS-C2960X-24PS-L"))
        );
        assert!(c.running.contains(" description x"));
        // the serial numbers show version printed are gone
        assert!(!c.log.contains("FOC1234X5YZ"), "{}", c.log);
        assert!(caps.next().unwrap().running.contains(" shutdown"));
    }

    #[test]
    fn show_version_names_the_hardware_where_no_inventory_does() {
        let lines = |t: &str| t.lines().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(
            stack_of_version(&lines(VERSION_2960X)),
            (Some("WS-C2960X-24PS-L".to_string()), vec![])
        );
        // a model vswitch has no model for comes back apart
        let ie = "Model Number                       : IE-4000-8GT4G-E\n";
        assert_eq!(
            stack_of_version(&lines(ie)),
            (None, vec!["IE-4000-8GT4G-E".to_string()])
        );
        let ie = "Model Number                       : IE-3000-8TC\n";
        assert_eq!(
            stack_of_version(&lines(ie)),
            (Some("IE-3000-8TC".to_string()), vec![])
        );
        // an IE-3300's inventory names its chassis alone
        let inventory = "NAME: \"Chassis\", DESCR: \"IE-3300-8T2S Expandable Chassis\"\nPID: IE-3300-8T2S      , VID: V06  , SN: <removed>\n";
        assert_eq!(
            stack_of(&lines(inventory)),
            (Some("IE-3300-8T2S".to_string()), vec![])
        );
        // IOS XE 16's release line
        let denali = "Cisco IOS Software [Denali], Catalyst L3 Switch Software (CAT3K_CAA-UNIVERSALK9-M), Version 16.3.9, RELEASE SOFTWARE (fc4)";
        assert_eq!(release_of(&lines(denali)).as_deref(), Some("16.3.9"));
    }

    #[test]
    fn a_switch_captured_again_gains_what_its_capture_lacks() {
        let d = Scratch::new("merge");
        let dir = d.0.join("c2960x-15.2(7)E");
        std::fs::create_dir_all(&dir).unwrap();
        let kept = dir.join("SW1-running-config.txt");
        std::fs::write(&kept, "!\nversion 15.2\nhostname SW1\nend\n").unwrap();
        std::fs::write(
            dir.join("SW1-show.log"),
            "SW1#show vlan brief\nkept answer\n",
        )
        .unwrap();
        // found in any case
        assert_eq!(captured(&d.0, "sw1"), std::slice::from_ref(&kept));
        let c = from_transcript(
            "SW1#show vlan brief\nnew answer\nSW1#show cdp neighbor detail\nneighbour\nSW1#show running-config\n!\nversion 15.2\nhostname SW1\nntp server 10.0.0.1\nend\n"
        )
        .unwrap();
        let (added, differs) = merge(&kept, &c).unwrap();
        assert_eq!(added, ["show cdp neighbor detail"]);
        assert!(differs);
        let log = std::fs::read_to_string(dir.join("SW1-show.log")).unwrap();
        assert!(
            log.contains("kept answer") && !log.contains("new answer"),
            "{log}"
        );
        assert!(
            log.contains("SW1#show cdp neighbor detail\nneighbour\n"),
            "{log}"
        );
        // again: nothing new
        assert_eq!(merge(&kept, &c).unwrap().0, Vec::<String>::new());
    }

    #[test]
    fn an_unidentified_switch_is_kept_apart_from_live_evidence() {
        let d = Scratch::new("pending");
        let c = from_transcript("SW1#show running-config\n!\nversion 15.2\nhostname SW1\nend\n")
            .unwrap();
        assert!(write(&c, &d.0.join("live"), None).is_err());
        let files = write(&c, &d.0.join("live"), Some(&d.0.join("pending"))).unwrap();
        assert!(
            files[0].ends_with("pending/unidentified-15.2/SW1-running-config.txt"),
            "{files:?}"
        );
        assert!(!d.0.join("live").exists());
        assert_eq!(
            unserialed("PID: X , VID: V1 , SN: FOC1234567\n").0,
            "PID: X , VID: V1 , SN: <removed>\n"
        );
    }

    #[test]
    fn a_member_keeps_its_number() {
        let one = |n: u32, pid: &str| {
            format!("NAME: \"Switch {n}\", DESCR: \"x\"\nPID: {pid} , VID: V01\n")
        };
        // a switch numbered 2 alone, and a stack numbered 1 and 3
        assert_eq!(
            stack_of(&lines(&one(2, "C9300-24P-E"))),
            (Some("2=C9300-24P".to_string()), vec![])
        );
        let pair = one(1, "C9300-24P-E") + &one(3, "C9300-48P");
        assert_eq!(
            stack_of(&lines(&pair)).0.as_deref(),
            Some("C9300-24P,3=C9300-48P")
        );
        // show version's table numbers its members too
        let two = VERSION_2960X.replace("*    1 28", "*    2 28");
        assert_eq!(
            stack_of_version(&lines(&two)).0.as_deref(),
            Some("2=WS-C2960X-24PS-L")
        );
        // the switch's own ports type at that number
        let c = from_transcript(&format!(
            "lab2#show version\nCisco IOS XE Software, Version 17.15.01\nlab2#show inventory\n{}lab2#show running-config\n!\nversion 17.15\nhostname lab2\ninterface GigabitEthernet2/0/1\n description a\nend\n",
            one(2, "C9300-24P-E")
        ))
        .unwrap();
        let stack = crate::platform::parse_stack(c.stack.as_deref().unwrap()).unwrap();
        assert!(stack.has("GigabitEthernet2/0/1"), "{:?}", c.stack);
    }

    #[test]
    fn a_member_or_card_vswitch_does_not_model_leaves_no_stack() {
        let inventory = "NAME: \"Switch 1\", DESCR: \"x\"\nPID: C9300-24P-E , VID: V01\nNAME: \"Switch 2\", DESCR: \"x\"\nPID: C9300-24ZZ , VID: V01\n";
        assert_eq!(
            stack_of(&lines(inventory)),
            (None, vec!["C9300-24ZZ".to_string()])
        );
        let chassis = "NAME: \"Slot 1 Linecard\", DESCR: \"x\"\nPID: C9400-LC-48ZZ , VID: V01\nNAME: \"Slot 3 Supervisor\", DESCR: \"x\"\nPID: C9400-SUP-1XL , VID: V02\n";
        assert_eq!(
            stack_of(&lines(chassis)),
            (None, vec!["C9400-LC-48ZZ".to_string()])
        );
        // so the capture is kept apart, not filed as live evidence of the rest
        let d = Scratch::new("partial");
        let c = from_transcript(&format!(
            "lab3#show version\nCisco IOS XE Software, Version 17.15.01\nlab3#show inventory\n{inventory}lab3#show running-config\n!\nversion 17.15\nhostname lab3\nend\n"
        ))
        .unwrap();
        assert!(write(&c, &d.0.join("live"), None).is_err());
        let files = write(&c, &d.0.join("live"), Some(&d.0.join("pending"))).unwrap();
        assert!(
            files[0].ends_with("pending/c9300-24zz-17.15.1/lab3-running-config.txt"),
            "{files:?}"
        );
    }

    #[test]
    fn a_session_typed_by_hand_is_read_through_its_abbreviations() {
        let t = format!(
            "SW1#sh ver\n{VERSION_2960X}SW1#sh inv\nNAME: \"1\", DESCR: \"x\"\nPID: WS-C2960X-24PS-L  , VID: V04\nSW1#sh run\n!\nversion 15.2\nhostname SW1\ninterface GigabitEthernet1/0/1\n description RUNNING\nend\nSW1#sh start\n!\nversion 15.2\nhostname SW1\ninterface GigabitEthernet1/0/1\n description SAVED\nend\nSW1#exit\n"
        );
        let c = from_transcript(&t).unwrap();
        assert_eq!(
            (c.version.as_deref(), c.stack.as_deref()),
            (Some("15.2(7)E0a"), Some("WS-C2960X-24PS-L"))
        );
        // the running configuration, not the saved one printed after it
        assert!(c.running.contains("description RUNNING"), "{}", c.running);
        assert!(!c.log.contains("description"), "{}", c.log);
        // nor after one a command of another name printed
        let t = "SW1#show running-config all\n!\nversion 15.2\nhostname SW1\ninterface GigabitEthernet1/0/1\n description RUNNING\nend\nSW1#show startup-config\n!\nversion 15.2\nhostname SW1\ninterface GigabitEthernet1/0/1\n description SAVED\nend\n";
        let c = from_transcript(t).unwrap();
        assert!(c.running.contains("description RUNNING"), "{}", c.running);
    }

    #[test]
    fn a_command_abbreviated_as_ios_takes_it_is_that_command() {
        assert!(is_command("sh run", "show running-config"));
        assert!(is_command("SHOW  VER", "show version"));
        assert!(is_command("sh int status", "show interfaces status"));
        assert!(!is_command("sh v", "show version"));
        assert!(!is_command(
            "show running-config all",
            "show running-config"
        ));
        assert!(!is_command("show interfaces", "show inventory"));
    }

    #[test]
    fn a_banner_line_reading_end_does_not_end_the_configuration() {
        let out = lines(
            "Building configuration...\n!\nversion 15.2\nhostname SW1\nbanner motd ^C\nAuthorized use only\nend\n^C\n!\nline con 0\n!\nend\n",
        );
        let c = configuration(&out);
        assert!(c.ends_with("end\n^C\n!\nline con 0\n!\nend\n"), "{c}");
    }

    #[test]
    fn a_show_tech_logged_at_its_prompt_splits_into_its_sections() {
        let tech = format!(
            "SW1#terminal length 0\nSW1#show tech-support\n\n------------------ show version ------------------\n\n\n{VERSION_2960X}------------------ show controllers ------------------\n\n\nasic\n\nSW1#exit\n"
        );
        let t = normalize(&tech);
        assert!(
            t.contains("SW1#show version\nCisco IOS Software, C2960X"),
            "{t}"
        );
        assert!(!t.contains("asic") && t.ends_with("SW1#exit\n"), "{t}");
        let c = from_transcript(&format!(
            "{t}SW1#show running-config\n!\nversion 15.2\nhostname SW1\nend\n"
        ))
        .unwrap();
        assert_eq!(c.version.as_deref(), Some("15.2(7)E0a"));
    }

    #[test]
    fn a_log_saved_with_a_byte_order_mark_reads_its_first_command() {
        let tool = format!("\u{feff}SW1#\nshow version\n{VERSION_2960X}SW1#\n");
        let t = normalize(&tool);
        assert!(t.starts_with("SW1#show version\nCisco IOS Software"), "{t}");
    }

    #[test]
    fn a_switch_s_files_in_two_cases_make_one_session() {
        let d = Scratch::new("case");
        std::fs::write(
            d.0.join("a.log"),
            format!("sw1#show version\n{VERSION_2960X}sw1#show inventory\nNAME: \"1\", DESCR: \"x\"\nPID: WS-C2960X-24PS-L  , VID: V04\n"),
        )
        .unwrap();
        std::fs::write(
            d.0.join("b.log"),
            "SW1#show interfaces trunk\nPort        Mode\nSW1#show running-config\n!\nversion 15.2\nhostname SW1\nend\n",
        )
        .unwrap();
        let caps = from_folder(&d.0).unwrap();
        assert_eq!(caps.len(), 1);
        let (host, c) = &caps[0];
        let c = c.as_ref().unwrap();
        // named as its files are written
        assert_eq!(host, &c.host);
        assert!(
            c.log.contains("#show inventory\n") && c.log.contains("#show interfaces trunk\n"),
            "{}",
            c.log
        );
    }

    #[test]
    fn a_session_without_a_configuration_adds_its_commands_to_a_kept_log_as_it_is() {
        let d = Scratch::new("bare");
        let dir = d.0.join("c2960x-15.2(7)E");
        std::fs::create_dir_all(&dir).unwrap();
        let kept = dir.join("SW1-running-config.txt");
        std::fs::write(&kept, "!\nversion 15.2\nhostname SW1\nend\n").unwrap();
        // a log another tool kept in Latin-1
        let old: &[u8] = b"SW1#show interfaces status\nGi1/0/1   Caf\xe9-AP   connected\n";
        std::fs::write(dir.join("SW1-show.log"), old).unwrap();
        let c = from_transcript(&format!(
            "SW1#show version\n{VERSION_2960X}SW1#show cdp neighbors detail\nneighbour\n"
        ))
        .unwrap();
        assert!(c.running.is_empty());
        let (added, differs) = merge(&kept, &c).unwrap();
        assert_eq!(added, ["show version", "show cdp neighbors detail"]);
        assert!(!differs);
        let log = std::fs::read(dir.join("SW1-show.log")).unwrap();
        assert!(log.starts_with(old));
        assert!(
            String::from_utf8_lossy(&log).ends_with("SW1#show cdp neighbors detail\nneighbour\n")
        );
    }

    #[test]
    fn a_value_swept_from_one_place_goes_from_every_other() {
        // every secret here is invented for the test
        let t = "SW9#show version\nCisco IOS XE Software, Version 17.15.01\nSW9#show running-config | include key|tacacs\n key 7 0822455D0A16INVENTED\nSW9#show vtp ?\n  password   VTP password\n  status     VTP domain status\n\nSW9#show running-config\n!\nversion 17.15\nhostname SW9\ntacacs server T1\n key 7 0822455D0A16INVENTED\nend\nSW9#exit\n";
        let c = from_transcript(t).unwrap();
        assert!(!c.running.contains("INVENTED"), "{}", c.running);
        // a filtered show prints the key without the block that makes it one
        assert!(!c.log.contains("INVENTED"), "{}", c.log);
        assert!(c.log.contains("\n key 7 <removed>\n"), "{}", c.log);
        assert!(
            c.swept.contains("value swept from another line"),
            "{}",
            c.swept
        );
        // a `?` answer names no value
        assert!(c.log.contains("\n  password   VTP password\n"), "{}", c.log);
    }

    #[test]
    fn a_utf16_file_reads_as_its_text() {
        let text = "hostname a\n";
        let mut le = vec![0xff, 0xfe];
        le.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let mut be = vec![0xfe, 0xff];
        be.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(text_of(&le), text);
        assert_eq!(text_of(&be), text);
        assert_eq!(text_of(text.as_bytes()), text);
    }

    #[test]
    fn a_capture_goes_into_the_one_kept_at_its_own_release() {
        let d = Scratch::new("releases");
        let out = d.0.join("live");
        let kept = |folder: &str| {
            let dir = out.join(folder);
            std::fs::create_dir_all(&dir).unwrap();
            let k = dir.join("SW1-running-config.txt");
            std::fs::write(&k, "!\nversion 15.2\nhostname SW1\nend\n").unwrap();
            k
        };
        let (old, new) = (kept("c2960x-15.2(2)E"), kept("c2960x-15.2(7)E"));
        let all = captured(&out, "sw1");
        assert_eq!(all, [old.clone(), new.clone()]);
        let c = from_transcript(&format!(
            "SW1#show version\n{VERSION_2960X}SW1#show running-config\n!\nversion 15.2\nhostname SW1\nend\n"
        ))
        .unwrap();
        // 15.2(7)E0a: the capture at 15.2(7)E, whichever is found first
        assert_eq!(kept_at(&c, &all, &out, None), Ok(Some(new.clone())));
        // captured only at another release: a capture of its own
        assert_eq!(
            kept_at(&c, std::slice::from_ref(&old), &out, None),
            Ok(None)
        );
        // no hardware named: the capture at the train the session names, never another's
        let bare = from_transcript(
            "SW1#show version\nCisco IOS Software, C2960X Software (C2960X-UNIVERSALK9-M), Version 15.2(7)E8, RELEASE SOFTWARE (fc1)\nSW1#show vlan brief\nx\n",
        )
        .unwrap();
        assert_eq!(kept_at(&bare, &all, &out, None), Ok(Some(new)));
        assert_eq!(
            kept_at(&bare, std::slice::from_ref(&old), &out, None),
            Ok(None)
        );
        // an IOS XE configuration's version line names its train; nothing named, no match
        let xe = from_transcript("SW1#show running-config\n!\nversion 17.15\nhostname SW1\nend\n")
            .unwrap();
        assert_eq!(kept_at(&xe, &all, &out, None), Ok(None));
        let none = from_transcript("SW1#show vlan brief\nx\n").unwrap();
        assert!(kept_at(&none, &all, &out, None).is_err());
    }
}
