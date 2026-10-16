//! Spanning tree as a switch shows it: the lab's switches and cables read into `stp`'s graph, with the
//! session's own configuration in place of its snapshot, recomputed for every `show`, and the
//! `show spanning-tree` forms laid out as the lab C9300 (17.6.4) and the lab 2960G (15.0(2)SE11)
//! print them. Converged state only: no BPDU counters, topology changes or ages.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::device::Device;
use crate::show::{short, value};
use crate::stp::{self, Instance};

/// One end of a cable: a port of a lab switch (by its place in the lab and its full name), or a
/// device the lab does not model, which sends no BPDUs.
#[derive(Clone, Debug, PartialEq)]
pub enum CableEnd {
    Port(usize, String),
    Device(String),
}

/// The lab as `lab::connect` left it: every switch's configuration and links, and every cable. A
/// session holds it with its own place (`me`), whose configuration it reads live instead.
#[derive(Clone, Debug)]
pub struct LabView {
    pub me: usize,
    pub switches: Arc<Vec<(String, Device)>>,
    pub cables: Arc<Vec<[CableEnd; 2]>>,
}

/// A port of the graph: its switch, its full name, the physical ports it stands for, and whether
/// the port number it carries is one IOS is known to print.
#[derive(Clone, Debug)]
pub struct Logical {
    pub sw: usize,
    pub name: String,
    pub members: Vec<String>,
    pub numbered: bool,
}

/// The lab's spanning tree: the graph, its converged state, and what each graph port is.
pub struct Spanning {
    pub graph: stp::Graph,
    pub converged: stp::Converged,
    pub ports: Vec<Logical>,
    pub me: usize,
    pub names: Vec<String>,
}

/// `spanning-tree mode`: `pvst` by default (the command reference: "The default mode is PVST+").
pub fn mode(d: &Device) -> &str {
    d.global
        .iter()
        .find_map(|e| e.text.strip_prefix("spanning-tree mode "))
        .unwrap_or("pvst")
}

/// `spanning-tree pathcost method`: by default long on IOS XE (the lab C9300, 17.6.4) and short on
/// classic IOS (the lab 2960G, 15.0(2)SE11); other releases are taken to share their family's
/// (inferred).
pub fn method(d: &Device) -> stp::Method {
    match d
        .global
        .iter()
        .find_map(|e| e.text.strip_prefix("spanning-tree pathcost method "))
    {
        Some("long") => stp::Method::Long,
        Some("short") => stp::Method::Short,
        _ if d.stack.classic() => stp::Method::Short,
        _ => stp::Method::Long,
    }
}

fn has(d: &Device, line: &str) -> bool {
    d.global.iter().any(|e| e.text == line)
}

/// VLANs `no spanning-tree vlan <list>` takes spanning tree off.
fn disabled(d: &Device) -> BTreeSet<u32> {
    d.global
        .iter()
        .filter_map(|e| e.text.strip_prefix("no spanning-tree vlan "))
        .filter(|l| !l.contains(' '))
        .flat_map(crate::generate::vlan_ids)
        .collect()
}

/// A global per-VLAN setting (`spanning-tree vlan 1,10 priority 4096`): the last line naming the
/// VLAN.
fn vlan_setting(d: &Device, v: u32, setting: &str) -> Option<u32> {
    d.global
        .iter()
        .rev()
        .filter_map(|e| {
            let mut w = e
                .text
                .strip_prefix("spanning-tree vlan ")?
                .split_whitespace();
            let list = w.next()?;
            (w.next()? == setting && crate::generate::vlan_ids(list).contains(&v))
                .then(|| w.next()?.parse().ok())
                .flatten()
        })
        .next()
}

/// An interface's per-VLAN setting (`spanning-tree vlan 10 cost 100`), else its plain one
/// (`spanning-tree cost 100`).
fn port_setting(lines: &[&str], v: u32, setting: &str) -> Option<u32> {
    let per_vlan = lines.iter().find_map(|l| {
        let mut w = l.strip_prefix("spanning-tree vlan ")?.split_whitespace();
        let list = w.next()?;
        (w.next()? == setting && crate::generate::vlan_ids(list).contains(&v))
            .then(|| w.next()?.parse().ok())
            .flatten()
    });
    per_vlan.or_else(|| {
        value(lines, &format!("spanning-tree {setting} "))?
            .parse()
            .ok()
    })
}

/// The member a port belongs to (`GigabitEthernet2/0/1` is member 2); a two-part name's is 1.
fn member_of(name: &str) -> u32 {
    let num = &name[name
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(name.len())..];
    let parts: Vec<&str> = num.split('/').collect();
    if parts.len() == 3 {
        parts[0].parse().unwrap_or(1)
    } else {
        1
    }
}

/// The port number in a port ID, where IOS is known to print it. IOS XE: a member's ports in order,
/// front ports, slot 1's listing and the application port (the lab C9300-24T, 17.6.4: Ap1/0/1 is 41),
/// each member after the first 168 on (the 9300 command reference's example: Fif2/0/17 is 128.185);
/// Port-channel n is 3048 plus n (the lab C9300, Po1 and Po2). Classic IOS: a two-part port's place
/// (the lab 2960G, Gi0/1 and Gi0/2), a stack member's 56 on (the 2960 15.2(2)E configuration guide's
/// example: Gi4/0/1 is 128.169). Other models and members than those measured are inferred; a
/// Catalyst 9400 and a classic port-channel have none. `ports`: the front-panel ports in IOS order,
/// the management port left out.
pub fn port_number(d: &Device, ports: &[String], name: &str) -> Option<u16> {
    let classic = d.stack.classic();
    if let Some(n) = name.strip_prefix("Port-channel") {
        let n: u16 = n.parse().ok()?;
        return (!classic).then_some(3048 + n);
    }
    if d.stack.family() == Some(crate::platform::Family::C9400) {
        return None;
    }
    let m = member_of(name);
    let place = ports
        .iter()
        .filter(|p| member_of(p) == m)
        .position(|p| p == name)?;
    let stride = if classic { 56 } else { 168 };
    u16::try_from((m - 1) * stride + place as u32 + 1).ok()
}

