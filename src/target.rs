//! A captured running configuration with a change applied, written as IOS would print it: the
//! target configuration a change script leads to. Interface lines replace their own command in
//! place, `no` removes it, a new line goes where the capture's release prints it, an allowed-VLAN
//! list is stored resolved, a port-channel's switchport lines reach its members, and a new
//! port-channel takes its place among the interfaces. A line this does not know is an error, never
//! a guess.

use std::collections::BTreeSet;

/// Interface commands in the order IOS prints them where the capture shows none (the order in the
/// two cores' 17.5 running configurations; native before allowed and nonegotiate after mode as IOS
/// writes them). No capture holds `enable`, which turns a 9400 supervisor's QSFP port on: it goes
/// first, where `generate` types it and the virtual switch prints it.
const KEYS: &[&str] = &[
    "enable",
    "description",
    "switchport access vlan",
    "switchport trunk native vlan",
    "switchport trunk allowed vlan",
    "switchport mode",
    "switchport nonegotiate",
    "shutdown",
    "speed",
    "duplex",
    "lacp rate",
    "channel-group",
    "spanning-tree portfast",
    "spanning-tree bpduguard",
];

fn key_of(line: &str) -> Option<usize> {
    let l = line.trim();
    KEYS.iter()
        .position(|k| l == *k || l.starts_with(&format!("{k} ")))
}

/// VLAN ids `1-4094` as IOS lists them: a range for three or more consecutive VLANs, single ids
/// otherwise (Cisco's example `1,2,4-3003,3005-4094`).
pub fn ios_list(set: &BTreeSet<u32>) -> String {
    let v: Vec<u32> = set.iter().copied().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j + 1 < v.len() && v[j + 1] == v[j] + 1 {
            j += 1;
        }
        if j - i >= 2 {
            out.push(format!("{}-{}", v[i], v[j]));
        } else {
            out.extend(v[i..=j].iter().map(u32::to_string));
        }
        i = j + 1;
    }
    out.join(",")
}

fn all() -> BTreeSet<u32> {
    (1..=4094).collect()
}

/// The allowed list after `switchport trunk allowed vlan <spec>`, from the current one (none
/// configured is every VLAN). The vlan-list format is `all | none | [add | remove | except] list`
/// (switchport trunk, Usage Guidelines).
pub fn allowed_after(current: Option<&str>, spec: &str) -> BTreeSet<u32> {
    let cur = match current {
        None => all(),
        Some("none") => BTreeSet::new(),
        Some(l) => crate::generate::vlan_ids(l),
    };
    let spec = spec.trim();
    let (op, list) = spec.split_once(' ').unwrap_or((spec, ""));
    match op {
        "all" => all(),
        "none" => BTreeSet::new(),
        "add" => cur
            .union(&crate::generate::vlan_ids(list))
            .copied()
            .collect(),
        "remove" => cur
            .difference(&crate::generate::vlan_ids(list))
            .copied()
            .collect(),
        "except" => all()
            .difference(&crate::generate::vlan_ids(list))
            .copied()
            .collect(),
        _ => crate::generate::vlan_ids(spec),
    }
}

/// The line IOS prints for an allowed set; none when every VLAN is allowed (the default).
pub fn allowed_line(set: &BTreeSet<u32>) -> Option<String> {
    if *set == all() {
        None
    } else if set.is_empty() {
        Some("switchport trunk allowed vlan none".into())
    } else {
        Some(format!("switchport trunk allowed vlan {}", ios_list(set)))
    }
}

#[derive(Debug, Clone)]
enum Item {
    Text(String),
    Block { header: String, lines: Vec<String> },
}

#[derive(Clone)]
pub struct Target {
    items: Vec<Item>,
    /// the running configuration lists no VLANs: VTP holds them
    vtp_vlans: bool,
    /// (a, b): an interface block of the capture prints a line of key a above one of key b
    order: BTreeSet<(usize, usize)>,
    /// the capture prints `spanning-tree portfast edge` and never the bare `spanning-tree
    /// portfast`, which its release stores in the edge form (15.2(6)E and 15.2(7)E captures)
    portfast_edge: bool,
    pub notes: Vec<String>,
}

