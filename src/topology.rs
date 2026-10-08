//! The plant as its captures show it (`vswitch topology`): a cable wherever a connected port's
//! description names another captured switch and that switch has a port whose description names
//! this one, and every switch a cable reaches. CDP and LLDP are off on the plant's switches; the
//! descriptions are what names the far end. Written as a lab file `vswitch lab` brings up and checks.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use crate::{capture, platform, show};

/// A captured switch: where its files are, the train its folder names and the hardware its session
/// names, with each front-panel port's description, link state and transceiver.
#[derive(Clone)]
pub struct Captured {
    pub host: String,
    pub config: PathBuf,
    pub release: String,
    pub stack: Option<String>,
    pub ports: Vec<Port>,
}

#[derive(Clone)]
pub struct Port {
    /// the short name `show interfaces status` prints (`Gi1/0/5`)
    pub name: String,
    /// the name its `interface` line gives (`GigabitEthernet1/0/5`)
    pub interface: String,
    pub description: String,
    /// `connected` in `show interfaces status`; None where the session has no such table
    pub connected: Option<bool>,
    /// the transceiver in the port: `show inventory`'s part where vswitch knows it, else the part
    /// the status table's Type names, else `show inventory`'s (`GLC-LH-SMD`)
    pub part: Option<String>,
    /// the side its `media-type` line fixes a dual-purpose port to (`sfp`, `rj45`)
    pub media: Option<String>,
}

/// Every capture under `live` (`<book>-<train>/<host>-running-config.txt` with its `-show.log`).
pub fn read(live: &Path) -> Result<Vec<Captured>, String> {
    let mut out = Vec::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(live)
        .map_err(|e| format!("{}: {e}", live.display()))?
        .flatten()
        .map(|d| d.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        let name = dir
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().to_string());
        let Some((_, release)) = name.split_once('-') else {
            continue;
        };
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .flatten()
            .map(|f| f.path())
            .collect();
        files.sort();
        for config in files {
            let file = config
                .file_name()
                .map_or(String::new(), |n| n.to_string_lossy().to_string());
            let Some(host) = file.strip_suffix("-running-config.txt") else {
                continue;
            };
            let text = std::fs::read_to_string(&config).map_err(|e| format!("{file}: {e}"))?;
            let log =
                std::fs::read_to_string(dir.join(format!("{host}-show.log"))).unwrap_or_default();
            let mut c = captured(host, config.clone(), release, &text, &log);
            // members a session does not name (the cores have no inventory): `<host>-stack.txt`
            if let Ok(spec) = std::fs::read_to_string(dir.join(format!("{host}-stack.txt"))) {
                c.stack = Some(spec.trim().to_string());
            }
            out.push(c);
        }
    }
    Ok(out)
}

/// The plant now: each switch at its newest capture where it was captured at several releases
/// (before and after an upgrade), names compared in any case; and a line naming each capture left
/// out.
pub fn current(caps: Vec<Captured>) -> (Vec<Captured>, Vec<String>) {
    let release = |c: &Captured| crate::release::Release::parse(&c.release);
    let mut keep: Vec<Captured> = Vec::new();
    let mut left = Vec::new();
    for c in caps {
        match keep
            .iter_mut()
            .find(|k| k.host.eq_ignore_ascii_case(&c.host))
        {
            Some(k) => {
                let old = if release(&c) > release(k) {
                    std::mem::replace(k, c)
                } else {
                    c
                };
                left.push(format!(
                    "{} at {} left out: the newest capture, at {}, stands for it",
                    old.host, old.release, k.release
                ));
            }
            None => keep.push(c),
        }
    }
    (keep, left)
}

/// A captured switch as a configuration of another moment describes its ports (a backup tool's
/// file): link state and transceivers still from its session.
pub fn reconfigured(c: &Captured, text: &str) -> Captured {
    let log = c
        .config
        .parent()
        .and_then(|d| std::fs::read_to_string(d.join(format!("{}-show.log", c.host))).ok())
        .unwrap_or_default();
    let mut n = captured(&c.host, c.config.clone(), &c.release, text, &log);
    n.stack = c.stack.clone();
    n
}

