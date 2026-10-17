//! `show version` and `show license summary` on a Catalyst 9300 at IOS XE 17.6.4, laid out as the lab
//! C9300-24T printed them, and the identity values they carry: the base MAC address from the switch's
//! own, the serial numbers derived from it, the uptime from the switch's clock.

use crate::device::Device;

/// The licence pair the switch booted at: its running configuration's `license boot level`, the
/// level IOS XE boots and keeps there until a reload (the lab C9300, 17.6.4); network-advantage with
/// dna-advantage without one, the factory's level.
fn levels(d: &Device) -> (String, String) {
    let line = d
        .global
        .iter()
        .find_map(|e| e.text.strip_prefix("license boot level "));
    let mut words = line
        .unwrap_or("network-advantage addon dna-advantage")
        .split_whitespace();
    let base = words.next().unwrap_or("network-advantage").to_string();
    let addon = match (words.next(), words.next()) {
        (Some("addon"), Some(a)) => a.to_string(),
        // a level with no addon names the matching DNA tier (inferred)
        _ => base.replace("network-", "dna-"),
    };
    (base, addon)
}

/// Whether the switch is one these layouts were measured on: a single Catalyst 9300 on the 17.6
/// train, which prints the 17.6.4 image's lines (inferred: vswitch keeps no maintenance release).
fn measured(d: &Device) -> bool {
    d.stack.family() == Some(crate::platform::Family::C9300)
        && d.stack.members.len() == 1
        && crate::release::Release::parse(&d.release).is_some_and(|r| r.config_version() == "17.6")
}