impl Spanning {
    /// The lab with `local` in its own place, or `local` alone outside a lab.
    pub fn of(local: &Device, view: Option<&LabView>) -> Spanning {
        let (devices, names, me): (Vec<&Device>, Vec<String>, usize) = match view {
            Some(v) => (
                v.switches
                    .iter()
                    .enumerate()
                    .map(|(i, (_, d))| if i == v.me { local } else { d })
                    .collect(),
                v.switches.iter().map(|(n, _)| n.clone()).collect(),
                v.me,
            ),
            None => (vec![local], vec![local.hostname.clone()], 0),
        };
        let cables: &[[CableEnd; 2]] = view.map_or(&[], |v| &v.cables[..]);
        // a link runs while both its ends do: a port shut at one end takes the other down
        let mut upset: BTreeSet<(usize, String)> = BTreeSet::new();
        for (sw, d) in devices.iter().enumerate() {
            for p in d.links.keys() {
                if crate::show::up(d, p).is_some() {
                    upset.insert((sw, p.clone()));
                }
            }
        }
        for [a, b] in cables {
            if let (CableEnd::Port(x, px), CableEnd::Port(y, py)) = (a, b) {
                let (ka, kb) = ((*x, px.clone()), (*y, py.clone()));
                if !upset.contains(&ka) || !upset.contains(&kb) {
                    upset.remove(&ka);
                    upset.remove(&kb);
                }
            }
        }
        let mut graph = stp::Graph::default();
        let mut ports: Vec<Logical> = vec![];
        let mut index: BTreeMap<(usize, String), usize> = BTreeMap::new();
        for (sw, d) in devices.iter().enumerate() {
            let mode = mode(d);
            let mut defined = crate::show::defined_vlans(d);
            defined.insert(1);
            let off = disabled(d);
            let b = graph.bridges.len();
            let mut vlans_here: BTreeSet<u32> = BTreeSet::new();
            // (name, members, mbps, trunk, half)
            let mut logical: Vec<(String, Vec<String>, u32, bool, bool)> = vec![];
            let mut channels: BTreeMap<u32, (Vec<String>, u32, bool, bool)> = BTreeMap::new();
            let physical: Vec<String> = d
                .stack
                .physical()
                .into_iter()
                .filter(|p| Some(p.as_str()) != d.stack.management())
                .collect();
            for p in physical.clone() {
                let c = d.config_of(&p);
                if c.contains(&"no switchport") {
                    continue;
                }
                // the application hosting port runs with no cable (the lab C9300, 17.6.4)
                if p.starts_with("AppGigabitEthernet") {
                    if !c.contains(&"shutdown") {
                        let trunk = c.contains(&"switchport mode trunk");
                        logical.push((p.clone(), vec![p.clone()], 1000, trunk, false));
                    }
                    continue;
                }
                if !upset.contains(&(sw, p.clone())) {
                    continue;
                }
                let Some(l) = d.links.get(&p) else { continue };
                if let Some((n, _)) = crate::show::channel_of(d, &p) {
                    if l.bundled {
                        if let Ok(n) = n.parse::<u32>() {
                            let e = channels.entry(n).or_insert((vec![], 0, l.trunk, l.half));
                            e.0.push(p.clone());
                            e.1 += crate::show::link_mbps(&p, l);
                        }
                        continue;
                    }
                    if crate::show::suspended(d, &p) {
                        continue;
                    }
                }
                logical.push((
                    p.clone(),
                    vec![p.clone()],
                    crate::show::link_mbps(&p, l),
                    l.trunk,
                    l.half,
                ));
            }
            for (n, (members, mbps, trunk, half)) in channels {
                let name = format!("Port-channel{n}");
                if !d.config_of(&name).contains(&"shutdown") {
                    logical.push((name, members, mbps, trunk, half));
                }
            }
            for (name, members, mbps, trunk, half) in logical {
                let l = d.config_of(&name);
                let vlans: BTreeSet<u32> = if trunk {
                    value(&l, "switchport trunk allowed vlan ")
                        .map_or_else(|| (1..=4094).collect(), crate::generate::vlan_ids)
                        .into_iter()
                        .filter(|v| defined.contains(v))
                        .collect()
                } else {
                    let v: u32 = value(&l, "switchport access vlan ")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1);
                    defined.contains(&v).then_some(v).into_iter().collect()
                };
                let vlans: BTreeSet<u32> = vlans.difference(&off).copied().collect();
                vlans_here.extend(&vlans);
                let pf = crate::lab::portfast(d, &l, trunk);
                let number = port_number(d, &physical, &name);
                let i = graph.ports.len();
                graph.ports.push(stp::Port {
                    bridge: b,
                    number: number.unwrap_or(u16::try_from(i + 1).unwrap_or(u16::MAX)),
                    mbps,
                    instances: vlans
                        .iter()
                        .filter_map(|&v| {
                            Some((
                                Instance::try_from(v).ok()?,
                                stp::PortInstance {
                                    priority: port_setting(&l, v, "port-priority")
                                        .and_then(|x| u8::try_from(x).ok())
                                        .unwrap_or(128),
                                    cost: port_setting(&l, v, "cost"),
                                },
                            ))
                        })
                        .collect(),
                    filter: l.contains(&"spanning-tree bpdufilter enable"),
                    guard: crate::lab::bpdu_guard(d, &l, pf),
                    portfast: pf,
                    // `spanning-tree link-type` overrides what the duplex implies (the command
                    // reference)
                    shared: match value(&l, "spanning-tree link-type ") {
                        Some("shared") => true,
                        Some("point-to-point") => false,
                        _ => half,
                    },
                });
                for m in &members {
                    index.insert((sw, m.clone()), i);
                }
                ports.push(Logical {
                    sw,
                    name,
                    members,
                    numbered: number.is_some(),
                });
            }
            graph.bridges.push(stp::Bridge {
                mac: d.mac,
                protocol: if mode == "rapid-pvst" {
                    stp::Protocol::RapidPvst
                } else {
                    stp::Protocol::Pvst
                },
                method: method(d),
                instances: vlans_here
                    .iter()
                    .filter_map(|&v| {
                        let priority = vlan_setting(d, v, "priority").unwrap_or(32768) + v;
                        let t = stp::Timers::default();
                        Some((
                            Instance::try_from(v).ok()?,
                            stp::BridgeInstance {
                                priority,
                                timers: stp::Timers {
                                    hello: vlan_setting(d, v, "hello-time").unwrap_or(t.hello),
                                    max_age: vlan_setting(d, v, "max-age").unwrap_or(t.max_age),
                                    forward_delay: vlan_setting(d, v, "forward-time")
                                        .unwrap_or(t.forward_delay),
                                },
                            },
                        ))
                    })
                    .collect(),
                // MST is phase 2: its trees are not computed yet
                unmodelled: mode == "mst",
            });
        }
        for [a, b] in cables {
            if let (CableEnd::Port(x, px), CableEnd::Port(y, py)) = (a, b)
                && let (Some(&i), Some(&j)) =
                    (index.get(&(*x, px.clone())), index.get(&(*y, py.clone())))
                && i != j
                && !graph
                    .edges
                    .iter()
                    .any(|&(p, q)| (p, q) == (i, j) || (p, q) == (j, i))
            {
                graph.edges.push((i, j));
            }
        }
        let converged = stp::converge(&graph);
        Spanning {
            graph,
            converged,
            ports,
            me,
            names,
        }
    }

    /// The local switch's instances, by VLAN.
    fn trees(&self) -> Vec<(Instance, &stp::Tree)> {
        self.converged
            .trees
            .iter()
            .filter(|((b, _), _)| *b == self.me)
            .map(|((_, i), t)| (*i, t))
            .collect()
    }

    /// The physical ports of the local switch BPDU guard err-disabled, and those whose link went down
    /// with a far end it err-disabled.
    fn local_ports(&self, set: &BTreeSet<usize>) -> BTreeSet<String> {
        set.iter()
            .filter(|&&p| self.ports[p].sw == self.me)
            .flat_map(|&p| self.ports[p].members.clone())
            .collect()
    }

    /// Links that filter BPDUs at both ends and close a loop, as lab findings.
    pub fn filter_findings(&self) -> Vec<String> {
        stp::filtered_loops(&self.graph, &self.converged)
            .into_iter()
            .map(|(p, q, v)| {
                let at = |x: usize| {
                    format!(
                        "{} {}",
                        self.names[self.ports[x].sw],
                        short(&self.ports[x].name)
                    )
                };
                format!(
                    "{} - {}: BPDU filter at both ends: the link closes a loop in VLAN {v} and neither end blocks",
                    at(p),
                    at(q)
                )
            })
            .collect()
    }
}

