//! The running configuration: global lines and the blocks a submode opens (interface, vlan,
//! line, vrf definition), changed by accepted lines as IOS changes them. A later line replaces an
//! earlier one of the same command; commands that take several instances (ntp server, username,
//! ip route, ...) keep one line per instance; `no` removes.

use crate::cli::{Item, Parse};
use crate::platform::Stack;

/// A configuration line and its replacement key.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub header: String,
    pub lines: Vec<Entry>,
}

#[derive(Clone, Debug)]
pub struct Device {
    pub stack: Stack,
    pub hostname: String,
    /// the IOS XE release, as the grammar names it (`17.15`, `26.x`); empty before a session sets it
    pub release: String,
    pub global: Vec<Entry>,
    pub blocks: Vec<Block>,
    /// top-level lines and blocks taken as a real switch held them (`live`), which print where
    /// that switch printed them: after the top-level line before them in its capture
    pub held_top: std::collections::BTreeSet<String>,
    pub held_after: crate::live::After,
    /// cabled ports with a link, by full name (`lab.rs`)
    pub links: std::collections::BTreeMap<String, Link>,
    /// interface headers whose address was typed at the console, which `show ip interface brief`
    /// reads as Method `manual` (the lab C9300, 17.6.4)
    pub manual: std::collections::BTreeSet<String>,
    /// the base MAC address its bridge IDs and VTP Device ID carry: a lab's or the switch list's, else
    /// one derived from the hardware
    pub mac: crate::stp::Mac,
    /// ports BPDU guard err-disabled, as `show interfaces status` prints them (`spanning.rs`), with
    /// the transceiver their link had
    pub errdisabled: std::collections::BTreeMap<String, Option<&'static crate::platform::Optic>>,
    /// what vlan.dat keeps beside the VTP lines: the configuration revision and the last change
    pub vtp: Vtp,
    /// booted with no startup configuration: the factory default's `no ip address` reads Method
    /// `unset` (the lab C9300, 17.6.4)
    pub factory: bool,
    /// ports whose forced speed `no speed` removed, by full name, with that speed:
    /// the lab C9300, 17.6.4 kept costing them at it (`spanning.rs`)
    pub held_speed: std::collections::BTreeMap<String, u32>,
    /// transceivers fitted in ports, by full name, and whether each is third-party (a lab's
    /// `[[switch.transceiver]]`)
    pub transceivers: std::collections::BTreeMap<String, (&'static crate::platform::Optic, bool)>,
    /// how the configuration replayed last laid itself out, where it was a switch's own (`Layout`)
    pub layout: Option<Layout>,
    /// when the configuration last changed, as seconds since 1970 UTC: a replayed configuration's
    /// own line, a factory boot, else the last line typed at the console
    pub last_change: Option<u64>,
    /// the user the replayed line named (`... 2026 by admin`); none once a line is typed here
    pub last_change_by: Option<String>,
}

/// A switch's own printed configuration as replayed: its top-level lines in order, the `!`, comment
/// and blank lines between each two as it printed them, the ` !` lines inside a block before a
/// line, and each line as it printed it (spaces and case). The running configuration prints them
/// where the same lines meet again. Lines are matched by `layout_key`.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    /// the separator lines between two top-level lines that followed each other, by their keys
    /// (`version 17.6` first; `end` last)
    pub pairs: std::collections::BTreeMap<(String, String), Vec<String>>,
    /// the top-level lines it held
    pub tops: std::collections::BTreeSet<String>,
    /// each top-level line's place
    pub order: std::collections::BTreeMap<String, usize>,
    /// the separator lines inside a block before one of its lines, by the block's header and line
    pub inner: std::collections::BTreeMap<(String, String), Vec<String>>,
    /// each line as printed, by `layout_key`: every form, where lines differing only in case or
    /// spacing were printed (` description Uplink` and ` description UPLINK` on two ports)
    pub raw: std::collections::BTreeMap<String, Vec<String>>,
    /// every line it held at any depth, by `layout_key` of the line unindented: a `no` line it
    /// printed is kept so (` no protocol udp`, `  no neighbor 2001:DB8:10::2 activate`)
    pub lines: std::collections::BTreeSet<String>,
}

/// The key a replayed line is matched by: its indent, then its words single-spaced and lowercase
/// (the lab C9300-24T, 17.6.4, printed `match cos  5 ` and `mdns-sd service-list TESTSLI IN`).
pub fn layout_key(l: &str) -> String {
    let body = l.trim_start();
    let indent = &l[..l.len() - body.len()];
    format!(
        "{indent}{}",
        body.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    )
}

impl Layout {
    /// The layout of a configuration as a switch printed it: its `version` line opens it, its
    /// `end` closes it. None for a configuration with no `version` line, written by hand.
    pub fn of(text: &str) -> Option<Layout> {
        let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
        let version = lines
            .find(|l| l.starts_with("version "))?
            .trim_end()
            .to_string();
        let mut out = Layout::default();
        let mut prev = layout_key(&version);
        let mut header = String::new();
        let mut seps: Vec<String> = vec![];
        let mut inner: Vec<String> = vec![];
        let mut banner: Option<String> = None;
        for raw in lines {
            let l = raw.trim_end();
            let t = l.trim_start();
            if let Some(d) = &banner {
                if l.contains(d.as_str()) {
                    banner = None;
                }
                continue;
            }
            if t.starts_with('!') {
                if l.starts_with([' ', '\t']) {
                    inner.push(raw.to_string());
                } else {
                    seps.append(&mut inner);
                    seps.push(raw.to_string());
                }
                continue;
            }
            // an empty line between top-level lines prints where it stood (the lab C9300-24T,
            // 17.6.4, after `udld enable` and `errdisable recovery interval 60`)
            if t.is_empty() {
                if raw.is_empty() {
                    seps.append(&mut inner);
                    seps.push(String::new());
                }
                continue;
            }
            let k = layout_key(l);
            let forms = out.raw.entry(k.clone()).or_default();
            if !forms.iter().any(|f| f == raw) {
                forms.push(raw.to_string());
            }
            out.lines.insert(layout_key(t));
            if l.starts_with([' ', '\t']) {
                if !inner.is_empty() {
                    out.inner
                        .insert((header.clone(), k), std::mem::take(&mut inner));
                }
                continue;
            }
            seps.append(&mut inner);
            out.pairs
                .insert((prev.clone(), k.clone()), std::mem::take(&mut seps));
            out.tops.insert(k.clone());
            let n = out.order.len();
            out.order.entry(k.clone()).or_insert(n);
            if l == "end" {
                break;
            }
            banner = crate::live::banner_opens(l);
            prev = k.clone();
            header = k;
        }
        Some(out)
    }
}

