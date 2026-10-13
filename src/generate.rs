//! A switch configuration from a design file: the stack, VLANs, port-channels and their member
//! ports, management, and services. The text is checked twice before it is written: every line is
//! typed into the switch (none may be refused), and the design is checked for consistency.

use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub hostname: String,
    /// members from 1: `C9300X-24Y+C9300X-NM-4C`
    pub stack: Vec<String>,
    /// the IOS XE release the switch runs (`17.9`), 17.15 when absent
    #[serde(default)]
    pub release: Option<String>,
    /// `switch N provision <model>` lines, one per stack member (a 9400 and a switch that does not
    /// stack take none); off for a change to a switch whose model is not the stack's
    #[serde(default = "yes")]
    pub provision: bool,
    /// comment lines at the top of the configuration
    #[serde(default)]
    pub notes: Vec<String>,
    /// `transparent`, `off`, `server` or `client`, written before the VLANs. A new switch is a VTP
    /// server by default and learns the domain over its first trunk, after which the domain keeps
    /// the VLAN database with the highest revision (VLAN configuration guide, Configuring VTP).
    #[serde(default)]
    pub vtp_mode: Option<String>,
    /// VLANs the switch already has, which trunks and access ports may name without defining them
    #[serde(default)]
    pub existing_vlans: Option<String>,
    #[serde(default)]
    pub system_mtu: Option<u32>,
    #[serde(default)]
    pub port_channel_load_balance: Option<String>,
    #[serde(default)]
    pub spanning_tree: Option<SpanningTree>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub name_servers: Vec<String>,
    #[serde(default)]
    pub ntp_servers: Vec<String>,
    #[serde(default)]
    pub syslog_hosts: Vec<String>,
    #[serde(default)]
    pub management: Option<Management>,
    /// SSH version 2 only on the VTY lines, local login through AAA, HTTP and HTTPS servers off;
    /// needs `domain`, which names the RSA key
    #[serde(default)]
    pub ssh_only: bool,
    /// the native VLAN of every trunk
    #[serde(default)]
    pub native_vlan: Option<u32>,
    #[serde(default)]
    pub vlan: Vec<Vlan>,
    #[serde(default)]
    pub port_channel: Vec<PortChannel>,
    /// access ports
    #[serde(default)]
    pub access: Vec<Access>,
    /// ports administratively down
    #[serde(default)]
    pub shutdown: Vec<String>,
    /// the change script around the configuration: checks, steps and rollback
    #[serde(default)]
    pub change: Option<Change>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub title: String,
    /// steps before the change that are not configuration lines
    #[serde(default)]
    pub steps: Vec<String>,
    /// `<command>   (<what it shows>)`; each command is typed into the switch
    #[serde(default)]
    pub before: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
    /// configuration lines that undo the change, typed into the switch after it
    #[serde(default)]
    pub rollback: Vec<String>,
    /// what the rollback leaves, after its heading
    #[serde(default)]
    pub rollback_note: Option<String>,
}

