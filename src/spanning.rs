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
/// device the lab does not model, which sends the BPDUs the lab file declares for it, if any.
#[derive(Clone, Debug, PartialEq)]
pub enum CableEnd {
    Port(usize, String),
    Device(String),
}

/// The lab as `lab::connect` left it: every switch's configuration and links, every cable, and what
/// the devices it does not model send. A session holds it with its own place (`me`), whose
/// configuration it reads live instead.
#[derive(Clone, Debug)]
pub struct LabView {
    pub me: usize,
    pub switches: Arc<Vec<(String, Device)>>,
    pub cables: Arc<Vec<[CableEnd; 2]>>,
    pub heard: Arc<BTreeMap<String, stp::Heard>>,
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

/// The lab's spanning tree: the graph, its converged state, what each graph port is, and the local
/// switch's MST configuration where it runs MST.
pub struct Spanning {
    pub graph: stp::Graph,
    pub converged: stp::Converged,
    pub ports: Vec<Logical>,
    pub me: usize,
    pub names: Vec<String>,
    pub mst: Option<Mst>,
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

/// `spanning-tree mst configuration`: the region's name and revision, and each instance's VLANs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mst {
    pub name: String,
    pub revision: u16,
    pub instances: BTreeMap<Instance, BTreeSet<u32>>,
}

impl Mst {
    pub fn of(d: &Device) -> Mst {
        let mut m = Mst::default();
        let Some(b) = d
            .blocks
            .iter()
            .find(|b| b.header == "spanning-tree mst configuration")
        else {
            return m;
        };
        for e in &b.lines {
            let t = e.text.trim();
            if let Some(n) = t.strip_prefix("name ") {
                m.name = n.to_string();
            } else if let Some(r) = t.strip_prefix("revision ") {
                m.revision = r.parse().unwrap_or(0);
            } else if let Some(rest) = t.strip_prefix("instance ") {
                let mut w = rest.splitn(3, ' ');
                if let (Some(Ok(i)), Some("vlan"), Some(list)) =
                    (w.next().map(str::parse::<Instance>), w.next(), w.next())
                {
                    m.instances
                        .entry(i)
                        .or_default()
                        .extend(crate::generate::vlan_ids(list));
                }
            }
        }
        m
    }

    /// The instance a VLAN maps to: an instance listing it, else the CIST.
    pub fn instance_of(&self, v: u32) -> Instance {
        self.instances
            .iter()
            .find(|(_, s)| s.contains(&v))
            .map_or(0, |(i, _)| *i)
    }