fn captured(host: &str, config: PathBuf, release: &str, text: &str, log: &str) -> Captured {
    let ex = capture::exchanges(log).map(|(_, e)| e).unwrap_or_default();
    // a session typed by hand abbreviates its commands (`sh inv`)
    let answer = |c: &str| {
        ex.iter()
            .rev()
            .find(|e| capture::is_command(&e.command, c) && !e.output.is_empty())
            .map(|e| e.output.clone())
    };
    let stack = match answer("show inventory").map(|o| capture::stack_of(&o)) {
        Some((Some(s), _)) => Some(s),
        _ => answer("show version").and_then(|o| capture::stack_of_version(&o).0),
    };
    let table = answer("show interfaces status");
    let status = table.as_deref().map(|o| status_of(o, "Status"));
    // a transceiver's part: the inventory's where vswitch knows it, else the one the Type the status
    // table prints names, else the inventory's (GLC-LH-SM, which the lab does not run)
    let inventory = answer("show inventory")
        .map(|o| parts_of(&o))
        .unwrap_or_default();
    let mut parts: BTreeMap<String, String> = inventory
        .iter()
        .filter(|(_, p)| platform::optic(p).is_some())
        .map(|(port, p)| (port.clone(), p.clone()))
        .collect();
    for (port, ty) in table
        .as_deref()
        .map(|o| status_of(o, "Type"))
        .unwrap_or_default()
    {
        if let Some(o) = platform::OPTICS
            .iter()
            .find(|o| o.shows == ty || (!ty.contains("BaseTX") && o.shows == format!("{ty} SFP")))
        {
            parts.entry(port).or_insert_with(|| o.part.to_string());
        }
    }
    for (port, p) in inventory {
        parts.entry(port).or_insert(p);
    }
    let mut ports = Vec::new();
    // a port's name, description and media-type line
    let mut current: Option<(String, String, Option<String>)> = None;
    let push = |c: Option<(String, String, Option<String>)>, ports: &mut Vec<Port>| {
        if let Some((name, description, media)) = c
            && !description.is_empty()
        {
            let short = show::short(&name);
            ports.push(Port {
                connected: status
                    .as_ref()
                    .map(|s| s.get(&short).is_some_and(|v| v == "connected")),
                part: parts.get(&short).cloned(),
                name: short,
                interface: name,
                description,
                media,
            });
        }
    };
    // a banner's text is no configuration line
    let mut banner: Option<String> = None;
    for line in text.lines() {
        if let Some(d) = &banner {
            if line.contains(d.as_str()) {
                banner = None;
            }
            continue;
        }
        if let Some(name) = line.strip_prefix("interface ") {
            push(current.take(), &mut ports);
            // the front panel only: a port-channel's members carry its cables
            let name = name.trim();
            if platform::locate(name).is_some() {
                current = Some((name.to_string(), String::new(), None));
            }
        } else if let Some(d) = line.strip_prefix(" description ") {
            if let Some(c) = current.as_mut() {
                c.1 = d.trim().to_string();
            }
        } else if let Some(m) = line.strip_prefix(" media-type ") {
            if let Some(c) = current.as_mut() {
                c.2 = Some(m.trim().to_string());
            }
        } else if !line.starts_with(' ') {
            push(current.take(), &mut ports);
            banner = capture::banner_opens(line);
        }
    }
    push(current.take(), &mut ports);
    Captured {
        host: host.to_string(),
        config,
        release: release.to_string(),
        stack,
        ports,
    }
}

/// Each port's `column` in `show interfaces status`, read at the header's own column (a Name may hold
/// spaces): Status as its first word, Type as the rest of the row.
fn status_of(table: &[String], column: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(header) = table.iter().find(|l| l.starts_with("Port ")) else {
        return out;
    };
    let Some(at) = header.find(column) else {
        return out;
    };
    for row in table {
        let Some(port) = row.split_whitespace().next() else {
            continue;
        };
        if row.len() > at && row.is_char_boundary(at) && port != "Port" {
            let rest = &row[at..];
            let value = if column == "Type" {
                rest.trim()
            } else {
                rest.split_whitespace().next().unwrap_or("")
            };
            out.insert(port.to_string(), value.to_string());
        }
    }
    out
}