/// The command of a check line, without the expectation in parentheses.
pub fn check_command(line: &str) -> &str {
    line.split("  (").next().unwrap_or(line).trim()
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Access {
    pub port: String,
    pub vlan: u32,
    pub description: String,
    /// host-facing: `spanning-tree portfast` and `spanning-tree bpduguard enable`
    #[serde(default)]
    pub edge: bool,
    /// a sample value to confirm before applying, written as a `! verify:` comment above the port
    #[serde(default)]
    pub verify: Option<String>,
    /// configured but administratively down, such as a spare port
    #[serde(default)]
    pub shutdown: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpanningTree {
    pub mode: String,
    #[serde(default)]
    pub priority: Option<u32>,
    #[serde(default = "all_vlans")]
    pub vlans: String,
}

fn all_vlans() -> String {
    "1-4094".into()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Management {
    pub address: String,
    pub mask: String,
    pub gateway: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vlan {
    pub id: u32,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub verify: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortChannel {
    pub id: u32,
    #[serde(default)]
    pub description: Option<String>,
    /// `459`, `450,456`, or `except 451-453,459`
    pub allowed: String,
    /// member ports, with the description each takes
    #[serde(default)]
    pub members: Vec<Member>,
    /// a port-channel the switch already has: only its allowed list is written, and IOS applies it
    /// to the members
    #[serde(default)]
    pub existing: bool,
    /// the far end, for the change script's port table when there are no members to name it
    #[serde(default)]
    pub connects: Option<String>,
    /// host-facing: `spanning-tree portfast trunk` and `spanning-tree bpduguard enable`
    #[serde(default)]
    pub edge: bool,
    /// `lacp rate fast` on the members
    #[serde(default)]
    pub lacp_fast: bool,
    #[serde(default)]
    pub verify: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub port: String,
    pub description: String,
    #[serde(default)]
    pub verify: Option<String>,
}

/// `! verify: <text>` above an entry whose value is a sample.
fn verify(c: &mut Vec<String>, v: &Option<String>) {
    if let Some(t) = v {
        c.push(format!("! verify: {t}"));
    }
}

/// The VLAN ids IOS takes.
const VLANS: std::ops::RangeInclusive<u32> = 1..=4094;

/// VLAN ids in a list such as `450,456` or `451-453`. A part naming a VLAN outside 1-4094 is left
/// out, as IOS refuses it, so a mistyped range costs nothing (`check` names it in a design).
pub fn vlan_ids(list: &str) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    for part in list.split(',') {
        let part = part.trim();
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>())
                    && VLANS.contains(&a)
                    && VLANS.contains(&b)
                {
                    out.extend(a..=b);
                }
            }
            None => {
                if let Ok(v) = part.parse()
                    && VLANS.contains(&v)
                {
                    out.insert(v);
                }
            }
        }
    }
    out
}

/// The parts of a VLAN list that name a VLAN outside 1-4094.
fn outside_vlans(list: &str) -> Vec<&str> {
    let out = |n: &str| {
        let n = n.trim();
        !n.is_empty()
            && n.bytes().all(|b| b.is_ascii_digit())
            && n.parse::<u32>().map_or(true, |v| !VLANS.contains(&v))
    };
    list.split(',')
        .map(str::trim)
        .filter(|part| part.split('-').any(out))
        .collect()
}

/// The VLANs a `vlan <list>` line defines (`vlan 10`, and a classic switch's `vlan 65-66` held as
/// a global line); none for the other `vlan` commands (`vlan internal allocation policy
/// ascending`, `vlan access-map`, `vlan dot1q tag native`).
pub fn vlan_definition(line: &str) -> Option<BTreeSet<u32>> {
    let list = line.trim().strip_prefix("vlan ")?.trim();
    (!list.is_empty()
        && list
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b',' || b == b'-'))
    .then(|| vlan_ids(list))
}

/// The VLANs a trunk carries: the allowed list, or every defined VLAN outside an `except` list.
pub fn carried(allowed: &str, defined: &BTreeSet<u32>) -> BTreeSet<u32> {
    match allowed.trim().strip_prefix("except ") {
        Some(ex) => {
            let ex = vlan_ids(ex);
            defined
                .iter()
                .copied()
                .filter(|v| !ex.contains(v))
                .collect()
        }
        None => vlan_ids(allowed),
    }
}