    /// An instance's VLANs: the CIST's are every VLAN no other instance lists.
    pub fn vlans(&self, i: Instance) -> BTreeSet<u32> {
        if i == 0 {
            (1..=4094).filter(|v| self.instance_of(*v) == 0).collect()
        } else {
            self.instances
                .get(&i)
                .map(|s| {
                    s.iter()
                        .copied()
                        .filter(|v| (1..=4094).contains(v))
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    pub fn region(&self) -> stp::Region {
        let map: BTreeMap<u16, Instance> = self
            .instances
            .iter()
            .flat_map(|(&i, s)| {
                s.iter()
                    .filter_map(move |&v| Some((u16::try_from(v).ok()?, i)))
            })
            .collect();
        stp::Region {
            name: self.name.clone(),
            revision: self.revision,
            digest: stp::mst_digest(&map),
        }
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

/// A global per-instance setting (`spanning-tree vlan 1,10 priority 4096`, `spanning-tree mst 1
/// priority 8192`, `kind` naming `vlan` or `mst`): the last line naming the instance.
fn instance_setting(d: &Device, kind: &str, v: u32, setting: &str) -> Option<u32> {
    let head = format!("spanning-tree {kind} ");
    d.global
        .iter()
        .rev()
        .filter_map(|e| {
            let mut w = e.text.strip_prefix(&head)?.split_whitespace();
            let list = w.next()?;
            (w.next()? == setting && crate::generate::vlan_ids(list).contains(&v))
                .then(|| w.next()?.parse().ok())
                .flatten()
        })
        .next()
}

fn vlan_setting(d: &Device, v: u32, setting: &str) -> Option<u32> {
    instance_setting(d, "vlan", v, setting)
}

/// A global MST setting (`spanning-tree mst max-hops 30`).
fn mst_setting(d: &Device, setting: &str) -> Option<u32> {
    d.global.iter().find_map(|e| {
        e.text
            .strip_prefix(&format!("spanning-tree mst {setting} "))?
            .parse()
            .ok()
    })
}

/// An interface's per-instance setting (`spanning-tree vlan 10 cost 100`, `spanning-tree mst 1 cost
/// 100`), else its plain one (`spanning-tree cost 100`) where `plain` allows it.
fn port_setting(lines: &[&str], kind: &str, v: u32, setting: &str, plain: bool) -> Option<u32> {
    let head = format!("spanning-tree {kind} ");
    let per = lines.iter().find_map(|l| {
        let mut w = l.strip_prefix(&head)?.split_whitespace();
        let list = w.next()?;
        (w.next()? == setting && crate::generate::vlan_ids(list).contains(&v))
            .then(|| w.next()?.parse().ok())
            .flatten()
    });
    per.or_else(|| {
        plain
            .then(|| {
                value(lines, &format!("spanning-tree {setting} "))?
                    .parse()
                    .ok()
            })
            .flatten()
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
/// Port-channel n is 3048 plus n (the lab C9300, Po1, Po2 and Po10). Classic IOS: a two-part port's
/// place (the lab 2960G, Gi0/1 and Gi0/2), a stack member's 56 on (the 2960 15.2(2)E configuration
/// guide's example: Gi4/0/1 is 128.169); a two-part switch's Port-channel n is 63 plus n (the lab
/// 2960G, Po1 is 64). Other models and members than those measured are inferred; a Catalyst 9400 and
/// a stacking classic switch's port-channel have none. `ports`: the front-panel ports in IOS order,
/// the management port left out.
pub fn port_number(d: &Device, ports: &[String], name: &str) -> Option<u16> {
    let classic = d.stack.classic();
    if let Some(n) = name.strip_prefix("Port-channel") {
        let n: u16 = n.parse().ok()?;
        return if !classic {
            Some(3048 + n)
        } else if d.stack.family().is_some_and(|f| f.classic_two_part()) {
            Some(63 + n)
        } else {
            None
        };
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

/// A physical port's speed (Mb/s) for its cost: its `speed` where set (the lab C9300 and 2960G: 100
/// costs 19 short, 10 costs 2000000 long), else its link's.
fn port_mbps(d: &Device, p: &str, l: &crate::device::Link) -> u32 {
    value(&d.config_of(p), "speed ")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| crate::show::link_mbps(p, l))
}

/// Whether a port runs half duplex: its link's, or `duplex half` set on it (the lab C9300, Type
/// `Shr`).
fn port_half(d: &Device, p: &str, l: &crate::device::Link) -> bool {
    l.half || d.config_of(p).contains(&"duplex half")
}

/// A logical port before it joins the graph: its name, members, speed, whether it trunks and
/// whether it runs half duplex.
struct Candidate {
    name: String,
    members: Vec<String>,
    mbps: u32,
    trunk: bool,
    half: bool,
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
        // what the devices the lab does not model send, by the port they reach
        let mut heard: BTreeMap<(usize, String), stp::Heard> = BTreeMap::new();
        for [a, b] in cables {
            if let (CableEnd::Port(x, px), CableEnd::Port(y, py)) = (a, b) {
                let (ka, kb) = ((*x, px.clone()), (*y, py.clone()));
                if !upset.contains(&ka) || !upset.contains(&kb) {
                    upset.remove(&ka);
                    upset.remove(&kb);
                }
            }
            for (e, f) in [(a, b), (b, a)] {
                if let (CableEnd::Port(x, px), CableEnd::Device(name)) = (e, f)
                    && let Some(h) = view.and_then(|v| v.heard.get(name))
                {
                    heard.insert((*x, px.clone()), h.clone());
                }
            }
        }
        let mut graph = stp::Graph::default();
        let mut ports: Vec<Logical> = vec![];
        let mut index: BTreeMap<(usize, String), usize> = BTreeMap::new();
        for (sw, d) in devices.iter().enumerate() {
            let mode = mode(d);
            let mst = (mode == "mst").then(|| Mst::of(d));
            let mut defined = crate::show::defined_vlans(d);
            defined.insert(1);
            let off = if mst.is_some() {
                BTreeSet::new()
            } else {
                disabled(d)
            };
            let b = graph.bridges.len();
            let mut here: BTreeSet<Instance> = BTreeSet::new();
            let mut logical: Vec<Candidate> = vec![];
            let mut channels: BTreeMap<u32, Candidate> = BTreeMap::new();
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
                        logical.push(Candidate {
                            name: p.clone(),
                            members: vec![p.clone()],
                            mbps: 1000,
                            trunk: c.contains(&"switchport mode trunk"),
                            half: false,
                        });
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
                            let e = channels.entry(n).or_insert(Candidate {
                                name: format!("Port-channel{n}"),
                                members: vec![],
                                mbps: 0,
                                trunk: l.trunk,
                                half: port_half(d, &p, l),
                            });
                            e.members.push(p.clone());
                            e.mbps += port_mbps(d, &p, l);
                        }
                        continue;
                    }
                    if crate::show::suspended(d, &p) {
                        continue;
                    }
                }
                logical.push(Candidate {
                    name: p.clone(),
                    members: vec![p.clone()],
                    mbps: port_mbps(d, &p, l),
                    trunk: l.trunk,
                    half: port_half(d, &p, l),
                });
            }
            for (_, ch) in channels {
                if !d.config_of(&ch.name).contains(&"shutdown") {
                    logical.push(ch);
                }
            }
            let loopguard = has(d, "spanning-tree loopguard default");
            for Candidate {
                name,
                members,
                mbps,
                trunk,
                half,
            } in logical
            {
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
                let untagged: u32 = if trunk {
                    l.iter()
                        .filter_map(|x| x.strip_prefix("switchport trunk native vlan "))
                        .find_map(|v| v.parse().ok())
                        .unwrap_or(1)
                } else {
                    value(&l, "switchport access vlan ")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1)
                };
                // the instances the port runs: each VLAN under PVST; the CIST and every MSTI one of
                // its VLANs maps to under MST (the lab 2960G: an access port in an MSTI's VLAN runs
                // both)
                let instances: BTreeMap<Instance, stp::PortInstance> = match &mst {
                    Some(m) => {
                        let mut ids: BTreeSet<Instance> = BTreeSet::new();
                        if !vlans.is_empty() {
                            ids.insert(0);
                        }
                        ids.extend(vlans.iter().map(|&v| m.instance_of(v)));
                        // the CIST takes the plain cost and port priority (the lab C9300, 17.6.4);
                        // an MSTI only its own (inferred)
                        ids.into_iter()
                            .map(|i| {
                                let v = u32::from(i);
                                (
                                    i,
                                    stp::PortInstance {
                                        priority: port_setting(
                                            &l,
                                            "mst",
                                            v,
                                            "port-priority",
                                            i == 0,
                                        )
                                        .and_then(|x| u8::try_from(x).ok())
                                        .unwrap_or(128),
                                        cost: port_setting(&l, "mst", v, "cost", i == 0),
                                    },
                                )
                            })
                            .collect()
                    }
                    None => vlans
                        .iter()
                        .filter_map(|&v| {
                            Some((
                                Instance::try_from(v).ok()?,
                                stp::PortInstance {
                                    priority: port_setting(&l, "vlan", v, "port-priority", true)
                                        .and_then(|x| u8::try_from(x).ok())
                                        .unwrap_or(128),
                                    cost: port_setting(&l, "vlan", v, "cost", true),
                                },
                            ))
                        })
                        .collect(),
                };
                here.extend(instances.keys());
                let pf = crate::lab::portfast(d, &l, trunk);
                let number = port_number(d, &physical, &name);
                let guard = value(&l, "spanning-tree guard ");
                let i = graph.ports.len();
                graph.ports.push(stp::Port {
                    bridge: b,
                    number: number.unwrap_or(u16::try_from(i + 1).unwrap_or(u16::MAX)),
                    mbps,
                    instances,
                    vlans: vlans
                        .iter()
                        .filter_map(|&v| u16::try_from(v).ok())
                        .collect(),
                    trunk,
                    untagged: u16::try_from(untagged).unwrap_or(1),
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
                    root_guard: guard == Some("root"),
                    // `loopguard default` guards every port that is not an edge port and sets no
                    // guard of its own (the command reference; inferred, not measured)
                    loop_guard: guard == Some("loop") || loopguard && guard.is_none() && !pf,
                    heard: members
                        .iter()
                        .find_map(|m| heard.get(&(sw, m.clone())))
                        .cloned(),
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
            let t = stp::Timers::default();
            let bridge = match &mst {
                Some(m) => {
                    let timers = stp::Timers {
                        hello: mst_setting(d, "hello-time").unwrap_or(t.hello),
                        max_age: mst_setting(d, "max-age").unwrap_or(t.max_age),
                        forward_delay: mst_setting(d, "forward-time").unwrap_or(t.forward_delay),
                    };
                    stp::Bridge {
                        mac: d.mac,
                        protocol: stp::Protocol::Mst,
                        method: method(d),
                        // the CIST's priority carries no system ID (the lab C9300: MST0 32768),
                        // an MSTI's its number (MST1 32769)
                        instances: here
                            .iter()
                            .map(|&i| {
                                let base = instance_setting(d, "mst", u32::from(i), "priority")
                                    .unwrap_or(32768);
                                (
                                    i,
                                    stp::BridgeInstance {
                                        priority: base + u32::from(i),
                                        timers,
                                    },
                                )
                            })
                            .collect(),
                        region: Some(m.region()),
                        max_hops: mst_setting(d, "max-hops").unwrap_or(20),
                    }
                }
                None => stp::Bridge {
                    mac: d.mac,
                    protocol: if mode == "rapid-pvst" {
                        stp::Protocol::RapidPvst
                    } else {
                        stp::Protocol::Pvst
                    },
                    method: method(d),
                    instances: here
                        .iter()
                        .map(|&i| {
                            let v = u32::from(i);
                            let priority = vlan_setting(d, v, "priority").unwrap_or(32768) + v;
                            (
                                i,
                                stp::BridgeInstance {
                                    priority,
                                    timers: stp::Timers {
                                        hello: vlan_setting(d, v, "hello-time").unwrap_or(t.hello),
                                        max_age: vlan_setting(d, v, "max-age").unwrap_or(t.max_age),
                                        forward_delay: vlan_setting(d, v, "forward-time")
                                            .unwrap_or(t.forward_delay),
                                    },
                                },
                            )
                        })
                        .collect(),
                    region: None,
                    max_hops: 20,
                },
            };
            graph.bridges.push(bridge);
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
            mst: (mode(local) == "mst").then(|| Mst::of(local)),
        }
    }

    /// The local switch's instances, in order.
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
                let what = if self.graph.bridges[self.ports[p].sw].protocol == stp::Protocol::Mst {
                    format!("MST{v}")
                } else {
                    format!("VLAN {v}")
                };
                format!(
                    "{} - {}: BPDU filter at both ends: the link closes a loop in {what} and neither end blocks",
                    at(p),
                    at(q)
                )
            })
            .collect()
    }

    /// The instance the local switch runs a VLAN in: the VLAN under PVST, its MST instance under MST.
    fn instance(&self, v: u32) -> Option<Instance> {
        match &self.mst {
            Some(m) => Some(m.instance_of(v)),
            None => Instance::try_from(v).ok(),
        }
    }

    /// An instance's name as the tables print it: `VLAN0010`, `MST1`.
    fn name(&self, i: Instance) -> String {
        if self.mst.is_some() {
            format!("MST{i}")
        } else {
            format!("VLAN{i:04}")
        }
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
    d.errdisabled = err
        .iter()
        .map(|p| (p.clone(), d.links.get(p).and_then(|l| l.optic)))
        .collect();
    for p in err.iter().chain(&down) {
        d.links.remove(p);
    }
    Cow::Owned(d)
}

/// Whether a local port (a port-channel by its own name) forwards VLAN `v`, in the VLAN's own tree or
/// its MST instance's: true where spanning tree does not run it on this switch.
pub fn forwards(sp: &Spanning, port: &str, v: u32) -> bool {
    let Some(i) = sp
        .ports
        .iter()
        .position(|l| l.sw == sp.me && l.name == port)
    else {
        return true;
    };
    let Some(inst) = sp.instance(v) else {
        return true;
    };
    match sp.converged.trees.get(&(sp.me, inst)) {
        Some(t) => t.ports.get(&i).is_none_or(|s| s.forwarding),
        None => true,
    }
}

fn no_model(c: &str) -> Vec<String> {
    vec![format!("% vswitch has no output model for '{c}' yet")]
}

/// The Type column: `P2p`, or `Shr` on half duplex (the lab C9300), with `Edge` for PortFast that
/// hears no BPDU, `Peer(STP)` for a Rapid PVST port hearing 802.1D, `Bound(...)` for an MST port at
/// its region's boundary, and the inconsistency holding a port; every row ends with a space.
fn kind(s: &stp::PortState) -> String {
    let mut t = if s.shared { "Shr" } else { "P2p" }.to_string();
    if s.edge {
        t.push_str(" Edge");
    }
    if let Some(b) = s.bound {
        t.push_str(&format!(" Bound({b})"));
    }
    if s.peer_stp {
        t.push_str(" Peer(STP)");
    }
    if let Some(i) = s.inconsistent {
        t.push(' ');
        t.push_str(i.flag());
    }
    t
}

/// A port's row: `BKN*` for a port a guard or PVST simulation holds, its cost straight after it (the
/// lab C9300, 17.6.4: `Root BKN*10000`).
fn row(first: &str, width: usize, s: &stp::PortState) -> String {
    let id = format!("{:>3}.{}", s.priority, s.number);
    let sts = if s.inconsistent.is_some() {
        "BKN*"
    } else if s.forwarding {
        "FWD "
    } else {
        "BLK "
    };
    format!(
        "{first:<width$} {} {sts}{:<9} {id:<8} {} ",
        s.role.short(),
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

/// A port's rows in a table, in port-number order; `None` where a port number it prints is not known.
fn rows(sp: &Spanning, t: &stp::Tree, width: usize) -> Option<Vec<String>> {
    let mut rows: Vec<(u16, String)> = vec![];
    for (&p, s) in &t.ports {
        if !sp.ports[p].numbered {
            return None;
        }
        rows.push((s.number, row(&short(&sp.ports[p].name), width, s)));
    }
    rows.sort();
    Some(rows.into_iter().map(|(_, r)| r).collect())
}

/// One instance's block of `show spanning-tree`, two blank lines after it; `None` where a port number
/// it prints is not known. MST prints protocol `mstp` and no aging time (the lab C9300 and 2960G).
fn block(sp: &Spanning, d: &Device, v: Instance, t: &stp::Tree) -> Option<Vec<String>> {
    let protocol = match mode(d) {
        "mst" => "mstp",
        "rapid-pvst" => "rstp",
        _ => "ieee",
    };
    let pad = "             ";
    let mut out = vec![
        String::new(),
        sp.name(v),
        format!("  Spanning tree enabled protocol {protocol}"),
        format!("  Root ID    Priority    {}", t.root.priority),
        format!("{pad}Address     {}", t.root.mac),
    ];
    match t.root_port {
        // a bridge that is not the root names its path cost and root port (the lab C9300 and 2960G)
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
    if sp.mst.is_none() {
        out.push(format!("{pad}Aging Time  {:<3} sec", aging(d, v)));
    }
    out.push(String::new());
    out.push("Interface           Role Sts Cost      Prio.Nbr Type".into());
    out.push(
        "------------------- ---- --- --------- -------- --------------------------------".into(),
    );
    out.extend(rows(sp, t, 19)?);
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
    let mode = mode(d);
    // UplinkFast and BackboneFast set under Rapid PVST print `enabled but inactive in rapid-pvst
    // mode` (the lab C9300, 17.6.4); under MST the same with the mode's name (inferred)
    let inactive = format!("enabled but inactive in {mode} mode");
    let fast = |b: bool| {
        if b && mode != "pvst" {
            inactive.clone()
        } else {
            on(b).to_string()
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
            l("UplinkFast", &fast(uplinkfast)),
            l("BackboneFast", &fast(backbonefast)),
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
            wide("UplinkFast", &fast(uplinkfast)),
            wide("BackboneFast", &fast(backbonefast)),
        ]
    }
}

fn method_name(d: &Device) -> &'static str {
    match method(d) {
        stp::Method::Long => "long",
        stp::Method::Short => "short",
    }
}

/// Instance names with each run of consecutive instances as one range (the lab C9300:
/// `VLAN0100-VLAN0139, VLAN0999`, `MST0-MST1`).
fn runs(sp: &Spanning, ids: &[Instance]) -> String {
    let mut out = vec![];
    let mut i = 0;
    while i < ids.len() {
        let mut j = i;
        while j + 1 < ids.len() && ids[j + 1] == ids[j] + 1 {
            j += 1;
        }
        out.push(if j > i {
            format!("{}-{}", sp.name(ids[i]), sp.name(ids[j]))
        } else {
            sp.name(ids[i])
        });
        i = j + 1;
    }
    out.join(", ")
}

fn summary(sp: &Spanning, d: &Device, totals: bool) -> Vec<String> {
    let trees = sp.trees();
    let roots: Vec<Instance> = trees
        .iter()
        .filter(|(_, t)| t.root == t.own)
        .map(|(v, _)| *v)
        .collect();
    let mst = sp.mst.is_some();
    let mut out = vec![
        if mst {
            "Switch is in mst mode (IEEE Standard)".to_string()
        } else {
            format!("Switch is in {} mode", mode(d))
        },
        // one line however many instances (the lab C9300: 41 VLANs in two runs)
        format!(
            "Root bridge for: {}",
            if roots.is_empty() {
                "none".to_string()
            } else {
                runs(sp, &roots)
            }
        ),
    ];
    out.extend(settings(d));
    // MST runs the long method whatever is configured (the lab 2960G, 15.0(2)SE11)
    out.push(if mst && method(d) == stp::Method::Short {
        "Configured Pathcost method used is short (Operational value is long)".to_string()
    } else {
        format!("Configured Pathcost method used is {}", method_name(d))
    });
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
            out.push(line(&sp.name(*v), b, f));
        }
    }
    if !totals {
        out.push(rule.into());
    }
    // `1 vlan` in the singular (the lab 2960G and the 9300 command reference's example); `2 msts`
    // (the lab C9300 and 2960G), `1 mst` taken to follow (inferred)
    let n = trees.len();
    let unit = if mst { "mst" } else { "vlan" };
    let count = if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    };
    out.push(line(&count, blk, fwd));
    out
}

/// The tables' first column header: `Vlan`, or under MST `MST Instance` (`root`, `bridge`) and `Mst
/// Instance` (`interface`), as the lab C9300 and 2960G print them.
fn column(sp: &Spanning, upper: bool) -> &'static str {
    match (sp.mst.is_some(), upper) {
        (false, _) => "Vlan",
        (true, true) => "MST Instance",
        (true, false) => "Mst Instance",
    }
}

fn root(sp: &Spanning) -> Vec<String> {
    let mut out = vec![
        String::new(),
        "                                        Root    Hello Max Fwd".to_string(),
        format!(
            "{:<23}Root ID          Cost    Time  Age Dly  Root Port",
            column(sp, true)
        ),
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
            sp.name(v),
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

fn bridge(sp: &Spanning, d: &Device, only: Option<&BTreeSet<Instance>>) -> Vec<String> {
    let protocol = match mode(d) {
        "mst" => "mstp",
        "rapid-pvst" => "rstp",
        _ => "ieee",
    };
    let mut out = vec![
        String::new(),
        "                                                   Hello  Max  Fwd".to_string(),
        format!(
            "{:<29}Bridge ID              Time  Age  Dly  Protocol",
            column(sp, true)
        ),
        "---------------- --------------------------------- -----  ---  ---  --------".to_string(),
    ];
    for (v, t) in sp.trees() {
        if only.is_some_and(|s| !s.contains(&v)) {
            continue;
        }
        let base = t.own.priority - u32::from(v);
        out.push(format!(
            "{:<16}{:>6} ({base:>5},{v:>4}) {}{:>5}{:>6}{:>5}  {protocol:<12}",
            sp.name(v),
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
        out.push(format!("{:<20} {}", sp.name(v), names.join(", ")));
    }
    out.push(String::new());
    out.push(format!(
        "Number of blocked ports (segments) in the system : {n}"
    ));
    out.push(String::new());
    out
}

/// `show spanning-tree inconsistentports`: each port a guard or PVST simulation holds, by instance,
/// with its full name; the Interface column 30 wide on IOS XE (the lab C9300, 17.6.4), 24 on classic
/// IOS (the lab 2960G, 15.0(2)SE11).
fn inconsistent(sp: &Spanning, d: &Device) -> Vec<String> {
    let w = if d.stack.classic() { 24 } else { 30 };
    let mut out = vec![
        String::new(),
        format!("{:<20} {:<w$} Inconsistency", "Name", "Interface"),
        format!("{} {} {}", "-".repeat(20), "-".repeat(w), "-".repeat(18)),
    ];
    let mut n = 0;
    for (v, t) in sp.trees() {
        let mut list: Vec<(u16, String)> = t
            .ports
            .iter()
            .filter_map(|(&p, s)| {
                Some((
                    s.number,
                    format!(
                        "{:<20} {:<w$} {}",
                        sp.name(v),
                        sp.ports[p].name,
                        s.inconsistent?.name()
                    ),
                ))
            })
            .collect();
        list.sort();
        n += list.len();
        out.extend(list.into_iter().map(|(_, r)| r));
    }
    out.push(String::new());
    out.push(format!(
        "Number of inconsistent ports (segments) in the system : {n}"
    ));
    out.push(String::new());
    out
}

/// A VLAN or MST list as `show spanning-tree mst` and `mst configuration` print it: runs of two or
/// more as ranges (the lab C9300: `1-9,11-4094`; a run of two is inferred).
fn vlan_list(set: &BTreeSet<u32>) -> String {
    let v: Vec<u32> = set.iter().copied().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j + 1 < v.len() && v[j + 1] == v[j] + 1 {
            j += 1;
        }
        out.push(if j > i {
            format!("{}-{}", v[i], v[j])
        } else {
            v[i].to_string()
        });
        i = j + 1;
    }
    out.join(",")
}

/// A bridge ID as `show spanning-tree mst` prints it: its priority and, in brackets, the priority's
/// base and system ID.
fn mst_id(id: &stp::BridgeId) -> String {
    format!(
        "address {}  priority      {:<5} ({} sysid {})",
        id.mac,
        id.priority,
        id.priority & 0xf000,
        id.priority & 0x0fff
    )
}

/// `show spanning-tree mst [<n>]`: each instance's section, laid out as the lab C9300 (17.6.4) and
/// the lab 2960G (15.0(2)SE11) print them; the Interface column 32 wide on IOS XE and 16 on classic
/// IOS. `None` where a port number it prints is not known.
fn mst(sp: &Spanning, d: &Device, only: Option<Instance>) -> Option<Vec<String>> {
    let m = sp.mst.as_ref()?;
    let w = if d.stack.classic() { 16 } else { 32 };
    let mut out = vec![String::new()];
    let mut any = false;
    for (i, t) in sp.trees() {
        if only.is_some_and(|o| o != i) {
            continue;
        }
        any = true;
        out.push(format!(
            "##### {:<8}vlans mapped:   {}",
            format!("MST{i}"),
            vlan_list(&m.vlans(i))
        ));
        out.push(format!("Bridge        {}", mst_id(&t.own)));
        let port = t
            .root_port
            .map(|p| short(&sp.ports[p].name))
            .unwrap_or_default();
        if i == 0 {
            if t.root == t.own {
                out.push("Root          this switch for the CIST".into());
            } else {
                out.push(format!("Root          {}", mst_id(&t.root)));
                out.push(format!(
                    "              port    {port:<16}path cost     {:<9}",
                    t.external
                ));
                if t.regional == t.own {
                    out.push("Regional Root this switch".into());
                } else {
                    out.push(format!("Regional Root {}", mst_id(&t.regional)));
                    out.push(format!(
                        "{:38}internal cost {:<9} rem hops {}",
                        "", t.internal, t.remaining
                    ));
                }
            }
            let r = &t.root_timers;
            out.push(format!(
                "Operational   hello time {} , forward delay {}, max age {}, txholdcount 6 ",
                r.hello, r.forward_delay, r.max_age
            ));
            let o = &t.timers;
            let hops = sp.graph.bridges[sp.me].max_hops;
            out.push(format!(
                "Configured    hello time {} , forward delay {}, max age {}, max hops {hops:>5}",
                o.hello, o.forward_delay, o.max_age
            ));
        } else if t.root == t.own {
            out.push(format!("Root          this switch for MST{i}"));
        } else {
            out.push(format!("Root          {}", mst_id(&t.root)));
            out.push(format!(
                "              port    {port:<16}cost          {:<9} rem hops {}",
                t.internal, t.remaining
            ));
        }
        out.push(String::new());
        out.push(format!(
            "{:<w$} Role Sts Cost      Prio.Nbr Type",
            "Interface"
        ));
        out.push(format!(
            "{:<w$} ---- --- --------- -------- --------------------------------",
            "-".repeat(16)
        ));
        out.extend(rows(sp, t, w)?);
        out.push(String::new());
    }
    any.then_some(out)
}

/// `show spanning-tree mst configuration`, as the lab C9300 (17.6.4) and 2960G (15.0(2)SE11) print
/// it; each instance's VLANs on one line (inferred for a list longer than measured).
fn mst_configuration(d: &Device) -> Vec<String> {
    let m = Mst::of(d);
    let mut out = vec![
        format!("Name      [{}]", m.name),
        format!(
            "Revision  {:<6}Instances configured {}",
            m.revision,
            1 + m.instances.keys().filter(|&&i| i != 0).count()
        ),
        String::new(),
        "Instance  Vlans mapped".into(),
        format!("--------  {}", "-".repeat(69)),
    ];
    let ids: BTreeSet<Instance> = std::iter::once(0)
        .chain(m.instances.keys().copied())
        .collect();
    for i in ids {
        out.push(format!("{:<10}{}", i, vlan_list(&m.vlans(i))));
    }
    out.push("-".repeat(79));
    out
}

/// `show spanning-tree interface <port>`: its row in each instance, or with `portfast` whether
/// PortFast is set on it in each. A port of the switch that runs no instance prints IOS's message
/// (the lab C9300, 17.6.4: `no spanning tree info available for GigabitEthernet1/0/10 `).
fn interface(sp: &Spanning, d: &Device, c: &str, words: &[&str]) -> Vec<String> {
    let Some((ty, num, used)) =
        crate::cli::interface_name(words.first().copied().unwrap_or(""), words.get(1).copied())
    else {
        return no_model(c);
    };
    let name = format!("{ty}{num}");
    let rest = &words[used..];
    let exists = d
        .stack
        .physical()
        .iter()
        .any(|p| p.eq_ignore_ascii_case(&name))
        || d.blocks
            .iter()
            .any(|b| b.header.eq_ignore_ascii_case(&format!("interface {name}")));
    let states: Vec<(Instance, stp::PortState)> = sp
        .ports
        .iter()
        .position(|l| l.sw == sp.me && l.name.eq_ignore_ascii_case(&name))
        .map(|i| {
            sp.trees()
                .into_iter()
                .filter_map(|(v, t)| Some((v, *t.ports.get(&i)?)))
                .collect()
        })
        .unwrap_or_default();
    if states.is_empty() {
        return if exists && rest.is_empty() {
            vec![
                format!("no spanning tree info available for {name} "),
                String::new(),
            ]
        } else {
            no_model(c)
        };
    }
    let i = sp
        .ports
        .iter()
        .position(|l| l.sw == sp.me && l.name.eq_ignore_ascii_case(&name))
        .unwrap_or_default();
    match rest {
        [] => {
            if !sp.ports[i].numbered {
                return no_model(c);
            }
            let mut out = vec![
                String::new(),
                format!("{:<20}Role Sts Cost      Prio.Nbr Type", column(sp, false)),
                "------------------- ---- --- --------- -------- --------------------------------"
                    .to_string(),
            ];
            out.extend(states.iter().map(|(v, s)| row(&sp.name(*v), 19, s)));
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
                        sp.name(*v),
                        if pf { "enabled" } else { "disabled" }
                    )
                })
                .collect()
        }
        _ => no_model(c),
    }
}

/// `show spanning-tree [vlan <list>]`: each instance's block, or for a list the instances it maps to;
/// a list with none prints IOS's message (the lab C9300, 17.6.4: `vlan 20`).
fn blocks(sp: &Spanning, d: &Device, c: &str, list: Option<&str>) -> Vec<String> {
    // under MST a VLAN the switch does not define has no instance to print (inferred, as PVST's)
    let defined = crate::show::defined_vlans(d);
    let exists =
        |v: &u32| sp.mst.is_none() || *v == 1 || (1002..=1005).contains(v) || defined.contains(v);
    let only: Option<BTreeSet<Instance>> = list.map(|l| {
        crate::generate::vlan_ids(l)
            .into_iter()
            .filter(exists)
            .filter_map(|v| sp.instance(v))
            .collect()
    });
    let mut out = vec![];
    for (v, t) in sp.trees() {
        if only.as_ref().is_some_and(|s| !s.contains(&v)) {
            continue;
        }
        match block(sp, d, v, t) {
            Some(b) => out.extend(b),
            None => return no_model(c),
        }
    }
    match (out.is_empty(), list) {
        (true, Some(l)) => vec![
            String::new(),
            format!("Spanning tree instance(s) for vlan {l} does not exist."),
            String::new(),
        ],
        (true, None) => no_model(c),
        (false, _) => out,
    }
}

/// What a `show spanning-tree ...` line prints (`c`, written out in full), or `None` for any other
/// command. The forms that print counters, topology changes or ages, and the MST digests, have no
/// output model.
pub fn show(local: &Device, view: Option<&LabView>, c: &str) -> Option<Vec<String>> {
    let rest = c.strip_prefix("show spanning-tree")?;
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let words: Vec<&str> = rest.split_whitespace().collect();
    let sp = Spanning::of(local, view);
    let mst_mode = sp.mst.is_some();
    let out = match words[..] {
        [] | ["active"] => blocks(&sp, local, c, None),
        ["vlan", list] => blocks(&sp, local, c, Some(list)),
        // the bridge table for the instances a list maps to (inferred: no capture holds it)
        ["vlan", list, "bridge"] => {
            let only: BTreeSet<Instance> = crate::generate::vlan_ids(list)
                .into_iter()
                .filter_map(|v| sp.instance(v))
                .collect();
            bridge(&sp, local, Some(&only))
        }
        ["summary"] => summary(&sp, local, false),
        ["summary", "totals"] => summary(&sp, local, true),
        ["root"] => root(&sp),
        ["bridge"] => bridge(&sp, local, None),
        ["blockedports"] => blocked(&sp),
        ["inconsistentports"] => inconsistent(&sp, local),
        ["pathcost", "method"] => vec![format!(
            "Spanning tree default pathcost method used is {}",
            method_name(local)
        )],
        ["interface", ref port @ ..] => interface(&sp, local, c, port),
        // the region as configured, whatever the mode (inferred outside MST mode)
        ["mst", "configuration"] => mst_configuration(local),
        ["mst"] if mst_mode => mst(&sp, local, None).unwrap_or_else(|| no_model(c)),
        ["mst", n] if mst_mode => n
            .parse::<Instance>()
            .ok()
            .and_then(|n| mst(&sp, local, Some(n)))
            .unwrap_or_else(|| no_model(c)),
        _ => no_model(c),
    };
    Some(out)
}

/// The priority `spanning-tree vlan <n> root primary` sets in VLAN `v`: 24576 where that makes the
/// switch the root, else the highest multiple of 4096 that beats the current root (the lab C9300,
/// 17.6.4: 4096 against a root at 4097 with a higher MAC; the command reference). `root secondary`
/// sets 28672 (the lab C9300).
pub fn root_priority(local: &Device, view: Option<&LabView>, v: u32, primary: bool) -> u32 {
    if !primary {
        return 28672;
    }
    let sp = Spanning::of(local, view);
    let Some(t) = Instance::try_from(v)
        .ok()
        .filter(|_| sp.mst.is_none())
        .and_then(|i| sp.converged.trees.get(&(sp.me, i)))
    else {
        return 24576;
    };
    if t.root == t.own {
        return 24576;
    }
    (0..=6u32)
        .rev()
        .map(|k| k * 4096)
        .find(|&p| {
            stp::BridgeId {
                priority: p + v,
                mac: local.mac,
            } < t.root
        })
        .unwrap_or(0)
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
        assert_eq!(
            shown(&mut b[0].session, "show interfaces status err-disabled"),
            [
                "Port         Name         Status       Reason               Err-disabled Vlans",
                "",
                "Gi1/0/3                   err-disabled bpduguard",
            ]
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

    const REGION: &str = "spanning-tree mode mst\nspanning-tree mst configuration\nname LAB\nrevision 3\ninstance 1 vlan 10\nexit\nvlan 10\ninterface range GigabitEthernet1/0/1 - 2\nswitchport mode trunk\n";

    #[test]
    fn one_mst_region_prints_its_instances_and_its_configuration() {
        let e = engine();
        let (mut b, _) = lab(
            &e,
            &[(T, REGION, 1), (T, REGION, 2)],
            &[("a:Gi1/0/1", "b:Gi1/0/1"), ("a:Gi1/0/2", "b:Gi1/0/2")],
        );
        let s = &mut b[1].session;
        assert_eq!(
            shown(s, "show running-config | section mst"),
            [
                "spanning-tree mode mst",
                "spanning-tree mst configuration",
                " name LAB",
                " revision 3",
                " instance 1 vlan 10",
            ]
        );
        assert_eq!(
            shown(s, "show spanning-tree mst configuration"),
            [
                "Name      [LAB]",
                "Revision  3     Instances configured 2",
                "",
                "Instance  Vlans mapped",
                "--------  ---------------------------------------------------------------------",
                "0         1-9,11-4094",
                "1         10",
                "-------------------------------------------------------------------------------",
            ]
        );
        assert_eq!(
            shown(s, "show spanning-tree mst 1"),
            [
                "",
                "##### MST1    vlans mapped:   10",
                "Bridge        address 0011.2233.4402  priority      32769 (32768 sysid 1)",
                "Root          address 0011.2233.4401  priority      32769 (32768 sysid 1)",
                "              port    Gi1/0/1         cost          20000     rem hops 19",
                "",
                "Interface                        Role Sts Cost      Prio.Nbr Type",
                "----------------                 ---- --- --------- -------- --------------------------------",
                "Gi1/0/1                          Root FWD 20000     128.1    P2p ",
                "Gi1/0/2                          Altn BLK 20000     128.2    P2p ",
                "",
            ]
        );
        let mst = shown(s, "show spanning-tree mst");
        assert_eq!(
            mst[2..7],
            [
                "Bridge        address 0011.2233.4402  priority      32768 (32768 sysid 0)",
                "Root          address 0011.2233.4401  priority      32768 (32768 sysid 0)",
                "              port    Gi1/0/1         path cost     0        ",
                "Regional Root address 0011.2233.4401  priority      32768 (32768 sysid 0)",
                "                                      internal cost 20000     rem hops 19",
            ]
        );
        let sum = shown(s, "show spanning-tree summary");
        assert_eq!(sum[0], "Switch is in mst mode (IEEE Standard)");
        assert_eq!(
            sum[sum.len() - 1],
            "2 msts                       2         0        0          3          5"
        );
        // VLAN 10 runs in MST1; VLAN 20 is not defined
        assert_eq!(shown(s, "show spanning-tree vlan 10")[1], "MST1");
        assert_eq!(
            shown(s, "show spanning-tree vlan 20"),
            [
                "",
                "Spanning tree instance(s) for vlan 20 does not exist.",
                ""
            ]
        );
        assert_eq!(
            shown(s, "show spanning-tree mst configuration digest"),
            ["% vswitch has no output model for 'show spanning-tree mst configuration digest' yet"]
        );
    }

    #[test]
    fn root_guard_and_loop_guard_hold_the_port_inconsistent() {
        let e = engine();
        let guarded = |kind: &str| {
            format!("{RAPID}interface GigabitEthernet1/0/1\nspanning-tree guard {kind}\n")
        };
        let filtered =
            format!("{RAPID}interface GigabitEthernet1/0/1\nspanning-tree bpdufilter enable\n");
        for (a, kind, flag, name) in [
            (RAPID.to_string(), "root", "*ROOT_Inc", "Root Inconsistent"),
            (filtered, "loop", "*LOOP_Inc", "Loop Inconsistent"),
        ] {
            let (mut b, _) = lab(
                &e,
                &[(T, &a, 1), (T, &guarded(kind), 2)],
                &[("a:Gi1/0/1", "b:Gi1/0/1")],
            );
            let s = &mut b[1].session;
            let out = shown(s, "show spanning-tree");
            assert!(
                out.contains(&format!(
                    "Gi1/0/1             Desg BKN*20000     128.1    P2p {flag} "
                )),
                "{kind}: {out:?}"
            );
            assert_eq!(
                shown(s, "show spanning-tree inconsistentports")[3..],
                [
                    format!("VLAN0001             GigabitEthernet1/0/1           {name}"),
                    String::new(),
                    "Number of inconsistent ports (segments) in the system : 1".to_string(),
                    String::new(),
                ]
            );
        }
    }

    #[test]
    fn short_costs_a_fixed_speed_and_half_duplex_reach_the_port_rows() {
        let e = engine();
        let short = format!(
            "{RAPID}spanning-tree pathcost method short\ninterface GigabitEthernet1/0/1\nspeed 100\nduplex half\n"
        );
        let (mut b, _) = lab(
            &e,
            &[(T, RAPID, 1), (T, &short, 2)],
            &[("a:Gi1/0/1", "b:Gi1/0/1"), ("a:Gi1/0/2", "b:Gi1/0/2")],
        );
        let s = &mut b[1].session;
        assert_eq!(
            rows(&shown(s, "show spanning-tree")),
            [
                "Gi1/0/1             Altn BLK 19        128.1    Shr ",
                "Gi1/0/2             Root FWD 4         128.2    P2p ",
                "Ap1/0/1             Desg FWD 4         128.41   P2p ",
            ]
        );
        assert_eq!(
            shown(s, "show spanning-tree pathcost method"),
            ["Spanning tree default pathcost method used is short"]
        );
    }

    #[test]
    fn root_primary_beats_the_current_root_and_vlan_settings_are_kept_per_vlan() {
        let e = engine();
        let root = format!("{RAPID}spanning-tree vlan 1 priority 4096\n");
        let (mut b, _) = lab(
            &e,
            &[(T, &root, 1), (T, RAPID, 2)],
            &[("a:Gi1/0/1", "b:Gi1/0/1")],
        );
        let s = &mut b[1].session;
        for l in [
            "configure terminal",
            "spanning-tree vlan 1 root primary",
            "spanning-tree vlan 10 root secondary",
            "spanning-tree vlan 1,10 forward-time 10",
            "no spanning-tree vlan 10 forward-time",
            "end",
        ] {
            s.type_line(l);
        }
        let mut lines = shown(s, "show running-config | include spanning-tree vlan");
        lines.sort();
        assert_eq!(
            lines,
            [
                "spanning-tree vlan 1 forward-time 10",
                "spanning-tree vlan 1 priority 0",
                "spanning-tree vlan 10 priority 28672",
            ]
        );
    }

    #[test]
    fn the_messages_for_a_port_and_a_vlan_spanning_tree_does_not_run() {
        let e = engine();
        let mut s = switch(&e, T, RAPID, 1);
        assert_eq!(
            shown(&mut s, "show spanning-tree interface Gi1/0/5"),
            [
                "no spanning tree info available for GigabitEthernet1/0/5 ",
                ""
            ]
        );
        assert_eq!(
            shown(&mut s, "show spanning-tree vlan 20"),
            [
                "",
                "Spanning tree instance(s) for vlan 20 does not exist.",
                ""
            ]
        );
    }

    #[test]
    fn a_c9300_and_a_2960g_number_their_lacp_channels_3058_and_64() {
        let (xe, c) = (engine(), classic());
        let mut built = vec![
            lab::Built {
                name: "a".into(),
                session: switch(
                    &xe,
                    T,
                    &format!(
                        "{RAPID}interface range GigabitEthernet1/0/1 - 2\nchannel-group 10 mode active\n"
                    ),
                    1,
                ),
                typed: 0,
                refused: vec![],
            },
            lab::Built {
                name: "b".into(),
                session: switch(
                    &c,
                    "WS-C2960G-8TC-L",
                    "spanning-tree mode rapid-pvst\ninterface Port-channel1\ninterface range GigabitEthernet0/1 - 2\nswitchport mode trunk\nchannel-group 1 mode active\n",
                    2,
                ),
                typed: 0,
                refused: vec![],
            },
        ];
        let cables: Vec<lab::Cable> = [("a:Gi1/0/1", "b:Gi0/1"), ("a:Gi1/0/2", "b:Gi0/2")]
            .iter()
            .map(|(a, b)| lab::Cable {
                a: a.to_string(),
                b: b.to_string(),
                optic: None,
                speed: None,
                half: false,
            })
            .collect();
        lab::connect(&mut built, &cables).unwrap();
        assert_eq!(
            rows(&shown(&mut built[0].session, "show spanning-tree")),
            [
                "Ap1/0/1             Desg FWD 20000     128.41   P2p ",
                "Po10                Desg FWD 10000     128.3058 P2p ",
            ]
        );
        let s = &mut built[1].session;
        assert_eq!(
            rows(&shown(s, "show spanning-tree")),
            ["Po1                 Root FWD 3         128.64   P2p "]
        );
        // the empty Po1 took its first member's switch port lines
        assert_eq!(
            shown(s, "show running-config | section Port-channel1"),
            ["interface Port-channel1", " switchport mode trunk"]
        );
        // 15.0(2)SE pads each member to 12
        assert_eq!(
            shown(s, "show etherchannel summary")
                .iter()
                .rfind(|l| !l.is_empty())
                .unwrap(),
            "1      Po1(SU)         LACP      Gi0/1(P)    Gi0/2(P)    "
        );
    }

    #[test]
    fn classic_ios_drops_the_mst_region_at_the_second_no_and_ios_xe_at_the_first() {
        let region =
            "spanning-tree mst configuration\nname LAB\nexit\nno spanning-tree mst configuration\n";
        let c = classic();
        let mut s = switch(&c, "WS-C2960G-8TC-L", region, 1);
        assert_eq!(
            shown(&mut s, "show running-config | include mst"),
            ["spanning-tree mst configuration"]
        );
        s.type_line("configure terminal");
        s.type_line("no spanning-tree mst configuration");
        s.type_line("end");
        assert!(shown(&mut s, "show running-config | include mst").is_empty());
        let e = engine();
        let mut s = switch(&e, T, region, 1);
        assert!(shown(&mut s, "show running-config | include mst").is_empty());
    }

    use crate::show;
}
