//! Spanning tree's converged state, computed from bridges and the logical ports cables join: for each
//! instance, the root bridge, each bridge's root path cost and root port, and each port's role and
//! state. Pure: reading configurations and assembling the links is `spanning.rs`. PVST, Rapid PVST and
//! MST converge on the same topology; listening and learning are not modelled.
//!
//! Every bridge's view of one instance is a node: a VLAN under PVST and Rapid PVST, the CIST (0) or an
//! MSTI under MST. A cable joins the nodes whose BPDUs cross it, and each set of joined nodes runs one
//! tree. The priority vectors are 802.1Q's (root, external root path cost, regional root, internal
//! root path cost, designated bridge, designated port, receiving port): a PVST bridge is a region of
//! its own, so its vector reduces to 802.1D's.

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
    Mst,
}

/// How a port's speed gives its default cost: IEEE 802.1D-1998 (`short`) or 802.1t (`long`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Short,
    Long,
}

/// A port's default cost at a speed in Mb/s. Measured: long, 1G 20000, 2x1G 10000 and 10M 2000000 on
/// the lab C9300 (17.6.4); short, 1G 4, 2x1G 3 and 100M 19 on the lab C9300 and the lab 2960G
/// (15.0(2)SE11). Other speeds follow 802.1t's 20,000,000 divided by the speed and 802.1D's table
/// (inferred).
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

/// The MST configuration digest: 802.1s HMAC-MD5, keyed as the standard gives, over the MSTID of each
/// VLAN 0 to 4095 as two bytes big-endian (reproduced against the lab C9300 and 2960G).
pub fn mst_digest(map: &BTreeMap<u16, Instance>) -> [u8; 16] {
    use md5::{Digest, Md5};
    const KEY: [u8; 16] = [
        0x13, 0xAC, 0x06, 0xA6, 0x2E, 0x47, 0xFD, 0x51, 0xF9, 0x5D, 0x2B, 0xA2, 0x43, 0xCD, 0x03,
        0x46,
    ];
    let mut table = Vec::with_capacity(8192);
    for v in 0..4096u16 {
        table.extend(map.get(&v).copied().unwrap_or(0).to_be_bytes());
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for (i, k) in KEY.iter().enumerate() {
        ipad[i] ^= k;
        opad[i] ^= k;
    }
    let mut inner = Md5::new();
    inner.update(ipad);
    inner.update(&table);
    let inner = inner.finalize();
    let mut outer = Md5::new();
    outer.update(opad);
    outer.update(inner);
    outer.finalize().into()
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

/// An MST region's identity: two bridges with all three equal are one region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub name: String,
    pub revision: u16,
    pub digest: [u8; 16],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bridge {
    pub mac: Mac,
    pub protocol: Protocol,
    /// the configured method; MST runs long whatever it is (the lab 2960G, 15.0(2)SE11)
    pub method: Method,
    /// the instances it runs; one a port of it takes part in, which `converge` keeps where a port is up
    pub instances: BTreeMap<Instance, BridgeInstance>,
    /// MST: its region
    pub region: Option<Region>,
    /// MST: the hops a BPDU crosses in the region before it is dropped
    pub max_hops: u32,
}

/// A port's settings in an instance: its priority and the cost configured on it, if any.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortInstance {
    pub priority: u8,
    pub cost: Option<u32>,
}

/// A BPDU as a device the lab does not model sends it: its root, its root path cost, its own bridge
/// ID and the port ID it sends from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bpdu {
    pub root: BridgeId,
    pub cost: u32,
    pub bridge: BridgeId,
    pub port: (u8, u16),
}

/// What a device the lab does not model sends a port: RSTP BPDUs or 802.1D ones, by VLAN (the VLAN
/// the receiving port puts an untagged one in).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Heard {
    pub rstp: bool,
    pub bpdus: BTreeMap<u16, Bpdu>,
}

/// A logical port: a switch port, or a port-channel with its bundled members' speed.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub bridge: usize,
    pub number: u16,
    pub mbps: u32,
    pub instances: BTreeMap<Instance, PortInstance>,
    /// the VLANs it carries: a trunk's allowed ones that exist, an access port's own
    pub vlans: BTreeSet<u16>,
    pub trunk: bool,
    /// a trunk's native VLAN, an access port's VLAN: where an untagged BPDU lands
    pub untagged: u16,
    /// `spanning-tree bpdufilter enable`: sends none and ignores what it hears
    pub filter: bool,
    /// BPDU guard: a BPDU heard err-disables it
    pub guard: bool,
    pub portfast: bool,
    /// half duplex, which IOS takes as a shared segment
    pub shared: bool,
    /// `spanning-tree guard root`: a superior BPDU blocks it
    pub root_guard: bool,
    /// `spanning-tree guard loop`: a non-designated port that stops hearing BPDUs blocks
    pub loop_guard: bool,
    /// what a device the lab does not model, at the cable's far end, sends
    pub heard: Option<Heard>,
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
    /// an MSTI's port at the region's boundary that is the CIST root port
    Master,
}

impl Role {
    /// As the port tables print it.
    pub fn short(self) -> &'static str {
        match self {
            Role::Root => "Root",
            Role::Designated => "Desg",
            Role::Alternate => "Altn",
            Role::Backup => "Back",
            Role::Master => "Mstr",
        }
    }
}

/// Why a guard or PVST simulation holds a port blocked (state `BKN*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inconsistency {
    Root,
    Loop,
    PvstSim,
}