/// Design problems: a trunk naming an undefined VLAN or one outside 1-4094, a port in two
/// port-channels or used twice, a port-channel without members or written twice, a setting an
/// existing port-channel does not take, and SSH without the domain name its key needs.
pub fn check(s: &Spec) -> Vec<String> {
    let mut problems = Vec::new();
    let lists = s
        .existing_vlans
        .iter()
        .map(|l| ("existing_vlans".to_string(), l.as_str()))
        .chain(s.port_channel.iter().map(|pc| {
            let allowed = pc.allowed.trim();
            (
                format!("Port-channel{} allowed", pc.id),
                allowed.strip_prefix("except ").unwrap_or(allowed),
            )
        }));
    for (what, list) in lists {
        let bad = outside_vlans(list);
        if !bad.is_empty() {
            problems.push(format!(
                "{what} names VLANs outside 1-4094: {}",
                bad.join(",")
            ));
        }
    }
    // the RSA key is named hostname.domain: IOS generates none until a domain name is set, and
    // the VTY lines then take SSH alone (SSH configuration guide, prerequisites)
    if s.ssh_only && s.domain.is_none() {
        problems.push(
            "ssh_only needs domain: IOS generates no RSA key, so runs no SSH, without a domain name"
                .to_string(),
        );
    }
    let mut defined: BTreeSet<u32> = s.vlan.iter().map(|v| v.id).collect();
    defined.extend(
        s.existing_vlans
            .as_deref()
            .map(vlan_ids)
            .unwrap_or_default(),
    );
    for a in &s.access {
        if !defined.contains(&a.vlan) {
            problems.push(format!(
                "{} is an access port on VLAN {}, which is not defined",
                a.port, a.vlan
            ));
        }
    }
    if let Some(n) = s.native_vlan
        && !defined.contains(&n)
    {
        problems.push(format!("native VLAN {n} is not defined"));
    }
    if s.provision && !s.vlan.is_empty() && s.vtp_mode.is_none() {
        problems.push(
            "a new switch defines VLANs as a VTP server by default; set vtp_mode".to_string(),
        );
    }
    // ports by the name IOS gives them, as `render` writes them: `Twe1/0/1` and
    // `TwentyFiveGigE 1/0/1` are one port
    let port = |p: &str| full(p).to_lowercase();
    let mut members = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for pc in &s.port_channel {
        if !ids.insert(pc.id) {
            problems.push(format!("Port-channel{} is written twice", pc.id));
        }
        if pc.members.is_empty() && !pc.existing {
            problems.push(format!("Port-channel{} has no members", pc.id));
        }
        if pc.existing && !pc.members.is_empty() {
            problems.push(format!(
                "Port-channel{} exists; its members take the allowed list from it",
                pc.id
            ));
        }
        if pc.existing && (pc.edge || pc.lacp_fast) {
            problems.push(format!(
                "Port-channel{} exists; only its description and allowed list are written, not edge or lacp_fast",
                pc.id
            ));
        }
        if !pc.allowed.trim().starts_with("except ") {
            for v in vlan_ids(&pc.allowed) {
                if !defined.contains(&v) {
                    problems.push(format!(
                        "Port-channel{} allows VLAN {v}, which is not defined",
                        pc.id
                    ));
                }
            }
        }
        for m in &pc.members {
            if !members.insert(port(&m.port)) {
                problems.push(format!("{} is in two port-channels", m.port));
            }
        }
    }
    let mut access = BTreeSet::new();
    for a in &s.access {
        if members.contains(&port(&a.port)) || !access.insert(port(&a.port)) {
            problems.push(format!("{} is an access port and used elsewhere", a.port));
        }
    }
    for p in &s.shutdown {
        if members.contains(&port(p)) {
            problems.push(format!("{p} is shut down and a port-channel member"));
        } else if access.contains(&port(p)) {
            problems.push(format!("{p} is shut down and an access port"));
        }
    }
    problems
}

/// Ports the design uses that the hardware leaves without traffic (`Stack::inactive`). A supervisor
/// QSFP port the design uses is enabled (`render` writes `enable` under it).
pub fn inactive_ports(s: &Spec, stack: &crate::platform::Stack) -> Vec<String> {
    let used = s
        .port_channel
        .iter()
        .flat_map(|pc| pc.members.iter().map(|m| m.port.as_str()))
        .chain(s.access.iter().map(|a| a.port.as_str()));
    let used: Vec<&str> = used.collect();
    let enabled: std::collections::BTreeSet<(u32, u32)> = used
        .iter()
        .filter_map(|p| stack.needs_enable(&full(p)))
        .collect();
    used.iter()
        .filter_map(|p| {
            stack
                .inactive_with(&full(p), &enabled)
                .map(|why| format!("{p} {why}"))
        })
        .collect()
}