/// A licence's title as its entitlement tag spells it: `Network Essentials`, `DNA Advantage`.
fn title(level: &str) -> String {
    level
        .split('-')
        .map(|w| match w {
            "dna" => "DNA".to_string(),
            w => {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `show license summary` on a Catalyst 9300 at 17.6.4: the booted level and its addon, each in use
/// once per switch, the entitlement tag cut to 22 characters and `...` past 25 (the lab C9300-24T:
/// `(C9300-24 Network Essen...)`, `(C9300-24 DNA Essentials)`). The tag's product, the model's
/// first five characters and port count, is inferred for other models.
pub fn license_summary(d: &Device) -> Vec<String> {
    if !measured(d) {
        return vec!["% vswitch has no output model for 'show license summary' yet".into()];
    }
    let model = &d.stack.members[0].model;
    let product: String = model.chars().take(8).collect();
    let (base, addon) = levels(d);
    let tag = |level: &str| {
        let t = format!("{product} {}", title(level));
        let t = if t.len() > 25 {
            format!("{}...", &t[..22])
        } else {
            t
        };
        format!("({t})")
    };
    let row = |level: &str| format!("  {level:<24}{:<30}{:>5} IN USE", tag(level), 1);
    vec![
        "License Usage:".into(),
        "  License                 Entitlement Tag               Count Status".into(),
        format!("  {}", "-".repeat(77)),
        row(&base),
        row(&addon),
        String::new(),
        String::new(),
    ]
}

/// An uptime as IOS prints it: years, weeks, days and hours where not zero, then minutes always
/// (`1 year, 47 weeks, 4 days, 3 hours, 0 minutes`, `14 weeks, 1 day, 42 minutes`; captures of
/// every family). A year of 365 days is inferred.
pub fn uptime(secs: u64) -> String {
    let mins = secs / 60;
    let rest = mins % 525_600;
    let units = [
        ("year", mins / 525_600),
        ("week", rest / 10_080),
        ("day", rest % 10_080 / 1440),
        ("hour", mins % 1440 / 60),
    ];
    let one = |n: u64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    let mut parts: Vec<String> = units
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(u, n)| one(*n, u))
        .collect();
    parts.push(one(mins % 60, "minute"));
    parts.join(", ")
}

/// A serial number derived from the switch's base MAC address: three letters, then four digits
/// and four letters or digits, as Cisco's are laid out. `kind` keeps the system's and the
/// motherboard's apart.
pub fn serial(d: &Device, kind: &str) -> String {
    use md5::{Digest, Md5};
    let h = Md5::digest([&d.mac.0[..], kind.as_bytes()].concat());
    const ALNUM: &[u8] = b"0123456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    // a site code, the manufacturing year since 1996 and week, four characters (the serial layout)
    let digits = format!("{}{:02}", 20 + h[0] % 10, 1 + h[1] % 52);
    let rest: String = h[4..8]
        .iter()
        .map(|b| char::from(ALNUM[usize::from(*b) % ALNUM.len()]))
        .collect();
    let prefix = if kind == "board" { "FOX" } else { "FOC" };
    format!("{prefix}{digits}{rest}")
}

/// The base MAC address as `show version` writes it, colon-separated in lower case.
fn colon_mac(d: &Device) -> String {
    d.mac
        .0
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

const GPL: &[&str] = &[
    "Cisco IOS-XE software, Copyright (c) 2005-2022 by cisco Systems, Inc.",
    "All rights reserved.  Certain components of Cisco IOS-XE software are",
    "licensed under the GNU General Public License (\"GPL\") Version 2.0.  The",
    "software code licensed under GPL Version 2.0 is free software that comes",
    "with ABSOLUTELY NO WARRANTY.  You can redistribute and/or modify such",
    "GPL code under the terms of GPL Version 2.0.  For more details, see the",
    "documentation or \"License Notice\" file accompanying the IOS-XE software,",
    "or the applicable URL provided on the flyer accompanying the IOS-XE",
    "software.",
];

const CRYPTO: &[&str] = &[
    "This product contains cryptographic features and is subject to United",
    "States and local country laws governing import, export, transfer and",
    "use. Delivery of Cisco cryptographic products does not imply",
    "third-party authority to import, export, distribute or use encryption.",
    "Importers, exporters, distributors and users are responsible for",
    "compliance with U.S. and local country laws. By using this product you",
    "agree to comply with applicable laws and regulations. If you are unable",
    "to comply with U.S. and local laws, return this product immediately.",
    "",
    "A summary of U.S. laws governing Cisco cryptographic products may be found at:",
    "http://www.cisco.com/wwl/export/crypto/tool/stqrg.html",
    "",
    "If you require further assistance please contact us by sending email to",
    "export@cisco.com.",
];

/// The interface counts `show version` lists: the VLAN interfaces as Virtual Ethernet (inferred
/// past one), then each port type of the switch, the management port left out.
fn interface_counts(d: &Device) -> Vec<String> {
    let ports = d.stack.physical();
    let mgmt = d.stack.management();
    let count = |prefix: &str| {
        ports
            .iter()
            .filter(|p| p.starts_with(prefix) && Some(p.as_str()) != mgmt)
            .count()
    };
    let vlans = d
        .blocks
        .iter()
        .filter(|b| b.header.starts_with("interface Vlan"))
        .count();
    let line =
        |n: usize, what: &str| format!("{n} {what} interface{}", if n == 1 { "" } else { "s" });
    let mut out = vec![line(vlans, "Virtual Ethernet")];
    for (prefix, what) in [
        ("GigabitEthernet", "Gigabit Ethernet"),
        ("TenGigabitEthernet", "Ten Gigabit Ethernet"),
        ("TwentyFiveGigE", "TwentyFive Gigabit Ethernet"),
        ("FortyGigabitEthernet", "Forty Gigabit Ethernet"),
    ] {
        let n = count(prefix);
        if n > 0 {
            out.push(line(n, what));
        }
    }
    out
}

/// `show version` on a single Catalyst 9300 at 17.6.4, as the lab C9300-24T printed it: the
/// image's banner, the uptime from `booted` to `now` (the control processor's 90 seconds longer,
/// inferred from two readings), the licence levels booted with the next reboot's the same, the
/// hardware, its interface counts, the base MAC address and the serial numbers derived from it.
/// The ROMMON version, memory sizes, assembly and revision numbers are the lab unit's.
pub fn show_version(d: &Device, now: u64, booted: u64) -> Vec<String> {
    if !measured(d) {
        return vec!["% vswitch has no output model for 'show version' yet".into()];
    }
    let model = &d.stack.members[0].model;
    let up = now.saturating_sub(booted);
    let (base, addon) = levels(d);
    let system = serial(d, "system");
    let mut out: Vec<String> = vec![
        "Cisco IOS XE Software, Version 17.06.04".into(),
        "Cisco IOS Software [Bengaluru], Catalyst L3 Switch Software (CAT9K_IOSXE), Version 17.6.4, RELEASE SOFTWARE (fc1)".into(),
        "Technical Support: http://www.cisco.com/techsupport".into(),
        "Copyright (c) 1986-2022 by Cisco Systems, Inc.".into(),
        "Compiled Sun 14-Aug-22 08:58 by mcpre".into(),
        String::new(),
        String::new(),
    ];
    out.extend(GPL.iter().map(|l| l.to_string()));
    out.extend([
        String::new(),
        String::new(),
        "ROM: IOS-XE ROMMON".into(),
        "BOOTLDR: System Bootstrap, Version 17.6.1r[FC2], RELEASE SOFTWARE (P)".into(),
        String::new(),
        format!("{} uptime is {}", d.hostname, uptime(up)),
        format!("Uptime for this control processor is {}", uptime(up + 90)),
        "System returned to ROM by Reload Command".into(),
        "System image file is \"flash:packages.conf\"".into(),
        "Last reload reason: Reload Command".into(),
        String::new(),
        String::new(),
        String::new(),
    ]);
    out.extend(CRYPTO.iter().map(|l| l.to_string()));
    out.extend([
        String::new(),
        String::new(),
        "Technology Package License Information: ".into(),
        String::new(),
        "-".repeat(78),
        "Technology-package                                     Technology-package".into(),
        "Current                        Type                       Next reboot  ".into(),
        "-".repeat(78),
        format!("{base:<20}\t{:<30}\t {base:<20}", "Smart License"),
        format!(
            "{addon:<20}\t{:<30}\t {addon:<30}",
            "Subscription Smart License"
        ),
        "AIR License Level: AIR DNA Advantage".into(),
        "Next reload AIR license Level: AIR DNA Advantage".into(),
        String::new(),
        String::new(),
        "Smart Licensing Status: Registration Not Applicable/Not Applicable".into(),
        String::new(),
        format!("cisco {model} (X86) processor with 1317097K/6147K bytes of memory."),
        format!("Processor board ID {system}"),
    ]);
    out.extend(interface_counts(d));
    out.extend([
        "2048K bytes of non-volatile configuration memory.".into(),
        "8388608K bytes of physical memory.".into(),
        "1638400K bytes of Crash Files at crashinfo:.".into(),
        "11264000K bytes of Flash at flash:.".into(),
        String::new(),
    ]);
    let field = |name: &str, v: &str| format!("{name:<35}: {v}");
    out.extend([
        field("Base Ethernet MAC Address", &colon_mac(d)),
        field("Motherboard Assembly Number", "73-19911-04"),
        field("Motherboard Serial Number", &serial(d, "board")),
        field("Model Revision Number", "K0"),
        field("Motherboard Revision Number", "B0"),
        field("Model Number", model),
        field("System Serial Number", &system),
        field("CLEI Code Number", ""),
        String::new(),
        String::new(),
        "Switch Ports Model              SW Version        SW Image              Mode   ".into(),
        "------ ----- -----              ----------        ----------            ----   ".into(),
    ]);
    let ports = d
        .stack
        .physical()
        .iter()
        .filter(|p| Some(p.as_str()) != d.stack.management())
        .count();
    out.push(format!(
        "*{:>5} {ports:<5} {model:<18} {:<17} {:<21} INSTALL",
        1, "17.06.04", "CAT9K_IOSXE"
    ));
    out.extend([String::new(), String::new()]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_uptime_names_its_units_down_to_minutes() {
        assert_eq!(uptime(60), "1 minute");
        assert_eq!(uptime(150), "2 minutes");
        assert_eq!(uptime(3 * 3600), "3 hours, 0 minutes");
        assert_eq!(
            uptime((14 * 7 + 1) * 86_400 + 42 * 60),
            "14 weeks, 1 day, 42 minutes"
        );
        assert_eq!(uptime(365 * 86_400 + 60), "1 year, 1 minute");
    }

    /// A C9300-24T with a network module at 17.6, its base MAC address 0011.2233.4401.
    fn c9300(global: &[&str]) -> Device {
        let mut d = Device::new(crate::platform::parse_stack("C9300-24T+C9300-NM-4G").unwrap());
        d.release = "17.6".into();
        d.mac = crate::stp::Mac::parse("0011.2233.4401").unwrap();
        for l in global {
            d.global.push(crate::device::Entry {
                key: l.to_string(),
                text: l.to_string(),
            });
        }
        d
    }

    #[test]
    fn the_licence_summary_follows_the_boot_level() {
        let rows = |d: &Device| license_summary(d)[3..5].to_vec();
        assert_eq!(
            rows(&c9300(&[])),
            [
                "  network-advantage       (C9300-24 Network Advan...)       1 IN USE",
                "  dna-advantage           (C9300-24 DNA Advantage)          1 IN USE",
            ]
        );
        let d = c9300(&["license boot level network-essentials addon dna-essentials"]);
        assert_eq!(
            rows(&d),
            [
                "  network-essentials      (C9300-24 Network Essen...)       1 IN USE",
                "  dna-essentials          (C9300-24 DNA Essentials)         1 IN USE",
            ]
        );
        assert_eq!(license_summary(&d)[0], "License Usage:");
        assert_eq!(license_summary(&d)[5..], ["", ""]);
        let mut other = c9300(&[]);
        other.release = "17.15".into();
        assert!(license_summary(&other)[0].contains("no output model"));
    }

    #[test]
    fn show_version_carries_the_switch_s_own_identity_and_uptime() {
        let d = c9300(&["license boot level network-essentials addon dna-essentials"]);
        let out = show_version(&d, 1_000 + 100, 1_000);
        let line = |p: &str| {
            out.iter()
                .find(|l| l.starts_with(p))
                .cloned()
                .unwrap_or_else(|| panic!("{p}"))
        };
        assert_eq!(
            line("Base Ethernet MAC Address"),
            "Base Ethernet MAC Address          : 00:11:22:33:44:01"
        );
        assert_eq!(line("Switch uptime"), "Switch uptime is 1 minute");
        assert_eq!(
            line("Uptime for this control"),
            "Uptime for this control processor is 3 minutes"
        );
        assert_eq!(
            line("network-essentials"),
            "network-essentials  \tSmart License                 \t network-essentials  "
        );
        let system = serial(&d, "system");
        assert_eq!(
            line("Processor board ID"),
            format!("Processor board ID {system}")
        );
        assert_eq!(
            line("System Serial Number"),
            format!("System Serial Number               : {system}")
        );
        // three letters, a year and week, four letters or digits; the board's its own
        assert_eq!(system.len(), 11);
        assert!(system[3..7].chars().all(|c| c.is_ascii_digit()));
        assert_ne!(serial(&d, "board"), system);
        assert_eq!(serial(&d, "system"), system);
        assert_eq!(line("28 Gigabit"), "28 Gigabit Ethernet interfaces");
        assert_eq!(
            line("*    1"),
            "*    1 41    C9300-24T          17.06.04          CAT9K_IOSXE           INSTALL"
        );
        assert_eq!(out[out.len() - 2..], ["", ""]);
    }

    #[test]
    fn a_licence_title_spells_each_word() {
        assert_eq!(title("network-essentials"), "Network Essentials");
        assert_eq!(title("dna-advantage"), "DNA Advantage");
    }
}
