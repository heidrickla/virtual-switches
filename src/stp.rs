//! Spanning tree's converged state, computed from bridges and the logical ports cables join: for each
//! instance, the root bridge, each bridge's root path cost and root port, and each port's role and
//! state. Pure: reading configurations and assembling the links is `spanning.rs`. PVST and Rapid PVST
//! converge on the same topology; listening and learning are not modelled.

use std::collections::{BTreeMap, BTreeSet};

/// A bridge's base MAC address, which its bridge IDs carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mac(pub [u8; 6]);

impl Mac {
    /// `xxxx.xxxx.xxxx`, as IOS writes one; a multicast address is no bridge's.
    pub fn parse(text: &str) -> Result<Mac, String> {
        let groups: Vec<&str> = text.split('.').collect();
        let hex = groups.len() == 3
            && groups
                .iter()
                .all(|g| g.len() == 4 && g.chars().all(|c| c.is_ascii_hexdigit()));
        if !hex {
            return Err(format!("{text} is not a MAC address as xxxx.xxxx.xxxx"));
        }
        let digits: String = groups.concat();
        let mut b = [0u8; 6];
        for (i, byte) in b.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&digits[2 * i..2 * i + 2], 16)
                .map_err(|_| format!("{text} is not a MAC address as xxxx.xxxx.xxxx"))?;
        }
        if b[0] & 0x01 != 0 {
            return Err(format!(
                "{text} is a multicast address; a switch's base MAC is unicast"
            ));
        }
        Ok(Mac(b))
    }

    /// A locally administered unicast address from a text, the same text always giving the same one.
    pub fn derive(seed: &str) -> Mac {
        use md5::{Digest, Md5};
        let d = Md5::digest(seed.as_bytes());
        let mut b = [0u8; 6];
        b.copy_from_slice(&d[..6]);
        b[0] = (b[0] & 0xfc) | 0x02;
        Mac(b)
    }
}

impl std::fmt::Display for Mac {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let b = self.0;
        write!(
            f,
            "{:02x}{:02x}.{:02x}{:02x}.{:02x}{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5]
        )
    }
}

/// A spanning-tree instance: a VLAN under PVST and Rapid PVST, an MST instance's number under MST.
pub type Instance = u16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Pvst,
    RapidPvst,
}

/// How a port's speed gives its default cost: IEEE 802.1D-1998 (`short`) or 802.1t (`long`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Short,
    Long,
}

/// A port's default cost at a speed in Mb/s. Measured: 1G is 20000 long on the lab C9300 (17.6.4), a
/// one-member bundle the same, and 4 short on the lab 2960G (15.0(2)SE11). Other speeds follow
/// 802.1t's 20,000,000 divided by the speed and 802.1D's table (inferred); a short bundle of two
/// or more gigabits is 3 as in the 2960 configuration guide's example (Po4).
pub fn default_cost(mbps: u32, method: Method) -> u32 {
    let mbps = mbps.max(1);
    match method {
        Method::Long => (20_000_000 / mbps).max(1),
        Method::Short => match mbps {
            m if m >= 100_000 => 1,
            m if m >= 10_000 => 2,
            m if m >= 2000 => 3,
            m if m >= 1000 => 4,
            m if m >= 100 => 19,
            _ => 100,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timers {
    pub hello: u32,
    pub max_age: u32,
    pub forward_delay: u32,
}

impl Default for Timers {
    fn default() -> Self {
        Timers {
            hello: 2,
            max_age: 20,
            forward_delay: 15,
        }
    }
}

/// A bridge's settings in one instance: its priority with the system ID extension added, its timers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BridgeInstance {
    pub priority: u32,
    pub timers: Timers,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bridge {
    pub mac: Mac,
    pub protocol: Protocol,
    pub method: Method,
    /// the instances it runs; one a port of it takes part in, which `converge` keeps where a port is up
    pub instances: BTreeMap<Instance, BridgeInstance>,
    /// a bridge whose tree this model does not compute (MST until it is modelled); it still sends
    /// BPDUs, which PortFast and BPDU guard hear
    pub unmodelled: bool,
}

/// A port's settings in an instance: its priority and the cost configured on it, if any.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortInstance {
    pub priority: u8,
    pub cost: Option<u32>,
}

/// A logical port: a switch port, or a port-channel with its bundled members' speed.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub bridge: usize,
    pub number: u16,
    pub mbps: u32,
    pub instances: BTreeMap<Instance, PortInstance>,
    /// `spanning-tree bpdufilter enable`: sends none and ignores what it hears
    pub filter: bool,
    /// BPDU guard: a BPDU heard err-disables it
    pub guard: bool,
    pub portfast: bool,
    /// half duplex, which IOS takes as a shared segment
    pub shared: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub bridges: Vec<Bridge>,
    pub ports: Vec<Port>,
    /// ports joined by a cable, each up at both ends
    pub edges: Vec<(usize, usize)>,
}

/// A bridge ID: the priority with the system ID extension, then the MAC; the lower is better.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BridgeId {
    pub priority: u32,
    pub mac: Mac,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Root,
    Designated,
    Alternate,
    Backup,
}