impl Inconsistency {
    /// The Type column's flag.
    pub fn flag(self) -> &'static str {
        match self {
            Inconsistency::Root => "*ROOT_Inc",
            Inconsistency::Loop => "*LOOP_Inc",
            Inconsistency::PvstSim => "*PVST_Inc",
        }
    }

    /// `show spanning-tree inconsistentports`'s name.
    pub fn name(self) -> &'static str {
        match self {
            Inconsistency::Root => "Root Inconsistent",
            Inconsistency::Loop => "Loop Inconsistent",
            Inconsistency::PvstSim => "PVST Sim. Inconsistent",
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
    /// a Rapid PVST port that hears 802.1D BPDUs
    pub peer_stp: bool,
    /// an MST port at its region's boundary: what it hears (`PVST`, `RSTP`, `STP`)
    pub bound: Option<&'static str>,
    pub inconsistent: Option<Inconsistency>,
}

/// One bridge's view of one instance.
#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    pub root: BridgeId,
    pub root_timers: Timers,
    pub own: BridgeId,
    pub timers: Timers,
    /// the root path cost the tables print: PVST's, the CIST's external one (its internal one where
    /// the root is inside the region, inferred), an MSTI's internal one
    pub cost: u32,
    /// MST: the CIST's external root path cost
    pub external: u32,
    /// MST: the internal root path cost to the regional root (the CIST) or the MSTI's root
    pub internal: u32,
    /// MST: the CIST's regional root; an MSTI's root
    pub regional: BridgeId,
    /// MST: the hops left in the region at this bridge
    pub remaining: u32,
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

    fn mst(&self, p: usize) -> bool {
        self.bridges[self.ports[p].bridge].protocol == Protocol::Mst
    }

    fn cost(&self, p: usize, i: Instance) -> u32 {
        let port = &self.ports[p];
        let bridge = &self.bridges[port.bridge];
        let method = if bridge.protocol == Protocol::Mst {
            Method::Long
        } else {
            bridge.method
        };
        port.instances
            .get(&i)
            .and_then(|x| x.cost)
            .unwrap_or_else(|| default_cost(port.mbps, method))
    }

    fn port_id(&self, p: usize, i: Instance) -> (u8, u16) {
        let port = &self.ports[p];
        (
            port.instances.get(&i).map_or(128, |x| x.priority),
            port.number,
        )
    }

    /// Two ports of MST bridges in one region.
    fn same_region(&self, p: usize, q: usize) -> bool {
        let (a, b) = (
            &self.bridges[self.ports[p].bridge],
            &self.bridges[self.ports[q].bridge],
        );
        a.protocol == Protocol::Mst
            && b.protocol == Protocol::Mst
            && a.region.is_some()
            && a.region == b.region
    }

    /// The VLAN a PVST bridge's port runs the CIST in against an MST bridge: VLAN 1 on a trunk (PVST
    /// simulation), its own VLAN on an access port.
    fn cist_vlan(&self, p: usize) -> Instance {
        let port = &self.ports[p];
        if port.trunk { 1 } else { port.untagged }
    }

    /// The instances a BPDU joins across the cable from `p` to `q`: (at p, at q).
    fn joins(&self, p: usize, q: usize) -> Vec<(Instance, Instance)> {
        let (a, b) = (&self.ports[p], &self.ports[q]);
        let mut out = vec![];
        match (self.mst(p), self.mst(q)) {
            (true, true) => {
                out.push((0, 0));
                if self.same_region(p, q) {
                    out.extend(
                        a.instances
                            .keys()
                            .filter(|&&i| i != 0 && b.instances.contains_key(&i))
                            .map(|&i| (i, i)),
                    );
                }
            }
            (true, false) => out.push((0, self.cist_vlan(q))),
            (false, true) => out.push((self.cist_vlan(p), 0)),
            (false, false) if a.trunk && b.trunk => {
                out.extend(
                    a.instances
                        .keys()
                        .filter(|i| b.instances.contains_key(i))
                        .map(|&i| (i, i)),
                );
            }
            // an untagged BPDU lands in the far port's access or native VLAN (the lab C9300's VLAN 1
            // reached the 2960G's VLAN 10 on an access link)
            (false, false) => out.push((a.untagged, b.untagged)),
        }
        out
    }
}

type Node = (usize, Instance);

/// An 802.1Q priority vector; the derived order compares its fields in turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Vector {
    root: BridgeId,
    ext: u64,
    regional: BridgeId,
    int: u64,
    bridge: BridgeId,
    port: (u8, u16),
    recv: (u8, u16),
}

impl Vector {
    fn own(id: BridgeId) -> Vector {
        Vector {
            root: id,
            ext: 0,
            regional: id,
            int: 0,
            bridge: id,
            port: (0, 0),
            recv: (0, 0),
        }
    }

    /// What a port sends: the bridge's root vector with itself as the designated bridge and port.
    fn sent(self, id: BridgeId, port: (u8, u16)) -> Vector {
        Vector {
            bridge: id,
            port,
            recv: (0, 0),
            ..self
        }
    }

    /// A sent vector as a port receives it: inside a region the internal cost grows, across a
    /// boundary the external one, and the receiving bridge is its own region's root.
    fn received(self, internal: bool, cost: u32, id: BridgeId, recv: (u8, u16)) -> Vector {
        if internal {
            Vector {
                int: self.int + u64::from(cost),
                recv,
                ..self
            }
        } else {
            Vector {
                root: self.root,
                ext: self.ext + u64::from(cost),
                regional: id,
                int: 0,
                bridge: self.regional,
                port: self.port,
                recv,
            }
        }
    }