/// Whether IOS prints a line of key `a` above one of key `b`: as the capture does where it shows
/// both in one block in one order (c2960x 15.2(7)E capture 8 at 15.2(7)E prints allowed above native), in
/// `KEYS` order otherwise.
fn above(order: &BTreeSet<(usize, usize)>, a: usize, b: usize) -> bool {
    match (order.contains(&(a, b)), order.contains(&(b, a))) {
        (true, false) => true,
        (false, true) => false,
        _ => a < b,
    }
}

fn parse(text: &str) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    for l in text.lines() {
        let l = l.trim_end().to_string();
        if l.starts_with(' ')
            && let Some(Item::Block { lines, .. }) = items.last_mut()
        {
            lines.push(l);
            continue;
        }
        if l.is_empty() || l.starts_with('!') {
            items.push(Item::Text(l));
        } else {
            items.push(Item::Block {
                header: l,
                lines: vec![],
            });
        }
    }
    items
}

/// `interface Port-channel40` sorts before every physical and VLAN interface, by number.
fn channel_number(header: &str) -> Option<u32> {
    header
        .strip_prefix("interface Port-channel")
        .and_then(|n| n.parse().ok())
}

impl Target {
    pub fn new(running: &str) -> Self {
        let items = parse(running);
        // a VTP server or client lists no VLAN; `vlan internal allocation policy ascending`, which
        // classic IOS and the IE-3300 print whatever the VTP mode, defines none
        let vtp_vlans = !items.iter().any(|i| match i {
            Item::Block { header, .. } => {
                crate::generate::vlan_definition(header).is_some()
                    || header == "vtp mode transparent"
                    || header == "vtp mode off"
            }
            Item::Text(_) => false,
        });
        let mut order = BTreeSet::new();
        let (mut edge, mut bare) = (false, false);
        for i in &items {
            let Item::Block { header, lines } = i else {
                continue;
            };
            if !header.starts_with("interface ") {
                continue;
            }
            let keys: Vec<usize> = lines.iter().filter_map(|l| key_of(l)).collect();
            for (n, &a) in keys.iter().enumerate() {
                order.extend(keys[n + 1..].iter().filter(|&&b| b != a).map(|&b| (a, b)));
            }
            for l in lines {
                match l.trim() {
                    "spanning-tree portfast" => bare = true,
                    "spanning-tree portfast edge" => edge = true,
                    _ => {}
                }
            }
        }
        Target {
            items,
            vtp_vlans,
            order,
            portfast_edge: edge && !bare,
            notes: vec![],
        }
    }

    fn find(&self, header: &str) -> Option<usize> {
        self.items
            .iter()
            .position(|i| matches!(i, Item::Block { header: h, .. } if h == header))
    }

    /// Places a new port-channel as IOS prints the interfaces: after the loopbacks (c9200 17.15 capture 27's
    /// Loopback0 opens them) and the port-channels numbered below it, before every other one.
    fn insert_interface(&mut self, header: &str) -> Result<usize, String> {
        let n = channel_number(header).ok_or(format!(
            "{header}: not in the running configuration, and only a new port-channel is placed"
        ))?;
        let at = self
            .items
            .iter()
            .position(|i| match i {
                Item::Block { header: h, .. } if h.starts_with("interface ") => {
                    !h.starts_with("interface Loopback") && channel_number(h).is_none_or(|m| m > n)
                }
                _ => false,
            })
            .ok_or("no interface to place a port-channel before")?;
        self.items.insert(at, Item::Text("!".into()));
        self.items.insert(
            at,
            Item::Block {
                header: header.into(),
                lines: vec![],
            },
        );
        Ok(at)
    }