/// The VLAN database's history, as `show vtp status` prints it: the configuration revision, and the
/// updater and time of the last change (`0.0.0.0` and `0-0-00 00:00:00` on a new database).
#[derive(Clone, Debug, PartialEq)]
pub struct Vtp {
    pub revision: u32,
    pub modified_by: String,
    pub modified_at: String,
}

impl Default for Vtp {
    fn default() -> Self {
        Vtp {
            revision: 0,
            modified_by: "0.0.0.0".into(),
            modified_at: "0-0-00 00:00:00".into(),
        }
    }
}

/// A port's link: the far end, the transceiver, and how the port runs once both ends have
/// negotiated.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    /// the far end's hostname and port, or the name of a device the lab does not model
    pub peer: String,
    pub optic: Option<&'static crate::platform::Optic>,
    /// the speed a copper link negotiates (Mb/s); the port's own when `None`
    pub speed: Option<u32>,
    /// negotiated half duplex
    pub half: bool,
    pub trunk: bool,
    /// bundled in its port-channel
    pub bundled: bool,
}

/// Commands whose `no` form IOS keeps in the running configuration, as the cores' running
/// configurations print it (9400 17.5, 2026-10-05): `no aaa new-model`, `no ip http server`.
const KEEPS_NO: &[&str] = &[
    "aaa new-model",
    "logging console",
    "ip forward-protocol nd",
    "ip http server",
    "ip http secure-server",
    // classic IOS (c2960x 15.2(7)E capture 8, 15.2(7)E0a); IOS XE spells the first `no ip domain lookup`
    "ip domain-lookup",
    "ip domain lookup",
    "ip igmp snooping",
    "cdp run",
];

/// Interface commands on by default whose `no` form IOS keeps under the interface (`no cdp enable`
/// under c9300 17.6 capture 4's ports at 9300 17.6, ` no lldp transmit`).
const KEEPS_NO_IN_BLOCK: &[&str] = &[
    "cdp enable",
    "lldp transmit",
    "lldp receive",
    "keepalive",
    "ip redirects",
    "ip proxy-arp",
    "ip unreachables",
];

/// Commands on by default whose `no` form IOS keeps: typed bare, they are the default again, which
/// prints nothing (no capture holds `cdp run` or `ip domain lookup`, 67 hold `no cdp run`).
fn default_on(k: &str, at_top: bool) -> bool {
    if at_top {
        ["cdp run", "ip domain-lookup", "ip domain lookup", "ip igmp snooping"].contains(&k)
            // `spanning-tree vlan 5`, not `spanning-tree vlan 5 priority 4096`
            || k.starts_with("spanning-tree vlan #")
            || k.starts_with("errdisable detect cause ")
    } else {
        KEEPS_NO_IN_BLOCK.contains(&k)
    }
}

/// Interfaces that route: `no ip address` is kept on them (the cores print it under
/// GigabitEthernet0/0 and Vlan1), not on a switch port.
fn routed(header: &str, lines: &[Entry]) -> bool {
    let name = header.trim_start_matches("interface ");
    ["Vlan", "Loopback", "Tunnel", "GigabitEthernet0/0"]
        .iter()
        .any(|p| name.starts_with(p))
        // a 2960-X's management port (c2960x 15.2(7)E capture 8 prints `no ip address` under it)
        || name == "FastEthernet0"
        || lines.iter().any(|e| e.text == "no switchport")
}

/// Which arguments of a command kept once per instance name the instance; a VRF's name always does.
#[derive(Clone, Copy)]
enum Names {
    /// the first n (`username admin`, whatever its privilege and secret)
    First(usize),
    /// every one
    All,
    /// every one but a privilege level (`privilege exec level 15 show` names `exec` and `show`; a
    /// line's `privilege level 15` names nothing)
    Level,
    /// a route's prefix, mask and next hop: the leading ones, a distance left out
    Route,
    /// the group number after the command's first word, 0 where the line names none
    /// (`standby ip 10.0.0.1` sets group 0)
    Group,
}