    /// The fields two ports compare: all of them inside a region; across a boundary what an RSTP
    /// BPDU carries, the region seen as its regional root.
    fn key(self, internal: bool) -> (BridgeId, u64, BridgeId, u64, BridgeId, (u8, u16)) {
        if internal {
            (
                self.root,
                self.ext,
                self.regional,
                self.int,
                self.bridge,
                self.port,
            )
        } else {
            (
                self.root,
                self.ext,
                self.regional,
                0,
                self.regional,
                self.port,
            )
        }
    }
}

/// The computation's state across instances.
struct Solver<'g> {
    g: &'g Graph,
    near: Vec<Option<usize>>,
    nodes: BTreeSet<Node>,
    /// (port, instance) -> (far port, far instance) where BPDUs cross
    adj: BTreeMap<(usize, Instance), (usize, Instance)>,
    /// the same where a filter at either end stops them
    silent: BTreeMap<(usize, Instance), (usize, Instance)>,
    /// ports of each node
    members: BTreeMap<Node, Vec<usize>>,
    vecs: BTreeMap<Node, Vector>,
    /// each port's role in each node, and whether a dispute holds it blocked
    roles: BTreeMap<(Node, usize), (Role, bool)>,
    root_ports: BTreeMap<Node, usize>,
    inconsistent: BTreeMap<(Node, usize), Inconsistency>,
}

