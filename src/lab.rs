//! Switches cabled together (`vswitch lab`): each switch built from its files, a design or a
//! configuration typed as a console boots it, and each cable checked from both ends as the two
//! switches bring the link up. A cabled port then shows its link in the show commands.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;

use crate::cli::Engine;
use crate::device::{Device, Link};
use crate::platform::{self, Optic, Stack};
use crate::release::Release;
use crate::show::short;
use crate::{check, generate, grammars, release, shell};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lab {
    pub switch: Vec<SwitchSpec>,
    #[serde(default)]
    pub cable: Vec<Cable>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchSpec {
    pub name: String,
    /// designs (`.toml`) and configurations, typed in this order; paths relative to the lab file
    pub files: Vec<String>,
    /// members as `--stack` takes them; the first design's when absent
    #[serde(default)]
    pub stack: Option<String>,
    /// the first design's release when absent, else a configuration's version line, else 17.15
    #[serde(default)]
    pub release: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cable {
    /// `switch:port`; a name alone is a device the lab does not model (a server, a firewall)
    pub a: String,
    pub b: String,
    /// the transceiver part number at both ends (`SFP-10G-SR`); none on copper
    #[serde(default)]
    pub optic: Option<String>,
    /// the speed a copper link negotiates when the far end runs slower (Mb/s)
    #[serde(default)]
    pub speed: Option<u32>,
    /// a far end that negotiates half duplex (a device on an old hub, `a-half a-100`)
    #[serde(default)]
    pub half: bool,
}

/// A switch ready to build: its hardware, release and grammar, and the text of each file.
pub struct Switch {
    pub name: String,
    pub stack_spec: String,
    pub stack: Stack,
    pub release: Release,
    /// shared by every switch at the same book and release (`Engines`)
    pub engine: Arc<Engine>,
    pub files: Vec<(String, String)>,
    /// VLANs its VLAN database already holds (a design's `existing_vlans`)
    pub existing: BTreeSet<u32>,
}

/// The engines a lab has loaded, by book and release: a grammar loads once per lab.
pub type Engines = BTreeMap<(&'static str, Release), Arc<Engine>>;

/// A switch built: its session after every file was typed, and the lines it refused.
pub struct Built<'a> {
    pub name: String,
    pub session: shell::Session<'a>,
    pub typed: usize,
    pub refused: Vec<(String, check::Typed)>,
}

/// Each cable's state, and what the two ends disagree on: findings break the link or the traffic
/// on it, notes are worth a look.
#[derive(Debug, Default)]
pub struct Report {
    pub links: Vec<String>,
    pub findings: Vec<String>,
    pub notes: Vec<String>,
}

impl Report {
    fn finding(&mut self, f: String) {
        if !self.findings.contains(&f) {
            self.findings.push(f);
        }
    }
    fn note(&mut self, n: String) {
        if !self.notes.contains(&n) {
            self.notes.push(n);
        }
    }
}

/// Reads a lab file and readies each switch: designs are checked and rendered, the release and
/// hardware resolved and checked together, as `generate` and `lint` do for one switch.
pub fn prepare(path: &Path) -> Result<(Vec<Switch>, Vec<Cable>), String> {
    let shown = path.display();
    let text = std::fs::read_to_string(path).map_err(|e| format!("{shown}: {e}"))?;
    let lab: Lab = toml::from_str(&text).map_err(|e| format!("{shown}: {e}"))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut out = Vec::new();
    let mut engines = Engines::new();
    for sw in &lab.switch {
        if out.iter().any(|s: &Switch| s.name == sw.name) {
            return Err(format!("{shown}: two switches named {}", sw.name));
        }
        let mut bodies = Vec::new();
        for f in &sw.files {
            let p = dir.join(f);
            let body = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            bodies.push((f.clone(), body));
        }
        out.push(ready(sw, bodies, &mut engines)?);
    }
    Ok((out, lab.cable))
}

/// Readies one switch from its files' text, named as the lab file names them.
pub fn ready(
    sw: &SwitchSpec,
    bodies: Vec<(String, String)>,
    engines: &mut Engines,
) -> Result<Switch, String> {
    let mut files = Vec::new();
    let mut designs = Vec::new();
    let mut own_release: Option<String> = None;
    let mut own_stack: Option<String> = None;
    let mut existing = BTreeSet::new();
    for (f, body) in &bodies {
        let body = body.trim_start_matches('\u{feff}').to_string();
        if f.ends_with(".toml") {
            let spec: generate::Spec = toml::from_str(&body).map_err(|e| format!("{f}: {e}"))?;
            let problems = generate::check(&spec);
            if !problems.is_empty() {
                return Err(format!("{f}: {}", problems.join("; ")));
            }
            own_release = own_release.or(spec.release.clone());
            own_stack = own_stack.or(Some(spec.stack.join(",")));
            existing.extend(
                spec.existing_vlans
                    .as_deref()
                    .map(generate::vlan_ids)
                    .unwrap_or_default(),
            );
            files.push((f.clone(), generate::render(&spec).join("\n")));
            designs.push(spec);
        } else {
            own_release = own_release.or(release::version_line(&body).map(str::to_string));
            own_stack = own_stack.or_else(|| platform::infer_stack(&body));
            files.push((f.clone(), body));
        }
    }
    let spec = sw.stack.clone().or(own_stack).ok_or(format!(
        "{}: give its hardware with stack = \"...\"; its files name none",
        sw.name
    ))?;
    let family = platform::parse_stack(&spec)
        .map_err(|e| format!("{}: {e}", sw.name))?
        .family();
    let book = grammars::book(family);
    let flag = sw
        .release
        .as_deref()
        .map(|t| grammars::resolve(t, book))
        .transpose()?;
    let own = own_release
        .as_deref()
        .map(|t| grammars::resolve(t, book))
        .transpose()?;
    let r = match (flag, own) {
        (Some(f), Some(o)) if !f.agrees(o, own_release.as_deref().unwrap_or("")) => {
            return Err(format!(
                "{}: release {f} disagrees with {}, which its files name",
                sw.name,
                o.named()
            ));
        }
        (Some(r), _) | (None, Some(r)) => r,
        // the newest release the book documents that the hardware runs
        (None, None) => grammars::fitting_release(
            book,
            &platform::parse_stack(&spec).map_err(|e| format!("{}: {e}", sw.name))?,
        ),
    };
    let stack = platform::checked_stack(&spec, r).map_err(|e| format!("{}: {e}", sw.name))?;
    for d in &designs {
        let inactive = generate::inactive_ports(d, &stack);
        if !inactive.is_empty() {
            return Err(format!("{}: {}", sw.name, inactive.join("; ")));
        }
    }
    Ok(Switch {
        name: sw.name.clone(),
        stack_spec: spec,
        stack,
        release: r,
        engine: engine(engines, book, r)?,
        files,
        existing,
    })
}

/// The engine of a book at a release, loaded once into `engines`.
pub fn engine(
    engines: &mut Engines,
    book: &'static str,
    r: Release,
) -> Result<Arc<Engine>, String> {
    if let Some(e) = engines.get(&(book, r)) {
        return Ok(e.clone());
    }
    let e = Arc::new(Engine::new(grammars::grammar(book, r)?));
    engines.insert((book, r), e.clone());
    Ok(e)
}

/// Types a switch's files into one session, then gives it the VLANs its database already holds.
pub fn build(sw: &Switch) -> Built<'_> {
    let mut s = shell::Session::new(&sw.engine, Device::new(sw.stack.clone()));
    let mut typed = 0;
    let mut refused = Vec::new();
    for (file, text) in &sw.files {
        for t in check::type_into(&mut s, text) {
            typed += 1;
            if t.error.is_some() {
                refused.push((file.clone(), t));
            }
        }
    }
    for id in &sw.existing {
        s.device.block(&format!("vlan {id}"));
    }
    s.type_line("end");
    Built {
        name: sw.name.clone(),
        session: s,
        typed,
        refused,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Access,
    Trunk,
    Desirable,
    Auto,
    /// `dot1q-tunnel`: a QinQ port, which never trunks and carries whatever the far end sends
    Tunnel,
    /// `private-vlan host` or `promiscuous`, which never trunks and takes its VLANs by association
    Private,
}

/// One end of a cable on a switch the lab models, as its configuration sets it up.
struct End {
    sw: usize,
    port: String,
    label: String,
    routed: bool,
    mode: Mode,
    nonegotiate: bool,
    channel: Option<(u32, String)>,
}

/// One port's view of its configuration: its own lines, then its port-channel's for the lines a
/// bundle runs (switchport mode, VLANs, spanning tree).
fn lines<'d>(d: &'d Device, port: &str, channel: Option<u32>) -> Vec<&'d str> {
    let mut v = d.config_of(port);
    if let Some(n) = channel {
        v.extend(d.config_of(&format!("Port-channel{n}")));
    }
    v
}