/// The local device as the lab leaves it: ports BPDU guard err-disabled marked so, and every port whose
/// link they took down without its link.
pub fn effective<'d>(local: &'d Device, view: Option<&LabView>) -> Cow<'d, Device> {
    if view.is_none() || local.links.is_empty() {
        return Cow::Borrowed(local);
    }
    let sp = Spanning::of(local, view);
    let err = sp.local_ports(&sp.converged.errdisabled);
    let down = sp.local_ports(&sp.converged.down);
    if err.is_empty() && down.is_empty() {
        return Cow::Borrowed(local);
    }
    let mut d = local.clone();
    for p in err.iter().chain(&down) {
        d.links.remove(p);
    }
    d.errdisabled = err;
    Cow::Owned(d)
}

/// Whether a local port (a port-channel by its own name) forwards VLAN `v`: true where spanning tree
/// does not run the VLAN on this switch.
pub fn forwards(sp: &Spanning, port: &str, v: u32) -> bool {
    let Some(i) = sp
        .ports
        .iter()
        .position(|l| l.sw == sp.me && l.name == port)
    else {
        return true;
    };
    let Ok(v) = Instance::try_from(v) else {
        return true;
    };
    match sp.converged.trees.get(&(sp.me, v)) {
        Some(t) => t.ports.get(&i).is_none_or(|s| s.forwarding),
        None => true,
    }
}

fn no_model(c: &str) -> Vec<String> {
    vec![format!("% vswitch has no output model for '{c}' yet")]
}

fn vlan_name(v: Instance) -> String {
    format!("VLAN{v:04}")
}

/// The Type column: `P2p`, or `Shr` on half duplex (inferred), with `Edge` for PortFast that hears no
/// BPDU and `Peer(STP)` for a Rapid PVST port facing PVST (inferred); every row ends with a space.
fn kind(s: &stp::PortState) -> String {
    let mut t = if s.shared { "Shr" } else { "P2p" }.to_string();
    if s.edge {
        t.push_str(" Edge");
    }
    if s.peer_stp {
        t.push_str(" Peer(STP)");
    }
    t
}

fn row(first: &str, s: &stp::PortState) -> String {
    let id = format!("{:>3}.{}", s.priority, s.number);
    format!(
        "{first:<19} {} {} {:<9} {id:<8} {} ",
        s.role.short(),
        if s.forwarding { "FWD" } else { "BLK" },
        s.cost,
        kind(s)
    )
}

/// The address aging time the block prints: `mac address-table aging-time` for the VLAN, else the
/// switch's, else 300 (the lab C9300, 17.6.4, converged; that it follows the setting is inferred).
fn aging(d: &Device, v: Instance) -> u32 {
    let mut global = None;
    for e in &d.global {
        let Some(rest) = e.text.strip_prefix("mac address-table aging-time ") else {
            continue;
        };
        let mut w = rest.split_whitespace();
        let Some(Ok(n)) = w.next().map(str::parse::<u32>) else {
            continue;
        };
        match (w.next(), w.next()) {
            (Some("vlan"), Some(list))
                if n > 0 && crate::generate::vlan_ids(list).contains(&u32::from(v)) =>
            {
                return n;
            }
            (None, _) => global = Some(n),
            _ => {}
        }
    }
    global.filter(|&n| n > 0).unwrap_or(300)
}

fn hello(t: &stp::Timers) -> String {
    format!(
        "Hello Time {:>3} sec  Max Age {:>2} sec  Forward Delay {:>2} sec",
        t.hello, t.max_age, t.forward_delay
    )
}

/// One VLAN's block of `show spanning-tree`, two blank lines after it; `None` where a port number it
/// prints is not known.
fn block(sp: &Spanning, d: &Device, v: Instance, t: &stp::Tree) -> Option<Vec<String>> {
    let protocol = if mode(d) == "rapid-pvst" {
        "rstp"
    } else {
        "ieee"
    };
    let pad = "             ";
    let mut out = vec![
        String::new(),
        vlan_name(v),
        format!("  Spanning tree enabled protocol {protocol}"),
        format!("  Root ID    Priority    {}", t.root.priority),
        format!("{pad}Address     {}", t.root.mac),
    ];
    match t.root_port {
        // a bridge that is not the root names its path cost and root port (the lab 2960G; IOS XE's
        // block is taken to be the same, inferred)
        Some(rp) if t.root != t.own => {
            let l = &sp.ports[rp];
            if !l.numbered {
                return None;
            }
            out.push(format!("{pad}Cost        {}", t.cost));
            out.push(format!(
                "{pad}Port        {} ({})",
                sp.graph.ports[rp].number, l.name
            ));
        }
        _ => out.push(format!("{pad}This bridge is the root")),
    }
    out.push(format!("{pad}{}", hello(&t.root_timers)));
    out.push(String::new());
    let base = t.own.priority - u32::from(v);
    out.push(format!(
        "  Bridge ID  Priority    {:<6} (priority {base} sys-id-ext {v})",
        t.own.priority
    ));
    out.push(format!("{pad}Address     {}", t.own.mac));
    out.push(format!("{pad}{}", hello(&t.timers)));
    out.push(format!("{pad}Aging Time  {:<3} sec", aging(d, v)));
    out.push(String::new());
    out.push("Interface           Role Sts Cost      Prio.Nbr Type".into());
    out.push(
        "------------------- ---- --- --------- -------- --------------------------------".into(),
    );
    let mut rows: Vec<(u16, String)> = vec![];
    for (&p, s) in &t.ports {
        if !sp.ports[p].numbered {
            return None;
        }
        rows.push((s.number, row(&short(&sp.ports[p].name), s)));
    }
    rows.sort();
    out.extend(rows.into_iter().map(|(_, r)| r));
    out.push(String::new());
    out.push(String::new());
    Some(out)
}