impl Solver<'_> {
    fn id(&self, x: Node) -> BridgeId {
        BridgeId {
            priority: self.g.bridges[x.0].instances[&x.1].priority,
            mac: self.g.bridges[x.0].mac,
        }
    }

    fn internal(&self, p: usize, q: usize) -> bool {
        self.g.same_region(p, q)
    }

    /// What port `p` of node `x` sends.
    fn designated(&self, x: Node, p: usize) -> Vector {
        self.vecs[&x].sent(self.id(x), self.g.port_id(p, x.1))
    }

    /// The vectors node `x` hears, by port.
    fn candidates(
        &self,
        x: Node,
        fixed: &BTreeMap<(usize, Instance), Vector>,
        deaf: &BTreeSet<(usize, Instance)>,
    ) -> Vec<(Vector, usize)> {
        let (_, i) = x;
        let id = self.id(x);
        let mut out = vec![];
        for &p in &self.members[&x] {
            if deaf.contains(&(p, i)) {
                continue;
            }
            let cost = self.g.cost(p, i);
            let pid = self.g.port_id(p, i);
            if let Some(&(q, j)) = self.adj.get(&(p, i)) {
                let y = (self.g.ports[q].bridge, j);
                if self.vecs.contains_key(&y) {
                    let v = self.designated(y, q);
                    out.push((v.received(self.internal(p, q), cost, id, pid), p));
                }
            } else if let Some(h) = fixed.get(&(p, i)) {
                out.push((h.received(false, cost, id, pid), p));
            }
        }
        out
    }

    /// Each node's root vector, its root port and its ports' roles, for one set of joined nodes.
    fn solve(
        &mut self,
        scope: &[Node],
        fixed: &BTreeMap<(usize, Instance), Vector>,
        deaf: &BTreeSet<(usize, Instance)>,
    ) {
        for &x in scope {
            self.vecs.insert(x, Vector::own(self.id(x)));
        }
        // the root vectors only fall, and each round settles one more hop
        for _ in 0..=scope.len() * 2 + 2 {
            let mut changed = false;
            for &x in scope {
                let best = self
                    .candidates(x, fixed, deaf)
                    .into_iter()
                    .map(|(v, _)| v)
                    .chain([Vector::own(self.id(x))])
                    .min()
                    .unwrap_or(Vector::own(self.id(x)));
                if self.vecs[&x] != best {
                    self.vecs.insert(x, best);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for &x in scope {
            let own = Vector::own(self.id(x));
            let best = self.vecs[&x];
            if best != own
                && let Some((_, p)) = self
                    .candidates(x, fixed, deaf)
                    .into_iter()
                    .find(|(v, _)| *v == best)
            {
                self.root_ports.insert(x, p);
            }
        }
        for &x in scope {
            let rp = self.root_ports.get(&x).copied();
            let (b, i) = x;
            for &p in &self.members[&x].clone() {
                let mine = self.designated(x, p);
                let state = if Some(p) == rp {
                    (Role::Root, false)
                } else if let Some(&(q, j)) = self.adj.get(&(p, i)) {
                    let y = (self.g.ports[q].bridge, j);
                    let internal = self.internal(p, q);
                    let theirs = self.designated(y, q);
                    let superior = theirs.key(internal) < mine.key(internal);
                    if deaf.contains(&(p, i)) {
                        // a root guard port that hears better is held; one that may not take what it
                        // hears (an MSTI's from a CIST alternate) stays designated, blocked while
                        // the far end disputes it
                        if self.g.ports[p].root_guard && superior {
                            self.inconsistent.insert((x, p), Inconsistency::Root);
                            (Role::Designated, false)
                        } else {
                            (Role::Designated, superior)
                        }
                    } else if !superior {
                        (Role::Designated, false)
                    } else if self.g.ports[q].bridge == b {
                        (Role::Backup, false)
                    } else {
                        (Role::Alternate, false)
                    }
                } else if let Some(h) = fixed.get(&(p, i)) {
                    let superior = h.key(false) < mine.key(false);
                    if self.g.ports[p].root_guard && superior {
                        self.inconsistent.insert((x, p), Inconsistency::Root);
                        (Role::Designated, false)
                    } else if superior {
                        (Role::Alternate, false)
                    } else {
                        (Role::Designated, false)
                    }
                } else {
                    (Role::Designated, false)
                };
                self.roles.insert((x, p), state);
            }
        }
    }

    /// The fixed vectors devices the lab does not model send, for the nodes in `scope`.
    fn external(&self, scope: &[Node], into: &mut BTreeMap<(usize, Instance), Vector>) {
        for &x in scope {
            for &p in &self.members[&x] {
                let Some(heard) = &self.g.ports[p].heard else {
                    continue;
                };
                let vlan = match (self.g.mst(p), x.1) {
                    (true, 0) => self.g.cist_vlan(p),
                    (true, _) => continue,
                    (false, v) => v,
                };
                if let Some(b) = heard.bpdus.get(&vlan) {
                    let v = Vector {
                        root: b.root,
                        ext: u64::from(b.cost),
                        regional: b.bridge,
                        int: 0,
                        bridge: b.bridge,
                        port: b.port,
                        recv: (0, 0),
                    };
                    into.insert((p, x.1), v);
                }
            }
        }
    }

    /// The far end of a port's cable where BPDUs cross it, in any instance.
    fn far(&self, p: usize) -> Option<usize> {
        let q = self.near[p]?;
        (!self.g.ports[p].filter && !self.g.ports[q].filter).then_some(q)
    }

    fn cist_role(&self, p: usize) -> Option<Role> {
        self.roles
            .get(&((self.g.ports[p].bridge, 0), p))
            .map(|r| r.0)
    }

    /// Hops from the regional root (an MSTI's root) along root ports inside the region.
    fn hops(&self, x: Node, seen: &mut BTreeSet<Node>) -> u32 {
        if !seen.insert(x) {
            return 0;
        }
        let v = self.vecs[&x];
        let id = self.id(x);
        let top = if x.1 == 0 { v.regional } else { v.root };
        if top == id {
            return 0;
        }
        let Some(&rp) = self.root_ports.get(&x) else {
            return 0;
        };
        match self.adj.get(&(rp, x.1)) {
            Some(&(q, j)) if self.internal(rp, q) => {
                1 + self.hops((self.g.ports[q].bridge, j), seen)
            }
            _ => 0,
        }
    }
}

/// The converged state: BPDU guard first (a port that hears a BPDU is err-disabled and its cable's far
/// end loses its link), then each instance's tree over the ports left: the trees MST bridges' CISTs
/// join, then MSTIs, then PVST's, which hear what the MST bridges' boundary ports send.
pub fn converge(g: &Graph) -> Converged {
    let near = g.neighbors();
    let hears = |p: usize, n: &[Option<usize>], gone: &BTreeSet<usize>| {
        !g.ports[p].filter
            && (n[p].is_some_and(|q| !gone.contains(&q) && g.sends(q))
                || g.ports[p]
                    .heard
                    .as_ref()
                    .is_some_and(|h| !h.bpdus.is_empty()))
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
    let mut nodes: BTreeSet<Node> = BTreeSet::new();
    let mut members: BTreeMap<Node, Vec<usize>> = BTreeMap::new();
    for (p, port) in g.ports.iter().enumerate() {
        if !live(p) {
            continue;
        }
        for &i in port.instances.keys() {
            if g.bridges[port.bridge].instances.contains_key(&i) {
                nodes.insert((port.bridge, i));
                members.entry((port.bridge, i)).or_default().push(p);
            }
        }
    }
    let member = |p: usize, i: Instance| {
        live(p) && g.ports[p].instances.contains_key(&i) && nodes.contains(&(g.ports[p].bridge, i))
    };
    let mut adj = BTreeMap::new();
    let mut silent = BTreeMap::new();
    for (p, &q) in near.iter().enumerate() {
        let Some(q) = q else { continue };
        if !live(p) || !live(q) {
            continue;
        }
        for (i, j) in g.joins(p, q) {
            if !member(p, i) || !member(q, j) {
                continue;
            }
            if g.ports[p].filter || g.ports[q].filter {
                silent.insert((p, i), (q, j));
            } else {
                adj.insert((p, i), (q, j));
            }
        }
    }
    // the sets of joined nodes
    let mut parent: BTreeMap<Node, Node> = nodes.iter().map(|&x| (x, x)).collect();
    fn find(c: &mut BTreeMap<Node, Node>, x: Node) -> Node {
        let up = c[&x];
        if up == x {
            return x;
        }
        let r = find(c, up);
        c.insert(x, r);
        r
    }
    for (&(p, i), &(q, j)) in &adj {
        let a = find(&mut parent, (g.ports[p].bridge, i));
        let b = find(&mut parent, (g.ports[q].bridge, j));
        if a != b {
            parent.insert(a, b);
        }
    }
    let mut sets: BTreeMap<Node, Vec<Node>> = BTreeMap::new();
    for &x in &nodes {
        let r = find(&mut parent, x);
        sets.entry(r).or_default().push(x);
    }
    let mut s = Solver {
        g,
        near: near.clone(),
        nodes,
        adj,
        silent,
        members,
        vecs: BTreeMap::new(),
        roles: BTreeMap::new(),
        root_ports: BTreeMap::new(),
        inconsistent: BTreeMap::new(),
    };
    let is_mst = |x: &Node| g.bridges[x.0].protocol == Protocol::Mst;
    let guarded: BTreeSet<(usize, Instance)> = s
        .members
        .iter()
        .flat_map(|(x, ps)| ps.iter().map(move |&p| (p, x.1)))
        .filter(|&(p, _)| g.ports[p].root_guard)
        .collect();
    let (cists, rest): (Vec<Vec<Node>>, Vec<Vec<Node>>) = sets
        .into_values()
        .partition(|set| set.iter().any(|x| is_mst(x) && x.1 == 0));
    for set in &cists {
        let mut fixed = BTreeMap::new();
        s.external(set, &mut fixed);
        s.solve(set, &fixed, &guarded);
    }
    let (mstis, pvst): (Vec<Vec<Node>>, Vec<Vec<Node>>) =
        rest.into_iter().partition(|set| set.iter().any(is_mst));
    for set in &mstis {
        // an MSTI's information is not taken from a port that is the CIST's alternate or backup at
        // the far end (the lab 2960G's Gi0/7 stayed `Desg BLK` in MST1 against the C9300's CIST
        // alternate; inferred as the rule)
        let mut deaf = guarded.clone();
        for &x in set {
            for &p in &s.members[&x] {
                if let Some(&(q, _)) = s.adj.get(&(p, x.1))
                    && matches!(s.cist_role(q), Some(Role::Alternate | Role::Backup))
                {
                    deaf.insert((p, x.1));
                }
            }
        }
        s.solve(set, &BTreeMap::new(), &deaf);
    }
    // an MST bridge's CIST designated port facing a PVST trunk sends its CIST information in each
    // VLAN the two share (PVST simulation)
    let mut injected: BTreeMap<(usize, Instance), Vector> = BTreeMap::new();
    for (p, &q) in near.iter().enumerate() {
        let Some(q) = q else { continue };
        if !g.mst(p) || g.mst(q) || !g.ports[q].trunk || !live(p) || !live(q) {
            continue;
        }
        if g.ports[p].filter || g.ports[q].filter {
            continue;
        }
        let x = (g.ports[p].bridge, 0);
        if s.roles.get(&(x, p)).map(|r| r.0) != Some(Role::Designated) {
            continue;
        }
        let v = s.designated(x, p);
        for &vlan in g.ports[q].instances.keys() {
            if vlan != 1 && g.ports[p].vlans.contains(&vlan) {
                injected.insert((q, vlan), v);
            }
        }
    }
    for set in &pvst {
        let mut fixed = injected.clone();
        s.external(set, &mut fixed);
        s.solve(set, &fixed, &guarded);
    }
    // MSTI ports at the boundary take the CIST's role, its root port as the master
    let boundary = |s: &Solver, p: usize| -> bool {
        g.ports[p].heard.is_some() || s.far(p).is_some_and(|q| !g.same_region(p, q))
    };
    let msti_ports: Vec<(Node, usize)> = s
        .roles
        .keys()
        .filter(|(x, p)| is_mst(x) && x.1 != 0 && boundary(&s, *p))
        .copied()
        .collect();
    for (x, p) in msti_ports {
        if let Some(&(role, _)) = s.roles.get(&((x.0, 0), p)) {
            let role = if role == Role::Root {
                Role::Master
            } else {
                role
            };
            s.roles.insert((x, p), (role, false));
            if let Some(&inc) = s.inconsistent.get(&((x.0, 0), p)) {
                s.inconsistent.insert((x, p), inc);
            }
        }
    }
    pvst_simulation(&mut s);
    loop_guard(&mut s);
    finish(&s, &errdisabled, &down, &gone)
}

/// PVST simulation at an MST bridge's port facing a PVST trunk (Cisco's rule): where the CIST root
/// port hears the root beyond it, every other VLAN's BPDU must be as good as VLAN 1's (the lab C9300:
/// VLAN 10 at 4106 against VLAN 1 at 4097 held Po10 `PVST_Inc`, VLAN 10 at 10 freed it; its
/// alternate port was never held); where the port is designated, no VLAN may send it better than its
/// CIST information (inferred). Either holds the port in every instance.
fn pvst_simulation(s: &mut Solver) {
    let g = s.g;
    let mut held = vec![];
    for p in 0..g.ports.len() {
        let Some(q) = s.far(p) else { continue };
        if !g.mst(p) || g.mst(q) || !g.ports[q].trunk {
            continue;
        }
        let b = g.ports[q].bridge;
        let others: Vec<Instance> = g.ports[q]
            .instances
            .keys()
            .copied()
            .filter(|&v| v != 1 && g.ports[p].vlans.contains(&v))
            .collect();
        let bad = match s.cist_role(p) {
            Some(Role::Root) => {
                let Some(one) = s
                    .vecs
                    .contains_key(&(b, 1))
                    .then(|| s.designated((b, 1), q))
                else {
                    continue;
                };
                others.iter().any(|&v| {
                    s.vecs.contains_key(&(b, v))
                        && s.designated((b, v), q).key(false) > one.key(false)
                })
            }
            Some(Role::Designated) => others
                .iter()
                .any(|&v| s.roles.get(&((b, v), q)).map(|r| r.0) == Some(Role::Designated)),
            _ => false,
        };
        if bad {
            held.push(p);
        }
    }
    for p in held {
        let keys: Vec<(Node, usize)> = s.roles.keys().filter(|(_, x)| *x == p).copied().collect();
        for k in keys {
            s.inconsistent.insert(k, Inconsistency::PvstSim);
        }
    }
}

/// Loop guard, converged: a guarded designated port whose cable's far end runs the instance but sends
/// nothing (a BPDU filter), and whose BPDUs would be better than its own, is held: it would be the
/// root or an alternate port had they arrived (the lab C9300's Gi1/1/1 against the 2960G's filtered
/// Gi0/7).
fn loop_guard(s: &mut Solver) {
    let g = s.g;
    let mut held = vec![];
    for (&(x, p), &(role, _)) in &s.roles {
        if !g.ports[p].loop_guard
            || role != Role::Designated
            || s.inconsistent.contains_key(&(x, p))
        {
            continue;
        }
        let Some(&(q, j)) = s.silent.get(&(p, x.1)) else {
            continue;
        };
        if g.ports[p].filter {
            continue;
        }
        let y = (g.ports[q].bridge, j);
        if !s.vecs.contains_key(&y) {
            continue;
        }
        let internal = s.internal(p, q);
        if s.designated(y, q).key(internal) < s.designated(x, p).key(internal) {
            held.push((x, p));
        }
    }
    for k in held {
        s.inconsistent.insert(k, Inconsistency::Loop);
    }
}

/// Each node's tree, with the Type column's flags.
fn finish(
    s: &Solver,
    errdisabled: &BTreeSet<usize>,
    down: &BTreeSet<usize>,
    gone: &BTreeSet<usize>,
) -> Converged {
    let g = s.g;
    let mut out = Converged {
        errdisabled: errdisabled.clone(),
        down: down.clone(),
        trees: BTreeMap::new(),
    };
    let ids: BTreeMap<BridgeId, Node> = s.nodes.iter().map(|&x| (s.id(x), x)).collect();
    for &x in &s.nodes {
        let (b, i) = x;
        let v = s.vecs[&x];
        let mst = g.bridges[b].protocol == Protocol::Mst;
        let timers_of = |id: BridgeId| {
            ids.get(&id).map_or_else(Timers::default, |&(rb, ri)| {
                g.bridges[rb].instances[&ri].timers
            })
        };
        let top = if i == 0 { v.regional } else { v.root };
        let max_hops = ids.get(&top).map_or(20, |&(rb, _)| g.bridges[rb].max_hops);
        let mut ports = BTreeMap::new();
        for &p in &s.members[&x] {
            let (role, dispute) = s.roles[&(x, p)];
            let inconsistent = s.inconsistent.get(&(x, p)).copied();
            let (priority, number) = g.port_id(p, i);
            let heard = !g.ports[p].filter
                && (s.near[p].is_some_and(|q| !gone.contains(&q) && g.sends(q))
                    || g.ports[p]
                        .heard
                        .as_ref()
                        .is_some_and(|h| !h.bpdus.is_empty()));
            let far = s.far(p).filter(|q| !gone.contains(q));
            let far_protocol = far.map(|q| g.bridges[g.ports[q].bridge].protocol);
            // a Rapid PVST port prints `Peer(STP)` facing PVST, and facing MST unless the far port is
            // the CIST's alternate or backup and not held inconsistent (the lab 2960G against the
            // C9300's MST: the measured cases; the rule is inferred from them)
            let peer_stp = g.bridges[b].protocol == Protocol::RapidPvst
                && match (far, far_protocol) {
                    (_, Some(Protocol::Pvst)) => true,
                    (Some(q), Some(Protocol::Mst)) => {
                        let far_inc = s.inconsistent.contains_key(&((g.ports[q].bridge, 0), q));
                        !matches!(s.cist_role(q), Some(Role::Alternate | Role::Backup)) || far_inc
                    }
                    // facing PVST behind its BPDU filter it keeps `Peer(STP)` (the lab C9300's s9c:
                    // migration does not revert when BPDUs stop; that it was heard first is assumed)
                    _ => {
                        g.ports[p].heard.as_ref().is_some_and(|h| !h.rstp)
                            || s.near[p].is_some_and(|q| {
                                !gone.contains(&q)
                                    && !g.ports[p].filter
                                    && g.bridges[g.ports[q].bridge].protocol == Protocol::Pvst
                            })
                    }
                };
            // an MST port facing PVST prints `Bound(PVST)` on a trunk; another region's port that
            // sends it BPDUs `Bound(RSTP)` (the lab C9300 and 2960G); an access port facing PVST or
            // Rapid PVST `Bound(STP)` or `Bound(RSTP)` (inferred)
            let bound = if !mst {
                None
            } else {
                match (far, far_protocol) {
                    (Some(q), Some(Protocol::Pvst | Protocol::RapidPvst)) => {
                        Some(if g.ports[q].trunk {
                            "PVST"
                        } else if far_protocol == Some(Protocol::RapidPvst) {
                            "RSTP"
                        } else {
                            "STP"
                        })
                    }
                    (Some(q), Some(Protocol::Mst)) if !g.same_region(p, q) => {
                        (s.cist_role(q) == Some(Role::Designated)).then_some("RSTP")
                    }
                    _ => g.ports[p]
                        .heard
                        .as_ref()
                        .filter(|h| !h.bpdus.is_empty())
                        .map(|h| if h.rstp { "RSTP" } else { "STP" }),
                }
            };
            ports.insert(
                p,
                PortState {
                    role,
                    forwarding: matches!(role, Role::Root | Role::Designated | Role::Master)
                        && !dispute
                        && inconsistent.is_none(),
                    cost: g.cost(p, i),
                    priority,
                    number,
                    edge: g.ports[p].portfast && !heard,
                    shared: g.ports[p].shared,
                    peer_stp,
                    bound,
                    inconsistent,
                },
            );
        }
        let clamp = |c: u64| u32::try_from(c).unwrap_or(u32::MAX);
        let mut seen = BTreeSet::new();
        let hops = if mst { s.hops(x, &mut seen) } else { 0 };
        out.trees.insert(
            x,
            Tree {
                root: v.root,
                root_timers: timers_of(v.root),
                own: s.id(x),
                timers: g.bridges[b].instances[&i].timers,
                cost: clamp(if mst && (i != 0 || v.ext == 0) {
                    v.int
                } else {
                    v.ext
                }),
                external: clamp(v.ext),
                internal: clamp(v.int),
                regional: top,
                remaining: max_hops.saturating_sub(hops),
                root_port: s.root_ports.get(&x).copied(),
                ports,
            },
        );
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
            region: None,
            max_hops: 20,
        }
    }

    fn mst_bridge(last: u8, revision: u16, instances: &[Instance]) -> Bridge {
        let mut b = bridge(last, 32768, instances);
        b.protocol = Protocol::Mst;
        b.region = Some(Region {
            name: "R".into(),
            revision,
            digest: [0; 16],
        });
        b
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
            vlans: vlans.iter().copied().collect(),
            trunk: true,
            untagged: 1,
            filter: false,
            guard: false,
            portfast: false,
            shared: false,
            root_guard: false,
            loop_guard: false,
            heard: None,
        }
    }

    /// An MST bridge's trunk port carrying `vlans`, in the instances listed.
    fn mst_port(bridge: usize, number: u16, instances: &[Instance], vlans: &[u16]) -> Port {
        let mut p = port(bridge, number, instances);
        p.vlans = vlans.iter().copied().collect();
        p
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
        assert_eq!(default_cost(2000, Method::Long), 10000);
        assert_eq!(default_cost(10, Method::Long), 2_000_000);
        assert_eq!(default_cost(10_000, Method::Long), 2000);
        assert_eq!(default_cost(1000, Method::Short), 4);
        assert_eq!(default_cost(2000, Method::Short), 3);
        assert_eq!(default_cost(100, Method::Short), 19);
    }

    #[test]
    fn the_mst_digest_of_an_empty_table_and_of_one_instance_differ() {
        // 802.1s's digest of every VLAN in the CIST, as published with the standard's key
        let none = mst_digest(&BTreeMap::new());
        let hex: String = none.iter().map(|b| format!("{b:02X}")).collect();
        assert_eq!(hex, "AC36177F50283CD4B83821D8AB26DE62");
        assert_ne!(mst_digest(&BTreeMap::from([(10, 1)])), none);
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

    #[test]
    fn an_access_link_joins_the_vlan_at_each_end() {
        let mut g = Graph {
            bridges: vec![bridge(1, 32768, &[1]), bridge(2, 32768, &[10])],
            ports: vec![port(0, 1, &[1]), port(1, 1, &[10])],
            edges: vec![(0, 1)],
        };
        for (p, v) in [(0, 1), (1, 10)] {
            g.ports[p].trunk = false;
            g.ports[p].untagged = v;
        }
        let c = converge(&g);
        // bridge 0's VLAN 1 (32769) is better than bridge 1's VLAN 10 (32778)
        assert_eq!(c.trees[&(1, 10)].root.mac, mac(1));
        assert_eq!(role(&c, 1, 10, 1), (Role::Root, true));
    }

    #[test]
    fn one_region_runs_its_msti_across_and_the_cist_regional_root_is_the_lowest_bridge() {
        let g = Graph {
            bridges: vec![mst_bridge(1, 1, &[0, 1]), mst_bridge(2, 1, &[0, 1])],
            ports: vec![
                mst_port(0, 1, &[0, 1], &[1, 10]),
                mst_port(1, 1, &[0, 1], &[1, 10]),
            ],
            edges: vec![(0, 1)],
        };
        let c = converge(&g);
        let t = &c.trees[&(1, 1)];
        assert_eq!((t.root.mac, t.cost, t.remaining), (mac(1), 20000, 19));
        assert_eq!(role(&c, 1, 1, 1), (Role::Root, true));
        // the CIST's root is inside the region: the tables print the internal cost
        let t = &c.trees[&(1, 0)];
        assert_eq!((t.regional.mac, t.external, t.cost), (mac(1), 0, 20000));
        assert_eq!(t.ports[&1].bound, None);
    }

    #[test]
    fn another_revision_is_another_region_whose_msti_ports_take_the_cist_roles() {
        let g = Graph {
            bridges: vec![mst_bridge(1, 1, &[0, 1]), mst_bridge(2, 2, &[0, 1])],
            ports: vec![
                mst_port(0, 1, &[0, 1], &[1, 10]),
                mst_port(1, 1, &[0, 1], &[1, 10]),
            ],
            edges: vec![(0, 1)],
        };
        let c = converge(&g);
        // bridge 1 roots its own MSTI; its CIST root port is the MSTI's master port
        assert_eq!(c.trees[&(1, 1)].root.mac, mac(2));
        assert_eq!(role(&c, 1, 1, 1), (Role::Master, true));
        let t = &c.trees[&(1, 0)];
        assert_eq!((t.regional.mac, t.cost, t.internal), (mac(2), 20000, 0));
        assert_eq!(t.ports[&1].bound, Some("RSTP"));
        // bridge 0's port is designated and sends; bridge 1's root port sends nothing back
        assert_eq!(c.trees[&(0, 0)].ports[&0].bound, None);
    }

    /// An MST bridge (0) and a Rapid PVST bridge (1) on a trunk carrying VLANs 1 and 10.
    fn simulation(vlan1: u32, vlan10: u32) -> Converged {
        let mut pvst = bridge(2, 0, &[1, 10]);
        pvst.instances.get_mut(&1).unwrap().priority = vlan1 + 1;
        pvst.instances.get_mut(&10).unwrap().priority = vlan10 + 10;
        let g = Graph {
            bridges: vec![mst_bridge(1, 1, &[0, 1]), pvst],
            ports: vec![mst_port(0, 1, &[0, 1], &[1, 10]), port(1, 1, &[1, 10])],
            edges: vec![(0, 1)],
        };
        converge(&g)
    }

    #[test]
    fn pvst_simulation_holds_a_root_port_hearing_a_worse_vlan_than_vlan_1() {
        let c = simulation(4096, 4096);
        let t = &c.trees[&(0, 0)];
        assert_eq!((t.root.priority, t.root.mac), (4097, mac(2)));
        let s = t.ports[&0];
        assert_eq!((s.role, s.forwarding), (Role::Root, false));
        assert_eq!(s.inconsistent, Some(Inconsistency::PvstSim));
        assert_eq!(s.bound, Some("PVST"));
        assert_eq!(c.trees[&(0, 1)].ports[&0].role, Role::Master);
        assert_eq!(
            c.trees[&(0, 1)].ports[&0].inconsistent,
            Some(Inconsistency::PvstSim)
        );
        // the PVST bridge roots both VLANs and hears nothing from the root port
        assert_eq!(role(&c, 1, 10, 1), (Role::Designated, true));
        assert!(c.trees[&(1, 1)].ports[&1].peer_stp);
    }

    #[test]
    fn pvst_simulation_is_consistent_where_every_vlan_is_as_good_as_vlan_1() {
        let c = simulation(4096, 0);
        let s = c.trees[&(0, 0)].ports[&0];
        assert_eq!(
            (s.role, s.forwarding, s.inconsistent),
            (Role::Root, true, None)
        );
        // the MST bridge as the root: the PVST bridge hears its CIST in every VLAN
        let c = simulation(32768, 32768);
        let s = c.trees[&(0, 0)].ports[&0];
        assert_eq!(
            (s.role, s.forwarding, s.inconsistent),
            (Role::Designated, true, None)
        );
        let t = &c.trees[&(1, 10)];
        assert_eq!(
            (t.root.priority, t.root.mac, t.cost),
            (32768, mac(1), 20000)
        );
        // the MST bridge's root ID beaten in VLAN 10 alone: the designated port is held
        let c = simulation(32768, 0);
        assert_eq!(
            c.trees[&(0, 0)].ports[&0].inconsistent,
            Some(Inconsistency::PvstSim)
        );
    }

    #[test]
    fn root_guard_holds_a_port_that_hears_a_better_root_and_the_tree_roots_elsewhere() {
        let mut g = Graph {
            bridges: vec![bridge(1, 4096, &[1]), bridge(2, 32768, &[1])],
            ports: vec![
                port(0, 1, &[1]),
                port(0, 2, &[1]),
                port(1, 1, &[1]),
                port(1, 2, &[1]),
            ],
            edges: vec![(0, 2), (1, 3)],
        };
        g.ports[2].root_guard = true;
        let c = converge(&g);
        let s = c.trees[&(1, 1)].ports[&2];
        assert_eq!((s.role, s.forwarding), (Role::Designated, false));
        assert_eq!(s.inconsistent, Some(Inconsistency::Root));
        assert_eq!(role(&c, 1, 1, 3), (Role::Root, true));
        // a guarded port hearing worse is an ordinary designated port
        let mut g2 = g.clone();
        g2.ports[2].root_guard = false;
        g2.ports[0].root_guard = true;
        let c = converge(&g2);
        assert_eq!(c.trees[&(0, 1)].ports[&0].inconsistent, None);
    }

    #[test]
    fn loop_guard_holds_a_port_whose_far_end_sends_nothing_where_it_would_be_alternate() {
        let mut g = Graph {
            bridges: vec![bridge(1, 4096, &[1]), bridge(2, 32768, &[1])],
            ports: vec![
                port(0, 1, &[1]),
                port(0, 2, &[1]),
                port(1, 1, &[1]),
                port(1, 2, &[1]),
            ],
            edges: vec![(0, 2), (1, 3)],
        };
        g.ports[3].loop_guard = true;
        assert_eq!(converge(&g).trees[&(1, 1)].ports[&3].inconsistent, None);
        g.ports[1].filter = true;
        let s = converge(&g).trees[&(1, 1)].ports[&3];
        assert_eq!((s.role, s.forwarding), (Role::Designated, false));
        assert_eq!(s.inconsistent, Some(Inconsistency::Loop));
        // on the root bridge's side the guarded port would stay designated: nothing to hold
        g.ports[3].loop_guard = false;
        g.ports[1].loop_guard = true;
        g.ports[1].filter = false;
        g.ports[3].filter = true;
        assert_eq!(converge(&g).trees[&(0, 1)].ports[&1].inconsistent, None);
    }

    #[test]
    fn a_device_the_lab_does_not_model_roots_the_vlan_it_sends() {
        let mut g = Graph {
            bridges: vec![bridge(1, 32768, &[54])],
            ports: vec![port(0, 1, &[54]), port(0, 2, &[54])],
            edges: vec![],
        };
        let root = BridgeId {
            priority: 0,
            mac: mac(9),
        };
        g.ports[0].heard = Some(Heard {
            rstp: true,
            bpdus: BTreeMap::from([(
                54,
                Bpdu {
                    root,
                    cost: 1500,
                    bridge: BridgeId {
                        priority: 8192,
                        mac: mac(8),
                    },
                    port: (128, 7),
                },
            )]),
        });
        let c = converge(&g);
        let t = &c.trees[&(0, 54)];
        assert_eq!((t.root, t.cost, t.root_port), (root, 21500, Some(0)));
        assert_eq!(role(&c, 0, 54, 1), (Role::Designated, true));
    }
}