/// Commands that keep one line per instance, and the arguments that name one: a line naming the
/// same instance replaces it, whatever it sets.
const MULTI: &[(&str, Names)] = &[
    ("ntp server", Names::First(1)),
    // c2960x 15.2(7)E capture 9 peers with 192.0.2.121 and 192.0.2.122
    ("ntp peer", Names::First(1)),
    // c2960 15.2(6)E capture 1's MST configuration maps instances 1 and 2
    ("instance", Names::All),
    // one line per key, server, address, route, name or range, as the command references describe them
    ("ntp authentication-key", Names::First(1)),
    ("ntp trusted-key", Names::All),
    ("ip helper-address", Names::First(1)),
    ("ip address secondary", Names::First(1)),
    ("ip host", Names::First(1)),
    ("ip prefix-list", Names::All),
    ("ip dhcp excluded-address", Names::All),
    ("ip domain list", Names::All),
    ("ip domain-list", Names::All),
    ("ipv6 route", Names::Route),
    ("tacacs-server host", Names::First(1)),
    ("radius-server host", Names::First(1)),
    // a MAC address in a VLAN
    ("mac address-table static", Names::First(2)),
    ("snmp-server view", Names::First(2)),
    ("boot system", Names::All),
    ("alias", Names::All),
    ("username", Names::First(1)),
    ("ip route", Names::Route),
    ("ip name-server", Names::All),
    ("logging host", Names::First(1)),
    ("snmp-server host", Names::All),
    ("snmp-server community", Names::First(1)),
    ("snmp-server user", Names::All),
    ("snmp-server group", Names::All),
    ("spanning-tree vlan", Names::First(1)),
    ("switch", Names::First(1)),
    ("ip access-list", Names::All),
    ("access-list", Names::All),
    // c2960x 15.2(7)E capture 8 prints 21 `snmp-server enable traps` lines and two port-security MACs
    ("snmp-server enable traps", Names::All),
    ("switchport port-security mac-address", Names::All),
    // c9200 17.15 capture 27 prints 13 `privilege exec level 15 <command>` lines
    ("privilege", Names::Level),
    // c2960x 15.2(2)E capture 1 holds two certificate chains, TP-self-signed and HTTPS_SS_CERT_KEYPAIR, each with
    // its trustpoint
    ("crypto pki certificate chain", Names::All),
    ("crypto pki trustpoint", Names::All),
    // a routing process's neighbors and networks (a BGP network by its prefix and mask, an OSPF one
    // by its address and wildcard)
    ("neighbor", Names::First(1)),
    ("network", Names::First(2)),
    // a class map's match lines, each kept, and a policy class's queue limit per DSCP value (the
    // lab C9300-24T, 17.6.4: `match dscp af41  af42  af43` beside `match dscp af31  af32  af33`;
    // `queue-limit dscp af41 percent 80` beside `queue-limit dscp af31 percent 80`)
    ("match dscp", Names::All),
    ("match ip dscp", Names::All),
    ("match precedence", Names::All),
    ("match ip precedence", Names::All),
    ("queue-limit dscp", Names::First(1)),
    // a MAC list's entries, and an IPv6 list's by number
    ("permit", Names::All),
    ("deny", Names::All),
    ("sequence", Names::First(1)),
    // an interface's HSRP groups, each with its own address and settings (the lab C9300-24T,
    // 17.6.4: `standby 1 ip 10.0.0.3` beside `standby 2 ip 10.0.0.4`)
    ("standby", Names::Group),
];

/// Instances whose keywords after the instance name a setting of it, each kept on its own line:
/// `spanning-tree vlan 10 priority 4096` beside `spanning-tree vlan 10 forward-time 10`,
/// `neighbor 10.0.0.1 remote-as 65001` beside `neighbor 10.0.0.1 activate`.
const SETTINGS: &[&str] = &["spanning-tree vlan", "switch", "neighbor", "standby"];

/// The levels a logging destination takes by name (`logging console informational`).
const LEVELS: &[&str] = &[
    "alerts",
    "critical",
    "debugging",
    "emergencies",
    "errors",
    "informational",
    "notifications",
    "warnings",
];

/// What a line lets in or out by (`transport input ssh`).
const TRANSPORTS: &[&str] = &["all", "none", "ssh", "telnet", "rlogin"];

/// How `service timestamps` stamps a message.
const STAMPS: &[&str] = &[
    "datetime",
    "localtime",
    "msec",
    "show-timezone",
    "uptime",
    "year",
];