/// `Twe1/0/1` written as IOS writes it, `TwentyFiveGigE1/0/1`; a name it does not know, as given.
pub fn full(port: &str) -> String {
    let words: Vec<&str> = port.split_whitespace().collect();
    match crate::cli::interface_name(words.first().copied().unwrap_or(""), words.get(1).copied()) {
        Some((ty, num, _)) => format!("{ty}{num}"),
        None => port.to_string(),
    }
}

/// The step before an `ssh_only` configuration: AAA local login needs a local account, and the
/// configuration holds no passwords. The configuration carries it as comments, the change script
/// as its first step.
const ACCOUNT: &[&str] = &[
    "Create a local account before applying this configuration, at the console:",
    "  username <name> privilege 15",
    "  username <name> algorithm-type scrypt secret masked-secret   (prompts for the password)",
];

/// The configuration, in the order a configuration is entered.
pub fn render(s: &Spec) -> Vec<String> {
    // a supervisor QSFP port runs once `enable` turns it on (Supervisor Module Installation Note)
    let stack = crate::platform::parse_stack(&s.stack.join(",")).ok();
    let qsfp = |port: &str| {
        stack
            .as_ref()
            .is_some_and(|st| st.needs_enable(&full(port)).is_some())
    };
    let mut c: Vec<String> = s.notes.iter().map(|n| format!("! {n}")).collect();
    if s.ssh_only {
        c.extend(ACCOUNT.iter().map(|l| format!("! {l}")));
    }
    c.push(format!("hostname {}", s.hostname));
    // each member by its number and model, as `--stack` names them; a 9400's cards and a switch
    // that does not stack take no such line (no capture of either holds one)
    let members = stack.as_ref().map_or(&[][..], |st| &st.members[..]);
    for m in members.iter().filter(|_| s.provision) {
        if crate::platform::model(&m.model).is_some_and(|x| {
            x.family != crate::platform::Family::C9400 && x.family.max_members() > 1
        }) {
            c.push(format!(
                "switch {} provision {}",
                m.number,
                m.model.to_lowercase()
            ));
        }
    }
    if let Some(m) = &s.vtp_mode {
        c.push(format!("vtp mode {m}"));
    }
    if s.ssh_only {
        c.push("aaa new-model".into());
        c.push("aaa authentication login default local".into());
        c.push("aaa authorization exec default local".into());
    }
    if let Some(m) = s.system_mtu {
        c.push(format!("system mtu {m}"));
    }
    if let Some(lb) = &s.port_channel_load_balance {
        c.push(format!("port-channel load-balance {lb}"));
    }
    if let Some(st) = &s.spanning_tree {
        c.push(format!("spanning-tree mode {}", st.mode));
        if let Some(p) = st.priority {
            c.push(format!("spanning-tree vlan {} priority {p}", st.vlans));
        }
    }
    if let Some(d) = &s.domain {
        c.push(format!("ip domain name {d}"));
    }
    let vrf = if s.management.is_some() {
        "vrf Mgmt-vrf "
    } else {
        ""
    };
    if !s.name_servers.is_empty() {
        c.push(format!("ip name-server {vrf}{}", s.name_servers.join(" ")));
    }
    for n in &s.ntp_servers {
        c.push(format!("ntp server {vrf}{n}"));
    }
    for h in &s.syslog_hosts {
        c.push(if s.management.is_some() {
            format!("logging host {h} vrf Mgmt-vrf")
        } else {
            format!("logging host {h}")
        });
    }
    if s.ssh_only {
        // SSH needs a host key; the hostname and domain name above name it
        c.push("crypto key generate rsa modulus 2048".into());
        c.push("ip ssh version 2".into());
        c.push("no ip http server".into());
        c.push("no ip http secure-server".into());
    }
    for v in &s.vlan {
        verify(&mut c, &v.verify);
        c.push(format!("vlan {}", v.id));
        if let Some(n) = &v.name {
            c.push(format!(" name {n}"));
        }
    }
    if let Some(m) = &s.management {
        c.push("interface GigabitEthernet0/0".into());
        c.push(" vrf forwarding Mgmt-vrf".into());
        c.push(format!(" ip address {} {}", m.address, m.mask));
        c.push(format!(
            "ip route vrf Mgmt-vrf 0.0.0.0 0.0.0.0 {}",
            m.gateway
        ));
    }
    let trunk = |c: &mut Vec<String>, allowed: &str| {
        if let Some(n) = s.native_vlan {
            c.push(format!(" switchport trunk native vlan {n}"));
        }
        c.push(format!(" switchport trunk allowed vlan {allowed}"));
        c.push(" switchport mode trunk".into());
        c.push(" switchport nonegotiate".into());
    };
    for pc in &s.port_channel {
        verify(&mut c, &pc.verify);
        c.push(format!("interface Port-channel{}", pc.id));
        if let Some(d) = &pc.description {
            c.push(format!(" description {d}"));
        }
        if pc.existing {
            c.push(format!(" switchport trunk allowed vlan {}", pc.allowed));
            continue;
        }
        trunk(&mut c, &pc.allowed);
        if pc.edge {
            c.push(" spanning-tree portfast trunk".into());
            c.push(" spanning-tree bpduguard enable".into());
        }
        for m in &pc.members {
            verify(&mut c, &m.verify);
            c.push(format!("interface {}", full(&m.port)));
            if qsfp(&m.port) {
                c.push(" enable".into());
            }
            c.push(format!(" description {}", m.description));
            trunk(&mut c, &pc.allowed);
            if pc.lacp_fast {
                c.push(" lacp rate fast".into());
            }
            c.push(format!(" channel-group {} mode active", pc.id));
            c.push(" no shutdown".into());
        }
    }
    for a in &s.access {
        verify(&mut c, &a.verify);
        c.push(format!("interface {}", full(&a.port)));
        if qsfp(&a.port) {
            c.push(" enable".into());
        }
        c.push(format!(" description {}", a.description));
        c.push(format!(" switchport access vlan {}", a.vlan));
        c.push(" switchport mode access".into());
        if a.edge {
            c.push(" spanning-tree portfast".into());
            c.push(" spanning-tree bpduguard enable".into());
        }
        c.push(
            if a.shutdown {
                " shutdown"
            } else {
                " no shutdown"
            }
            .into(),
        );
    }
    for p in &s.shutdown {
        c.push(format!("interface {}", full(p)));
        c.push(" shutdown".into());
    }
    if s.ssh_only {
        c.push("line vty 0 15".into());
        c.push(" transport input ssh".into());
    }
    c.push("end".into());
    c
}