/// The part number `show inventory` gives each port's transceiver (`NAME: "Gi1/0/5"` then `PID:`).
fn parts_of(inventory: &[String]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut name: Option<String> = None;
    for l in inventory {
        let l = l.trim();
        if let Some(rest) = l.strip_prefix("NAME: \"") {
            name = rest.split('"').next().map(show::short);
        } else if let (Some(n), Some(rest)) = (name.take(), l.strip_prefix("PID: ")) {
            let pid = rest.split(',').next().unwrap_or("").trim();
            if !pid.is_empty() && platform::locate(&n).is_some() {
                out.insert(n, pid.to_string());
            }
        }
    }
    out
}

/// The captured switch a description names, and whether it names it whole: a whole hostname, or
/// the end of one that shares this switch's site prefix (c9300 17.15 capture 5's `ES1A (AREA1 RACK)` names
/// c9200 17.15 capture 24).
pub fn named(text: &str, me: &str, hosts: &[&str]) -> Option<(String, bool)> {
    let words: Vec<&str> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    let lower = me.to_lowercase();
    for w in &words {
        if let Some(h) = hosts
            .iter()
            .find(|h| h.eq_ignore_ascii_case(w) && !h.eq_ignore_ascii_case(me))
        {
            return Some((h.to_string(), true));
        }
    }
    for w in words.iter().filter(|w| w.len() >= 3) {
        let w = w.to_lowercase();
        for h in hosts {
            let hl = h.to_lowercase();
            if hl != lower
                && hl.len() > w.len()
                && hl.ends_with(&w)
                && lower.starts_with(&hl[..hl.len() - w.len()])
            {
                return Some((h.to_string(), false));
            }
        }
    }
    None
}

/// A cable both ends name, and what the matching could not settle.
pub struct Linked {
    pub cables: Vec<(String, String, Option<String>)>,
    pub report: Vec<String>,
    /// the cables a port at either end of takes a transceiver on that no capture names: the lab
    /// brings each up once the lab file gives it an optic
    pub unnamed: BTreeSet<(String, String)>,
}

/// Whether a port runs its cable on a transceiver: an SFP or uplink port, a dual-purpose one only
/// where `media-type sfp` fixes it to its SFP side.
fn takes_optic(stack: Option<&platform::Stack>, p: &Port) -> bool {
    stack.is_some_and(|s| {
        s.slot_speeds(&p.interface).is_some()
            && (!s.dual_purpose(&p.interface) || p.media.as_deref() == Some("sfp"))
    })
}

/// One end's ports naming the other, where it names more than the other names back: a port naming
/// the far switch by its site suffix alone gives way to one naming it whole (`EU1 Printer` beside
/// an uplink). The ports kept, and those left.
fn trim<'p>(mine: &[(&'p Port, bool)], other: usize) -> (Vec<&'p Port>, Vec<&'p Port>) {
    let mut extra = mine.len().saturating_sub(other);
    let (mut kept, mut left) = (vec![], vec![]);
    for &(p, whole) in mine {
        if extra > 0 && !whole {
            extra -= 1;
            left.push(p);
        } else {
            kept.push(p);
        }
    }
    (kept, left)
}