/// Commands IOS keeps one line of whatever value it is given: these keywords after the command
/// choose the value (`switchport mode trunk` replaces `switchport mode access`).
const CHOICES: &[(&str, &[&str])] = &[
    (
        "switchport mode",
        &[
            "access",
            "trunk",
            "dynamic",
            "auto",
            "desirable",
            "dot1q-tunnel",
            "private-vlan",
            "host",
            "promiscuous",
        ],
    ),
    (
        "switchport trunk encapsulation",
        &["dot1q", "isl", "negotiate"],
    ),
    (
        "switchport port-security violation",
        &["protect", "restrict", "shutdown", "vlan"],
    ),
    (
        "switchport port-security aging type",
        &["absolute", "inactivity"],
    ),
    ("switchport voice vlan", &["dot1p", "none", "untagged"]),
    ("spanning-tree mode", &["pvst", "rapid-pvst", "mst"]),
    // an interface's; the global `spanning-tree portfast default` lines name another command
    (
        "spanning-tree portfast",
        &["disable", "edge", "network", "trunk"],
    ),
    ("spanning-tree link-type", &["point-to-point", "shared"]),
    ("spanning-tree guard", &["loop", "none", "root"]),
    ("spanning-tree bpduguard", &["enable", "disable"]),
    ("spanning-tree bpdufilter", &["enable", "disable"]),
    ("spanning-tree pathcost method", &["long", "short"]),
    ("spanning-tree vlan root", &["primary", "secondary"]),
    ("vtp mode", &["client", "server", "transparent", "off"]),
    ("vtp version", &["1", "2", "3"]),
    ("duplex", &["auto", "full", "half"]),
    (
        "speed",
        &[
            "10",
            "100",
            "1000",
            "2500",
            "5000",
            "10000",
            "25000",
            "40000",
            "100000",
            "auto",
            "nonegotiate",
        ],
    ),
    (
        "channel-group mode",
        &["active", "passive", "on", "auto", "desirable", "non-silent"],
    ),
    (
        "port-channel load-balance",
        &[
            "dst-ip",
            "dst-mac",
            "dst-mixed-ip-port",
            "dst-port",
            "src-dst-ip",
            "src-dst-mac",
            "src-dst-mixed-ip-port",
            "src-dst-port",
            "src-ip",
            "src-mac",
            "src-mixed-ip-port",
            "src-port",
        ],
    ),
    ("udld port", &["aggressive", "disable"]),
    ("udld", &["aggressive", "enable"]),
    ("lacp rate", &["fast", "normal"]),
    ("power inline", &["auto", "never", "static", "max"]),
    ("mls qos trust", &["cos", "dscp", "ip-precedence"]),
    ("ip ssh version", &["1", "2"]),
    // one boot level, whatever its add-on
    (
        "license boot level",
        &[
            "network-advantage",
            "network-essentials",
            "addon",
            "dna-advantage",
            "dna-essentials",
        ],
    ),
    ("ip http authentication", &["aaa", "enable", "local"]),
    (
        "vlan internal allocation policy",
        &["ascending", "descending"],
    ),
    ("diagnostic bootup level", &["minimal", "complete"]),
    ("power redundancy-mode", &["combined", "redundant"]),
    (
        "authentication port-control",
        &["auto", "force-authorized", "force-unauthorized"],
    ),
    (
        "authentication host-mode",
        &["multi-auth", "multi-domain", "multi-host", "single-host"],
    ),
    ("authentication order", &["dot1x", "mab", "webauth"]),
    ("authentication priority", &["dot1x", "mab", "webauth"]),
    (
        "access-session port-control",
        &["auto", "force-authorized", "force-unauthorized"],
    ),
    (
        "access-session host-mode",
        &["multi-auth", "multi-domain", "multi-host", "single-host"],
    ),
    ("dot1x pae", &["authenticator", "supplicant", "both"]),
    ("transport input", TRANSPORTS),
    ("transport output", TRANSPORTS),
    ("transport preferred", TRANSPORTS),
    // a line's login: local, or a named list
    ("login", &["local", "authentication"]),
    ("logging console", LEVELS),
    ("logging monitor", LEVELS),
    ("logging trap", LEVELS),
    ("logging buffered", LEVELS),
    ("logging origin-id", &["hostname", "ip", "ipv6", "string"]),
    ("service timestamps debug", STAMPS),
    ("service timestamps log", STAMPS),
    ("flowcontrol receive", &["desired", "off", "on"]),
    ("flowcontrol send", &["desired", "off", "on"]),
    ("ip arp inspection validate", &["dst-mac", "ip", "src-mac"]),
    ("file prompt", &["alert", "noisy", "quiet"]),
    ("state", &["active", "suspend"]),
];

/// A line's keywords, as the grammar spells them and without its `no`.
fn keywords(p: &Parse) -> Vec<String> {
    p.items
        .iter()
        .filter_map(|i| match i {
            Item::Kw { kw, .. } if !kw.eq_ignore_ascii_case("no") => Some(kw.to_lowercase()),
            _ => None,
        })
        .collect()
}

/// The command a line's keywords name, the value they choose left out (`CHOICES`).
fn command(words: &[String]) -> String {
    for (stem, values) in CHOICES {
        let n = stem.split(' ').count();
        if words.len() > n
            && words[..n].join(" ") == *stem
            && words[n..].iter().all(|w| values.contains(&w.as_str()))
        {
            return stem.to_string();
        }
    }
    words.join(" ")
}

/// The words of the arguments that name an instance (`MULTI`), lowercase: of a free-text argument,
/// its first words (some books read `snmp-server community`'s string to the end of the line).
fn instance(p: &Parse, names: Names) -> Vec<String> {
    let mut out = Vec::new();
    let mut counted = 0;
    let mut after: Option<&str> = None;
    // a route's leading arguments end at the first keyword after them
    let mut route_done = false;
    for i in &p.items {
        match i {
            Item::Kw { kw, .. } => {
                route_done = counted > 0;
                after = Some(kw.as_str());
            }
            Item::Arg { value, .. } => {
                let previous = after.take();
                for word in value.split_whitespace().map(str::to_lowercase) {
                    if previous.is_some_and(|k| k.eq_ignore_ascii_case("vrf")) {
                        out.push(word);
                        continue;
                    }
                    let names_it = match names {
                        Names::First(n) => counted < n,
                        Names::All => true,
                        Names::Level => !previous.is_some_and(|k| k.eq_ignore_ascii_case("level")),
                        Names::Route => !route_done && !word.chars().all(|c| c.is_ascii_digit()),
                        Names::Group => false,
                    };
                    counted += 1;
                    if names_it {
                        out.push(word);
                    }
                }
            }
        }
    }
    out
}

/// The group a line names (`Names::Group`): the number after its first keyword, else `0`.
fn group(p: &Parse) -> Vec<String> {
    let mut items = p
        .items
        .iter()
        .skip_while(|i| matches!(i, Item::Kw { kw, .. } if kw.eq_ignore_ascii_case("no")));
    items.next();
    match items.next() {
        Some(Item::Arg { value, .. }) if value.chars().all(|c| c.is_ascii_digit()) => {
            vec![value.clone()]
        }
        _ => vec!["0".to_string()],
    }
}