/// `show spanning-tree summary`'s settings: IOS XE pads its labels to two widths (the lab C9300,
/// 17.6.4, as the 9300 command reference prints them); classic IOS to one, `EtherChannel misconfig
/// guard` after `Loopguard Default` (the lab 2960G, 15.0(2)SE11).
fn settings(d: &Device) -> Vec<String> {
    let on = |b: bool| if b { "enabled" } else { "disabled" };
    let either = |a: &str, b: &str| has(d, a) || has(d, b);
    let rapid = mode(d) == "rapid-pvst";
    // the command reference's 9300 example prints BackboneFast `enabled but inactive in rapid-pvst
    // mode`; UplinkFast is taken to read the same (inferred)
    let fast = |b: bool| {
        if b && rapid {
            "enabled but inactive in rapid-pvst mode"
        } else {
            on(b)
        }
    };
    let portfast = either(
        "spanning-tree portfast default",
        "spanning-tree portfast edge default",
    );
    let guard = either(
        "spanning-tree portfast bpduguard default",
        "spanning-tree portfast edge bpduguard default",
    );
    let filter = either(
        "spanning-tree portfast bpdufilter default",
        "spanning-tree portfast edge bpdufilter default",
    );
    let loopguard = has(d, "spanning-tree loopguard default");
    let misconfig = !has(d, "no spanning-tree etherchannel guard misconfig");
    let uplinkfast = d
        .global
        .iter()
        .any(|e| e.text.starts_with("spanning-tree uplinkfast"));
    let backbonefast = has(d, "spanning-tree backbonefast");
    if d.stack.classic() {
        let l = |name: &str, v: &str| format!("{name:<28} is {v}");
        vec![
            l("Extended system ID", "enabled"),
            l("Portfast Default", on(portfast)),
            l("PortFast BPDU Guard Default", on(guard)),
            l("Portfast BPDU Filter Default", on(filter)),
            l("Loopguard Default", on(loopguard)),
            l("EtherChannel misconfig guard", on(misconfig)),
            l("UplinkFast", fast(uplinkfast)),
            l("BackboneFast", fast(backbonefast)),
        ]
    } else {
        let wide = |name: &str, v: &str| format!("{name:<39} is {v}");
        let narrow = |name: &str, v: &str| format!("{name:<38} is {v}");
        vec![
            wide("EtherChannel misconfig guard", on(misconfig)),
            wide("Extended system ID", "enabled"),
            wide("Portfast Default", on(portfast)),
            narrow("PortFast BPDU Guard Default", on(guard)),
            narrow("Portfast BPDU Filter Default", on(filter)),
            narrow("Loopguard Default", on(loopguard)),
            wide("UplinkFast", fast(uplinkfast)),
            wide("BackboneFast", fast(backbonefast)),
        ]
    }
}

fn method_name(d: &Device) -> &'static str {
    match method(d) {
        stp::Method::Long => "long",
        stp::Method::Short => "short",
    }
}

fn summary(sp: &Spanning, d: &Device, totals: bool) -> Vec<String> {
    let trees = sp.trees();
    let roots: Vec<String> = trees
        .iter()
        .filter(|(_, t)| t.root == t.own)
        .map(|(v, _)| vlan_name(*v))
        .collect();
    let mut out = vec![
        format!("Switch is in {} mode", mode(d)),
        // one line however many VLANs (inferred: no capture roots more than three)
        format!(
            "Root bridge for: {}",
            if roots.is_empty() {
                "none".to_string()
            } else {
                roots.join(", ")
            }
        ),
    ];
    out.extend(settings(d));
    out.push(format!(
        "Configured Pathcost method used is {}",
        method_name(d)
    ));
    out.push(String::new());
    out.push("Name                   Blocking Listening Learning Forwarding STP Active".into());
    let rule = "---------------------- -------- --------- -------- ---------- ----------";
    out.push(rule.into());
    let line = |name: &str, blk: usize, fwd: usize| {
        format!(
            "{name:<22}{blk:>8}{:>10}{:>9}{fwd:>11}{:>11}",
            0,
            0,
            blk + fwd
        )
    };
    // `summary totals` keeps the header and the total line alone (inferred: no capture holds it)
    let (mut blk, mut fwd) = (0, 0);
    for (v, t) in &trees {
        let f = t.ports.values().filter(|s| s.forwarding).count();
        let b = t.ports.len() - f;
        blk += b;
        fwd += f;
        if !totals {
            out.push(line(&vlan_name(*v), b, f));
        }
    }
    if !totals {
        out.push(rule.into());
    }
    // `1 vlan` in the singular (the lab 2960G and the 9300 command reference's example)
    let n = trees.len();
    let count = if n == 1 {
        "1 vlan".to_string()
    } else {
        format!("{n} vlans")
    };
    out.push(line(&count, blk, fwd));
    out
}

fn root(sp: &Spanning) -> Vec<String> {
    let mut out = vec![
        String::new(),
        "                                        Root    Hello Max Fwd".to_string(),
        "Vlan                   Root ID          Cost    Time  Age Dly  Root Port".to_string(),
        "---------------- -------------------- --------- ----- --- ---  ------------".to_string(),
    ];
    for (v, t) in sp.trees() {
        let port = t
            .root_port
            .map(|p| short(&sp.ports[p].name))
            .unwrap_or_default();
        let r = &t.root_timers;
        out.push(format!(
            "{:<16}{:>6} {}{:>10}{:>5}{:>5}{:>4}  {port:<16}",
            vlan_name(v),
            t.root.priority,
            t.root.mac,
            t.cost,
            r.hello,
            r.max_age,
            r.forward_delay
        ));
    }
    out
}