    /// Applies change lines (configuration mode, as the change script types them).
    pub fn apply(&mut self, change: &[String]) -> Result<(), String> {
        // the blocks the following sub-commands go to; None skips them (a VTP VLAN)
        let mut ctx: Option<Vec<usize>> = Some(vec![]);
        for raw in change {
            let line = raw.trim_end();
            let t = line.trim();
            // under an interface `enable` turns a supervisor's QSFP port on; only an unindented
            // one is EXEC's
            let exec = ["end", "exit", "configure terminal"].contains(&t)
                || (t == "enable" && !line.starts_with(' '));
            if t.is_empty() || t.starts_with('!') || exec {
                if t == "exit" {
                    ctx = Some(vec![]);
                }
                continue;
            }
            if line.starts_with(' ') {
                let Some(blocks) = &ctx else { continue };
                if blocks.is_empty() {
                    return Err(format!("'{t}' has no interface to go to"));
                }
                for &b in &blocks.clone() {
                    self.sub(b, t)?;
                }
                continue;
            }
            if let Some(name) = t.strip_prefix("interface range ") {
                let mut v = Vec::new();
                for h in expand_range(name)? {
                    v.push(
                        self.find(&h)
                            .ok_or(format!("{h}: not in the running configuration"))?,
                    );
                }
                ctx = Some(v);
            } else if let Some(name) = t.strip_prefix("no interface ") {
                let h = header(name);
                let at = self
                    .find(&h)
                    .ok_or(format!("{h}: not in the running configuration"))?;
                self.items.remove(at);
                if matches!(self.items.get(at), Some(Item::Text(s)) if s == "!") {
                    self.items.remove(at);
                }
                ctx = Some(vec![]);
            } else if let Some(name) = t.strip_prefix("interface ") {
                let h = header(name);
                let at = match self.find(&h) {
                    Some(i) => i,
                    None => self.insert_interface(&h)?,
                };
                ctx = Some(vec![at]);
            } else if crate::generate::vlan_definition(t.strip_prefix("no ").unwrap_or(t)).is_some()
            {
                if !self.vtp_vlans {
                    return Err(format!(
                        "'{t}': VLANs in the running configuration are not modelled"
                    ));
                }
                self.notes.push(format!(
                    "'{t}' changes the VTP VLAN database, which the running configuration does not list."
                ));
                ctx = None;
            } else if let Some(h) = t.strip_prefix("hostname ") {
                if self.find(t).is_none() {
                    return Err(format!(
                        "hostname {h} differs from the running configuration's"
                    ));
                }
                ctx = Some(vec![]);
            } else {
                return Err(format!("'{t}': a global line this does not model"));
            }
        }
        Ok(())
    }

    fn sub(&mut self, at: usize, cmd: &str) -> Result<(), String> {
        self.sub_one(at, cmd)?;
        // a port-channel's switchport lines reach its members
        let Item::Block { header, .. } = &self.items[at] else {
            return Ok(());
        };
        let body = cmd.strip_prefix("no ").unwrap_or(cmd);
        if let Some(n) = channel_number(header)
            && body.starts_with("switchport")
        {
            let member = format!(" channel-group {n} mode ");
            let members: Vec<usize> = (0..self.items.len())
                .filter(|&i| {
                    matches!(&self.items[i], Item::Block { lines, .. }
                        if lines.iter().any(|l| l.starts_with(&member)))
                })
                .collect();
            for m in members {
                self.sub_one(m, cmd)?;
            }
        }
        Ok(())
    }