/// The replacement key of a line: the command its keywords name, without the value they choose
/// (`switchport mode trunk` replaces `switchport mode access`); for a command kept once per
/// instance, ` #` and the arguments that name the instance (`username admin`, whatever it sets;
/// `ntp server vrf Mgmt-vrf 192.0.2.121` and `... 192.0.2.122` are two). A command whose every
/// argument names its instance is named by its keywords as well (`access-list 103 permit tcp ...`
/// beside `... permit udp ...`).
pub fn key(p: &Parse) -> String {
    let words = keywords(p);
    // a line with no keyword is named by its words (an object group's ` 10.99.0.0 255.255.0.0`)
    if words.is_empty() {
        let c = p.canonical();
        return c.strip_prefix("no ").unwrap_or(&c).to_lowercase();
    }
    let joined = words.join(" ");
    let multi = MULTI
        .iter()
        .find(|(m, _)| joined == *m || joined.starts_with(&format!("{m} ")));
    let Some((m, names)) = multi else {
        return command(&words);
    };
    let named = match names {
        Names::Group => group(p),
        _ => instance(p, *names),
    };
    // a value the secret sweep took out tells no instance from another (two `snmp-server
    // community <removed>` lines): the whole line names it, and only a line typed again is the same
    if matches!(names, Names::All) || named.iter().any(|n| n == crate::sweep::REMOVED) {
        return format!("{joined} #{}", instance(p, Names::All).join(" "));
    }
    let head = if SETTINGS.contains(m) {
        command(&words)
    } else {
        m.to_string()
    };
    format!("{head} #{}", named.join(" "))
}

/// Whether a `no` line keyed `k` removes the entry keyed `e`: the same command, and the same
/// instance; every instance where the `no` names none (`no ntp server`), or names its first words
/// (`no ip prefix-list LAN`, whatever its entries); every setting of the instance where it names no
/// setting (`no neighbor 10.0.0.1`). A `no` naming less than a whole command (`whole` false:
/// `no spanning-tree bpduguard`) also removes the commands it begins.
fn negates(k: &str, e: &str, whole: bool) -> bool {
    if k == e {
        return true;
    }
    match (k.split_once(" #"), e.split_once(" #")) {
        (Some((kc, ki)), Some((ec, ei))) => {
            let longer = ec == kc || ec.starts_with(&format!("{kc} "));
            let begins = ki.is_empty() || ki == ei || ei.starts_with(&format!("{ki} "));
            longer && begins
        }
        (None, _) => !whole && e.starts_with(&format!("{k} ")),
        _ => false,
    }
}

/// The priority `spanning-tree vlan <list> root primary` (24576) or `secondary` (28672) sets; none
/// for any other line, `diameter` among them.
fn root_priority(p: &Parse) -> Option<&'static str> {
    let words: Vec<String> = keywords(p);
    let n = words.len();
    let tail = words.get(n.checked_sub(2)?..)?;
    let starts = words.first().map(String::as_str) == Some("spanning-tree")
        && words.get(1).map(String::as_str) == Some("vlan")
        && n == 4
        && matches!(p.items.last(), Some(Item::Kw { .. }));
    match (starts, tail[0].as_str(), tail[1].as_str()) {
        (true, "root", "primary") => Some("24576"),
        (true, "root", "secondary") => Some("28672"),
        _ => None,
    }
}

/// A global `spanning-tree vlan <list> root primary|secondary`: its VLAN list and whether it is
/// `primary`.
pub(crate) fn root_request(p: &Parse) -> Option<(String, bool)> {
    let primary = root_priority(p)? == "24576";
    let list = p.items.iter().find_map(|i| match i {
        Item::Arg { value, .. } => Some(value.clone()),
        Item::Kw { .. } => None,
    })?;
    Some((list, primary))
}

/// A global `spanning-tree vlan <list> priority|forward-time|hello-time|max-age [<value>]`: its VLAN
/// list, the setting and the value, none for the `no` form.
pub(crate) fn vlan_setting(p: &Parse) -> Option<(String, String, Option<String>)> {
    let words = keywords(p);
    let ["spanning-tree", "vlan", setting] =
        words.iter().map(String::as_str).collect::<Vec<_>>()[..]
    else {
        return None;
    };
    if !["priority", "forward-time", "hello-time", "max-age"].contains(&setting) {
        return None;
    }
    let mut args = p.items.iter().filter_map(|i| match i {
        Item::Arg { value, .. } => Some(value.clone()),
        Item::Kw { .. } => None,
    });
    let list = args.next()?;
    let value = args.next().filter(|_| !p.negated);
    if value.is_none() && !p.negated {
        return None;
    }
    Some((list, setting.to_string(), value))
}

/// The line `spanning-tree vlan <list> priority <priority>` a root request stores for some of its
/// VLANs.
pub(crate) fn as_priority(p: &Parse, list: &str, priority: u32) -> Parse {
    let mut q = p.clone();
    for i in q.items.iter_mut() {
        if let Item::Arg { value, .. } = i {
            *value = list.to_string();
            break;
        }
    }
    let n = q.items.len();
    let tok = match &q.items[n - 2] {
        Item::Kw { tok, .. } => *tok,
        Item::Arg { .. } => 0,
    };
    q.items.truncate(n - 2);
    q.items.push(Item::Kw {
        kw: "priority".into(),
        tok,
        exact: true,
    });
    q.items.push(Item::Arg {
        name: "priority".into(),
        value: priority.to_string(),
        tok: tok + 1,
        typed: true,
    });
    q
}

fn first_arg(p: &Parse) -> Option<String> {
    p.items.iter().find_map(|i| match i {
        Item::Arg { value, .. } => Some(value.clone()),
        _ => None,
    })
}

impl Device {
    pub fn new(stack: Stack) -> Self {
        Device {
            mac: crate::stp::Mac::derive(&stack.spec()),
            errdisabled: Default::default(),
            stack,
            hostname: "Switch".into(),
            release: String::new(),
            global: vec![],
            blocks: vec![],
            held_top: Default::default(),
            held_after: Default::default(),
            links: Default::default(),
            manual: Default::default(),
            vtp: Vtp::default(),
            factory: false,
            held_speed: Default::default(),
            transceivers: Default::default(),
            layout: None,
            last_change: None,
            last_change_by: None,
        }
    }