/// Each pair of switches whose descriptions name each other, their ports matched in port order, a
/// whole name before a site suffix where one end names more; a port down at either end is no cable.
pub fn link(caps: &[Captured]) -> Linked {
    let hosts: Vec<&str> = caps.iter().map(|c| c.host.as_str()).collect();
    let stacks: BTreeMap<&str, Option<platform::Stack>> = caps
        .iter()
        .map(|c| {
            let st = c
                .stack
                .as_deref()
                .and_then(|s| platform::parse_stack(s).ok());
            (c.host.as_str(), st)
        })
        .collect();
    // (from host, to host) -> ports of `from` naming `to`, connected or of unknown state, each with
    // whether it names `to` whole; and the ports naming it that are down
    let mut naming: BTreeMap<(String, String), Vec<(&Port, bool)>> = BTreeMap::new();
    let mut down: BTreeMap<(String, String), Vec<&Port>> = BTreeMap::new();
    let mut report = Vec::new();
    for c in caps {
        for p in &c.ports {
            let Some((far, whole)) = named(&p.description, &c.host, &hosts) else {
                continue;
            };
            if p.connected == Some(false) {
                down.entry((c.host.clone(), far)).or_default().push(p);
                continue;
            }
            naming
                .entry((c.host.clone(), far))
                .or_default()
                .push((p, whole));
        }
    }
    let names = |ports: &[&Port]| -> String {
        ports
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut cables = Vec::new();
    let mut unnamed = BTreeSet::new();
    for ((a, b), ours) in &naming {
        if a > b && naming.contains_key(&(b.clone(), a.clone())) {
            continue;
        }
        let ports: Vec<&Port> = ours.iter().map(|(p, _)| *p).collect();
        let Some(theirs) = naming.get(&(b.clone(), a.clone())) else {
            let back = match down.get(&(b.clone(), a.clone())) {
                Some(d) => format!("whose {} naming {a} is down", names(d)),
                None => format!("which names no port back to {a}"),
            };
            report.push(format!("{a} {} names {b}, {back}", names(&ports)));
            continue;
        };
        let (mine, my_left) = trim(ours, theirs.len());
        let (yours, your_left) = trim(theirs, ours.len());
        for (host, far, left) in [(a, b, &my_left), (b, a, &your_left)] {
            if !left.is_empty() {
                report.push(format!(
                    "{host} {} names {far} by its site suffix only, beside a port naming it whole; no cable",
                    names(left)
                ));
            }
        }
        for (x, y) in mine.iter().zip(yours.iter()) {
            let optic = [&x.part, &y.part]
                .into_iter()
                .flatten()
                .find(|pid| platform::optic(pid).is_some())
                .cloned();
            if let (Some(px), Some(py)) = (&x.part, &y.part)
                && px != py
            {
                report.push(format!(
                    "{a}:{} holds {px}, {b}:{} {py}; the lab runs {}",
                    x.name,
                    y.name,
                    optic.as_deref().unwrap_or("copper")
                ));
            }
            let ends = (format!("{a}:{}", x.name), format!("{b}:{}", y.name));
            let optical: Vec<&str> = [(a, x, &ends.0), (b, y, &ends.1)]
                .into_iter()
                .filter(|(h, p, _)| takes_optic(stacks[h.as_str()].as_ref(), p))
                .map(|(_, _, e)| e.as_str())
                .collect();
            if optic.is_none() && !optical.is_empty() {
                let what = if optical.len() > 1 {
                    "transceivers"
                } else {
                    "transceiver"
                };
                report.push(format!(
                    "{} - {}: no capture names the {what} at {} (no show inventory, no Type in \
                     show interfaces status); the lab file gives the cable no optic",
                    ends.0,
                    ends.1,
                    optical.join(" and ")
                ));
                unnamed.insert(ends.clone());
            }
            cables.push((ends.0, ends.1, optic));
        }
        if mine.len() != yours.len() {
            report.push(format!(
                "{a} has {} port(s) naming {b}, {b} {} naming {a}; matched {} in port order",
                mine.len(),
                yours.len(),
                mine.len().min(yours.len())
            ));
        }
    }
    Linked {
        cables,
        report,
        unnamed,
    }
}

/// A TOML basic string holding `s` whatever it holds (a folder named `o'live`).
fn quoted(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The lab file: each cabled switch at its own train and hardware, files relative to `lab_dir`,
/// then the cables.
pub fn lab_file(caps: &[Captured], linked: &Linked, lab_dir: &Path) -> String {
    let mut out = String::from(
        "# The plant as its captures show it (vswitch topology): a cable where both ends' descriptions\n# name each other. Check with: vswitch lab <this file>\n",
    );
    let cabled = |h: &str| {
        linked
            .cables
            .iter()
            .any(|(a, b, _)| a.starts_with(&format!("{h}:")) || b.starts_with(&format!("{h}:")))
    };
    for c in caps.iter().filter(|c| cabled(&c.host)) {
        out.push_str(&format!(
            "\n[[switch]]\nname = {}\nfiles = [{}]\nrelease = {}\n",
            quoted(&c.host),
            quoted(&relative(&c.config, lab_dir)),
            quoted(&c.release)
        ));
        if let Some(s) = &c.stack {
            out.push_str(&format!("stack = {}\n", quoted(s)));
        }
    }
    for (a, b, optic) in &linked.cables {
        out.push('\n');
        if linked.unnamed.contains(&(a.clone(), b.clone())) {
            out.push_str("# no capture names this cable's transceiver: give it an optic\n");
        }
        out.push_str(&format!(
            "[[cable]]\na = {}\nb = {}\n",
            quoted(a),
            quoted(b)
        ));
        if let Some(o) = optic {
            out.push_str(&format!("optic = {}\n", quoted(o)));
        }
    }
    out
}

/// A path with its `.` and `..` read off its own components, as Windows reads them and Unix leaves
/// them.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() && !out.has_root() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// `path` from `dir`, with forward slashes (`../data/live/...`).
fn relative(path: &Path, dir: &Path) -> String {
    let abs = |p: &Path| lexical(&std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()));
    let (p, d) = (abs(path), abs(dir));
    let pc: Vec<_> = p.components().collect();
    let dc: Vec<_> = d.components().collect();
    let common = pc.iter().zip(&dc).take_while(|(a, b)| a == b).count();
    // another drive: the whole path
    if common == 0 {
        return p.to_string_lossy().replace('\\', "/");
    }
    let mut parts: Vec<String> = vec!["..".into(); dc.len() - common];
    parts.extend(
        pc[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().to_string()),
    );
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_names_a_host_whole_or_by_its_site_suffix() {
        let hosts = [
            "plntL2arDS1A",
            "plntL2arES1A",
            "plntL2bkES1A",
            "plntPCNsw-5",
        ];
        let named = |text: &str, me: &str| named(text, me, &hosts);
        let at = |host: &str, whole: bool| Some((host.to_string(), whole));
        assert_eq!(
            named("ES1A (AREA1 RACK)", "plntL2arDS1A"),
            at("plntL2arES1A", false)
        );
        assert_eq!(
            named("PLNTL2ARDS1A", "plntL2arES1A"),
            at("plntL2arDS1A", true)
        );
        assert_eq!(
            named("plntPCNsw-5", "plntL2arES1A"),
            at("plntPCNsw-5", true)
        );
        // another site's ES1A is not this site's
        assert_eq!(
            named("ES1A (cluster)", "plntL2arDS1A"),
            at("plntL2arES1A", false)
        );
        assert_eq!(named("ES1A", "plntL2bkDS9Z"), at("plntL2bkES1A", false));
        assert_eq!(named("Crossover", "plntL2arDS1A"), None);
        assert_eq!(named("plntL2arDS1A", "plntL2arDS1A"), None);
    }

    #[test]
    fn a_cable_needs_both_ends_to_name_each_other() {
        let port = |name: &str, d: &str, up: Option<bool>, part: Option<&str>| Port {
            name: name.into(),
            interface: name.replace("Gi", "GigabitEthernet"),
            description: d.into(),
            connected: up,
            part: part.map(Into::into),
            media: None,
        };
        let cap = |host: &str, ports| Captured {
            host: host.into(),
            config: PathBuf::from(format!("{host}-running-config.txt")),
            release: "17.15".into(),
            stack: None,
            ports,
        };
        let caps = vec![
            cap(
                "siteDS1A",
                vec![
                    port("Gi1/0/5", "ES1A", Some(true), Some("GLC-LH-SMD")),
                    port("Gi1/0/6", "ES2A", Some(true), None),
                    port("Gi1/0/7", "ES3A", Some(false), None),
                ],
            ),
            cap(
                "siteES1A",
                vec![port("Gi1/1/1", "siteDS1A", None, Some("GLC-LH-SMD"))],
            ),
            cap(
                "siteES2A",
                vec![port("Gi1/1/1", "uplink", Some(true), None)],
            ),
            cap(
                "siteES3A",
                vec![port("Gi1/1/1", "siteDS1A", Some(true), None)],
            ),
        ];
        let l = link(&caps);
        assert_eq!(
            l.cables,
            [(
                "siteDS1A:Gi1/0/5".to_string(),
                "siteES1A:Gi1/1/1".to_string(),
                Some("GLC-LH-SMD".to_string())
            )]
        );
        // ES2A names nothing back; DS1A's port to ES3A is down
        assert!(
            l.report
                .iter()
                .any(|r| r.starts_with("siteDS1A Gi1/0/6 names siteES2A")),
            "{:?}",
            l.report
        );
        assert!(
            l.report
                .iter()
                .any(|r| r.starts_with("siteES3A Gi1/1/1 names siteDS1A")),
            "{:?}",
            l.report
        );
        let text = lab_file(&caps, &l, Path::new("."));
        assert!(
            text.contains("name = \"siteDS1A\"") && !text.contains("name = \"siteES2A\""),
            "{text}"
        );
    }

    #[cfg(all(test, captures))]
    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/topology-tests.rs"));

    fn port(name: &str, d: &str, up: Option<bool>) -> Port {
        Port {
            name: name.into(),
            interface: name.replace("Gi", "GigabitEthernet"),
            description: d.into(),
            connected: up,
            part: None,
            media: None,
        }
    }

    fn cap(host: &str, release: &str, stack: Option<&str>, ports: Vec<Port>) -> Captured {
        Captured {
            host: host.into(),
            config: PathBuf::from(format!("c2960x-{release}/{host}-running-config.txt")),
            release: release.into(),
            stack: stack.map(Into::into),
            ports,
        }
    }

    #[test]
    fn a_switch_captured_at_two_releases_is_one_switch_at_its_newest() {
        let caps = vec![
            cap(
                "siteA",
                "15.2(2)E",
                None,
                vec![port("Gi1/0/1", "siteB", Some(true))],
            ),
            cap(
                "siteA",
                "15.2(7)E",
                None,
                vec![port("Gi1/0/1", "siteB", Some(true))],
            ),
            cap(
                "siteB",
                "15.2(7)E",
                None,
                vec![
                    port("Gi1/0/1", "siteA", Some(true)),
                    port("Gi1/0/2", "siteA", Some(true)),
                ],
            ),
        ];
        let (now, left) = current(caps);
        assert_eq!(
            now.iter()
                .map(|c| (c.host.as_str(), c.release.as_str()))
                .collect::<Vec<_>>(),
            [("siteA", "15.2(7)E"), ("siteB", "15.2(7)E")]
        );
        assert_eq!(left.len(), 1, "{left:?}");
        let l = link(&now);
        assert_eq!(l.cables.len(), 1, "{:?}", l.cables);
        let text = lab_file(&now, &l, Path::new("."));
        assert_eq!(text.matches("name = \"siteA\"").count(), 1, "{text}");
    }

    #[test]
    fn a_whole_name_is_cabled_before_a_site_suffix_and_a_far_port_down_is_named() {
        // siteRCTL's printer port names siteEU1 by its suffix, its uplink by its whole name
        let caps = vec![
            cap(
                "siteRCTL",
                "15.2(7)E",
                None,
                vec![
                    port("Gi1/0/1", "EU1 Printer", Some(true)),
                    port("Gi1/0/2", "siteEU1", Some(true)),
                ],
            ),
            cap(
                "siteEU1",
                "15.2(7)E",
                None,
                vec![port("Gi1/0/1", "siteRCTL", Some(true))],
            ),
            cap(
                "siteA",
                "15.2(7)E",
                None,
                vec![port("Gi1/0/1", "siteB", Some(true))],
            ),
            cap(
                "siteB",
                "15.2(7)E",
                None,
                vec![port("Gi1/0/9", "siteA", Some(false))],
            ),
        ];
        let l = link(&caps);
        assert_eq!(
            l.cables,
            [(
                "siteEU1:Gi1/0/1".to_string(),
                "siteRCTL:Gi1/0/2".to_string(),
                None
            )]
        );
        assert!(
            l.report.contains(
                &"siteA Gi1/0/1 names siteB, whose Gi1/0/9 naming siteA is down".to_string()
            ),
            "{:?}",
            l.report
        );
    }

    #[test]
    fn a_cable_on_transceivers_no_capture_names_is_reported() {
        let mut a = port("Gi1/0/25", "siteB", None);
        let mut b = port("Gi1/0/26", "siteA", None);
        let caps = |a: &Port, b: &Port| {
            vec![
                cap(
                    "siteA",
                    "15.2(7)E",
                    Some("WS-C2960X-24PS-L"),
                    vec![a.clone()],
                ),
                cap(
                    "siteB",
                    "15.2(7)E",
                    Some("WS-C2960X-24PS-L"),
                    vec![b.clone()],
                ),
            ]
        };
        let l = link(&caps(&a, &b));
        assert_eq!(l.cables.len(), 1);
        assert!(
            l.unnamed
                .contains(&("siteA:Gi1/0/25".to_string(), "siteB:Gi1/0/26".to_string())),
            "{:?}",
            l.report
        );
        // a transceiver named at either end, or copper ports, leave nothing to name
        b.part = Some("GLC-LH-SMD".into());
        assert!(link(&caps(&a, &b)).unnamed.is_empty());
        (a.name, a.interface) = ("Gi1/0/5".into(), "GigabitEthernet1/0/5".into());
        (b.name, b.interface, b.part) = ("Gi1/0/6".into(), "GigabitEthernet1/0/6".into(), None);
        assert!(link(&caps(&a, &b)).unnamed.is_empty());
    }

    #[test]
    fn a_known_transceiver_from_the_status_table_stands_before_an_unknown_inventory_part() {
        let log = "sw#show inventory\nNAME: \"GigabitEthernet1/0/26\", DESCR: \"1000BaseLX SFP\"\nPID: GLC-LH-SM          , VID: V02\nsw#show interfaces status\n\nPort      Name               Status       Vlan       Duplex  Speed Type\nGi1/0/26  far                connected    trunk        full   1000 1000BaseLX SFP\n";
        let config = "interface GigabitEthernet1/0/26\n description far\n media-type sfp\n";
        let c = captured("sw", PathBuf::from("x"), "15.2(7)E", config, log);
        assert_eq!(c.ports[0].part.as_deref(), Some("GLC-LH-SMD"));
        assert_eq!(c.ports[0].media.as_deref(), Some("sfp"));
    }

    #[test]
    fn the_lab_file_reads_back_whatever_the_paths_hold() {
        let caps = vec![
            Captured {
                config: PathBuf::from("o'live/c9300-17.15/a-running-config.txt"),
                ..cap("a", "17.15", None, vec![port("Gi1/0/1", "b", Some(true))])
            },
            cap("b", "17.15", None, vec![port("Gi1/0/1", "a", Some(true))]),
        ];
        let l = link(&caps);
        let text = lab_file(&caps, &l, Path::new("."));
        let v: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(
            v["switch"][0]["files"][0].as_str(),
            Some("o'live/c9300-17.15/a-running-config.txt")
        );
        // a folder out of `..` names the same files as the folder it reads as
        assert_eq!(
            relative(Path::new("/w/data/live/x"), Path::new("/w/lab/..")),
            "data/live/x"
        );
    }
}