fn bridge(sp: &Spanning, d: &Device) -> Vec<String> {
    let protocol = if mode(d) == "rapid-pvst" {
        "rstp"
    } else {
        "ieee"
    };
    let mut out = vec![
        String::new(),
        "                                                   Hello  Max  Fwd".to_string(),
        "Vlan                         Bridge ID              Time  Age  Dly  Protocol".to_string(),
        "---------------- --------------------------------- -----  ---  ---  --------".to_string(),
    ];
    for (v, t) in sp.trees() {
        let base = t.own.priority - u32::from(v);
        out.push(format!(
            "{:<16}{:>6} ({base:>5},{v:>4}) {}{:>5}{:>6}{:>5}  {protocol:<12}",
            vlan_name(v),
            t.own.priority,
            t.own.mac,
            t.timers.hello,
            t.timers.max_age,
            t.timers.forward_delay
        ));
    }
    out
}

fn blocked(sp: &Spanning) -> Vec<String> {
    let mut out = vec![
        String::new(),
        "Name                 Blocked Interfaces List".to_string(),
        "-------------------- ------------------------------------".to_string(),
    ];
    let mut n = 0;
    for (v, t) in sp.trees() {
        let mut list: Vec<(u16, String)> = t
            .ports
            .iter()
            .filter(|(_, s)| !s.forwarding)
            .map(|(&p, s)| (s.number, short(&sp.ports[p].name)))
            .collect();
        if list.is_empty() {
            continue;
        }
        list.sort();
        n += list.len();
        let names: Vec<String> = list.into_iter().map(|(_, p)| p).collect();
        // one line however many ports (inferred)
        out.push(format!("{:<20} {}", vlan_name(v), names.join(", ")));
    }
    out.push(String::new());
    out.push(format!(
        "Number of blocked ports (segments) in the system : {n}"
    ));
    out.push(String::new());
    out
}

/// `show spanning-tree interface <port>`: its row in each VLAN, or with `portfast` whether PortFast is
/// set on it in each.
fn interface(sp: &Spanning, c: &str, words: &[&str]) -> Vec<String> {
    let Some((ty, num, used)) =
        crate::cli::interface_name(words.first().copied().unwrap_or(""), words.get(1).copied())
    else {
        return no_model(c);
    };
    let name = format!("{ty}{num}");
    let rest = &words[used..];
    let Some(i) = sp
        .ports
        .iter()
        .position(|l| l.sw == sp.me && l.name.eq_ignore_ascii_case(&name))
    else {
        return no_model(c);
    };
    let states: Vec<(Instance, stp::PortState)> = sp
        .trees()
        .into_iter()
        .filter_map(|(v, t)| Some((v, *t.ports.get(&i)?)))
        .collect();
    if states.is_empty() {
        return no_model(c);
    }
    match rest {
        [] => {
            if !sp.ports[i].numbered {
                return no_model(c);
            }
            let mut out = vec![
                String::new(),
                "Vlan                Role Sts Cost      Prio.Nbr Type".to_string(),
                "------------------- ---- --- --------- -------- --------------------------------"
                    .to_string(),
            ];
            out.extend(states.iter().map(|(v, s)| row(&vlan_name(*v), s)));
            out
        }
        // PortFast as configured, whether or not BPDUs keep the port from being an edge (the lab
        // C9300: `enabled` on a port that hears its own switch)
        [p] if "portfast".starts_with(&p.to_ascii_lowercase()) => {
            let pf = sp.graph.ports[i].portfast;
            states
                .iter()
                .map(|(v, _)| {
                    format!(
                        "{:<20}{}",
                        vlan_name(*v),
                        if pf { "enabled" } else { "disabled" }
                    )
                })
                .collect()
        }
        _ => no_model(c),
    }
}

