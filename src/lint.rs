//! Checks on a configuration as it stands, however it was written (a capture, a console session, a
//! generated file): what IOS takes but leaves broken or contradictory. Each finding names the
//! interface and the rule.

use crate::device::Device;
use crate::generate::{vlan_definition, vlan_ids};
use crate::target::ios_list;
use std::collections::{BTreeMap, BTreeSet};

/// What the checks found: findings IOS acts on (a member suspended, a setting with no effect), notes
/// on choices a design may intend, and the checks they could not make.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub findings: Vec<String>,
    pub notes: Vec<String>,
    pub skipped: Vec<String>,
}

/// One interface's settings, read from its block.
struct Port<'d> {
    name: &'d str,
    lines: Vec<&'d str>,
}

impl<'d> Port<'d> {
    fn value(&self, prefix: &str) -> Option<&'d str> {
        self.lines.iter().find_map(|l| l.strip_prefix(prefix))
    }

    fn has(&self, line: &str) -> bool {
        self.lines.contains(&line)
    }

    /// The switchport mode; dynamic auto with no mode line, as `show interfaces switchport` prints
    /// such a port (c9300 17.15 capture 5's Te1/1/1-8).
    fn mode(&self) -> &'d str {
        self.value("switchport mode ").unwrap_or("dynamic auto")
    }

    /// The VLANs a trunk carries: its allowed list, every VLAN when none is set.
    fn allowed(&self) -> Option<BTreeSet<u32>> {
        match self.value("switchport trunk allowed vlan ") {
            None | Some("all") => None,
            Some("none") => Some(BTreeSet::new()),
            Some(list) => Some(vlan_ids(list)),
        }
    }

    /// The native VLAN a line sets; `switchport trunk native vlan tag` sets tagging, not the VLAN.
    fn native_set(&self) -> Option<u32> {
        self.lines
            .iter()
            .filter_map(|l| l.strip_prefix("switchport trunk native vlan "))
            .find_map(|v| v.parse().ok())
    }

    fn native(&self) -> u32 {
        self.native_set().unwrap_or(1)
    }

    /// The VRF the interface forwards in: `vrf forwarding`, or the `ip vrf forwarding` of an
    /// `ip vrf`; "" for the global table.
    fn vrf(&self) -> &'d str {
        self.value("vrf forwarding ")
            .or_else(|| self.value("ip vrf forwarding "))
            .unwrap_or("")
    }

    fn channel(&self) -> Option<(u32, &'d str)> {
        let spec = self.value("channel-group ")?;
        let mut w = spec.split_whitespace();
        let n = w.next()?.parse().ok()?;
        let mode = w.nth(1).unwrap_or("");
        Some((n, mode))
    }
}

fn protocol(mode: &str) -> &'static str {
    match mode {
        "active" | "passive" => "LACP",
        "desirable" | "auto" => "PAgP",
        _ => "on",
    }
}

/// An IPv4 address and mask as numbers.
fn ipv4(text: &str) -> Option<u32> {
    let parts: Vec<u32> = text.split('.').filter_map(|p| p.parse().ok()).collect();
    match parts[..] {
        [a, b, c, d] if parts.iter().all(|p| *p < 256) => Some(a << 24 | b << 16 | c << 8 | d),
        _ => None,
    }
}

pub fn lint(d: &Device) -> Report {
    checks(d, false)
}

/// `lint` on a stack inferred from the configuration (`platform::infer_stack`). A running
/// configuration lists a module slot's ports whatever module is fitted, so the inferred module is
/// unknown: the hardware rule leaves the slot ports out and says so.
pub fn lint_inferred(d: &Device) -> Report {
    checks(d, true)
}