    /// The ports a third-party transceiver err-disables as `gbic-invalid`: IOS XE refuses it while
    /// neither `service unsupported-transceiver` nor `no errdisable detect cause gbic-invalid` is
    /// configured (the lab C9300, 17.6.4, with both and with neither; either one alone keeping
    /// the port up is inferred from what each command does).
    pub fn gbic_invalid(&self) -> Vec<String> {
        let has = |l: &str| self.global.iter().any(|e| e.text == l);
        if self.stack.classic()
            || has("service unsupported-transceiver")
            || has("no errdisable detect cause gbic-invalid")
            || has("no errdisable detect cause all")
        {
            return vec![];
        }
        self.transceivers
            .iter()
            .filter(|(_, (_, third))| *third)
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// A global line's value after `prefix` (`vtp mode `), as stored.
    pub fn global_value(&self, prefix: &str) -> Option<&str> {
        self.global.iter().find_map(|e| e.text.strip_prefix(prefix))
    }

    /// The VTP version, mode and domain the VTP lines set: version 1, server and no domain by default.
    pub fn vtp_settings(&self) -> (String, String, String) {
        (
            self.global_value("vtp version ").unwrap_or("1").to_string(),
            self.global_value("vtp mode ")
                .unwrap_or("server")
                .to_string(),
            self.global_value("vtp domain ").unwrap_or("").to_string(),
        )
    }

    /// The VLAN database's contents: each VLAN block's header and lines, in order. A change to it is a
    /// change VTP counts.
    pub fn vlan_contents(&self) -> Vec<(String, Vec<String>)> {
        let mut v: Vec<(String, Vec<String>)> = self
            .blocks
            .iter()
            .filter(|b| crate::show::vlan_ids_of(&b.header).is_some())
            .map(|b| {
                (
                    b.header.clone(),
                    b.lines.iter().map(|e| e.text.clone()).collect(),
                )
            })
            .collect();
        v.sort();
        v
    }

    /// An interface's configuration lines.
    pub fn config_of(&self, name: &str) -> Vec<&str> {
        let header = format!("interface {name}");
        self.blocks
            .iter()
            .find(|b| b.header == header)
            .map(|b| b.lines.iter().map(|e| e.text.as_str()).collect())
            .unwrap_or_default()
    }

    /// Whether the running configuration holds the VLANs and the VTP lines: in VTP transparent (or
    /// off) mode. A VTP server or client, the default, keeps them in the VLAN database (vlan.dat)
    /// instead ("In VTP Versions 1 and 2, the VTP mode must be transparent for VTP and VLAN information
    /// to be saved in the running configuration file", 9300 command reference).
    pub fn vlans_in_running_config(&self) -> bool {
        self.global
            .iter()
            .any(|e| e.text == "vtp mode transparent" || e.text == "vtp mode off")
    }

    /// The block opened by `header`, created when new.
    pub fn block(&mut self, header: &str) -> usize {
        if let Some(i) = self.blocks.iter().position(|b| b.header == header) {
            return i;
        }
        self.blocks.push(Block {
            header: header.to_string(),
            lines: vec![],
        });
        self.blocks.len() - 1
    }

    pub fn remove_block(&mut self, header: &str) {
        self.blocks.retain(|b| b.header != header);
    }

    /// Applies an accepted configuration line, globally or inside block `at`. A port-channel's
    /// switchport lines reach its members, as IOS applies them (the cores' members mirror their
    /// port-channel; `target.rs` follows the same rule). A line a real switch of this platform and
    /// release printed (`printed`) keeps its `no` form as that switch keeps it (`no service dhcp`,
    /// ` no ip igmp snooping tcn flood`). `whole`: a `no` line names a whole command, which the
    /// books take without it (`negates`).
    pub fn apply_as(&mut self, p: &Parse, at: Option<usize>, printed: bool, whole: bool) {
        // a SPAN session's interfaces print short, however typed (the lab C9300-24T, 17.6.4:
        // `monitor session 1 source interface Gi1/0/11`)
        let shortened;
        let p = if at.is_none() && key(p).starts_with("monitor session") {
            let mut q = p.clone();
            for it in &mut q.items {
                if let Item::Arg { value, .. } = it
                    && crate::cli::interface_name(value, None).is_some()
                {
                    *value = value
                        .split(" , ")
                        .map(crate::show::short)
                        .collect::<Vec<_>>()
                        .join(" , ");
                }
            }
            shortened = q;
            &shortened
        } else {
            p
        };
        self.apply_one(p, at, printed, whole);
        let Some(b) = at else { return };
        let Some(n) = self.blocks[b].header.strip_prefix("interface Port-channel") else {
            return;
        };
        if !key(p).starts_with("switchport") {
            return;
        }
        let member = format!("channel-group {n} mode ");
        let members: Vec<usize> = (0..self.blocks.len())
            .filter(|&i| {
                self.blocks[i]
                    .lines
                    .iter()
                    .any(|e| e.text.starts_with(&member))
            })
            .collect();
        for m in members {
            self.apply_one(p, Some(m), false, whole);
        }
    }

    /// `switch <n> provision <model>` for a member the stack lacks provisions it absent: its
    /// interfaces take configuration and print, and none runs (the lab C9300-24T, 17.6.4, member 2
    /// provisioned `c9300-48p`). `no` removes an absent member and its interfaces (inferred).
    fn provision(&mut self, p: &Parse) {
        let text = p.canonical();
        let words: Vec<&str> = text.split_whitespace().collect();
        let (negated, rest) = match words[..] {
            ["no", ref rest @ ..] => (true, rest),
            ref rest => (false, rest),
        };
        let ["switch", n, "provision", ref ty @ ..] = rest[..] else {
            return;
        };
        let Ok(n) = n.parse::<u32>() else { return };
        let members = &mut self.stack.members;
        if negated {
            if !members.iter().any(|m| m.number == n && m.absent) {
                return;
            }
            members.retain(|m| m.number != n);
            self.blocks.retain(|b| {
                let Some(name) = b.header.strip_prefix("interface ") else {
                    return true;
                };
                !crate::platform::locate(name).is_some_and(|(_, m, _, _)| m == n)
            });
            return;
        }
        let Some(model) = ty.first().and_then(|t| crate::platform::model(t)) else {
            return;
        };
        if members.iter().any(|m| m.number == n) {
            return;
        }
        let at = members.partition_point(|m| m.number < n);
        members.insert(
            at,
            crate::platform::Member {
                number: n,
                model: model.name.to_string(),
                module: None,
                absent: true,
            },
        );
    }

    fn apply_one(&mut self, p: &Parse, at: Option<usize>, printed: bool, whole: bool) {
        let k = key(p);
        if at.is_none() && k == "hostname" {
            if p.negated {
                self.hostname = "Switch".into();
            } else if let Some(h) = first_arg(p) {
                self.hostname = h;
            }
        }
        if at.is_none() {
            self.provision(p);
        }
        // IOS XE disables a stand-alone LACP member by default and prints `no port-channel
        // standalone-disable` under a port-channel (the lab C9300, 17.6.4); classic IOS does not
        let standalone_off = k == "port-channel standalone-disable" && !self.stack.classic();
        // a device-tracking policy gleans from every protocol by default and keeps `no protocol
        // <p>` (the lab C9300-24T, 17.6.4: ` no protocol udp`)
        let gleaned = k.starts_with("protocol ")
            && at.is_some_and(|b| self.blocks[b].header.starts_with("device-tracking policy "));
        let keeps_no = printed
            || gleaned
            || match at {
                // `no spanning-tree vlan 1` (c2960x 15.2(7)E capture 8) is kept per VLAN
                // an err-disable cause is detected by default: `no errdisable detect cause
                // gbic-invalid` is kept (the lab C9300, 17.6.4)
                None => {
                    KEEPS_NO.contains(&k.as_str())
                        || k.starts_with("spanning-tree vlan #")
                        || k.starts_with("errdisable detect cause ")
                }
                // `no vtp` on a port (c9400 17.5 capture 2 prints it); `no switchport` on a routed port (the 9300
                // configuration guide's `show running-config interface GigabitEthernet1/0/9`)
                Some(b) => {
                    k == "vtp"
                        || k == "switchport"
                        || KEEPS_NO_IN_BLOCK.contains(&k.as_str())
                        || standalone_off
                        || (k == "ip address" || k == "ip route-cache")
                            && routed(&self.blocks[b].header, &self.blocks[b].lines)
                }
            };
        let vlan = at.is_some_and(|b| self.blocks[b].header.starts_with("vlan "));
        let lines = match at {
            Some(b) => &mut self.blocks[b].lines,
            None => &mut self.global,
        };
        // a VLAN is active unless suspended, and active prints nothing (inferred: the lab
        // C9300-24T, 17.6.4, printed ` state suspend` and no ` state active`)
        if vlan && k == "state" && !p.negated && p.canonical() == "state active" {
            lines.retain(|e| e.key != k);
            return;
        }
        // an interface has one primary address: `ip address dhcp`, an address or `no ip address`
        // replaces the other (the lab C9300, 17.6.4: Vlan1's `no ip address`, then `ip address dhcp`)
        if at.is_some() && (k == "ip address" || k == "ip address dhcp") {
            lines.retain(|e| e.key == k || (e.key != "ip address" && e.key != "ip address dhcp"));
        }
        if at.is_some() && k == "switchport" {
            if p.negated {
                // a routed port: its switch port lines go
                lines.retain(|e| !e.key.starts_with("switchport "));
            } else {
                // a switch port again, the default, which prints nothing: its address goes
                lines.retain(|e| {
                    e.key != k
                        && e.key != "ip route-cache"
                        && e.key != "ip address"
                        && !e.key.starts_with("ip address ")
                });
                return;
            }
        }
        // a default turned back on prints nothing, unless a real switch printed it so; an argument
        // sets something else (`keepalive 10`), but an instance's names the default's instance
        let bare = k.contains(" #") || !p.items.iter().any(|i| matches!(i, Item::Arg { .. }));
        if !p.negated
            && !printed
            && bare
            && (default_on(&k, at.is_none()) || standalone_off || gleaned)
        {
            lines.retain(|e| e.key != k);
            return;
        }
        // IOS stores `root primary` and `root secondary` as the priority they set (the lab C9300,
        // 17.6.4: `spanning-tree vlan 1 priority 24576` alone); 24576 and 28672 whatever the other
        // bridges' priorities, which the command reference lowers below a better root's (inferred)
        if at.is_none()
            && !p.negated
            && let Some(priority) = root_priority(p)
        {
            let mut q = p.clone();
            let n = q.items.len();
            let tok = match &q.items[n - 2] {
                Item::Kw { tok, .. } => *tok,
                _ => 0,
            };
            q.items.truncate(n - 2);
            q.items.push(Item::Kw {
                kw: "priority".into(),
                tok,
                exact: true,
            });
            q.items.push(Item::Arg {
                name: "priority".into(),
                value: priority.into(),
                tok: tok + 1,
                typed: true,
            });
            let k = key(&q);
            apply_to(lines, &q, k, keeps_no, whole);
            return;
        }
        let text = p.canonical();
        // IOS stores an allowed-VLAN list resolved: `except`, `add` and `remove` change the list
        // there is, and every VLAN allowed prints no line
        if let Some(spec) = text
            .strip_prefix("switchport trunk allowed vlan ")
            .filter(|_| !p.negated)
        {
            let k = "switchport trunk allowed vlan".to_string();
            let at = lines.iter().position(|e| e.key == k);
            let current = at.map(|i| lines[i].text[k.len() + 1..].to_string());
            let set = crate::target::allowed_after(current.as_deref(), spec);
            match (at, crate::target::allowed_line(&set)) {
                (Some(i), Some(t)) => lines[i].text = t,
                (Some(i), None) => {
                    lines.remove(i);
                }
                (None, Some(t)) => put(lines, Entry { key: k, text: t }),
                (None, None) => {}
            }
            return;
        }
        apply_to(lines, p, k, keeps_no, whole);
    }
}

/// What the key of the line that closes a section (`exit-address-family`) adds to its header's.
const CLOSES: &str = " #exit";

/// The key of the line that closes the section whose header is keyed `section`.
pub fn closing(section: &str) -> String {
    format!("{section}{CLOSES}")
}

/// Adds an entry to a block's lines ahead of its address and service families: a router's own lines
/// print before them, whenever typed. Its other sections keep their place (the lab C9300-24T,
/// 17.6.4, printed `bmp server 1` and the peer templates before `bgp router-id`, as typed).
fn put(lines: &mut Vec<Entry>, e: Entry) {
    let first = lines
        .iter()
        .filter_map(|x| x.key.strip_suffix(CLOSES))
        .filter(|s| {
            let h = s.trim_start();
            h.starts_with("address-family ") || h.starts_with("service-family ")
        })
        .filter_map(|s| lines.iter().position(|h| h.key == s))
        .min();
    match first {
        Some(i) => lines.insert(i, e),
        None => lines.push(e),
    }
}

/// Applies a line keyed `k` to a block's or a section's lines: it replaces the entry of the same key,
/// or is added; a `no` removes what it negates (`negates`), or where IOS keeps the `no` form
/// (`keeps_no`) stands in that entry's place.
pub fn apply_to(lines: &mut Vec<Entry>, p: &Parse, k: String, keeps_no: bool, whole: bool) {
    if p.negated && !keeps_no {
        lines.retain(|e| !negates(&k, &e.key, whole));
        return;
    }
    let text = p.canonical();
    match lines.iter_mut().find(|e| e.key == k) {
        Some(e) => e.text = text,
        None => put(lines, Entry { key: k, text }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(items: &[(&str, bool)], negated: bool) -> Parse {
        Parse {
            command: 0,
            line: 0,
            negated,
            items: items
                .iter()
                .enumerate()
                .map(|(t, (s, kw))| {
                    if *kw {
                        Item::Kw {
                            kw: s.to_string(),
                            tok: t,
                            exact: true,
                        }
                    } else {
                        Item::Arg {
                            name: "a".into(),
                            value: s.to_string(),
                            tok: t,
                            typed: true,
                        }
                    }
                })
                .collect(),
        }
    }

    fn texts(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.text.as_str()).collect()
    }

    #[test]
    fn a_routed_port_keeps_no_switchport_until_it_is_a_switch_port_again() {
        let mut d = Device::new(Stack::single("C9300X-24Y", None));
        let b = d.block("interface TwentyFiveGigE1/0/1");
        d.apply_as(&p(&[("switchport", true)], true), Some(b), false, true);
        assert_eq!(texts(&d.blocks[b].lines), vec!["no switchport"]);
        d.apply_as(&p(&[("switchport", true)], false), Some(b), false, true);
        assert!(d.blocks[b].lines.is_empty());
    }

    #[test]
    fn a_later_line_replaces_the_same_command_and_instances_are_kept() {
        let mut d = Device::new(Stack::single("C9300X-24Y", None));
        let b = d.block("interface TwentyFiveGigE1/0/1");
        d.apply_as(
            &p(
                &[
                    ("switchport", true),
                    ("access", true),
                    ("vlan", true),
                    ("10", false),
                ],
                false,
            ),
            Some(b),
            false,
            true,
        );
        d.apply_as(
            &p(
                &[
                    ("switchport", true),
                    ("access", true),
                    ("vlan", true),
                    ("20", false),
                ],
                false,
            ),
            Some(b),
            false,
            true,
        );
        assert_eq!(texts(&d.blocks[b].lines), vec!["switchport access vlan 20"]);
        d.apply_as(
            &p(
                &[("ntp", true), ("server", true), ("192.0.2.121", false)],
                false,
            ),
            None,
            false,
            true,
        );
        d.apply_as(
            &p(
                &[("ntp", true), ("server", true), ("192.0.2.122", false)],
                false,
            ),
            None,
            false,
            true,
        );
        assert_eq!(
            texts(&d.global),
            vec!["ntp server 192.0.2.121", "ntp server 192.0.2.122"]
        );
        d.apply_as(
            &p(
                &[("ntp", true), ("server", true), ("192.0.2.121", false)],
                true,
            ),
            None,
            false,
            true,
        );
        assert_eq!(texts(&d.global), vec!["ntp server 192.0.2.122"]);
        d.apply_as(
            &p(&[("hostname", true), ("dc1-stack-sw01", false)], false),
            None,
            false,
            true,
        );
        assert_eq!(d.hostname, "dc1-stack-sw01");
        d.apply_as(
            &p(
                &[("switchport", true), ("access", true), ("vlan", true)],
                true,
            ),
            Some(b),
            false,
            true,
        );
        assert!(d.blocks[b].lines.is_empty());
    }
}