fn value<'a>(lines: &[&'a str], prefix: &str) -> Option<&'a str> {
    lines.iter().find_map(|l| l.strip_prefix(prefix))
}

fn end(built: &[Built], names: &[&str], text: &str) -> Result<Option<End>, String> {
    let Some((sw, port)) = text.split_once(':') else {
        return Ok(None);
    };
    let i = names
        .iter()
        .position(|n| *n == sw.trim())
        .ok_or(format!("cable end {text}: no switch named {sw}"))?;
    let d = &built[i].session.device;
    let full = crate::cli::interface_name(port.trim(), None)
        .map(|(ty, num, _)| format!("{ty}{num}"))
        .filter(|f| d.stack.physical().contains(f))
        .ok_or(format!(
            "cable end {text}: {sw} has no port {}",
            port.trim()
        ))?;
    let c = d.config_of(&full);
    let channel = value(&c, "channel-group ").and_then(|v| {
        let mut w = v.split_whitespace();
        let n = w.next()?.parse().ok()?;
        (w.next() == Some("mode")).then(|| (n, w.next().unwrap_or("on").to_string()))
    });
    // a member runs its bundle's switchport lines where it sets none of its own, as IOS applies
    // them to each member
    let l = lines(d, &full, channel.as_ref().map(|c| c.0));
    let mode = match value(&l, "switchport mode ") {
        // the default mode is dynamic auto (9300 command reference, switchport mode)
        None | Some("dynamic auto") => Mode::Auto,
        Some("dynamic desirable") => Mode::Desirable,
        Some("trunk") => Mode::Trunk,
        Some(m) if m.starts_with("private-vlan trunk") => Mode::Trunk,
        Some("dot1q-tunnel") => Mode::Tunnel,
        Some(m) if m.starts_with("private-vlan ") => Mode::Private,
        // access, and any other mode, which sends no DTP and never trunks
        Some(_) => Mode::Access,
    };
    Ok(Some(End {
        sw: i,
        label: format!("{} {}", names[i], short(&full)),
        routed: l.contains(&"no switchport"),
        nonegotiate: l.contains(&"switchport nonegotiate"),
        port: full,
        mode,
        channel,
    }))
}

/// Whether a port trunks once DTP has run: trunk mode always, dynamic modes when the far end
/// negotiates one (a device the lab does not model, a `nonegotiate` port and a routed port send no
/// DTP; a tunnel or private VLAN port never trunks).
fn trunks(x: Mode, far: Option<(Mode, bool)>) -> bool {
    match (x, far) {
        (Mode::Access | Mode::Tunnel | Mode::Private, _) => false,
        (Mode::Trunk, _) => true,
        (_, None | Some((_, true))) => false,
        (_, Some((Mode::Trunk | Mode::Desirable, _))) => true,
        (_, Some((Mode::Auto, _))) => x == Mode::Desirable,
        (_, Some((Mode::Access | Mode::Tunnel | Mode::Private, _))) => false,
    }
}

fn protocol(mode: &str) -> &'static str {
    match mode {
        "active" | "passive" => "LACP",
        "desirable" | "auto" => "PAgP",
        _ => "on",
    }
}

/// Whether two channel modes form a bundle: `on` with `on`, LACP with one end active, PAgP with one
/// end desirable (9300 configuration guide, EtherChannel modes).
fn bundles(x: &str, y: &str) -> bool {
    match (protocol(x), protocol(y)) {
        ("on", "on") => true,
        ("LACP", "LACP") => x == "active" || y == "active",
        ("PAgP", "PAgP") => x == "desirable" || y == "desirable",
        _ => false,
    }
}

/// The transceiver speeds an end's port runs on this cable, or None where its RJ-45 side carries
/// it: a dual-purpose uplink takes a cable naming an optic in its SFP side and copper in its RJ-45
/// side, unless `media-type rj45` or `media-type sfp` fixes the side.
fn slot(d: &Device, e: &End, part: Option<&str>) -> Option<Vec<u32>> {
    let speeds = d.stack.slot_speeds(&e.port)?;
    if !d.stack.dual_purpose(&e.port) {
        return Some(speeds);
    }
    match value(&d.config_of(&e.port), "media-type ") {
        Some("rj45") => None,
        Some("sfp") => Some(speeds),
        _ => part.map(|_| speeds),
    }
}

/// The speeds (Mb/s) an RJ-45 port negotiates: Fast Ethernet 10 and 100, Gigabit 10 to 1000, a
/// Multigigabit port 100 to its own (9300 data sheet: 100M, 1G, 2.5G, 5G and 10G).
fn copper_speeds(port: &str) -> &'static [u32] {
    match port.split(|c: char| c.is_ascii_digit()).next() {
        Some("FastEthernet") => &[10, 100],
        Some("TwoGigabitEthernet") => &[100, 1000, 2500],
        Some("FiveGigabitEthernet") => &[100, 1000, 2500, 5000],
        Some("TenGigabitEthernet") => &[100, 1000, 2500, 5000, 10_000],
        _ => &[10, 100, 1000],
    }
}