fn checks(d: &Device, inferred: bool) -> Report {
    let mut r = Report::default();
    let ports: Vec<Port> = d
        .blocks
        .iter()
        .filter_map(|b| {
            Some(Port {
                name: b.header.strip_prefix("interface ")?,
                lines: b.lines.iter().map(|e| e.text.as_str()).collect(),
            })
        })
        .collect();
    let mut notes: Vec<String> = vec![];
    let mut found = |at: &str, what: String| r.findings.push(format!("{at}: {what}"));

    // VLANs: a VTP server's database is in vlan.dat, so a capture of one holds none. A VLAN
    // block defines them, and so does a range the switch holds as a global line (`vlan 65-66` on
    // the IE-3000s at 15.2(6)E)
    let mut defined: BTreeSet<u32> = [1, 1002, 1003, 1004, 1005].into();
    let vlan_lines = d.blocks.iter().map(|b| b.header.as_str());
    for line in vlan_lines.chain(d.global.iter().map(|e| e.text.as_str())) {
        if let Some(ids) = vlan_definition(line) {
            defined.extend(ids);
        }
    }
    let database = d.vlans_in_running_config() || defined.len() > 5;
    // VTP version 3 keeps extended-range VLANs on a server as well (VLAN configuration guide)
    let v3 = d.global.iter().any(|e| e.text == "vtp version 3");
    if !d.vlans_in_running_config() && !v3 {
        for id in defined.iter().filter(|v| (1006..=4094).contains(*v)) {
            found(
                &format!("vlan {id}"),
                "an extended-range VLAN needs VTP transparent mode (VTP versions 1 and 2, command reference)".into(),
            );
        }
    }
    for p in &ports {
        if p.has("no switchport") {
            continue;
        }
        if p.mode() == "trunk" {
            if let Some(allowed) = p.allowed() {
                // a list naming VLANs; one that carries most of them is an `all` or `except` list
                // resolved, and names no VLAN in particular
                let missing: BTreeSet<u32> = allowed.difference(&defined).copied().collect();
                if database && !missing.is_empty() && allowed.len() <= 1000 {
                    found(
                        p.name,
                        format!("carries VLANs {} that are not defined", ios_list(&missing)),
                    );
                }
                // a configured native VLAN left off the list drops untagged frames, which a design
                // may intend; the default VLAN 1 is often pruned on purpose (c9400 17.5 capture 2's Gi1/0/48)
                if let Some(native) = p.native_set()
                    && !allowed.contains(&native)
                {
                    notes.push(format!(
                        "{}: native VLAN {native} is not in its allowed list, so untagged frames are dropped",
                        p.name
                    ));
                }
            }
            // portfast on a trunk takes `trunk` (STP guide); plain portfast, and the `edge` form
            // classic IOS takes from 15.2(2)E, act only off a trunk
            for (line, fix) in [
                ("spanning-tree portfast", "spanning-tree portfast trunk"),
                (
                    "spanning-tree portfast edge",
                    "spanning-tree portfast edge trunk",
                ),
            ] {
                if p.has(line) {
                    found(
                        p.name,
                        format!(
                            "`{line}` acts only while the port is not trunking; a trunk takes `{fix}`"
                        ),
                    );
                }
            }
        } else if let Some(v) = p.value("switchport access vlan ")
            && database
            && v.parse().is_ok_and(|v: u32| !defined.contains(&v))
        {
            found(
                p.name,
                format!(
                    "access VLAN {v} is not defined (IOS creates it, unnamed, when the line is typed)"
                ),
            );
        }
        if p.has("spanning-tree bpduguard enable") && p.has("spanning-tree bpdufilter enable") {
            found(
                p.name,
                "BPDU filter drops the BPDUs BPDU guard would act on".into(),
            );
        }
    }
    if !database {
        r.skipped.push(
            "VLAN definitions: the configuration holds none, as a VTP server keeps them in vlan.dat; trunk and access VLANs are not checked against them".into(),
        );
    }

    // port-channels: members match each other and their port-channel, or IOS suspends them
    let mut groups: BTreeMap<u32, Vec<&Port>> = BTreeMap::new();
    for p in &ports {
        if let Some((n, _)) = p.channel() {
            groups.entry(n).or_default().push(p);
        }
    }
    let settings = |p: &Port| -> Vec<(&'static str, String)> {
        vec![
            ("switchport mode", p.mode().to_string()),
            (
                "switchport access vlan",
                p.value("switchport access vlan ")
                    .unwrap_or("1")
                    .to_string(),
            ),
            (
                "switchport trunk allowed vlan",
                p.value("switchport trunk allowed vlan ")
                    .unwrap_or("all")
                    .to_string(),
            ),
            ("switchport trunk native vlan", p.native().to_string()),
        ]
    };
    for (n, members) in &groups {
        let protocols: BTreeSet<&str> = members
            .iter()
            .filter_map(|m| m.channel().map(|c| protocol(c.1)))
            .collect();
        if protocols.len() > 1 {
            found(
                &format!("Port-channel{n}"),
                format!(
                    "members mix {}",
                    protocols.into_iter().collect::<Vec<_>>().join(" and ")
                ),
            );
        }
        let po_name = format!("Port-channel{n}");
        let reference = ports
            .iter()
            .find(|p| p.name == po_name)
            .or(members.first().copied());
        let Some(reference) = reference else { continue };
        let want = settings(reference);
        for m in members {
            if m.has("shutdown") {
                found(m.name, format!("is shut down and a member of {po_name}"));
            }
            if std::ptr::eq(*m, reference) {
                continue;
            }
            for ((what, a), (_, b)) in settings(m).iter().zip(&want) {
                if a != b {
                    found(
                        m.name,
                        format!(
                            "{what} is {a}, {} has {b}: IOS suspends a member that differs",
                            reference.name
                        ),
                    );
                }
            }
            for what in ["speed ", "duplex "] {
                let mine = m.value(what).unwrap_or("auto");
                let first = members[0].value(what).unwrap_or("auto");
                if mine != first {
                    found(
                        m.name,
                        format!("{}is {mine}, {} has {first}", what, members[0].name),
                    );
                }
            }
        }
    }

    // addresses: one interface per address, no two subnets overlapping in a VRF. Every address
    // counts, a secondary as the primary, each against the other interfaces' addresses
    let mut nets: Vec<(&str, &str, u32, u32)> = vec![];
    for p in &ports {
        let vrf = p.vrf();
        let mut mine = vec![];
        let mut said = BTreeSet::new();
        for spec in p.lines.iter().filter_map(|l| l.strip_prefix("ip address ")) {
            let mut w = spec.split_whitespace();
            let (Some(a), Some(m)) = (w.next().and_then(ipv4), w.next().and_then(ipv4)) else {
                continue;
            };
            for (other, ovrf, oa, om) in &nets {
                if *ovrf != vrf {
                    continue;
                }
                let what = if *oa == a {
                    format!("has the address {other} has")
                } else if a & (m & om) == oa & (m & om) {
                    format!("its subnet overlaps {other}'s")
                } else {
                    continue;
                };
                if said.insert(what.clone()) {
                    found(p.name, what);
                }
            }
            mine.push((p.name, vrf, a, m));
        }
        nets.extend(mine);
    }

    // ports the hardware leaves without traffic, configured as if they carried it
    let enabled = crate::show::enabled_uplinks(d);
    let mut slot_ports = false;
    for p in &ports {
        if p.lines.is_empty() || p.has("shutdown") {
            continue;
        }
        if inferred && matches!(crate::platform::locate(p.name), Some((_, _, 1, _))) {
            slot_ports = true;
            continue;
        }
        if let Some(why) = d.stack.inactive_with(p.name, &enabled) {
            found(p.name, format!("is configured but {why}"));
        }
    }
    if slot_ports {
        r.skipped.push(
            "network module ports: the hardware is inferred from the configuration, which lists a module slot whatever is fitted; give it with --stack".into(),
        );
    }

    // PoE: a port without it takes `power inline` to no effect, a `max` above the port's own is
    // more than it gives, and static ports reserve their power from the member's budget
    let mut reserved: BTreeMap<u32, u32> = BTreeMap::new();
    for p in &ports {
        if !p.lines.iter().any(|l| l.starts_with("power inline")) {
            continue;
        }
        let Some(poe) = d.stack.poe(p.name) else {
            found(p.name, "has no PoE: `power inline` does nothing".into());
            continue;
        };
        let most = poe.kind.max_mw();
        let (admin, mw) = crate::show::poe_setting(&p.lines, most);
        if mw > most {
            found(
                p.name,
                format!("power inline {admin} max {mw} is more than the port gives ({most} mW)"),
            );
        }
        if admin == "static"
            && let Some(member) = p.name[p.name.find(|c: char| c.is_ascii_digit()).unwrap_or(0)..]
                .split('/')
                .next()
                .and_then(|n| n.parse().ok())
        {
            *reserved.entry(member).or_default() += mw.min(most);
        }
    }
    for (member, mw) in reserved {
        let budget = d
            .stack
            .members
            .iter()
            .find(|m| m.number == member)
            .and_then(|m| crate::platform::model(&m.model))
            .and_then(|m| m.poe)
            .and_then(|p| p.watts);
        if let Some(w) = budget
            && mw > w * 1000
        {
            notes.push(format!(
                "member {member}: static PoE reservations total {} W, more than the {w} W its default power supply gives",
                mw / 1000
            ));
        }
    }
    r.notes = notes;
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{Block, Entry};

    fn device(blocks: &[(&str, &[&str])], global: &[&str]) -> Device {
        let mut d = Device::new(crate::platform::parse_stack("C9300X-24Y").unwrap());
        d.global = global
            .iter()
            .map(|t| Entry {
                key: t.to_string(),
                text: t.to_string(),
            })
            .collect();
        d.blocks = blocks
            .iter()
            .map(|(h, lines)| Block {
                header: h.to_string(),
                lines: lines
                    .iter()
                    .map(|t| Entry {
                        key: t.to_string(),
                        text: t.to_string(),
                    })
                    .collect(),
            })
            .collect();
        d
    }

    #[test]
    fn poe_lines_need_a_poe_port_within_its_power() {
        let on = |stack: &str, blocks: &[(&str, &[&str])]| {
            let mut d = device(blocks, &[]);
            d.stack = crate::platform::parse_stack(stack).unwrap();
            lint(&d)
        };
        let r = on(
            "C9300X-24Y",
            &[("interface TwentyFiveGigE1/0/1", &["power inline never"])],
        );
        assert_eq!(
            r.findings,
            ["TwentyFiveGigE1/0/1: has no PoE: `power inline` does nothing"]
        );
        // a PoE+ port gives 30 W, a UPOE port 60 W
        let r = on(
            "C9300-48P",
            &[
                (
                    "interface GigabitEthernet1/0/1",
                    &["power inline static max 60000"],
                ),
                (
                    "interface GigabitEthernet1/0/2",
                    &["power inline auto max 30000"],
                ),
            ],
        );
        assert_eq!(
            r.findings,
            [
                "GigabitEthernet1/0/1: power inline static max 60000 is more than the port gives (30000 mW)"
            ]
        );
        assert!(
            on(
                "C9300-48U",
                &[(
                    "interface GigabitEthernet1/0/1",
                    &["power inline static max 60000"]
                )]
            )
            .findings
            .is_empty()
        );
        // the uplinks are not PoE ports
        let r = on(
            "C9300-48P+C9300-NM-8X",
            &[
                ("interface TenGigabitEthernet1/1/1", &["power inline never"]),
                ("interface AppGigabitEthernet1/0/1", &["power inline never"]),
            ],
        );
        assert_eq!(r.findings.len(), 2, "{r:?}");
        // fifteen static 30 W ports reserve 450 W of a C9300-48P's 437 W
        let ports: Vec<String> = (1..=15)
            .map(|n| format!("interface GigabitEthernet1/0/{n}"))
            .collect();
        let blocks: Vec<(&str, &[&str])> = ports
            .iter()
            .map(|h| (h.as_str(), &["power inline static"][..]))
            .collect();
        let r = on("C9300-48P", &blocks);
        assert!(r.findings.is_empty(), "{r:?}");
        assert_eq!(
            r.notes,
            [
                "member 1: static PoE reservations total 450 W, more than the 437 W its default power supply gives"
            ]
        );
        assert!(on("C9300-48P", &blocks[..14]).notes.is_empty());
        // a 9400 PoE card's budget is the chassis's: its ports take PoE, no budget is checked
        let r = on(
            "1=C9400-LC-48U,3=C9400-SUP-1XL",
            &[(
                "interface GigabitEthernet1/0/1",
                &["power inline static max 60000"],
            )],
        );
        assert!(r.findings.is_empty() && r.notes.is_empty(), "{r:?}");
    }

    #[test]
    fn trunks_and_access_ports_name_vlans_the_database_has() {
        let d = device(
            &[
                ("vlan 10", &[]),
                (
                    "interface TwentyFiveGigE1/0/1",
                    &[
                        "switchport trunk native vlan 99",
                        "switchport trunk allowed vlan 10,20-21",
                        "switchport mode trunk",
                        "spanning-tree portfast",
                    ],
                ),
                (
                    "interface TwentyFiveGigE1/0/2",
                    &["switchport access vlan 30", "switchport mode access"],
                ),
            ],
            &["vtp mode transparent"],
        );
        let r = lint(&d);
        let has = |s: &str| r.findings.iter().any(|f| f.contains(s));
        assert!(
            has("TwentyFiveGigE1/0/1: carries VLANs 20,21 that are not defined"),
            "{r:?}"
        );
        assert!(has("a trunk takes `spanning-tree portfast trunk`"));
        assert!(has("TwentyFiveGigE1/0/2: access VLAN 30 is not defined"));
        assert!(r.skipped.is_empty());
        // a native VLAN left off the list may be a black hole on purpose: a note
        assert!(!has("native"));
        assert_eq!(
            r.notes,
            [
                "TwentyFiveGigE1/0/1: native VLAN 99 is not in its allowed list, so untagged frames are dropped"
            ]
        );
    }

    #[test]
    fn a_vlan_range_held_as_a_global_line_defines_its_vlans() {
        // the IE-3000s at 15.2(6)E keep `vlan 65-66` at the top level, not as a VLAN block
        let d = device(
            &[
                ("vlan 70", &[]),
                (
                    "interface TwentyFiveGigE1/0/1",
                    &["switchport access vlan 66"],
                ),
                (
                    "interface TwentyFiveGigE1/0/2",
                    &["switchport access vlan 67"],
                ),
            ],
            &[
                "vtp mode transparent",
                "vlan 65-66",
                "vlan internal allocation policy ascending",
            ],
        );
        assert_eq!(
            lint(&d).findings,
            [
                "TwentyFiveGigE1/0/2: access VLAN 67 is not defined (IOS creates it, unnamed, when the line is typed)"
            ]
        );
    }

    #[test]
    fn a_trunk_names_the_portfast_and_native_vlan_it_has() {
        let d = device(
            &[
                (
                    "interface TwentyFiveGigE1/0/1",
                    &[
                        "switchport trunk native vlan tag",
                        "switchport trunk allowed vlan 10,20",
                        "switchport mode trunk",
                        "spanning-tree portfast edge",
                    ],
                ),
                (
                    "interface TwentyFiveGigE1/0/2",
                    &[
                        "switchport trunk native vlan tag",
                        "switchport trunk native vlan 99",
                        "switchport trunk allowed vlan 10,20",
                        "switchport mode trunk",
                    ],
                ),
            ],
            &[],
        );
        let r = lint(&d);
        // the `edge` form acts only off a trunk too
        assert_eq!(
            r.findings,
            [
                "TwentyFiveGigE1/0/1: `spanning-tree portfast edge` acts only while the port is not trunking; a trunk takes `spanning-tree portfast edge trunk`"
            ]
        );
        // `native vlan tag` sets tagging, not a native VLAN
        assert_eq!(
            r.notes,
            [
                "TwentyFiveGigE1/0/2: native VLAN 99 is not in its allowed list, so untagged frames are dropped"
            ]
        );
    }

    #[cfg(all(test, captures))]
    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/lint-tests.rs"));

    #[test]
    fn an_all_but_list_names_no_vlan_in_particular() {
        // `except 451-452` resolves to nearly every VLAN; the cluster core trunks carry such lists
        let d = device(
            &[(
                "interface TwentyFiveGigE1/0/1",
                &[
                    "switchport trunk allowed vlan 1-450,453-4094",
                    "switchport mode trunk",
                ],
            )],
            &["vtp mode transparent"],
        );
        assert!(lint(&d).findings.is_empty(), "{:?}", lint(&d));
    }

    #[test]
    fn a_vtp_server_capture_skips_the_vlan_checks() {
        let d = device(
            &[(
                "interface TwentyFiveGigE1/0/1",
                &[
                    "switchport trunk allowed vlan 10,20",
                    "switchport mode trunk",
                ],
            )],
            &[],
        );
        let r = lint(&d);
        assert!(
            r.findings.iter().all(|f| !f.contains("not defined")),
            "{r:?}"
        );
        assert_eq!(r.skipped.len(), 1);
        // VLAN 1 left off a trunk with the default native VLAN is a choice: neither a finding nor
        // a note
        assert!(r.findings.iter().all(|f| !f.contains("native")), "{r:?}");
        assert!(r.notes.is_empty(), "{r:?}");
    }

    #[test]
    fn port_channel_members_match_their_port_channel() {
        let d = device(
            &[
                (
                    "interface Port-channel10",
                    &["switchport trunk allowed vlan 10", "switchport mode trunk"],
                ),
                (
                    "interface TwentyFiveGigE1/0/1",
                    &[
                        "switchport trunk allowed vlan 10",
                        "switchport mode trunk",
                        "channel-group 10 mode active",
                    ],
                ),
                (
                    "interface TwentyFiveGigE1/0/2",
                    &[
                        "switchport trunk allowed vlan 10,20",
                        "switchport mode trunk",
                        "channel-group 10 mode on",
                        "speed 1000",
                        "shutdown",
                    ],
                ),
                (
                    "interface TwentyFiveGigE1/0/3",
                    &["channel-group 10 mode active"],
                ),
            ],
            &["vtp mode transparent"],
        );
        let r = lint(&d);
        let has = |s: &str| r.findings.iter().any(|f| f.contains(s));
        assert!(has("Port-channel10: members mix LACP and on"), "{r:?}");
        // a port with no mode line runs the default
        assert!(has(
            "TwentyFiveGigE1/0/3: switchport mode is dynamic auto, Port-channel10 has trunk"
        ));
        assert!(has(
            "TwentyFiveGigE1/0/2: speed is 1000, TwentyFiveGigE1/0/1 has auto"
        ));
        assert!(has(
            "TwentyFiveGigE1/0/2: switchport trunk allowed vlan is 10,20, Port-channel10 has 10"
        ));
        assert!(has(
            "TwentyFiveGigE1/0/2: is shut down and a member of Port-channel10"
        ));
        assert!(!has("TwentyFiveGigE1/0/1: switchport"));
    }

    #[test]
    fn an_extended_range_vlan_needs_transparent_mode() {
        let server = device(&[("vlan 2000", &[]), ("vlan 20", &[])], &[]);
        assert!(
            lint(&server)
                .findings
                .iter()
                .any(|f| f.starts_with("vlan 2000: an extended-range VLAN needs VTP transparent"))
        );
        let transparent = device(&[("vlan 2000", &[])], &["vtp mode transparent"]);
        assert!(lint(&transparent).findings.is_empty());
        // VTP version 3 keeps them on a server
        let v3 = device(&[("vlan 2000", &[])], &["vtp version 3", "vtp mode server"]);
        assert!(lint(&v3).findings.is_empty(), "{:?}", lint(&v3));
    }

    #[test]
    fn addresses_and_bpdu_settings_do_not_contradict() {
        let d = device(
            &[
                ("interface Vlan10", &["ip address 10.0.10.1 255.255.255.0"]),
                ("interface Vlan20", &["ip address 10.0.10.1 255.255.255.0"]),
                (
                    "interface Vlan30",
                    &["ip address 10.0.10.129 255.255.255.128"],
                ),
                (
                    "interface Vlan40",
                    &[
                        "vrf forwarding Mgmt-vrf",
                        "ip address 10.0.10.1 255.255.255.0",
                    ],
                ),
                (
                    "interface Vlan50",
                    &["ip vrf forwarding B", "ip address 10.0.10.1 255.255.255.0"],
                ),
                (
                    "interface TwentyFiveGigE1/0/3",
                    &[
                        "spanning-tree bpduguard enable",
                        "spanning-tree bpdufilter enable",
                    ],
                ),
            ],
            &[],
        );
        let r = lint(&d);
        let has = |s: &str| r.findings.iter().any(|f| f.contains(s));
        assert!(has("Vlan20: has the address Vlan10 has"), "{r:?}");
        assert!(has("Vlan30: its subnet overlaps Vlan10's"));
        assert!(!has("Vlan40"), "another VRF");
        assert!(!has("Vlan50"), "an `ip vrf`'s VRF: {r:?}");
        assert!(has("TwentyFiveGigE1/0/3: BPDU filter drops"));
    }

    #[test]
    fn every_address_counts_a_secondary_as_the_primary() {
        let d = device(
            &[
                (
                    "interface Vlan10",
                    &[
                        "ip address 10.0.20.1 255.255.255.0 secondary",
                        "ip address 10.0.10.1 255.255.255.0",
                    ],
                ),
                ("interface Vlan20", &["ip address 10.0.10.1 255.255.255.0"]),
                (
                    "interface Vlan30",
                    &[
                        "ip address 10.0.30.1 255.255.255.0",
                        "ip address 10.0.20.2 255.255.255.0 secondary",
                        "ip address 10.0.20.3 255.255.255.0 secondary",
                    ],
                ),
            ],
            &[],
        );
        // one finding per pair of interfaces, however many of their addresses meet
        assert_eq!(
            lint(&d).findings,
            [
                "Vlan20: has the address Vlan10 has",
                "Vlan30: its subnet overlaps Vlan10's"
            ]
        );
    }
}