impl Role {
    /// As the port tables print it.
    pub fn short(self) -> &'static str {
        match self {
            Role::Root => "Root",
            Role::Designated => "Desg",
            Role::Alternate => "Altn",
            Role::Backup => "Back",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortState {
    pub role: Role,
    pub forwarding: bool,
    pub cost: u32,
    pub priority: u8,
    pub number: u16,
    /// PortFast with no BPDU heard
    pub edge: bool,
    pub shared: bool,
    /// a Rapid PVST port that hears a PVST bridge
    pub peer_stp: bool,
}

/// One bridge's view of one instance.
#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    pub root: BridgeId,
    pub root_timers: Timers,
    pub own: BridgeId,
    pub timers: Timers,
    pub cost: u32,
    pub root_port: Option<usize>,
    pub ports: BTreeMap<usize, PortState>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Converged {
    /// ports BPDU guard err-disabled
    pub errdisabled: BTreeSet<usize>,
    /// ports whose cable's far end was err-disabled: their link is down
    pub down: BTreeSet<usize>,
    /// each bridge's instances, by (bridge, instance)
    pub trees: BTreeMap<(usize, Instance), Tree>,
}

impl Graph {
    fn neighbors(&self) -> Vec<Option<usize>> {
        let mut n = vec![None; self.ports.len()];
        for &(a, b) in &self.edges {
            if n[a].is_none() && n[b].is_none() && a != b {
                n[a] = Some(b);
                n[b] = Some(a);
            }
        }
        n
    }

    /// Whether a port sends BPDUs: it takes part in an instance and no filter stops it.
    fn sends(&self, p: usize) -> bool {
        let port = &self.ports[p];
        !port.filter && !port.instances.is_empty()
    }

    fn cost(&self, p: usize, i: Instance) -> u32 {
        let port = &self.ports[p];
        port.instances
            .get(&i)
            .and_then(|x| x.cost)
            .unwrap_or_else(|| default_cost(port.mbps, self.bridges[port.bridge].method))
    }

    fn port_id(&self, p: usize, i: Instance) -> (u8, u16) {
        let port = &self.ports[p];
        (
            port.instances.get(&i).map_or(128, |x| x.priority),
            port.number,
        )
    }
}

/// The converged state: BPDU guard first (a port that hears a BPDU is err-disabled and its cable's far
/// end loses its link), then each instance's tree over the ports left.
pub fn converge(g: &Graph) -> Converged {
    let near = g.neighbors();
    let hears = |p: usize, n: &[Option<usize>], gone: &BTreeSet<usize>| {
        !g.ports[p].filter && n[p].is_some_and(|q| !gone.contains(&q) && g.sends(q))
    };
    let none = BTreeSet::new();
    let errdisabled: BTreeSet<usize> = (0..g.ports.len())
        .filter(|&p| g.ports[p].guard && hears(p, &near, &none))
        .collect();
    let down: BTreeSet<usize> = errdisabled
        .iter()
        .filter_map(|&p| near[p])
        .filter(|q| !errdisabled.contains(q))
        .collect();
    let gone: BTreeSet<usize> = errdisabled.union(&down).copied().collect();
    let live = |p: usize| !gone.contains(&p);
    let mut out = Converged {
        errdisabled,
        down,
        trees: BTreeMap::new(),
    };
    let all: BTreeSet<Instance> = g
        .ports
        .iter()
        .flat_map(|p| p.instances.keys().copied())
        .collect();
    for i in all {
        // the bridges running the instance with a port up in it
        let members: Vec<usize> = (0..g.bridges.len())
            .filter(|&b| {
                !g.bridges[b].unmodelled
                    && g.bridges[b].instances.contains_key(&i)
                    && (0..g.ports.len()).any(|p| {
                        live(p) && g.ports[p].bridge == b && g.ports[p].instances.contains_key(&i)
                    })
            })
            .collect();
        let runs = |b: usize| members.contains(&b);
        let id = |b: usize| BridgeId {
            priority: g.bridges[b].instances[&i].priority,
            mac: g.bridges[b].mac,
        };
        let in_i =
            |p: usize| live(p) && runs(g.ports[p].bridge) && g.ports[p].instances.contains_key(&i);
        // the links BPDUs cross in this instance
        let link = |p: usize| -> Option<usize> {
            let q = near[p]?;
            (in_i(p) && in_i(q) && !g.ports[p].filter && !g.ports[q].filter).then_some(q)
        };
        // each bridge's component: its root is the best bridge ID in it
        let mut comp: BTreeMap<usize, usize> = members.iter().map(|&b| (b, b)).collect();
        fn find(c: &mut BTreeMap<usize, usize>, b: usize) -> usize {
            let parent = c[&b];
            if parent == b {
                return b;
            }
            let r = find(c, parent);
            c.insert(b, r);
            r
        }
        for p in 0..g.ports.len() {
            if let Some(q) = link(p) {
                let (a, b) = (
                    find(&mut comp, g.ports[p].bridge),
                    find(&mut comp, g.ports[q].bridge),
                );
                if a != b {
                    comp.insert(a, b);
                }
            }
        }
        let mut root_of: BTreeMap<usize, usize> = BTreeMap::new();
        for &b in &members {
            let c = find(&mut comp, b);
            let best = root_of.entry(c).or_insert(b);
            if id(b) < id(*best) {
                *best = b;
            }
        }
        // root path costs: a bridge's is the far bridge's plus its own receiving port's cost
        let mut dist: BTreeMap<usize, u64> = BTreeMap::new();
        for &b in &members {
            let c = find(&mut comp, b);
            if root_of[&c] == b {
                dist.insert(b, 0);
            }
        }
        loop {
            let mut changed = false;
            for p in 0..g.ports.len() {
                let Some(q) = link(p) else { continue };
                let (b, c) = (g.ports[p].bridge, g.ports[q].bridge);
                let Some(&dc) = dist.get(&c) else { continue };
                let via = dc + u64::from(g.cost(p, i));
                if dist.get(&b).is_none_or(|&db| via < db) {
                    dist.insert(b, via);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for &b in &members {
            let c = find(&mut comp, b);
            let root = root_of[&c];
            let rpc = dist.get(&b).copied().unwrap_or(0);
            let ports: Vec<usize> = (0..g.ports.len())
                .filter(|&p| g.ports[p].bridge == b && in_i(p))
                .collect();
            // the root port: the best vector heard, then its own port ID
            let root_port = (root != b)
                .then(|| {
                    ports
                        .iter()
                        .filter_map(|&p| {
                            let q = link(p)?;
                            let far = g.ports[q].bridge;
                            Some((
                                (
                                    dist.get(&far).copied()? + u64::from(g.cost(p, i)),
                                    id(far),
                                    g.port_id(q, i),
                                    g.port_id(p, i),
                                ),
                                p,
                            ))
                        })
                        .min()
                        .map(|(_, p)| p)
                })
                .flatten();
            let mut states = BTreeMap::new();
            for &p in &ports {
                let role = match link(p) {
                    _ if Some(p) == root_port => Role::Root,
                    None => Role::Designated,
                    Some(q) => {
                        let far = g.ports[q].bridge;
                        let mine = (rpc, id(b), g.port_id(p, i));
                        let theirs = (
                            dist.get(&far).copied().unwrap_or(u64::MAX),
                            id(far),
                            g.port_id(q, i),
                        );
                        if mine < theirs {
                            Role::Designated
                        } else if far == b {
                            Role::Backup
                        } else {
                            Role::Alternate
                        }
                    }
                };
                let heard = hears(p, &near, &gone);
                let peer_stp = g.bridges[b].protocol == Protocol::RapidPvst
                    && link(p)
                        .is_some_and(|q| g.bridges[g.ports[q].bridge].protocol == Protocol::Pvst);
                let (priority, number) = g.port_id(p, i);
                states.insert(
                    p,
                    PortState {
                        role,
                        forwarding: matches!(role, Role::Root | Role::Designated),
                        cost: g.cost(p, i),
                        priority,
                        number,
                        edge: g.ports[p].portfast && !heard,
                        shared: g.ports[p].shared,
                        peer_stp,
                    },
                );
            }
            let rpc = u32::try_from(rpc).unwrap_or(u32::MAX);
            out.trees.insert(
                (b, i),
                Tree {
                    root: id(root),
                    root_timers: g.bridges[root].instances[&i].timers,
                    own: id(b),
                    timers: g.bridges[b].instances[&i].timers,
                    cost: rpc,
                    root_port,
                    ports: states,
                },
            );
        }
    }
    out
}

/// Links whose two ends both filter BPDUs and that close a loop in an instance: the ports at both ends
/// forward, and the two bridges are joined by forwarding ports without it (a cable between two ports of
/// one switch always is). Each as its two ports and the instance.
pub fn filtered_loops(g: &Graph, c: &Converged) -> Vec<(usize, usize, Instance)> {
    let near = g.neighbors();
    let mut out = vec![];
    for (&(b, i), tree) in &c.trees {
        for &p in tree.ports.keys() {
            let Some(q) = near[p] else { continue };
            if p > q || !g.ports[p].filter || !g.ports[q].filter {
                continue;
            }
            let far = g.ports[q].bridge;
            let Some(far_tree) = c.trees.get(&(far, i)) else {
                continue;
            };
            let both =
                tree.ports[&p].forwarding && far_tree.ports.get(&q).is_some_and(|s| s.forwarding);
            if both && (far == b || joined_without(g, c, i, b, far, (p, q))) {
                out.push((p, q, i));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Whether two bridges are joined in an instance by links forwarding at both ends, one link left out.
fn joined_without(
    g: &Graph,
    c: &Converged,
    i: Instance,
    from: usize,
    to: usize,
    skip: (usize, usize),
) -> bool {
    let near = g.neighbors();
    let fwd = |p: usize| {
        c.trees
            .get(&(g.ports[p].bridge, i))
            .and_then(|t| t.ports.get(&p))
            .is_some_and(|s| s.forwarding)
    };
    let mut seen = BTreeSet::from([from]);
    let mut stack = vec![from];
    while let Some(b) = stack.pop() {
        if b == to {
            return true;
        }
        for p in (0..g.ports.len()).filter(|&p| g.ports[p].bridge == b) {
            let Some(q) = near[p] else { continue };
            if (p, q) == skip || (q, p) == skip || !fwd(p) || !fwd(q) {
                continue;
            }
            let far = g.ports[q].bridge;
            if seen.insert(far) {
                stack.push(far);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(last: u8) -> Mac {
        Mac([0x00, 0x11, 0x22, 0x33, 0x44, last])
    }

    fn bridge(last: u8, priority: u32, vlans: &[Instance]) -> Bridge {
        Bridge {
            mac: mac(last),
            protocol: Protocol::RapidPvst,
            method: Method::Long,
            instances: vlans
                .iter()
                .map(|&v| {
                    (
                        v,
                        BridgeInstance {
                            priority: priority + u32::from(v),
                            timers: Timers::default(),
                        },
                    )
                })
                .collect(),
            unmodelled: false,
        }
    }

    fn port(bridge: usize, number: u16, vlans: &[Instance]) -> Port {
        Port {
            bridge,
            number,
            mbps: 1000,
            instances: vlans
                .iter()
                .map(|&v| {
                    (
                        v,
                        PortInstance {
                            priority: 128,
                            cost: None,
                        },
                    )
                })
                .collect(),
            filter: false,
            guard: false,
            portfast: false,
            shared: false,
        }
    }

    fn role(c: &Converged, b: usize, i: Instance, p: usize) -> (Role, bool) {
        let s = c.trees[&(b, i)].ports[&p];
        (s.role, s.forwarding)
    }

    #[test]
    fn a_mac_parses_as_ios_writes_it_and_a_multicast_one_is_refused() {
        assert_eq!(Mac::parse("0011.2233.4401").unwrap(), mac(1));
        assert_eq!(mac(1).to_string(), "0011.2233.4401");
        assert!(Mac::parse("0111.2233.4401").is_err());
        assert!(Mac::parse("0011.2233.44").is_err());
        assert!(Mac::parse("0011.2233.44zz").is_err());
        let d = Mac::derive("sw1\nC9300-24T");
        assert_eq!(d, Mac::derive("sw1\nC9300-24T"));
        assert_ne!(d, Mac::derive("sw2\nC9300-24T"));
        assert_eq!(d.0[0] & 0x03, 0x02, "locally administered unicast");
    }

    #[test]
    fn default_costs_follow_the_method() {
        assert_eq!(default_cost(1000, Method::Long), 20000);
        assert_eq!(default_cost(10_000, Method::Long), 2000);
        assert_eq!(default_cost(1000, Method::Short), 4);
        assert_eq!(default_cost(2000, Method::Short), 3);
        assert_eq!(default_cost(100, Method::Short), 19);
    }

    #[test]
    fn a_self_loop_blocks_the_higher_port_id_as_backup() {
        let g = Graph {
            bridges: vec![bridge(1, 32768, &[1])],
            ports: vec![port(0, 1, &[1]), port(0, 2, &[1])],
            edges: vec![(0, 1)],
        };
        let c = converge(&g);
        assert_eq!(role(&c, 0, 1, 0), (Role::Designated, true));
        assert_eq!(role(&c, 0, 1, 1), (Role::Backup, false));
        assert!(c.trees[&(0, 1)].root_port.is_none());
    }

    #[test]
    fn a_triangle_blocks_one_alternate_port_on_the_bridge_furthest_from_the_root() {
        // bridges 0, 1, 2; 0 has the lowest MAC; links 0-1, 1-2, 2-0
        let g = Graph {
            bridges: vec![
                bridge(1, 32768, &[1]),
                bridge(2, 32768, &[1]),
                bridge(3, 32768, &[1]),
            ],
            ports: vec![
                port(0, 1, &[1]),
                port(1, 1, &[1]),
                port(1, 2, &[1]),
                port(2, 1, &[1]),
                port(2, 2, &[1]),
                port(0, 2, &[1]),
            ],
            edges: vec![(0, 1), (2, 3), (4, 5)],
        };
        let c = converge(&g);
        assert_eq!(c.trees[&(1, 1)].root.mac, mac(1));
        assert_eq!(c.trees[&(1, 1)].cost, 20000);
        assert_eq!(role(&c, 1, 1, 1), (Role::Root, true));
        assert_eq!(role(&c, 2, 1, 4), (Role::Root, true));
        // between 1 and 2 the lower bridge ID is designated
        assert_eq!(role(&c, 1, 1, 2), (Role::Designated, true));
        assert_eq!(role(&c, 2, 1, 3), (Role::Alternate, false));
    }

    #[test]
    fn a_redundant_pair_blocks_the_far_port_with_the_higher_designated_port_id() {
        let g = Graph {
            bridges: vec![bridge(1, 4096, &[1]), bridge(2, 32768, &[1])],
            ports: vec![
                port(0, 1, &[1]),
                port(0, 2, &[1]),
                port(1, 1, &[1]),
                port(1, 2, &[1]),
            ],
            edges: vec![(0, 2), (1, 3)],
        };
        let c = converge(&g);
        assert_eq!(role(&c, 1, 1, 2), (Role::Root, true));
        assert_eq!(role(&c, 1, 1, 3), (Role::Alternate, false));
        assert_eq!(role(&c, 0, 1, 0), (Role::Designated, true));
        assert_eq!(role(&c, 0, 1, 1), (Role::Designated, true));
    }

    #[test]
    fn bpdu_guard_err_disables_the_port_that_hears_and_its_far_end_goes_down() {
        let mut g = Graph {
            bridges: vec![bridge(1, 32768, &[1])],
            ports: vec![port(0, 3, &[1]), port(0, 4, &[1]), port(0, 41, &[1])],
            edges: vec![(0, 1)],
        };
        g.ports[0].guard = true;
        let c = converge(&g);
        assert_eq!(c.errdisabled, BTreeSet::from([0]));
        assert_eq!(c.down, BTreeSet::from([1]));
        assert_eq!(
            c.trees[&(0, 1)].ports.keys().copied().collect::<Vec<_>>(),
            [2]
        );
        // a filtering far end sends nothing to hear
        g.ports[1].filter = true;
        assert!(converge(&g).errdisabled.is_empty());
    }

    #[test]
    fn filtering_both_ends_forwards_both_and_names_the_loop() {
        let mut g = Graph {
            bridges: vec![bridge(1, 32768, &[1]), bridge(2, 32768, &[1])],
            ports: vec![
                port(0, 1, &[1]),
                port(0, 2, &[1]),
                port(1, 1, &[1]),
                port(1, 2, &[1]),
            ],
            edges: vec![(0, 2), (1, 3)],
        };
        g.ports[1].filter = true;
        g.ports[3].filter = true;
        let c = converge(&g);
        assert_eq!(role(&c, 1, 1, 3), (Role::Designated, true));
        assert_eq!(role(&c, 1, 1, 2), (Role::Root, true));
        assert_eq!(filtered_loops(&g, &c), [(1, 3, 1)]);
        // one end filtering leaves no finding
        g.ports[1].filter = false;
        assert!(filtered_loops(&g, &converge(&g)).is_empty());
    }

    #[test]
    fn an_instance_lives_only_where_a_port_is_up_in_it_and_portfast_alone_is_edge() {
        let mut g = Graph {
            bridges: vec![bridge(1, 32768, &[1, 10])],
            ports: vec![port(0, 5, &[1])],
            edges: vec![],
        };
        g.ports[0].portfast = true;
        let c = converge(&g);
        assert!(c.trees.contains_key(&(0, 1)) && !c.trees.contains_key(&(0, 10)));
        assert!(c.trees[&(0, 1)].ports[&0].edge);
    }
}