fn speeds_text(speeds: &[u32]) -> String {
    speeds
        .iter()
        .map(|s| platform::speed_text(*s))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The physical link: whether both ends run, and run the same medium at one speed. `slot` is what
/// this end's port takes on the cable (`slot`), `far_holds` whether the far end can hold its
/// transceiver. A problem is the reason the link stays down.
fn physical(
    d: &Device,
    e: &End,
    c: &Cable,
    optic: Option<&'static Optic>,
    slot: Option<&[u32]>,
    far_holds: bool,
) -> Result<(), String> {
    let conf = d.config_of(&e.port);
    if conf.contains(&"shutdown") {
        return Err(format!("{} is shut down", e.label));
    }
    if let Some(why) = d
        .stack
        .uplink_off(&e.port, &crate::show::enabled_uplinks(d))
    {
        return Err(format!("{} is inactive: {why}", e.label));
    }
    // the media-type line that fixed a dual-purpose port's side
    let side = value(&conf, "media-type ")
        .filter(|m| d.stack.dual_purpose(&e.port) && matches!(*m, "rj45" | "sfp"))
        .map(|m| format!(" (media-type {m})"))
        .unwrap_or_default();
    let part = c.optic.as_deref();
    match (slot, part, optic) {
        // a copper SFP at the far end meets an RJ-45 port (c9200 17.15 capture 15's GLC-T to its DS1A's Gi1/0/4)
        (None, Some(_), Some(o)) if o.negotiates && far_holds => Ok(()),
        (None, Some(p), _) => Err(format!("{} is RJ-45{side}; the cable names {p}", e.label)),
        (None, None, _) => match c.speed {
            Some(s) if !copper_speeds(&e.port).contains(&s) => Err(format!(
                "{} runs {}; the cable's speed is {s}",
                e.label,
                speeds_text(copper_speeds(&e.port))
            )),
            _ => Ok(()),
        },
        (Some([]), _, _) => Err(format!(
            "{} takes a QSFP transceiver, which vswitch does not model",
            e.label
        )),
        (Some(_), None, _) => Err(format!(
            "{} takes a transceiver{side}; name the cable's optic",
            e.label
        )),
        (Some(_), Some(p), None) => Err(format!(
            "{p} is not a transceiver vswitch knows ({})",
            platform::OPTICS
                .iter()
                .map(|o| o.part)
                .collect::<Vec<_>>()
                .join(", ")
        )),
        (Some(speeds), Some(_), Some(o)) if !speeds.contains(&o.mbps) => Err(format!(
            "{} runs {}; {} runs {}",
            e.label,
            speeds_text(speeds),
            o.part,
            platform::speed_text(o.mbps)
        )),
        (Some(_), Some(_), Some(o)) => match value(&conf, "speed ") {
            Some(s) if s.parse::<u32>().is_ok_and(|v| v != o.mbps) => Err(format!(
                "{} is set to speed {s}; {} runs {}",
                e.label,
                o.part,
                platform::speed_text(o.mbps)
            )),
            _ => Ok(()),
        },
    }
}

/// BPDU guard on a port: set on it (or its bundle), or PortFast with BPDU guard by default.
fn bpdu_guard(d: &Device, l: &[&str], portfast: bool) -> bool {
    if l.contains(&"spanning-tree bpduguard disable") {
        return false;
    }
    l.contains(&"spanning-tree bpduguard enable")
        || portfast
            && d.global.iter().any(|g| {
                g.text == "spanning-tree portfast bpduguard default"
                    || g.text == "spanning-tree portfast edge bpduguard default"
            })
}

/// PortFast on a port: set on it (or its bundle), or by default on an access port. On a trunking port
/// only the `trunk` form acts; the plain form acts while the port is not trunking (STP guide, as
/// `lint` reads it).
fn portfast(d: &Device, l: &[&str], trunk: bool) -> bool {
    if l.contains(&"spanning-tree portfast disable") {
        return false;
    }
    l.iter()
        .any(|x| match x.strip_prefix("spanning-tree portfast") {
            Some("" | " edge") => !trunk,
            Some(" trunk" | " edge trunk") => true,
            _ => false,
        })
        || !trunk
            && d.global.iter().any(|g| {
                g.text == "spanning-tree portfast default"
                    || g.text == "spanning-tree portfast edge default"
            })
}

/// A trunk's native VLAN: its `switchport trunk native vlan <id>`, else 1. `native vlan tag`, a line
/// of its own, tags the native VLAN and names none.
fn native<'a>(l: &[&'a str]) -> &'a str {
    l.iter()
        .filter_map(|x| x.strip_prefix("switchport trunk native vlan "))
        .find(|v| v.parse::<u32>().is_ok())
        .unwrap_or("1")
}

fn allowed(l: &[&str]) -> BTreeSet<u32> {
    value(l, "switchport trunk allowed vlan ")
        .map_or_else(|| (1..=4094).collect(), generate::vlan_ids)
}

fn defined(d: &Device) -> BTreeSet<u32> {
    let mut v: BTreeSet<u32> = d
        .blocks
        .iter()
        .filter_map(|b| b.header.strip_prefix("vlan "))
        .flat_map(generate::vlan_ids)
        .collect();
    v.insert(1);
    v
}

/// A routed port's IPv4 address and mask, and the address as written.
fn address<'a>(l: &[&'a str]) -> Option<(u32, u32, &'a str)> {
    let mut w = value(l, "ip address ")?.split_whitespace();
    let ip = |t: &str| -> Option<u32> {
        let o: Vec<u32> = t.split('.').filter_map(|p| p.parse().ok()).collect();
        (o.len() == 4).then(|| o.iter().fold(0, |a, b| a << 8 | b))
    };
    let text = w.next()?;
    Some((ip(text)?, ip(w.next()?)?, text))
}

/// The MTU a switch gives a link running at `mbps`: a classic switch's `system mtu jumbo` at 1000
/// Mb/s and above and its `system mtu` below (the 2960-X book's `system mtu ?`), an IOS XE switch's
/// `system mtu` at any speed; 1500 where unset. `system mtu routing` sets neither.
fn link_mtu(d: &Device, mbps: u32) -> u32 {
    let key = if d.stack.classic() && mbps >= 1000 {
        "system mtu jumbo "
    } else {
        "system mtu "
    };
    d.global
        .iter()
        .find_map(|g| g.text.strip_prefix(key)?.parse().ok())
        .unwrap_or(1500)
}

/// What joins two switches at Layer 2: the MTU each gives a link between them, and whether a link
/// trunks at both ends.
#[derive(Default)]
struct Joined {
    mtus: BTreeSet<(u32, u32)>,
    trunk: bool,
}