/// The VLAN column of the port table.
fn vlans_text(allowed: &str, native: Option<u32>) -> String {
    let a = match allowed.trim().strip_prefix("except ") {
        Some(ex) => format!("trunk, all except {ex}"),
        None => format!("trunk {allowed}"),
    };
    match native {
        Some(n) => format!("{a}; native {n}"),
        None => a,
    }
}

/// The change script: title, port table, notes, steps and checks before, the configuration to
/// paste, checks after, and the rollback.
pub fn render_change(s: &Spec, config: &[String]) -> Vec<String> {
    let Some(ch) = &s.change else {
        return vec![];
    };
    let mut c = vec![ch.title.clone(), String::new(), "Port assignment".into()];
    c.push("| Port | Connects to | Port-channel | VLANs |".into());
    c.push("|---|---|---|---|".into());
    for pc in &s.port_channel {
        let native = if pc.existing { None } else { s.native_vlan };
        let vl = vlans_text(&pc.allowed, native);
        if pc.members.is_empty() {
            let far = pc
                .connects
                .as_deref()
                .or(pc.description.as_deref())
                .unwrap_or("");
            c.push(format!("| Po{} | {far} | Po{} | {vl} |", pc.id, pc.id));
        }
        for m in &pc.members {
            c.push(format!(
                "| {} | {} | Po{} | {vl} |",
                m.port, m.description, pc.id
            ));
        }
    }
    for a in &s.access {
        let down = if a.shutdown { ", shut down" } else { "" };
        c.push(format!(
            "| {} | {} | - | access {}{down} |",
            a.port, a.description, a.vlan
        ));
    }
    c.push(String::new());
    c.push("Notes".into());
    for n in s.notes.iter().filter(|n| !n.starts_with("Paste")) {
        c.push(format!("- {n}"));
    }
    for l in config.iter().filter(|l| l.starts_with("! verify: ")) {
        c.push(format!("- {}", &l[2..]));
    }
    c.push(String::new());
    c.push("Before the change".into());
    // the configuration's own first step, which its comments carry and the paste block drops
    if s.ssh_only {
        c.extend(ACCOUNT.iter().map(|l| l.to_string()));
    }
    c.extend(ch.steps.iter().cloned());
    c.extend(ch.before.iter().cloned());
    c.push(String::new());
    c.push("Change (paste as one block)".into());
    c.push("enable".into());
    c.push("configure terminal".into());
    c.extend(config.iter().filter(|l| !l.starts_with('!')).cloned());
    c.push("write memory".into());
    c.push(String::new());
    c.push("Checks after".into());
    c.extend(ch.after.iter().cloned());
    if !ch.rollback.is_empty() {
        c.push(String::new());
        c.push(match &ch.rollback_note {
            Some(n) => format!("Rollback (paste as one block; {n})"),
            None => "Rollback (paste as one block)".into(),
        });
        c.push("enable".into());
        c.push("configure terminal".into());
        c.extend(ch.rollback.iter().cloned());
        c.push("end".into());
        c.push("write memory".into());
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlan_lists_and_except_lists() {
        assert_eq!(vlan_ids("450,456"), [450, 456].into());
        assert_eq!(vlan_ids("451-453,459"), [451, 452, 453, 459].into());
        let defined: BTreeSet<u32> = [410, 450, 451, 456, 459, 499].into();
        assert_eq!(
            carried("except 451-453,456,459", &defined),
            [410, 450, 499].into()
        );
        // a part outside 1-4094 names no VLAN, and a mistyped range expands to nothing
        assert_eq!(vlan_ids("10,4090-4100,0,5000"), [10].into());
        assert_eq!(
            outside_vlans("10,4090-4100,0,99999999999"),
            ["4090-4100", "0", "99999999999"]
        );
        // a `vlan` line defines VLANs only with a list after it
        assert_eq!(vlan_definition("vlan 11,65-66"), Some([11, 65, 66].into()));
        assert_eq!(
            vlan_definition("vlan internal allocation policy ascending"),
            None
        );
        assert_eq!(vlan_definition("vlan access-map drop 10"), None);
        assert_eq!(vlan_definition("vlan dot1q tag native"), None);
    }

    fn spec(toml: &str) -> Spec {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn a_spare_access_port_is_configured_and_left_down() {
        let s = spec(
            r#"
hostname = "x"
stack = ["C9300X-24Y"]
vtp_mode = "transparent"
[[vlan]]
id = 499
[[access]]
port = "Twe1/0/5"
vlan = 499
description = "spare"
shutdown = true
[[access]]
port = "Twe1/0/6"
vlan = 499
description = "host"
"#,
        );
        assert!(check(&s).is_empty());
        let c = render(&s);
        let block = |name: &str| -> Vec<String> {
            let at = c.iter().position(|l| l == name).unwrap();
            c[at + 1..]
                .iter()
                .take_while(|l| l.starts_with(' '))
                .cloned()
                .collect()
        };
        let spare = block("interface TwentyFiveGigE1/0/5");
        assert!(spare.contains(&" switchport access vlan 499".to_string()));
        assert_eq!(spare.last().map(String::as_str), Some(" shutdown"));
        let host = block("interface TwentyFiveGigE1/0/6");
        assert_eq!(host.last().map(String::as_str), Some(" no shutdown"));
    }

    #[test]
    fn one_port_written_two_ways_is_one_port() {
        let s = spec(
            r#"
hostname = "x"
stack = ["C9300X-24Y"]
vtp_mode = "transparent"
shutdown = ["TwentyFiveGigE 1/0/1", "Twe1/0/3"]
[[vlan]]
id = 10
[[port_channel]]
id = 11
allowed = "10"
members = [{ port = "Twe1/0/1", description = "a" }, { port = "Twe1/0/2", description = "b" }]
[[port_channel]]
id = 12
allowed = "10"
members = [{ port = "TwentyFiveGigE1/0/1", description = "c" }]
[[access]]
port = "twentyfivegige 1/0/2"
vlan = 10
description = "h"
[[access]]
port = "Twe1/0/3"
vlan = 10
description = "h"
"#,
        );
        assert_eq!(
            check(&s),
            [
                "TwentyFiveGigE1/0/1 is in two port-channels",
                "twentyfivegige 1/0/2 is an access port and used elsewhere",
                "TwentyFiveGigE 1/0/1 is shut down and a port-channel member",
                "Twe1/0/3 is shut down and an access port",
            ]
        );
    }

    #[test]
    fn a_design_that_render_would_drop_or_ios_would_refuse_is_named() {
        let s = spec(
            r#"
hostname = "x"
stack = ["C9300X-24Y"]
ssh_only = true
existing_vlans = "1,10,5000"
[[port_channel]]
id = 11
allowed = "10"
members = [{ port = "Twe1/0/1", description = "a" }]
[[port_channel]]
id = 11
allowed = "except 4090-4100"
members = [{ port = "Twe1/0/2", description = "b" }]
[[port_channel]]
id = 12
allowed = "10"
existing = true
edge = true
"#,
        );
        assert_eq!(
            check(&s),
            [
                "existing_vlans names VLANs outside 1-4094: 5000",
                "Port-channel11 allowed names VLANs outside 1-4094: 4090-4100",
                "ssh_only needs domain: IOS generates no RSA key, so runs no SSH, without a domain name",
                "Port-channel11 is written twice",
                "Port-channel12 exists; only its description and allowed list are written, not edge or lacp_fast",
            ]
        );
    }

    #[test]
    fn provision_lines_name_each_member_by_its_number_and_model() {
        let provisions = |stack: &str| -> Vec<String> {
            render(&spec(&format!("hostname = \"x\"\nstack = {stack}\n")))
                .into_iter()
                .filter(|l| l.starts_with("switch "))
                .collect()
        };
        assert_eq!(
            provisions(r#"["C9300X-24Y+C9300X-NM-4C", "3=C9300-48P"]"#),
            [
                "switch 1 provision c9300x-24y",
                "switch 3 provision c9300-48p"
            ]
        );
        assert_eq!(
            provisions(r#"["WS-C2960X-24PS-L"]"#),
            ["switch 1 provision ws-c2960x-24ps-l"]
        );
        // a 2960-Plus does not stack and a 9400 is a chassis: no capture of either holds the line
        assert!(provisions(r#"["WS-C2960+24TC-L"]"#).is_empty());
        assert!(provisions(r#"["1=C9400-LC-48S", "3=C9400-SUP-1XL"]"#).is_empty());
    }

    #[test]
    fn the_change_script_carries_the_account_step_the_configuration_comments_hold() {
        let s = spec(
            r#"
hostname = "sw1"
stack = ["C9300-48P"]
domain = "example.net"
ssh_only = true
[change]
title = "SSH only"
"#,
        );
        let config = render(&s);
        let c = render_change(&s, &config);
        let before = c.iter().position(|l| l == "Before the change").unwrap();
        assert_eq!(c[before + 1..before + 4], *ACCOUNT);
        for l in ACCOUNT {
            assert!(config.contains(&format!("! {l}")), "{l}");
        }
    }

    #[test]
    fn design_problems_are_named() {
        let s: Spec = toml::from_str(
            r#"
hostname = "x"
stack = ["C9300X-24Y"]
native_vlan = 499
[[vlan]]
id = 459
[[port_channel]]
id = 11
description = "a"
allowed = "459,460"
members = [{ port = "Twe1/0/1", description = "a" }]
[[port_channel]]
id = 12
description = "b"
allowed = "459"
members = [{ port = "Twe1/0/1", description = "b" }]
"#,
        )
        .unwrap();
        let p = check(&s);
        assert!(p.iter().any(|x| x.contains("native VLAN 499")));
        assert!(p.iter().any(|x| x.contains("allows VLAN 460")));
        assert!(p.iter().any(|x| x.contains("in two port-channels")));
    }
}