    fn sub_one(&mut self, at: usize, cmd: &str) -> Result<(), String> {
        let Item::Block { header, lines } = &mut self.items[at] else {
            return Err("not a block".into());
        };
        let negated = cmd.starts_with("no ");
        let body = cmd.strip_prefix("no ").unwrap_or(cmd);
        let k = key_of(body).ok_or(format!("{header}: '{cmd}' is not a command this places"))?;
        for l in lines.iter() {
            if key_of(l).is_none() {
                return Err(format!(
                    "{header}: cannot place '{}' in IOS order",
                    l.trim()
                ));
            }
        }
        let key = KEYS[k];
        let text = if negated {
            None
        } else if key == "switchport trunk allowed vlan" {
            let current: Option<String> = {
                let mut parts = lines
                    .iter()
                    .filter_map(|l| l.trim().strip_prefix("switchport trunk allowed vlan "))
                    .map(|r| r.strip_prefix("add ").unwrap_or(r).to_string());
                parts.next().map(|first| {
                    std::iter::once(first)
                        .chain(parts)
                        .collect::<Vec<_>>()
                        .join(",")
                })
            };
            allowed_line(&allowed_after(current.as_deref(), &body[key.len()..]))
        } else if body == "spanning-tree portfast" && self.portfast_edge {
            Some("spanning-tree portfast edge".to_string())
        } else {
            Some(body.to_string())
        };
        // a line the block has keeps its place, so the capture's own order stands; a new one goes
        // above the first line it prints above
        let was = lines.iter().position(|l| key_of(l) == Some(k));
        lines.retain(|l| key_of(l) != Some(k));
        if let Some(t) = text {
            let to = was.unwrap_or_else(|| {
                lines
                    .iter()
                    .position(|l| key_of(l).is_some_and(|m| above(&self.order, k, m)))
                    .unwrap_or(lines.len())
            });
            lines.insert(to, format!(" {t}"));
        }
        Ok(())
    }

    pub fn render(&self) -> Vec<String> {
        let mut out = Vec::new();
        for i in &self.items {
            match i {
                Item::Text(t) => out.push(t.clone()),
                Item::Block { header, lines } => {
                    out.push(header.clone());
                    out.extend(lines.iter().cloned());
                }
            }
        }
        out
    }
}

/// Whether a rollback applied after the change leads back to the captured configuration, line for
/// line; the first line that differs otherwise.
pub fn restores(running: &str, after: &Target, rollback: &[String]) -> Result<(), String> {
    let mut back = after.clone();
    back.apply(rollback)?;
    let want: Vec<&str> = running.lines().map(str::trim_end).collect();
    let got = back.render();
    match (0..want.len().max(got.len()))
        .find(|&i| want.get(i).copied() != got.get(i).map(String::as_str))
    {
        None => Ok(()),
        Some(i) => Err(format!(
            "line {} is {:?}, the capture has {:?}",
            i + 1,
            got.get(i),
            want.get(i)
        )),
    }
}

/// The header of the interface a line names, as the capture prints it: `Te2/0/3`, which the switch
/// takes, is `interface TenGigabitEthernet2/0/3`.
fn header(name: &str) -> String {
    format!("interface {}", crate::generate::full(name))
}