/// Brings up every cable from both ends: the physical link, DTP, the channel, then what the two
/// configurations disagree on. Each linked port gets its link (`Device::links`).
pub fn connect(built: &mut [Built], cables: &[Cable]) -> Result<Report, String> {
    let names: Vec<String> = built.iter().map(|b| b.name.clone()).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut r = Report::default();
    let mut used: BTreeSet<(usize, String)> = BTreeSet::new();
    // channel group -> far ends, to check that a bundle's members all reach one bundle
    let mut far_of: BTreeMap<(usize, u32), BTreeSet<String>> = BTreeMap::new();
    // switch pairs joined by a Layer 2 link
    let mut l2_pairs: BTreeMap<(usize, usize), Joined> = BTreeMap::new();
    for c in cables {
        let a = end(built, &names, &c.a)?;
        let b = end(built, &names, &c.b)?;
        let (x, y) = match (a, b) {
            (Some(x), y) => (x, y),
            (None, Some(y)) => (y, None),
            (None, None) => {
                return Err(format!("cable {} - {}: neither end is a switch", c.a, c.b));
            }
        };
        let device_name = if y.is_none() {
            [&c.a, &c.b]
                .into_iter()
                .find(|t| !t.contains(':'))
                .cloned()
                .unwrap_or_default()
        } else {
            String::new()
        };
        let mut twice = false;
        for e in std::iter::once(&x).chain(y.as_ref()) {
            if !used.insert((e.sw, e.port.clone())) {
                r.finding(format!("{} is cabled twice", e.label));
                twice = true;
            }
        }
        if twice {
            continue;
        }
        let optic = c.optic.as_deref().and_then(platform::optic);
        let ends: Vec<&End> = std::iter::once(&x).chain(y.as_ref()).collect();
        let label = ends
            .iter()
            .map(|e| e.label.clone())
            .chain((y.is_none()).then(|| device_name.clone()))
            .collect::<Vec<_>>()
            .join(" - ");
        let slots: Vec<Option<Vec<u32>>> = ends
            .iter()
            .map(|e| slot(&built[e.sw].session.device, e, c.optic.as_deref()))
            .collect();
        let mut down = vec![];
        for (i, e) in ends.iter().enumerate() {
            // the far end holds the cable's transceiver in a slot, or is a device the lab does not
            // model
            let far_holds = slots.len() == 1 || slots[1 - i].is_some();
            let d = &built[e.sw].session.device;
            if let Err(why) = physical(d, e, c, optic, slots[i].as_deref(), far_holds) {
                down.push(why);
            }
        }
        if !down.is_empty() {
            let shut_only = down.iter().all(|d| d.ends_with("is shut down"));
            r.links.push(format!("{label}: down, {}", down.join("; ")));
            for d in down {
                if shut_only {
                    r.note(format!("{label}: {d}"));
                } else {
                    r.finding(format!("{label}: {d}"));
                }
            }
            continue;
        }
        // DTP and the channel; a routed port sends no DTP
        let far = |e: Option<&End>| e.map(|e| (e.mode, e.nonegotiate || e.routed));
        let tx = !x.routed && trunks(x.mode, far(y.as_ref()));
        let ty = y
            .as_ref()
            .is_some_and(|y| !y.routed && trunks(y.mode, far(Some(&x))));
        let (bx, by) = match (&x.channel, y.as_ref().map(|y| &y.channel)) {
            (Some((_, mx)), Some(Some((_, my)))) => {
                let both = bundles(mx, my);
                (both || mx == "on", both || my == "on")
            }
            (Some((_, mx)), Some(None)) => (mx == "on", false),
            (None, Some(Some((_, my)))) => (false, my == "on"),
            (None, Some(None)) | (None, None) => (false, false),
            // a device the lab does not model runs LACP or a static bundle, no PAgP
            (Some((_, mx)), None) => (protocol(mx) != "PAgP", false),
        };
        // the transceiver is an end's own only where its port holds it; an RJ-45 port facing a
        // copper SFP negotiates copper, at the SFP's speed where that is below its own
        let medium = |e: &End, holds: bool| -> (Option<&'static Optic>, Option<u32>) {
            if holds {
                return (optic, c.speed);
            }
            let top = copper_speeds(&e.port).last().copied().unwrap_or(1000);
            let slower = optic.map(|o| o.mbps).filter(|m| *m < top);
            (None, c.speed.or(slower))
        };
        let peer_of =
            |e: &End| format!("{} {}", built[e.sw].session.device.hostname, short(&e.port));
        let (px, py) = match &y {
            Some(y) => (peer_of(y), peer_of(&x)),
            None => (device_name.clone(), String::new()),
        };
        let mut state = vec!["connected".to_string()];
        state.push(if tx {
            "trunk".into()
        } else if x.routed {
            "routed".into()
        } else if x.mode == Mode::Tunnel {
            "tunnel".into()
        } else {
            "access".into()
        });
        if let Some((n, m)) = &x.channel {
            state.push(format!("Po{n} {m} {}", if bx { "(P)" } else { "(I)" }));
        }
        if let Some(o) = optic {
            state.push(format!("{} {}", platform::speed_text(o.mbps), o.part));
        }
        r.links.push(format!("{label}: {}", state.join(", ")));
        let (ox, sx) = medium(&x, slots[0].is_some());
        built[x.sw].session.device.links.insert(
            x.port.clone(),
            Link {
                peer: px,
                optic: ox,
                speed: sx,
                half: c.half,
                trunk: tx,
                bundled: bx,
            },
        );
        if let Some(y) = &y {
            let (oy, sy) = medium(y, slots[1].is_some());
            built[y.sw].session.device.links.insert(
                y.port.clone(),
                Link {
                    peer: py,
                    optic: oy,
                    speed: sy,
                    half: c.half,
                    trunk: ty,
                    bundled: by,
                },
            );
        }
        // what goes wrong between the two configurations
        if let Some((n, m)) = &x.channel {
            let to = match &y {
                Some(y) => match &y.channel {
                    Some((k, _)) => format!("{} Po{k}", names[y.sw]),
                    None => format!("{} in no channel", y.label),
                },
                None => device_name.clone(),
            };
            far_of.entry((x.sw, *n)).or_default().insert(to);
            if y.is_none() && protocol(m) == "PAgP" {
                r.note(format!(
                    "{label}: {device_name} is taken to run no PAgP; Po{n} member {} stays stand-alone",
                    short(&x.port)
                ));
            }
        }
        let Some(y) = y else { continue };
        if let Some((n, _)) = &y.channel {
            let to = match &x.channel {
                Some((k, _)) => format!("{} Po{k}", names[x.sw]),
                None => format!("{} in no channel", x.label),
            };
            far_of.entry((y.sw, *n)).or_default().insert(to);
        }
        match (&x.channel, &y.channel) {
            (Some((_, mx)), Some((_, my))) if !bundles(mx, my) => {
                let why = if protocol(mx) == protocol(my) {
                    format!("both ends {mx}: neither starts {}", protocol(mx))
                } else {
                    format!(
                        "mode {mx} ({}) against mode {my} ({})",
                        protocol(mx),
                        protocol(my)
                    )
                };
                r.finding(format!("{label}: the channel does not form, {why}"));
            }
            (Some((n, m)), None) | (None, Some((n, m))) => {
                r.finding(format!(
                    "{label}: Po{n} (mode {m}) at one end only; the other end is a plain port"
                ));
            }
            _ => {}
        }
        let (dx, dy) = (&built[x.sw].session.device, &built[y.sw].session.device);
        let cx = x.channel.as_ref().map(|c| c.0);
        let cy = y.channel.as_ref().map(|c| c.0);
        let (lx, ly) = (lines(dx, &x.port, cx), lines(dy, &y.port, cy));
        // a bundle's lines are named once, by the bundle
        let pair = match (cx, cy) {
            (Some(a), Some(b)) if bx && by => {
                format!("{} Po{a} - {} Po{b}", names[x.sw], names[y.sw])
            }
            _ => label.clone(),
        };
        if x.routed != y.routed {
            let (routed, l2) = if x.routed { (&x, &y) } else { (&y, &x) };
            r.finding(format!(
                "{label}: {} is routed (no switchport), {} a switch port",
                routed.label, l2.label
            ));
            continue;
        }
        if x.routed {
            match (address(&lx), address(&ly)) {
                (Some((a, _, ip)), Some((b, _, _))) if a == b => {
                    r.finding(format!("{pair}: both ends have address {ip}"))
                }
                (Some((a, m, _)), Some((b, n, _))) if (a & m, m) != (b & n, n) => r.finding(
                    format!("{pair}: the two addresses are on different subnets"),
                ),
                (Some(_), Some(_)) => {}
                _ => r.note(format!("{pair}: a routed end has no IPv4 address")),
            }
            continue;
        }
        // a cable between two ports of one switch joins no two switches
        if x.sw != y.sw {
            let mbps = optic.map(|o| o.mbps).or(c.speed).unwrap_or_else(|| {
                [&x, &y]
                    .iter()
                    .map(|e| copper_speeds(&e.port).last().copied().unwrap_or(1000))
                    .min()
                    .unwrap_or(1000)
            });
            let (mx, my) = (link_mtu(dx, mbps), link_mtu(dy, mbps));
            let (switches, mtus) = if x.sw < y.sw {
                ((x.sw, y.sw), (mx, my))
            } else {
                ((y.sw, x.sw), (my, mx))
            };
            let joined = l2_pairs.entry(switches).or_default();
            joined.mtus.insert(mtus);
            // VTP advertisements cross trunks only
            joined.trunk |= tx && ty;
        }
        match (tx, ty) {
            (true, false) | (false, true) => {
                let (t, a) = if tx { (&x, &y) } else { (&y, &x) };
                // a trunk into a dot1q-tunnel port is QinQ: every frame crosses, tunnelled
                if a.mode != Mode::Tunnel {
                    r.finding(format!(
                        "{label}: {} trunks, {} does not: only untagged frames cross",
                        t.label, a.label
                    ));
                }
            }
            (true, true) => {
                let (nx, ny) = (native(&lx), native(&ly));
                if nx != ny {
                    r.finding(format!(
                        "{pair}: native VLAN {nx} against {ny}: untagged frames change VLAN"
                    ));
                }
                let known: BTreeSet<u32> = defined(dx).union(&defined(dy)).copied().collect();
                let (ax, ay) = (allowed(&lx), allowed(&ly));
                let only = |a: &BTreeSet<u32>, b: &BTreeSet<u32>| -> BTreeSet<u32> {
                    a.difference(b)
                        .filter(|v| known.contains(v))
                        .copied()
                        .collect()
                };
                for (side, set) in [(names[x.sw], only(&ax, &ay)), (names[y.sw], only(&ay, &ax))] {
                    if !set.is_empty() {
                        r.note(format!(
                            "{pair}: VLANs {} allowed at {side} only",
                            crate::target::ios_list(&set)
                        ));
                    }
                }
            }
            // a tunnel port takes whatever the far end sends, and a private VLAN port its VLANs by
            // association, not `switchport access vlan`
            (false, false)
                if [x.mode, y.mode]
                    .iter()
                    .any(|m| matches!(m, Mode::Tunnel | Mode::Private)) => {}
            (false, false) => {
                let vx = value(&lx, "switchport access vlan ").unwrap_or("1");
                let vy = value(&ly, "switchport access vlan ").unwrap_or("1");
                if vx != vy {
                    r.finding(format!(
                        "{label}: access VLAN {vx} against {vy}: the two VLANs join"
                    ));
                }
            }
        }
        for (e, l, d, t) in [(&x, &lx, dx, tx), (&y, &ly, dy, ty)] {
            let pf = portfast(d, l, t);
            if bpdu_guard(d, l, pf) {
                r.finding(format!(
                    "{label}: BPDU guard on {}: the far switch's BPDUs err-disable it",
                    e.label
                ));
            } else if pf {
                r.note(format!(
                    "{label}: PortFast on {}, which faces a switch",
                    e.label
                ));
            }
        }
    }
    for ((sw, n), fars) in &far_of {
        if fars.len() > 1 {
            let list: Vec<&str> = fars.iter().map(String::as_str).collect();
            r.finding(format!(
                "{} Po{n}: its members reach {}",
                names[*sw],
                list.join(" and ")
            ));
        }
    }
    for ((i, j), joined) in l2_pairs {
        let (di, dj) = (&built[i].session.device, &built[j].session.device);
        for (mi, mj) in joined.mtus {
            if mi != mj {
                r.note(format!(
                    "{} and {}: system MTU {mi} against {mj}: larger frames drop at the smaller",
                    names[i], names[j]
                ));
            }
        }
        if joined.trunk && !di.vlans_in_running_config() && !dj.vlans_in_running_config() {
            r.note(format!(
                "{} and {}: both VTP servers or clients: one VTP domain's highest revision sets both VLAN databases",
                names[i], names[j]
            ));
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        Engine::new(grammars::grammar("c9300", release::DEFAULT).unwrap())
    }

    /// Switches `a` and `b` from configuration text on the given hardware, cabled as listed.
    fn lab<'a>(
        e: &'a Engine,
        a: (&str, &str),
        b: (&str, &str),
        cables: &[(&str, &str, Option<&str>)],
    ) -> (Vec<Built<'a>>, Report) {
        let cables: Vec<Cable> = cables
            .iter()
            .map(|(x, y, o)| Cable {
                a: x.to_string(),
                b: y.to_string(),
                optic: o.map(str::to_string),
                speed: None,
                half: false,
            })
            .collect();
        lab_with(e, a, b, &cables)
    }

    /// `lab` with whole cables.
    fn lab_with<'a>(
        e: &'a Engine,
        a: (&str, &str),
        b: (&str, &str),
        cables: &[Cable],
    ) -> (Vec<Built<'a>>, Report) {
        let mut built: Vec<Built> = [("a", a), ("b", b)]
            .into_iter()
            .map(|(n, (stack, config))| {
                let st = platform::parse_stack(stack).unwrap();
                let (s, _) = check::type_config(e, st, config);
                Built {
                    name: n.into(),
                    session: s,
                    typed: 0,
                    refused: vec![],
                }
            })
            .collect();
        let r = connect(&mut built, cables).unwrap();
        (built, r)
    }

    const X: &str = "C9300X-24Y";
    const ONE: &[(&str, &str, Option<&str>)] = &[("a:Twe1/0/1", "b:Twe1/0/1", Some("SFP-10G-SR"))];

    fn port(lines: &str) -> String {
        format!("interface TwentyFiveGigE1/0/1\n{lines}\n")
    }

    fn has(v: &[String], text: &str) -> bool {
        v.iter().any(|f| f.contains(text))
    }

    #[test]
    fn a_cable_comes_up_only_when_both_ends_can_run_it() {
        let e = engine();
        let (built, r) = lab(&e, (X, ""), (X, ""), &[("a:Twe1/0/1", "b:Twe1/0/1", None)]);
        assert!(has(&r.findings, "a Twe1/0/1 takes a transceiver"), "{r:?}");
        assert!(built[0].session.device.links.is_empty());
        let (_, r) = lab(
            &e,
            (X, ""),
            (X, ""),
            &[("a:Twe1/0/1", "b:Twe1/0/1", Some("GLC-FE-100FX"))],
        );
        assert!(
            has(&r.findings, "runs 25G, 10G, 1000; GLC-FE-100FX runs 100"),
            "{r:?}"
        );
        let (_, r) = lab(
            &e,
            (X, ""),
            (X, ""),
            &[("a:Twe1/0/1", "b:Twe1/0/1", Some("SFP-X"))],
        );
        assert!(
            has(&r.findings, "SFP-X is not a transceiver vswitch knows"),
            "{r:?}"
        );
        let (_, r) = lab(
            &e,
            ("C9300-24T", ""),
            (X, ""),
            &[("a:Gi1/0/1", "b:Twe1/0/1", Some("SFP-10G-SR"))],
        );
        assert!(has(&r.findings, "a Gi1/0/1 is RJ-45"), "{r:?}");
        // 100BASE-FX only where a switch ran it: the 9400 SFP card
        let (_, r) = lab(
            &e,
            ("C9300-24S", ""),
            ("C9300-24S", ""),
            &[("a:Gi1/0/1", "b:Gi1/0/1", Some("GLC-FE-100FX"))],
        );
        assert!(
            has(&r.findings, "a Gi1/0/1 runs 1000; GLC-FE-100FX runs 100"),
            "{r:?}"
        );
        let (_, r) = lab(&e, (X, &port(" speed 1000")), (X, ""), ONE);
        assert!(has(&r.findings, "is set to speed 1000"), "{r:?}");
        // a port shut down keeps the link down, which may be the plan: a note
        let (built, r) = lab(&e, (X, &port(" shutdown")), (X, ""), ONE);
        assert!(
            r.findings.is_empty() && has(&r.notes, "a Twe1/0/1 is shut down"),
            "{r:?}"
        );
        assert!(built[1].session.device.links.is_empty());
        let (_, r) = lab(
            &e,
            (X, ""),
            (X, ""),
            &[
                ("a:Twe1/0/1", "b:Twe1/0/1", Some("SFP-10G-SR")),
                ("a:Twe1/0/1", "b:Twe1/0/2", Some("SFP-10G-SR")),
            ],
        );
        assert!(has(&r.findings, "a Twe1/0/1 is cabled twice"), "{r:?}");
    }

    #[test]
    fn a_supervisor_uplink_the_pair_leaves_off_has_no_link() {
        let e = Engine::new(grammars::grammar("c9400", Release::parse("17.5").unwrap()).unwrap());
        let core = "1=C9400-LC-48S,2=C9400-LC-24XS,3=C9400-SUP-1XL,4=C9400-SUP-1XL";
        let (_, r) = lab(
            &e,
            (core, ""),
            (core, ""),
            &[("a:Te3/0/5", "b:Te3/0/1", Some("SFP-10G-LR"))],
        );
        assert!(has(&r.findings, "a Te3/0/5 is inactive"), "{r:?}");
    }

    #[test]
    fn dtp_and_vlans_are_checked_from_both_ends() {
        let e = engine();
        let trunk = port(" switchport mode trunk");
        let (_, r) = lab(&e, (X, &trunk), (X, &port(" switchport mode access")), ONE);
        assert!(
            has(&r.findings, "a Twe1/0/1 trunks, b Twe1/0/1 does not"),
            "{r:?}"
        );
        // dynamic auto, the default, trunks with a trunk that negotiates
        let (built, r) = lab(&e, (X, &trunk), (X, ""), ONE);
        assert!(r.findings.is_empty(), "{r:?}");
        assert!(built[1].session.device.links["TwentyFiveGigE1/0/1"].trunk);
        let (_, r) = lab(
            &e,
            (X, &port(" switchport mode trunk\n switchport nonegotiate")),
            (X, ""),
            ONE,
        );
        assert!(has(&r.findings, "trunks, b Twe1/0/1 does not"), "{r:?}");
        let (_, r) = lab(
            &e,
            (
                X,
                &port(" switchport mode trunk\n switchport trunk native vlan 10"),
            ),
            (X, &trunk),
            ONE,
        );
        assert!(has(&r.findings, "native VLAN 10 against 1"), "{r:?}");
        let (_, r) = lab(
            &e,
            (
                X,
                &format!(
                    "vlan 20\n{}",
                    port(" switchport mode trunk\n switchport trunk allowed vlan 1-10")
                ),
            ),
            (X, &trunk),
            ONE,
        );
        assert!(has(&r.notes, "VLANs 20 allowed at b only"), "{r:?}");
        let (_, r) = lab(
            &e,
            (
                X,
                &port(" switchport mode access\n switchport access vlan 10"),
            ),
            (
                X,
                &port(" switchport mode access\n switchport access vlan 20"),
            ),
            ONE,
        );
        assert!(has(&r.findings, "access VLAN 10 against 20"), "{r:?}");
    }

    #[test]
    fn a_channel_forms_only_when_its_modes_meet() {
        let e = engine();
        let two: &[(&str, &str, Option<&str>)] = &[
            ("a:Twe1/0/1", "b:Twe1/0/1", Some("SFP-10G-SR")),
            ("a:Twe1/0/2", "b:Twe1/0/2", Some("SFP-10G-SR")),
        ];
        let ch = |n: u32, mode: &str| {
            format!(
                "interface range TwentyFiveGigE1/0/1 - 2\n switchport mode trunk\n channel-group {n} mode {mode}\n"
            )
        };
        let (built, r) = lab(&e, (X, &ch(1, "active")), (X, &ch(1, "passive")), two);
        assert!(r.findings.is_empty(), "{r:?}");
        let summary = crate::show::etherchannel_summary(&built[1].session.device);
        // the table's row, then the blank line that closes it
        assert_eq!(
            summary[summary.len() - 2],
            "1      Po1(SU)         LACP        Twe1/0/1(P)     Twe1/0/2(P)     "
        );
        let (built, r) = lab(&e, (X, &ch(1, "passive")), (X, &ch(1, "passive")), two);
        assert!(
            has(&r.findings, "the channel does not form, both ends passive"),
            "{r:?}"
        );
        let summary = crate::show::etherchannel_summary(&built[0].session.device);
        assert_eq!(
            &summary[summary.len() - 2],
            // IOS XE suspends a member with no LACP partner (the lab C9300, 17.6.4)
            "1      Po1(SD)         LACP        Twe1/0/1(s)     Twe1/0/2(s)     "
        );
        let (built, r) = lab(&e, (X, &ch(1, "on")), (X, &ch(1, "active")), two);
        assert!(
            has(&r.findings, "mode on (on) against mode active (LACP)"),
            "{r:?}"
        );
        assert!(built[0].session.device.links["TwentyFiveGigE1/0/1"].bundled);
        assert!(!built[1].session.device.links["TwentyFiveGigE1/0/1"].bundled);
        let (_, r) = lab(
            &e,
            (X, &ch(1, "active")),
            (X, &port(" switchport mode trunk")),
            &two[..1],
        );
        assert!(
            has(&r.findings, "Po1 (mode active) at one end only"),
            "{r:?}"
        );
        let split = "interface TwentyFiveGigE1/0/1\n switchport mode trunk\n channel-group 1 mode active\ninterface TwentyFiveGigE1/0/2\n switchport mode trunk\n channel-group 2 mode active\n";
        let (_, r) = lab(&e, (X, &ch(1, "active")), (X, split), two);
        assert!(
            has(&r.findings, "a Po1: its members reach b Po1 and b Po2"),
            "{r:?}"
        );
    }

    #[test]
    fn layer_3_and_spanning_tree_are_checked_from_both_ends() {
        let e = engine();
        let routed = |ip: &str| port(&format!(" no switchport\n ip address {ip} 255.255.255.252"));
        let (_, r) = lab(&e, (X, &routed("10.0.0.1")), (X, ""), ONE);
        assert!(
            has(
                &r.findings,
                "a Twe1/0/1 is routed (no switchport), b Twe1/0/1 a switch port"
            ),
            "{r:?}"
        );
        let (_, r) = lab(&e, (X, &routed("10.0.0.1")), (X, &routed("10.0.0.2")), ONE);
        assert!(r.findings.is_empty(), "{r:?}");
        let (_, r) = lab(&e, (X, &routed("10.0.0.1")), (X, &routed("10.0.0.5")), ONE);
        assert!(has(&r.findings, "different subnets"), "{r:?}");
        let trunk = port(" switchport mode trunk");
        let (_, r) = lab(
            &e,
            (
                X,
                &port(" switchport mode trunk\n spanning-tree bpduguard enable"),
            ),
            (X, &trunk),
            ONE,
        );
        assert!(has(&r.findings, "BPDU guard on a Twe1/0/1"), "{r:?}");
        let (_, r) = lab(
            &e,
            (
                X,
                &format!(
                    "spanning-tree portfast bpduguard default\n{}",
                    port(" switchport mode trunk\n spanning-tree portfast trunk")
                ),
            ),
            (X, &trunk),
            ONE,
        );
        assert!(has(&r.findings, "BPDU guard on a Twe1/0/1"), "{r:?}");
        let (_, r) = lab(
            &e,
            (
                X,
                &port(" switchport mode trunk\n spanning-tree portfast trunk"),
            ),
            (X, &trunk),
            ONE,
        );
        assert!(
            r.findings.is_empty() && has(&r.notes, "PortFast on a Twe1/0/1"),
            "{r:?}"
        );
    }

    #[test]
    fn two_linked_switches_compare_mtu_and_vtp() {
        let e = engine();
        let trunk = port(" switchport mode trunk");
        let (_, r) = lab(
            &e,
            (X, &format!("system mtu 9198\n{trunk}")),
            (X, &trunk),
            ONE,
        );
        assert!(
            has(&r.notes, "a and b: system MTU 9198 against 1500"),
            "{r:?}"
        );
        assert!(
            has(&r.notes, "a and b: both VTP servers or clients"),
            "{r:?}"
        );
        let (_, r) = lab(
            &e,
            (X, &format!("vtp mode transparent\n{trunk}")),
            (X, &trunk),
            ONE,
        );
        assert!(!has(&r.notes, "VTP"), "{r:?}");
    }

    #[test]
    fn a_linked_port_shows_its_link() {
        let e = engine();
        let a = format!(
            "vlan 10\ninterface Vlan10\n ip address 10.0.10.1 255.255.255.0\ninterface Vlan20\n ip address 10.0.20.1 255.255.255.0\n{}",
            port(" switchport mode trunk\n switchport trunk native vlan 10")
        );
        let (mut built, _) = lab(
            &e,
            (X, &a),
            (
                X,
                &port(" switchport mode trunk\n switchport trunk native vlan 10"),
            ),
            ONE,
        );
        let d = &built[0].session.device;
        let status = crate::show::interfaces_status(d);
        assert!(status.contains(&"Twe1/0/1                        connected    trunk        full    10G SFP-10GBase-SR".to_string()), "{status:?}");
        assert!(
            !crate::show::vlan_brief(d, 80)
                .iter()
                .any(|l| l.contains("Twe1/0/1,") || l.ends_with("Twe1/0/1"))
        );
        let trunk = crate::show::interfaces_trunk(d);
        assert_eq!(
            trunk[2],
            "Twe1/0/1       on               802.1q         trunking      10"
        );
        assert_eq!(trunk[5], "Twe1/0/1       1-4094");
        assert_eq!(trunk[8], "Twe1/0/1       1,10");
        // a VLAN interface is up while its VLAN is carried by a running port
        let brief = crate::show::ip_interface_brief(d);
        let row = |n: &str| {
            brief
                .iter()
                .find(|l| l.starts_with(&format!("{n} ")))
                .unwrap()
                .clone()
        };
        assert!(
            row("Vlan10").ends_with("up                    up      "),
            "{brief:?}"
        );
        assert!(
            row("Vlan20").ends_with("down                  down    "),
            "{brief:?}"
        );
        assert!(
            row("TwentyFiveGigE1/0/1").ends_with("up                    up      "),
            "{brief:?}"
        );
        assert!(
            row("TwentyFiveGigE1/0/2").ends_with("down                  down    "),
            "{brief:?}"
        );
        // shut down at the console, the port drops its link: no trunk, and Vlan10 loses its carrier
        // (up, its protocol down: VLAN 10 exists)
        let s = &mut built[0].session;
        assert_eq!(
            s.script("end\nconfigure terminal\ninterface TwentyFiveGigE1/0/1\n shutdown\nend\n")
                .1,
            0
        );
        assert!(crate::show::interfaces_trunk(&s.device).is_empty());
        let brief = crate::show::ip_interface_brief(&s.device);
        assert!(
            brief
                .iter()
                .any(|l| l.starts_with("Vlan10 ") && l.ends_with("up                    down    ")),
            "{brief:?}"
        );
    }

    #[test]
    fn dtp_trunks_as_the_mode_table_says() {
        use Mode::*;
        assert!(trunks(Trunk, None));
        assert!(!trunks(Access, Some((Trunk, false))));
        assert!(trunks(Auto, Some((Trunk, false))));
        assert!(!trunks(Auto, Some((Trunk, true))));
        assert!(trunks(Desirable, Some((Auto, false))));
        assert!(!trunks(Auto, Some((Auto, false))));
        assert!(trunks(Auto, Some((Desirable, false))));
        assert!(!trunks(Desirable, None));
    }

    #[test]
    fn channel_modes_bundle_as_the_guide_says() {
        assert!(bundles("active", "passive"));
        assert!(!bundles("passive", "passive"));
        assert!(bundles("desirable", "auto"));
        assert!(!bundles("auto", "auto"));
        assert!(bundles("on", "on"));
        assert!(!bundles("on", "active"));
        assert!(!bundles("active", "desirable"));
    }

    #[test]
    fn a_copper_sfp_is_its_slot_s_and_the_rj_45_end_runs_copper() {
        let e = engine();
        // c9300 17.15 capture 3's Gi1/0/4 (C9300-24T) on its ES1A's GLC-T (C9200L-24T-4G), which the DS1A's
        // status shows `a-full a-1000 10/100/1000BaseTX`
        let (built, r) = lab(
            &e,
            ("C9300-24T", ""),
            ("C9200L-24T-4G", ""),
            &[("a:Gi1/0/4", "b:Gi1/1/1", Some("GLC-T"))],
        );
        assert!(r.findings.is_empty(), "{r:?}");
        let row = |b: &Built, port: &str| {
            crate::show::interfaces_status(&b.session.device)
                .into_iter()
                .find(|l| l.starts_with(&format!("{port} ")))
                .unwrap_or_default()
        };
        assert!(
            row(&built[0], "Gi1/0/4").ends_with("a-full a-1000 10/100/1000BaseTX"),
            "{}",
            row(&built[0], "Gi1/0/4")
        );
        assert!(row(&built[1], "Gi1/1/1").ends_with("a-1000 10/100/1000BaseTX SFP"));
        // a Multigigabit port on a copper SFP runs the SFP's speed
        let (built, _) = lab(
            &e,
            ("C9300-24UX", ""),
            ("C9200L-24T-4G", ""),
            &[("a:Te1/0/1", "b:Gi1/1/1", Some("GLC-T"))],
        );
        let link = &built[0].session.device.links["TenGigabitEthernet1/0/1"];
        assert_eq!((link.optic, link.speed), (None, Some(1000)));
        // two RJ-45 ports hold no SFP between them
        let (_, r) = lab(
            &e,
            ("C9300-24T", ""),
            ("C9300-24T", ""),
            &[("a:Gi1/0/5", "b:Gi1/0/5", Some("GLC-T"))],
        );
        assert!(
            has(&r.findings, "a Gi1/0/5 is RJ-45; the cable names GLC-T"),
            "{r:?}"
        );
    }

    #[test]
    fn each_port_takes_the_medium_its_hardware_gives() {
        let e = engine();
        let link =
            |a: (&str, &str), cables: &[(&str, &str, Option<&str>)]| lab(&e, a, (X, ""), cables).1;
        // the IE-3010's SFP ports take a 100BASE-FX optic and no copper; its others are copper
        let r = link(
            ("IE-3010-16S-8PC", ""),
            &[
                ("a:Fa0/1", "plc", Some("GLC-FE-100FX")),
                ("a:Fa0/2", "pc", None),
                ("a:Fa0/17", "camera", None),
            ],
        );
        assert_eq!(
            r.findings,
            ["a Fa0/2 - pc: a Fa0/2 takes a transceiver; name the cable's optic"]
        );
        // the C9400-LC-24XY's ports 21-24 are SFP+
        let r = link(
            ("1=C9400-LC-24XY,3=C9400-SUP-1XL", ""),
            &[
                ("a:Te1/0/21", "server", Some("SFP-10G-SR")),
                ("a:Te1/0/22", "pc", None),
            ],
        );
        assert_eq!(
            r.findings,
            ["a Te1/0/22 - pc: a Te1/0/22 takes a transceiver; name the cable's optic"]
        );
        // a Multigigabit module's ports are RJ-45
        let r = link(
            ("C9300-48T+C9300-NM-4M", ""),
            &[
                ("a:Te1/1/1", "ap", None),
                ("a:Te1/1/2", "server", Some("SFP-10G-SR")),
            ],
        );
        assert_eq!(
            r.findings,
            ["a Te1/1/2 - server: a Te1/1/2 is RJ-45; the cable names SFP-10G-SR"]
        );
        // the management port is copper on a model whose front ports are SFP
        let r = link(("C9300-24S", ""), &[("a:Gi0/0", "oob", None)]);
        assert!(r.findings.is_empty(), "{r:?}");
        // a supervisor's QSFP port says it takes a transceiver vswitch does not model
        let e9400 = Engine::new(grammars::grammar("c9400", release::DEFAULT).unwrap());
        let (_, r) = lab(
            &e9400,
            (
                "1=C9400-LC-48S,3=C9400-SUP-1XL",
                "interface FortyGigabitEthernet3/0/9\n enable\n",
            ),
            ("1=C9400-LC-48S,3=C9400-SUP-1XL", ""),
            &[("a:Fo3/0/9", "dc", Some("SFP-10G-SR"))],
        );
        assert_eq!(
            r.findings,
            ["a Fo3/0/9 - dc: a Fo3/0/9 takes a QSFP transceiver, which vswitch does not model"]
        );
        // a copper cable's speed is one its RJ-45 port runs
        let cable = |a: &str, speed: u32| Cable {
            a: a.into(),
            b: "phone".into(),
            optic: None,
            speed: Some(speed),
            half: false,
        };
        let (built, r) = lab_with(
            &e,
            ("C9300-24UX", ""),
            (X, ""),
            &[cable("a:Te1/0/1", 7), cable("a:Te1/0/2", 2500)],
        );
        assert_eq!(
            r.findings,
            [
                "a Te1/0/1 - phone: a Te1/0/1 runs 100, 1000, 2500, 5000, 10G; the cable's speed is 7"
            ]
        );
        assert_eq!(
            built[0].session.device.links["TenGigabitEthernet1/0/2"].speed,
            Some(2500)
        );
    }

    #[test]
    fn a_dual_purpose_uplink_takes_copper_or_an_optic_as_its_media_type_allows() {
        // WS-C2960G-24TC-L, Gi0/21-24 (data sheet); the captured 2960s set `media-type rj45` and
        // `media-type sfp` on theirs
        let e =
            Engine::new(grammars::grammar("c2960", Release::parse("15.0(2)SE").unwrap()).unwrap());
        let g = "WS-C2960G-24TC-L";
        let (built, r) = lab(
            &e,
            (
                g,
                "interface GigabitEthernet0/21\n media-type rj45\ninterface GigabitEthernet0/23\n media-type sfp\n",
            ),
            (g, "interface GigabitEthernet0/21\n media-type rj45\n"),
            &[
                ("a:Gi0/21", "server", None),
                ("a:Gi0/22", "pc", None),
                ("a:Gi0/23", "pc2", None),
                ("a:Gi0/24", "fibre", Some("GLC-SX-MMD")),
                ("b:Gi0/21", "fibre2", Some("GLC-SX-MMD")),
            ],
        );
        assert_eq!(
            r.findings,
            [
                "a Gi0/23 - pc2: a Gi0/23 takes a transceiver (media-type sfp); name the cable's optic",
                "b Gi0/21 - fibre2: b Gi0/21 is RJ-45 (media-type rj45); the cable names GLC-SX-MMD"
            ]
        );
        let links = &built[0].session.device.links;
        assert!(links["GigabitEthernet0/22"].optic.is_none());
        assert!(links["GigabitEthernet0/24"].optic.is_some());
    }

    #[test]
    fn ports_that_never_trunk_and_a_bundle_s_mode_are_read_as_ios_runs_them() {
        let e = engine();
        let trunk = port(" switchport mode trunk");
        // QinQ: a trunk into a dot1q-tunnel port carries every frame, tunnelled
        let (built, r) = lab(
            &e,
            (
                X,
                &port(" switchport access vlan 100\n switchport mode dot1q-tunnel"),
            ),
            (X, &trunk),
            ONE,
        );
        assert!(r.findings.is_empty(), "{r:?}");
        assert!(has(&r.links, "connected, tunnel"), "{r:?}");
        assert!(!built[0].session.device.links["TwentyFiveGigE1/0/1"].trunk);
        // a tunnel port sends no DTP, so a dynamic port facing it stays access, and its VLAN is
        // the tunnel's outer one, which no access VLAN at the far end need match
        let (built, r) = lab(
            &e,
            (
                X,
                &port(" switchport access vlan 100\n switchport mode dot1q-tunnel"),
            ),
            (X, &port(" switchport mode dynamic desirable")),
            ONE,
        );
        assert!(!built[1].session.device.links["TwentyFiveGigE1/0/1"].trunk);
        assert!(r.findings.is_empty(), "{r:?}");
        // a private VLAN host port never trunks
        let (_, r) = lab(
            &e,
            (X, &port(" switchport mode private-vlan host")),
            (X, &trunk),
            ONE,
        );
        assert!(
            has(&r.findings, "b Twe1/0/1 trunks, a Twe1/0/1 does not"),
            "{r:?}"
        );
        // nor does a dynamic port facing a routed one, which sends no DTP
        let (built, _) = lab(
            &e,
            (X, &port(" switchport mode dynamic desirable")),
            (
                X,
                &port(" no switchport\n ip address 10.0.0.1 255.255.255.252"),
            ),
            ONE,
        );
        assert!(!built[0].session.device.links["TwentyFiveGigE1/0/1"].trunk);
        // a member runs its bundle's mode, the bundle typed before it
        let po = |native: &str| {
            format!(
                "interface Port-channel1\n switchport mode trunk\n{native}interface TwentyFiveGigE1/0/1\n channel-group 1 mode active\n"
            )
        };
        let (built, r) = lab(
            &e,
            (X, &po(" switchport trunk native vlan 20\n")),
            (X, &po("")),
            ONE,
        );
        assert!(built[0].session.device.links["TwentyFiveGigE1/0/1"].trunk);
        assert!(has(&r.findings, "native VLAN 20 against 1"), "{r:?}");
    }

    #[test]
    fn portfast_native_vlan_tag_and_addresses_are_read_as_ios_runs_them() {
        let e = engine();
        let trunk = port(" switchport mode trunk");
        // plain PortFast acts only while the port does not trunk: no BPDU guard by default
        let (_, r) = lab(
            &e,
            (
                X,
                &format!(
                    "spanning-tree portfast bpduguard default\n{}",
                    port(" switchport mode trunk\n spanning-tree portfast")
                ),
            ),
            (X, &trunk),
            ONE,
        );
        assert!(r.findings.is_empty() && !has(&r.notes, "PortFast"), "{r:?}");
        // `native vlan tag` names no native VLAN
        let (_, r) = lab(
            &e,
            (
                X,
                &port(
                    " switchport mode trunk\n switchport trunk native vlan tag\n switchport trunk native vlan 10",
                ),
            ),
            (
                X,
                &port(" switchport mode trunk\n switchport trunk native vlan 10"),
            ),
            ONE,
        );
        assert!(r.findings.is_empty(), "{r:?}");
        // one address at both ends
        let routed = port(" no switchport\n ip address 10.0.0.1 255.255.255.252");
        let (_, r) = lab(&e, (X, &routed), (X, &routed), ONE);
        assert!(has(&r.findings, "both ends have address 10.0.0.1"), "{r:?}");
    }

    #[test]
    fn two_switch_notes_need_two_switches_and_vtp_a_trunk() {
        let e = engine();
        // VTP advertisements cross trunks only
        let access = port(" switchport mode access");
        let (_, r) = lab(&e, (X, &access), (X, &access), ONE);
        assert!(!has(&r.notes, "VTP"), "{r:?}");
        // a cable between two ports of one switch joins no two switches
        let (_, r) = lab(
            &e,
            (
                X,
                "interface range TwentyFiveGigE1/0/1 - 2\n switchport mode trunk\n",
            ),
            (X, ""),
            &[("a:Twe1/0/1", "a:Twe1/0/2", Some("SFP-10G-SR"))],
        );
        assert!(r.notes.is_empty(), "{r:?}");
        // a 2960-X's Gigabit links run its jumbo MTU; `system mtu` is its 10/100 ports'
        let x =
            Engine::new(grammars::grammar("c2960x", Release::parse("15.2(7)E").unwrap()).unwrap());
        let gi = "interface GigabitEthernet1/0/1\n switchport mode trunk\n";
        let (_, r) = lab(
            &x,
            ("WS-C2960X-24PS-L", &format!("system mtu jumbo 9000\n{gi}")),
            ("WS-C2960X-24PS-L", &format!("system mtu 1600\n{gi}")),
            &[("a:Gi1/0/1", "b:Gi1/0/1", None)],
        );
        assert!(
            has(&r.notes, "a and b: system MTU 9000 against 1500"),
            "{r:?}"
        );
        assert!(!has(&r.notes, "1600"), "{r:?}");
    }
}