/// What a `show spanning-tree ...` line prints (`c`, written out in full), or `None` for any other
/// command. MST and the forms that print counters, topology changes or ages have no output model yet.
pub fn show(local: &Device, view: Option<&LabView>, c: &str) -> Option<Vec<String>> {
    let rest = c.strip_prefix("show spanning-tree")?;
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let words: Vec<&str> = rest.split_whitespace().collect();
    if mode(local) == "mst" {
        return Some(no_model(c));
    }
    let sp = Spanning::of(local, view);
    let all = |vlans: Option<BTreeSet<u32>>| -> Vec<String> {
        let mut out = vec![];
        for (v, t) in sp.trees() {
            if vlans.as_ref().is_some_and(|s| !s.contains(&u32::from(v))) {
                continue;
            }
            match block(&sp, local, v, t) {
                Some(b) => out.extend(b),
                None => return no_model(c),
            }
        }
        if out.is_empty() { no_model(c) } else { out }
    };
    let out = match words[..] {
        [] | ["active"] => all(None),
        ["vlan", list] => all(Some(crate::generate::vlan_ids(list).into_iter().collect())),
        ["summary"] => summary(&sp, local, false),
        ["summary", "totals"] => summary(&sp, local, true),
        ["root"] => root(&sp),
        ["bridge"] => bridge(&sp, local),
        ["blockedports"] => blocked(&sp),
        // no inconsistency is modelled: root guard, loop guard and PVID mismatches are not
        ["inconsistentports"] => vec![
            String::new(),
            "Name                 Interface                      Inconsistency".into(),
            "-------------------- ------------------------------ ------------------".into(),
            String::new(),
            "Number of inconsistent ports (segments) in the system : 0".into(),
            String::new(),
        ],
        ["pathcost", "method"] => vec![format!(
            "Spanning tree default pathcost method used is {}",
            method_name(local)
        )],
        ["interface", ref port @ ..] => interface(&sp, c, port),
        _ => no_model(c),
    };
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Engine;
    use crate::release::Release;
    use crate::{check, grammars, lab, platform, release, shell};

    fn engine() -> Engine {
        Engine::new(grammars::grammar("c9300", release::DEFAULT).unwrap())
    }

    fn classic() -> Engine {
        Engine::new(grammars::grammar("c2960", Release::parse("15.0(2)SE").unwrap()).unwrap())
    }

    fn mac(last: u8) -> stp::Mac {
        stp::Mac([0x00, 0x11, 0x22, 0x33, 0x44, last])
    }

    /// A switch typed from its configuration, back at `#`, with a synthetic base MAC.
    fn switch<'a>(e: &'a Engine, stack: &str, config: &str, last: u8) -> shell::Session<'a> {
        let st = platform::parse_stack(stack).unwrap();
        let (mut s, typed) = check::type_config(e, st, config);
        let refused: Vec<_> = typed.iter().filter(|t| t.error.is_some()).collect();
        assert!(refused.is_empty(), "{refused:?}");
        s.type_line("end");
        s.device.mac = mac(last);
        s
    }

    /// Switches `a`, `b`, ... cabled as listed.
    fn lab<'a>(
        e: &'a Engine,
        switches: &[(&str, &str, u8)],
        cables: &[(&str, &str)],
    ) -> (Vec<lab::Built<'a>>, lab::Report) {
        let mut built: Vec<lab::Built> = switches
            .iter()
            .enumerate()
            .map(|(i, (stack, config, last))| lab::Built {
                name: ((b'a' + i as u8) as char).to_string(),
                session: switch(e, stack, config, *last),
                typed: 0,
                refused: vec![],
            })
            .collect();
        let cables: Vec<lab::Cable> = cables
            .iter()
            .map(|(a, b)| lab::Cable {
                a: a.to_string(),
                b: b.to_string(),
                optic: None,
                speed: None,
                half: false,
            })
            .collect();
        let r = lab::connect(&mut built, &cables).unwrap();
        (built, r)
    }

    fn shown(s: &mut shell::Session<'_>, line: &str) -> Vec<String> {
        s.type_line(line).0
    }

    fn rows(out: &[String]) -> Vec<&str> {
        out.iter()
            .map(String::as_str)
            .filter(|l| l.contains(" FWD ") || l.contains(" BLK "))
            .collect()
    }

    const RAPID: &str = "spanning-tree mode rapid-pvst\n";
    const T: &str = "C9300-24T";

    #[test]
    fn a_c9300_alone_runs_vlan_1_on_its_application_port_and_with_it_shut_runs_none() {
        let e = engine();
        let mut s = switch(&e, T, RAPID, 1);
        assert_eq!(
            shown(&mut s, "show spanning-tree"),
            [
                "",
                "VLAN0001",
                "  Spanning tree enabled protocol rstp",
                "  Root ID    Priority    32769",
                "             Address     0011.2233.4401",
                "             This bridge is the root",
                "             Hello Time   2 sec  Max Age 20 sec  Forward Delay 15 sec",
                "",
                "  Bridge ID  Priority    32769  (priority 32768 sys-id-ext 1)",
                "             Address     0011.2233.4401",
                "             Hello Time   2 sec  Max Age 20 sec  Forward Delay 15 sec",
                "             Aging Time  300 sec",
                "",
                "Interface           Role Sts Cost      Prio.Nbr Type",
                "------------------- ---- --- --------- -------- --------------------------------",
                "Ap1/0/1             Desg FWD 20000     128.41   P2p ",
                "",
                "",
            ]
        );
        assert_eq!(
            shown(&mut s, "show spanning-tree pathcost method"),
            ["Spanning tree default pathcost method used is long"]
        );
        let mut s = switch(
            &e,
            T,
            &format!("{RAPID}interface AppGigabitEthernet1/0/1\nshutdown\n"),
            1,
        );
        assert_eq!(
            shown(&mut s, "show spanning-tree"),
            ["% vswitch has no output model for 'show spanning-tree' yet"]
        );
    }

    #[test]
    fn a_redundant_pair_blocks_the_far_bridge_s_second_port_and_names_its_root_port() {
        let e = engine();
        let (mut b, r) = lab(
            &e,
            &[(T, RAPID, 1), (T, RAPID, 2)],
            &[("a:Gi1/0/1", "b:Gi1/0/1"), ("a:Gi1/0/2", "b:Gi1/0/2")],
        );
        assert!(r.findings.is_empty(), "{r:?}");
        let s = &mut b[1].session;
        let out = shown(s, "show spanning-tree");
        assert_eq!(
            out[3..7],
            [
                "  Root ID    Priority    32769",
                "             Address     0011.2233.4401",
                "             Cost        20000",
                "             Port        1 (GigabitEthernet1/0/1)",
            ]
        );
        assert_eq!(
            rows(&out),
            [
                "Gi1/0/1             Root FWD 20000     128.1    P2p ",
                "Gi1/0/2             Altn BLK 20000     128.2    P2p ",
                "Ap1/0/1             Desg FWD 20000     128.41   P2p ",
            ]
        );
        assert_eq!(
            shown(s, "show spanning-tree root")[4],
            "VLAN0001         32769 0011.2233.4401     20000    2   20  15  Gi1/0/1         "
        );
        assert_eq!(
            shown(s, "show spanning-tree blockedports")[3..6],
            [
                "VLAN0001             Gi1/0/2",
                "",
                "Number of blocked ports (segments) in the system : 1",
            ]
        );
        assert_eq!(
            shown(s, "show spanning-tree interface Gi1/0/2"),
            [
                "",
                "Vlan                Role Sts Cost      Prio.Nbr Type",
                "------------------- ---- --- --------- -------- --------------------------------",
                "VLAN0001            Altn BLK 20000     128.2    P2p ",
            ]
        );
        let a = &mut b[0].session;
        assert_eq!(
            rows(&shown(a, "show spanning-tree"))[..2],
            [
                "Gi1/0/1             Desg FWD 20000     128.1    P2p ",
                "Gi1/0/2             Desg FWD 20000     128.2    P2p ",
            ]
        );
    }

    #[test]
    fn a_priority_cost_or_shutdown_typed_in_the_shell_changes_the_tree_at_once() {
        let e = engine();
        let (mut b, _) = lab(
            &e,
            &[(T, RAPID, 1), (T, RAPID, 2)],
            &[("a:Gi1/0/1", "b:Gi1/0/1"), ("a:Gi1/0/2", "b:Gi1/0/2")],
        );
        let s = &mut b[1].session;
        s.type_line("configure terminal");
        s.type_line("spanning-tree vlan 1 priority 4096");
        s.type_line("end");
        let out = shown(s, "show spanning-tree");
        assert_eq!(out[5], "             This bridge is the root");
        assert!(
            rows(&out).iter().all(|r| r.contains("Desg FWD")),
            "{out:#?}"
        );
        s.type_line("configure terminal");
        s.type_line("no spanning-tree vlan 1 priority 4096");
        s.type_line("interface GigabitEthernet1/0/1");
        s.type_line("spanning-tree cost 50000");
        s.type_line("end");
        assert_eq!(
            rows(&shown(s, "show spanning-tree"))[..2],
            [
                "Gi1/0/1             Altn BLK 50000     128.1    P2p ",
                "Gi1/0/2             Root FWD 20000     128.2    P2p ",
            ]
        );
        s.type_line("configure terminal");
        s.type_line("interface GigabitEthernet1/0/2");
        s.type_line("shutdown");
        s.type_line("end");
        let out = shown(s, "show spanning-tree");
        assert_eq!(out[6], "             Port        1 (GigabitEthernet1/0/1)");
        assert_eq!(
            rows(&out)[..1],
            ["Gi1/0/1             Root FWD 50000     128.1    P2p "]
        );
    }

    #[test]
    fn a_triangle_blocks_the_highest_bridge_s_port_toward_the_second() {
        let e = engine();
        let (mut b, _) = lab(
            &e,
            &[(T, RAPID, 1), (T, RAPID, 2), (T, RAPID, 3)],
            &[
                ("a:Gi1/0/1", "b:Gi1/0/1"),
                ("b:Gi1/0/2", "c:Gi1/0/2"),
                ("c:Gi1/0/1", "a:Gi1/0/2"),
            ],
        );
        assert_eq!(
            rows(&shown(&mut b[1].session, "show spanning-tree"))[..2],
            [
                "Gi1/0/1             Root FWD 20000     128.1    P2p ",
                "Gi1/0/2             Desg FWD 20000     128.2    P2p ",
            ]
        );
        assert_eq!(
            rows(&shown(&mut b[2].session, "show spanning-tree"))[..2],
            [
                "Gi1/0/1             Root FWD 20000     128.1    P2p ",
                "Gi1/0/2             Altn BLK 20000     128.2    P2p ",
            ]
        );
    }

    #[test]
    fn a_port_channel_is_one_port_costed_by_its_bundled_speed() {
        let e = engine();
        let po = format!(
            "{RAPID}interface range GigabitEthernet1/0/1 - 2\nchannel-group 1 mode active\n"
        );
        let (mut b, r) = lab(
            &e,
            &[(T, &po, 1), (T, &po, 2)],
            &[("a:Gi1/0/1", "b:Gi1/0/1"), ("a:Gi1/0/2", "b:Gi1/0/2")],
        );
        assert!(r.findings.is_empty(), "{r:?}");
        let out = shown(&mut b[1].session, "show spanning-tree");
        assert_eq!(
            rows(&out),
            [
                "Ap1/0/1             Desg FWD 20000     128.41   P2p ",
                "Po1                 Root FWD 10000     128.3049 P2p ",
            ]
        );
        assert_eq!(out[6], "             Port        3049 (Port-channel1)");
    }

    #[test]
    fn bpdu_filter_at_both_ends_of_a_loop_forwards_both_and_is_a_finding() {
        let e = engine();
        let filter =
            format!("{RAPID}interface GigabitEthernet1/0/2\nspanning-tree bpdufilter enable\n");
        let (mut b, r) = lab(
            &e,
            &[(T, &filter, 1), (T, &filter, 2)],
            &[("a:Gi1/0/1", "b:Gi1/0/1"), ("a:Gi1/0/2", "b:Gi1/0/2")],
        );
        assert!(
            r.findings
                .iter()
                .any(|f| f == "a Gi1/0/2 - b Gi1/0/2: BPDU filter at both ends: the link closes a loop in VLAN 1 and neither end blocks"),
            "{r:?}"
        );
        assert_eq!(
            rows(&shown(&mut b[1].session, "show spanning-tree"))[..2],
            [
                "Gi1/0/1             Root FWD 20000     128.1    P2p ",
                "Gi1/0/2             Desg FWD 20000     128.2    P2p ",
            ]
        );
        // filtering one end only is no finding
        let (_, r) = lab(
            &e,
            &[(T, &filter, 1), (T, RAPID, 2)],
            &[("a:Gi1/0/1", "b:Gi1/0/1"), ("a:Gi1/0/2", "b:Gi1/0/2")],
        );
        assert!(
            !r.findings.iter().any(|f| f.contains("BPDU filter")),
            "{r:?}"
        );
    }

    #[test]
    fn bpdu_guard_err_disables_the_port_and_its_far_end_goes_notconnect() {
        let e = engine();
        let guard =
            format!("{RAPID}interface GigabitEthernet1/0/3\nspanning-tree bpduguard enable\n");
        let (mut b, _) = lab(
            &e,
            &[(T, &guard, 1), (T, RAPID, 2)],
            &[("a:Gi1/0/3", "b:Gi1/0/3")],
        );
        let status = |s: &mut shell::Session<'_>| -> String {
            shown(s, "show interfaces status")
                .into_iter()
                .find(|l| l.starts_with("Gi1/0/3 "))
                .unwrap()
        };
        assert_eq!(
            status(&mut b[0].session),
            "Gi1/0/3                         err-disabled 1            auto   auto 10/100/1000BaseTX"
        );
        assert_eq!(
            status(&mut b[1].session),
            "Gi1/0/3                         notconnect   1            auto   auto 10/100/1000BaseTX"
        );
        for x in b.iter_mut() {
            let s = &mut x.session;
            assert_eq!(
                rows(&shown(s, "show spanning-tree")),
                ["Ap1/0/1             Desg FWD 20000     128.41   P2p "]
            );
        }
    }

    #[test]
    fn the_trunk_table_lists_only_the_vlans_a_port_forwards() {
        let e = engine();
        let trunks = "vlan 10\ninterface range GigabitEthernet1/0/1 - 2\nswitchport mode trunk\nswitchport trunk allowed vlan 1,10\n";
        let (mut b, _) = lab(
            &e,
            &[
                (T, &format!("{RAPID}{trunks}"), 1),
                (
                    T,
                    &format!("{RAPID}spanning-tree vlan 10 priority 4096\n{trunks}"),
                    2,
                ),
            ],
            &[("a:Gi1/0/1", "b:Gi1/0/1"), ("a:Gi1/0/2", "b:Gi1/0/2")],
        );
        let out = shown(&mut b[1].session, "show interfaces trunk");
        let at = out
            .iter()
            .position(|l| l.contains("spanning tree forwarding"))
            .unwrap();
        assert_eq!(out[at + 1..], ["Gi1/0/1        1,10", "Gi1/0/2        10"]);
        let out = shown(&mut b[0].session, "show interfaces trunk");
        let at = out
            .iter()
            .position(|l| l.contains("spanning tree forwarding"))
            .unwrap();
        assert_eq!(out[at + 1..], ["Gi1/0/1        1,10", "Gi1/0/2        1"]);
    }

    #[test]
    fn pvst_prints_ieee_and_a_rapid_pvst_port_facing_it_prints_peer_stp() {
        let e = engine();
        let (mut b, _) = lab(
            &e,
            &[(T, "spanning-tree mode pvst\n", 1), (T, RAPID, 2)],
            &[("a:Gi1/0/1", "b:Gi1/0/1")],
        );
        let out = shown(&mut b[0].session, "show spanning-tree");
        assert_eq!(out[2], "  Spanning tree enabled protocol ieee");
        let out = shown(&mut b[1].session, "show spanning-tree");
        assert_eq!(
            rows(&out)[0],
            "Gi1/0/1             Root FWD 20000     128.1    P2p Peer(STP) "
        );
        assert_eq!(
            shown(&mut b[0].session, "show spanning-tree summary")[0],
            "Switch is in pvst mode"
        );
    }

    #[test]
    fn ios_xe_and_classic_ios_lay_out_the_summary_each_its_own_way() {
        let e = engine();
        let mut s = switch(&e, T, RAPID, 1);
        assert_eq!(
            shown(&mut s, "show spanning-tree summary"),
            [
                "Switch is in rapid-pvst mode",
                "Root bridge for: VLAN0001",
                "EtherChannel misconfig guard            is enabled",
                "Extended system ID                      is enabled",
                "Portfast Default                        is disabled",
                "PortFast BPDU Guard Default            is disabled",
                "Portfast BPDU Filter Default           is disabled",
                "Loopguard Default                      is disabled",
                "UplinkFast                              is disabled",
                "BackboneFast                            is disabled",
                "Configured Pathcost method used is long",
                "",
                "Name                   Blocking Listening Learning Forwarding STP Active",
                "---------------------- -------- --------- -------- ---------- ----------",
                "VLAN0001                     0         0        0          1          1",
                "---------------------- -------- --------- -------- ---------- ----------",
                "1 vlan                       0         0        0          1          1",
            ]
        );
        let c = classic();
        let g = "WS-C2960G-8TC-L";
        let (mut b, _) = lab(
            &c,
            &[(g, "", 0x10), (g, "", 0x20)],
            &[("a:Gi0/1", "b:Gi0/1"), ("a:Gi0/2", "b:Gi0/2")],
        );
        let s = &mut b[1].session;
        assert_eq!(
            shown(s, "show spanning-tree"),
            [
                "",
                "VLAN0001",
                "  Spanning tree enabled protocol ieee",
                "  Root ID    Priority    32769",
                "             Address     0011.2233.4410",
                "             Cost        4",
                "             Port        1 (GigabitEthernet0/1)",
                "             Hello Time   2 sec  Max Age 20 sec  Forward Delay 15 sec",
                "",
                "  Bridge ID  Priority    32769  (priority 32768 sys-id-ext 1)",
                "             Address     0011.2233.4420",
                "             Hello Time   2 sec  Max Age 20 sec  Forward Delay 15 sec",
                "             Aging Time  300 sec",
                "",
                "Interface           Role Sts Cost      Prio.Nbr Type",
                "------------------- ---- --- --------- -------- --------------------------------",
                "Gi0/1               Root FWD 4         128.1    P2p ",
                "Gi0/2               Altn BLK 4         128.2    P2p ",
                "",
                "",
            ]
        );
        assert_eq!(
            shown(s, "show spanning-tree summary"),
            [
                "Switch is in pvst mode",
                "Root bridge for: none",
                "Extended system ID           is enabled",
                "Portfast Default             is disabled",
                "PortFast BPDU Guard Default  is disabled",
                "Portfast BPDU Filter Default is disabled",
                "Loopguard Default            is disabled",
                "EtherChannel misconfig guard is enabled",
                "UplinkFast                   is disabled",
                "BackboneFast                 is disabled",
                "Configured Pathcost method used is short",
                "",
                "Name                   Blocking Listening Learning Forwarding STP Active",
                "---------------------- -------- --------- -------- ---------- ----------",
                "VLAN0001                     1         0        0          1          2",
                "---------------------- -------- --------- -------- ---------- ----------",
                "1 vlan                       1         0        0          1          2",
            ]
        );
        assert_eq!(
            shown(s, "show spanning-tree root")[4],
            "VLAN0001         32769 0011.2233.4410         4    2   20  15  Gi0/1           "
        );
    }

    #[test]
    fn a_shell_edit_reaches_the_bridge_table_and_root_primary_is_stored_as_its_priority() {
        let e = engine();
        let mut s = switch(&e, T, RAPID, 1);
        s.type_line("configure terminal");
        s.type_line("vlan 10");
        s.type_line("exit");
        s.type_line("spanning-tree vlan 1 root primary");
        s.type_line("interface AppGigabitEthernet1/0/1");
        s.type_line("end");
        assert!(
            show::running_config(&s.device)
                .contains(&"spanning-tree vlan 1 priority 24576".to_string())
        );
        assert_eq!(
            shown(&mut s, "show spanning-tree bridge")[4],
            "VLAN0001         24577 (24576,   1) 0011.2233.4401    2    20   15  rstp        "
        );
    }

    #[test]
    fn the_vtp_device_id_is_the_base_mac() {
        let e = engine();
        let mut s = switch(&e, T, RAPID, 0x2a);
        assert!(
            shown(&mut s, "show vtp status")
                .contains(&"Device ID                       : 0011.2233.442a".to_string())
        );
    }

    #[test]
    fn link_type_and_address_aging_reach_the_block_and_an_mst_neighbour_trips_bpdu_guard() {
        let e = engine();
        let config = format!(
            "{RAPID}mac address-table aging-time 600\ninterface AppGigabitEthernet1/0/1\nspanning-tree link-type shared\n"
        );
        let mut s = switch(&e, T, &config, 1);
        let out = shown(&mut s, "show spanning-tree");
        assert_eq!(out[11], "             Aging Time  600 sec");
        assert_eq!(
            rows(&out),
            ["Ap1/0/1             Desg FWD 20000     128.41   Shr "]
        );
        let guard =
            format!("{RAPID}interface GigabitEthernet1/0/3\nspanning-tree bpduguard enable\n");
        let (mut b, _) = lab(
            &e,
            &[(T, &guard, 1), (T, "spanning-tree mode mst\n", 2)],
            &[("a:Gi1/0/3", "b:Gi1/0/3")],
        );
        assert_eq!(
            rows(&shown(&mut b[0].session, "show spanning-tree")),
            ["Ap1/0/1             Desg FWD 20000     128.41   P2p "]
        );
        assert!(
            shown(&mut b[0].session, "show interfaces status")
                .iter()
                .any(|l| l.starts_with("Gi1/0/3 ") && l.contains("err-disabled"))
        );
    }

    use crate::show;
}