/// `TenGigabitEthernet2/0/3 - 4` -> the two interface headers.
fn expand_range(spec: &str) -> Result<Vec<String>, String> {
    let (first, last) = spec.split_once(" - ").ok_or(format!(
        "interface range {spec}: one 'a - b' range is modelled"
    ))?;
    let (stem, a) = first
        .rsplit_once('/')
        .ok_or(format!("interface range {spec}"))?;
    let (a, b): (u32, u32) = (
        a.parse().map_err(|_| format!("interface range {spec}"))?,
        last.trim()
            .parse()
            .map_err(|_| format!("interface range {spec}"))?,
    );
    Ok((a..=b).map(|p| header(&format!("{stem}/{p}"))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_lists_resolve_as_ios_prints_them() {
        let s = allowed_after(None, "except 451-453,456,459");
        assert_eq!(ios_list(&s), "1-450,454,455,457,458,460-4094");
        assert_eq!(
            ios_list(&allowed_after(None, "except 451-452,456,459")),
            "1-450,453-455,457,458,460-4094"
        );
        assert_eq!(
            ios_list(&allowed_after(None, "remove 3,3004")),
            "1,2,4-3003,3005-4094"
        );
        assert_eq!(allowed_line(&allowed_after(Some("none"), "all")), None);
        assert_eq!(
            allowed_line(&allowed_after(Some("10,20"), "add 30")).unwrap(),
            "switchport trunk allowed vlan 10,20,30"
        );
        assert_eq!(
            allowed_line(&allowed_after(None, "none")).unwrap(),
            "switchport trunk allowed vlan none"
        );
    }

    const RUN: &str = "!\nhostname core\n!\ninterface Port-channel5\n switchport mode trunk\n!\ninterface GigabitEthernet0/0\n vrf forwarding Mgmt-vrf\n!\ninterface TenGigabitEthernet2/0/1\n description up\n switchport mode trunk\n channel-group 5 mode on\n!\ninterface TenGigabitEthernet2/0/3\n switchport trunk allowed vlan none\n!\ninterface GigabitEthernet5/0/19\n switchport trunk allowed vlan none\n shutdown\n!\nend\n";

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_change_lands_in_ios_order_and_its_rollback_restores_the_capture() {
        let change = lines(&[
            "hostname core",
            "vlan 499",
            "interface Port-channel40",
            " description stack",
            " switchport trunk allowed vlan except 451-453",
            " switchport mode trunk",
            "interface TenGigabitEthernet2/0/3",
            " switchport mode trunk",
            " switchport trunk allowed vlan except 451-453",
            " channel-group 40 mode active",
            " no shutdown",
            "interface GigabitEthernet5/0/19",
            " spanning-tree portfast",
            " switchport mode access",
            " switchport access vlan 450",
            " no shutdown",
            "end",
        ]);
        let mut t = Target::new(RUN);
        t.apply(&change).unwrap();
        let out = t.render().join("\n");
        assert!(out.contains("interface Port-channel5\n switchport mode trunk\n!\ninterface Port-channel40\n description stack\n switchport trunk allowed vlan 1-450,454-4094\n switchport mode trunk\n!\ninterface GigabitEthernet0/0"), "{out}");
        assert!(out.contains("interface TenGigabitEthernet2/0/3\n switchport trunk allowed vlan 1-450,454-4094\n switchport mode trunk\n channel-group 40 mode active\n!"), "{out}");
        assert!(out.contains("interface GigabitEthernet5/0/19\n switchport access vlan 450\n switchport trunk allowed vlan none\n switchport mode access\n spanning-tree portfast\n!"), "{out}");
        assert_eq!(t.notes.len(), 1, "VLAN 499 goes to the VTP database");
        let rollback = lines(&[
            "interface TenGigabitEthernet2/0/3",
            " no switchport mode",
            " no channel-group",
            " switchport trunk allowed vlan none",
            "interface GigabitEthernet5/0/19",
            " no switchport access vlan",
            " no switchport mode",
            " no spanning-tree portfast",
            " shutdown",
            "exit",
            "no interface Port-channel40",
            "no vlan 499",
        ]);
        t.apply(&rollback).unwrap();
        assert_eq!(t.render().join("\n") + "\n", RUN);
    }

    #[test]
    fn a_port_channel_s_allowed_list_reaches_its_members() {
        let mut t = Target::new(RUN);
        t.apply(&lines(&[
            "interface Port-channel5",
            " switchport trunk allowed vlan except 451-452",
        ]))
        .unwrap();
        let out = t.render().join("\n");
        assert_eq!(
            out.matches("switchport trunk allowed vlan 1-450,453-4094")
                .count(),
            2,
            "{out}"
        );
        t.apply(&lines(&[
            "interface Port-channel5",
            " switchport trunk allowed vlan all",
        ]))
        .unwrap();
        assert_eq!(t.render().join("\n") + "\n", RUN);
    }

    #[test]
    fn a_rollback_that_leaves_a_line_behind_is_named() {
        let mut t = Target::new(RUN);
        let change = lines(&[
            "interface TenGigabitEthernet2/0/3",
            " channel-group 40 mode active",
            " description x",
        ]);
        t.apply(&change).unwrap();
        let full = lines(&[
            "interface TenGigabitEthernet2/0/3",
            " no channel-group",
            " no description",
        ]);
        assert!(restores(RUN, &t, &full).is_ok());
        let short = lines(&["interface TenGigabitEthernet2/0/3", " no description"]);
        let e = restores(RUN, &t, &short).unwrap_err();
        assert!(e.contains("channel-group 40 mode active"), "{e}");
    }

    #[test]
    fn a_supervisor_qsfp_port_keeps_its_enable_and_a_rollback_removes_it() {
        let run = "!\ninterface FortyGigabitEthernet3/0/9\n!\nend\n";
        let mut t = Target::new(run);
        t.apply(&lines(&[
            "interface FortyGigabitEthernet3/0/9",
            " enable",
            " description bmc",
            " switchport access vlan 450",
            " switchport mode access",
            " no shutdown",
            "end",
        ]))
        .unwrap();
        let out = t.render().join("\n");
        assert!(out.contains("interface FortyGigabitEthernet3/0/9\n enable\n description bmc\n switchport access vlan 450\n switchport mode access\n!"), "{out}");
        let rollback = |enable: &[&str]| {
            let mut v = vec![
                "interface FortyGigabitEthernet3/0/9",
                " no description",
                " no switchport access vlan",
                " no switchport mode",
            ];
            v.extend(enable);
            lines(&v)
        };
        assert!(restores(run, &t, &rollback(&[" no enable"])).is_ok());
        let e = restores(run, &t, &rollback(&[])).unwrap_err();
        assert!(e.contains("\" enable\""), "{e}");
        // an unindented `enable` is EXEC's
        assert!(
            Target::new(run)
                .apply(&lines(&["enable", "configure terminal"]))
                .is_ok()
        );
    }

    #[test]
    fn a_vtp_server_capture_takes_a_vlan_into_its_database() {
        // classic IOS prints `vlan internal allocation policy ascending` in every VTP mode
        let run = "!\nvlan internal allocation policy ascending\n!\ninterface GigabitEthernet0/5\n!\nend\n";
        let mut t = Target::new(run);
        t.apply(&lines(&[
            "vlan 20",
            " name x",
            "interface GigabitEthernet0/5",
            " switchport access vlan 20",
        ]))
        .unwrap();
        assert_eq!(
            t.notes,
            [
                "'vlan 20' changes the VTP VLAN database, which the running configuration does not list."
            ]
        );
        // VLANs the capture lists, a classic switch's held range among them, are not modelled
        for listed in ["!\nvlan 10\n name a\n!\nend\n", "!\nvlan 65-66\n!\nend\n"] {
            assert!(Target::new(listed).apply(&lines(&["vlan 20"])).is_err());
        }
    }

    #[test]
    fn a_release_that_prints_portfast_edge_keeps_it() {
        // 15.2(6)E and 15.2(7)E keep a typed `spanning-tree portfast` as `... edge` (c2960x 15.2(7)E capture 1)
        let run = "!\ninterface GigabitEthernet1/0/1\n description host\n switchport access vlan 535\n speed 1000\n spanning-tree portfast edge\n!\ninterface GigabitEthernet1/0/2\n!\nend\n";
        let mut t = Target::new(run);
        t.apply(&lines(&[
            "interface GigabitEthernet1/0/1",
            " switchport mode access",
            " spanning-tree portfast",
            " spanning-tree bpduguard enable",
            "interface GigabitEthernet1/0/2",
            " spanning-tree portfast",
        ]))
        .unwrap();
        let out = t.render().join("\n");
        assert!(out.contains("interface GigabitEthernet1/0/1\n description host\n switchport access vlan 535\n switchport mode access\n speed 1000\n spanning-tree portfast edge\n spanning-tree bpduguard enable\n!"), "{out}");
        assert!(
            out.contains("interface GigabitEthernet1/0/2\n spanning-tree portfast edge\n!"),
            "{out}"
        );
        let rollback = lines(&[
            "interface GigabitEthernet1/0/1",
            " no switchport mode",
            " no spanning-tree bpduguard",
            "interface GigabitEthernet1/0/2",
            " no spanning-tree portfast",
        ]);
        assert!(restores(run, &t, &rollback).is_ok());
    }

    #[test]
    fn a_new_port_channel_follows_the_loopbacks() {
        let run =
            "!\ninterface Loopback0\n description mgmt\n!\ninterface GigabitEthernet0/0\n!\nend\n";
        let mut t = Target::new(run);
        t.apply(&lines(&["interface Port-channel1", " description up"]))
            .unwrap();
        assert_eq!(
            t.render().join("\n") + "\n",
            "!\ninterface Loopback0\n description mgmt\n!\ninterface Port-channel1\n description up\n!\ninterface GigabitEthernet0/0\n!\nend\n"
        );
    }

    #[test]
    fn lines_keep_the_capture_s_order_and_new_ones_take_its_release_s() {
        // c2960x 15.2(7)E capture 8 at 15.2(7)E prints the allowed list above the native VLAN
        let run = "!\ninterface Port-channel1\n switchport trunk allowed vlan 14,410,450\n switchport trunk native vlan 14\n switchport mode trunk\n!\ninterface GigabitEthernet1/0/1\n switchport trunk native vlan 14\n!\nend\n";
        let mut t = Target::new(run);
        t.apply(&lines(&[
            "interface Port-channel1",
            " description uplink",
            "interface GigabitEthernet1/0/1",
            " switchport mode trunk",
            " switchport trunk allowed vlan 14",
        ]))
        .unwrap();
        let out = t.render().join("\n");
        assert!(out.contains("interface Port-channel1\n description uplink\n switchport trunk allowed vlan 14,410,450\n switchport trunk native vlan 14\n switchport mode trunk\n!"), "{out}");
        assert!(out.contains("interface GigabitEthernet1/0/1\n switchport trunk allowed vlan 14\n switchport trunk native vlan 14\n switchport mode trunk\n!"), "{out}");
        let rollback = lines(&[
            "interface Port-channel1",
            " no description",
            "interface GigabitEthernet1/0/1",
            " no switchport trunk allowed vlan",
            " no switchport mode",
        ]);
        assert!(restores(run, &t, &rollback).is_ok());
    }

    #[test]
    fn an_interface_named_as_the_switch_takes_it_is_the_capture_s() {
        // the switch takes `Te2/0/3` and `Po40`; the capture prints the full names
        let run =
            "!\ninterface TenGigabitEthernet2/0/3\n!\ninterface TenGigabitEthernet2/0/4\n!\nend\n";
        let mut t = Target::new(run);
        t.apply(&lines(&[
            "interface Po40",
            " description stack",
            "interface range TenGigabitEthernet2/0/3 - 4",
            " channel-group 40 mode active",
        ]))
        .unwrap();
        assert!(
            t.render()
                .join("\n")
                .contains("interface Port-channel40\n description stack\n!\ninterface TenGigabitEthernet2/0/3\n channel-group 40 mode active\n!")
        );
        let rollback = lines(&[
            "interface range Te2/0/3 - 4",
            " no channel-group",
            "exit",
            "no interface Po40",
        ]);
        assert!(restores(run, &t, &rollback).is_ok());
    }

    #[test]
    fn what_this_does_not_model_is_an_error() {
        let mut t = Target::new(RUN);
        assert!(t.apply(&lines(&["ntp server 10.0.0.1"])).is_err());
        assert!(
            t.apply(&lines(&[
                "interface TenGigabitEthernet2/0/3",
                " ip address 10.0.0.1 255.0.0.0"
            ]))
            .is_err()
        );
        assert!(
            t.apply(&lines(&["interface TenGigabitEthernet9/0/1"]))
                .is_err()
        );
        assert!(
            t.apply(&lines(&["interface GigabitEthernet0/0", " description x"]))
                .is_err()
        );
    }
}
