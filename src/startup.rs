//! `show startup-config`: the configuration `write memory` saved, as IOS prints it from NVRAM
//! (c9300 17.15 capture 1 at 17.15.02 and the lab 2960G at 15.0(2)SE11, 2026-10-07). `Using <n> out of <m>
//! bytes` comes first: n counts the saved text from its first `!` through `end`, each line with its
//! newline, plus one (2267 for the 2960G's 2266), and m is the NVRAM the platform's `show version`
//! names. Each certificate is the NVRAM file IOS keeps it in, not its hex. A blank line follows `end`.

/// The NVRAM a book's switches have, in bytes, as their captured `show version` names it
/// (`2048K bytes of non-volatile configuration memory.`). No 9400 capture holds `show version`: it
/// takes the 9200's and 9300's 2048K, the IOS XE Catalyst 9000s whose captures do.
pub fn nvram(book: &str) -> u64 {
    crate::corpus::get()
        .nvram
        .iter()
        .find(|(b, _)| b == book)
        .map_or(2048 * 1024, |(_, n)| *n)
}

/// The licence level a switch leaves the factory at: `license boot level` survives `write erase`, so
/// a switch reset to its default keeps the level it booted at (the lab C9300, 17.6.4).
pub const FACTORY_LEVEL: &str = "network-advantage addon dna-advantage";

/// The configuration a switch boots with no startup configuration, as the C9300 at 17.6 prints it
/// (the lab C9300-24T, 17.6.4, after `write erase` and a reload, at two licence levels): every line
/// and every `!` between them, the self-signed certificate made at `at` (seconds since 1970 UTC) for
/// the switch's own identity; and as the 2960-X at 15.2(2)E prints it (`c2960x_factory`). None
/// elsewhere: no capture holds another platform's or release's default.
pub fn factory(d: &crate::device::Device, level: &str, at: u64) -> Option<Vec<String>> {
    let book = crate::grammars::book(d.stack.family());
    let release = crate::release::Release::parse(&d.release)?;
    if book == "c2960x" && Some(release) == crate::release::Release::parse("15.2(2)E") {
        return Some(c2960x_factory(d));
    }
    if book != "c9300" || release.config_version() != "17.6" {
        return None;
    }
    // the `!` runs that differ between the two levels measured: network-essentials, network-advantage
    let advantage = level.starts_with("network-advantage");
    let n = |essentials: usize, adv: usize| if advantage { adv } else { essentials };
    let mut out: Vec<String> = vec![];
    let mut push = |lines: &[&str]| out.extend(lines.iter().map(|l| l.to_string()));
    push(&[
        "!",
        "version 17.6",
        "service timestamps debug datetime msec",
        "service timestamps log datetime msec",
        "! Call-home is enabled by Smart-Licensing.",
        "service call-home",
        "platform punt-keepalive disable-kernel-core",
        "!",
        "hostname Switch",
        "!",
        "!",
        "vrf definition Mgmt-vrf",
        " !",
        " address-family ipv4",
        " exit-address-family",
        " !",
        " address-family ipv6",
        " exit-address-family",
        "!",
        "!",
        "no aaa new-model",
    ]);
    let bangs =
        |out: &mut Vec<String>, k: usize| out.extend(std::iter::repeat_n("!".to_string(), k));
    for m in &d.stack.members {
        // one line per member (inferred for a stack: the lab switch stood alone)
        out.push(format!(
            "switch {} provision {}",
            m.number,
            m.model.to_ascii_lowercase()
        ));
    }
    bangs(&mut out, n(8, 12));
    out.push("login on-success log".into());
    bangs(&mut out, n(6, 9));
    let tp = trustpoint_number(d);
    let tp_name = format!("TP-self-signed-{tp}");
    out.extend([
        format!("crypto pki trustpoint {tp_name}"),
        " enrollment selfsigned".into(),
        format!(" subject-name cn=IOS-Self-Signed-Certificate-{tp}"),
        " revocation-check none".into(),
        format!(" rsakeypair {tp_name}"),
        "!".into(),
        "crypto pki trustpoint SLA-TrustPoint".into(),
        " enrollment pkcs12".into(),
        " revocation-check crl".into(),
        "!".into(),
        "!".into(),
        format!("crypto pki certificate chain {tp_name}"),
        " certificate self-signed 01".into(),
    ]);
    out.extend(cert_lines(&self_signed(d, tp, at)));
    out.extend([
        "crypto pki certificate chain SLA-TrustPoint".to_string(),
        " certificate ca 01".into(),
    ]);
    out.extend(cert_lines(&licensing_root(d)));
    bangs(&mut out, n(1, 2));
    out.push(format!("license boot level {level}"));
    let mut push = |lines: &[&str]| out.extend(lines.iter().map(|l| l.to_string()));
    push(&[
        "!",
        "!",
        "diagnostic bootup level minimal",
        "!",
        "spanning-tree mode rapid-pvst",
        "spanning-tree extend system-id",
        // the lab C9300-24T's; other models' watermark inferred to match
        "memory free low-watermark processor 131696",
        "!",
        "!",
        "redundancy",
        " mode sso",
        "!",
        "!",
        "!",
        "!",
        "!",
        "!",
        "transceiver type all",
        " monitoring",
        "!",
        "!",
    ]);
    for (name, description) in CLASS_MAPS {
        out.push(format!("class-map match-any {name}"));
        if !description.is_empty() {
            out.push(format!("  description {description}"));
        }
    }
    out.extend(["!".to_string(), "policy-map system-cpp-policy".into()]);
    out.extend(["!".to_string(), "! ".into()]);
    bangs(&mut out, n(9, 10));
    if let Some(m) = d.stack.management() {
        out.extend([
            format!("interface {m}"),
            " vrf forwarding Mgmt-vrf".into(),
            " no ip address".into(),
            " shutdown".into(),
            " negotiation auto".into(),
            "!".into(),
        ]);
    }
    for p in d.stack.physical() {
        out.extend([format!("interface {p}"), "!".into()]);
    }
    let mut push = |lines: &[&str]| out.extend(lines.iter().map(|l| l.to_string()));
    push(&[
        "interface Vlan1",
        " no ip address",
        " shutdown",
        "!",
        "ip forward-protocol nd",
        "ip http server",
        "ip http authentication local",
        "ip http secure-server",
    ]);
    bangs(&mut out, n(4, 6));
    let mut push = |lines: &[&str]| out.extend(lines.iter().map(|l| l.to_string()));
    push(&[
        "control-plane",
        " service-policy input system-cpp-policy",
        "!",
        "!",
        "line con 0",
        " stopbits 1",
        "line vty 0 4",
        " login",
        " transport input ssh",
        "line vty 5 31",
        " login",
        " transport input ssh",
        "!",
        "call-home",
        " ! If contact email address in call-home is configured as sch-smart-licensing@cisco.com",
        " ! the email address configured in Cisco Smart License Portal will be used as contact email address to send SCH notifications.",
        " contact-email-addr sch-smart-licensing@cisco.com",
        " profile \"CiscoTAC-1\"",
        "  active",
        "  destination transport-method http",
        "!",
        "!",
        "!",
        "!",
        "!",
        "!",
        "end",
    ]);
    Some(out)
}

/// The configuration a 2960-X at 15.2(2)E boots with no startup configuration, as the lab
/// WS-C2960X-24PS-L printed it (15.2(2)E5, 2026-10-29, uncabled, so autoinstall found no DHCP
/// server and changed nothing): every line and every `!` between them, the ports and Vlan1 with
/// nothing under them, and `line con 0` and `line vty 5 15` with no `line vty 0 4`.
fn c2960x_factory(d: &crate::device::Device) -> Vec<String> {
    let lines = |l: &[&str]| l.iter().map(|l| l.to_string()).collect::<Vec<_>>();
    let bangs = |k: usize| vec!["!".to_string(); k];
    let mut out = lines(&[
        "!",
        "version 15.2",
        "no service pad",
        "service timestamps debug datetime msec",
        "service timestamps log datetime msec",
        "no service password-encryption",
        "!",
        "hostname Switch",
        "!",
        "boot-start-marker",
        "boot-end-marker",
        "!",
        "!",
        "no aaa new-model",
    ]);
    for m in &d.stack.members {
        // one line per member (inferred for a stack: the lab switch stood alone)
        out.push(format!(
            "switch {} provision {}",
            m.number,
            m.model.to_ascii_lowercase()
        ));
    }
    out.extend(bangs(10));
    out.extend(lines(&[
        "spanning-tree mode pvst",
        "spanning-tree extend system-id",
    ]));
    out.extend(bangs(4));
    out.push("vlan internal allocation policy ascending".into());
    out.extend(bangs(11));
    for p in d.stack.physical() {
        out.extend([format!("interface {p}"), "!".into()]);
    }
    out.extend(lines(&[
        "interface Vlan1",
        "!",
        "ip http server",
        "ip http secure-server",
    ]));
    out.extend(bangs(5));
    out.extend(lines(&["line con 0", "line vty 5 15", "!", "end"]));
    out
}

/// The control-plane class maps IOS XE 17.6 makes, in its order, with their descriptions as it
/// prints them, trailing spaces kept (the lab C9300, 17.6.4).
pub const CLASS_MAPS: [(&str, &str); 22] = [
    ("system-cpp-police-ewlc-control", "EWLC Control "),
    ("system-cpp-police-topology-control", "Topology control"),
    (
        "system-cpp-police-sw-forward",
        "Sw forwarding, L2 LVX data packets, LOGGING, Transit Traffic",
    ),
    ("system-cpp-default", "EWLC Data, Inter FED Traffic "),
    (
        "system-cpp-police-sys-data",
        "Openflow, Exception, EGR Exception, NFL Sampled Data, RPF Failed",
    ),
    ("system-cpp-police-punt-webauth", "Punt Webauth"),
    ("system-cpp-police-l2lvx-control", "L2 LVX control packets"),
    (
        "system-cpp-police-forus",
        "Forus Address resolution and Forus traffic",
    ),
    (
        "system-cpp-police-multicast-end-station",
        "MCAST END STATION",
    ),
    ("system-cpp-police-high-rate-app", "High Rate Applications "),
    ("system-cpp-police-multicast", "MCAST Data"),
    ("system-cpp-police-l2-control", "L2 control"),
    ("system-cpp-police-dot1x-auth", "DOT1X Auth"),
    (
        "system-cpp-police-data",
        "ICMP redirect, ICMP_GEN and BROADCAST",
    ),
    (
        "system-cpp-police-stackwise-virt-control",
        "Stackwise Virtual OOB",
    ),
    ("non-client-nrt-class", ""),
    (
        "system-cpp-police-routing-control",
        "Routing control and Low Latency",
    ),
    ("system-cpp-police-protocol-snooping", "Protocol snooping"),
    ("system-cpp-police-dhcp-snooping", "DHCP snooping"),
    (
        "system-cpp-police-ios-routing",
        "L2 control, Topology control, Routing control, Low Latency",
    ),
    (
        "system-cpp-police-system-critical",
        "System Critical and Gold Pkt",
    ),
    (
        "system-cpp-police-ios-feature",
        "ICMPGEN,BROADCAST,ICMP,L2LVXCntrl,ProtoSnoop,PuntWebauth,MCASTData,Transit,DOT1XAuth,Swfwd,LOGGING,L2LVXData,ForusTraffic,ForusARP,McastEndStn,Openflow,Exception,EGRExcption,NflSampled,RpfFailed",
    ),
];

/// Bytes standing in for a switch's own key material: MD5 of the base MAC address, `kind` and a
/// counter, chained. The same switch gets the same bytes every time.
fn identity_bytes(d: &crate::device::Device, kind: &str, n: usize) -> Vec<u8> {
    use md5::{Digest, Md5};
    let mut out = Vec::with_capacity(n + 16);
    let mut i: u32 = 0;
    while out.len() < n {
        out.extend(Md5::digest(
            [&d.mac.0[..], kind.as_bytes(), &i.to_be_bytes()].concat(),
        ));
        i += 1;
    }
    out.truncate(n);
    out
}

/// The self-signed trustpoint's number: ten digits, as IOS's are, from the switch's identity (each
/// switch keeps its own across boots: the lab C9300, 17.6.4).
pub fn trustpoint_number(d: &crate::device::Device) -> u32 {
    let b = identity_bytes(d, "trustpoint", 4);
    1_000_000_000 + u32::from_be_bytes([b[0], b[1], b[2], b[3]]) % 3_294_967_296
}

/// A DER element of `tag` holding `parts`.
fn der(tag: u8, parts: &[&[u8]]) -> Vec<u8> {
    let body = parts.concat();
    let mut out = vec![tag];
    match body.len() {
        n if n < 0x80 => out.push(n as u8),
        n if n < 0x100 => out.extend([0x81, n as u8]),
        n => out.extend([0x82, (n >> 8) as u8, n as u8]),
    }
    out.extend(body);
    out
}

const RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
const SHA1_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x05];
const SHA256_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];

/// An X.509 v3 certificate laid out as IOS XE prints its two: serial 01, an RSA 2048 key with
/// exponent 65537, the key, key identifiers and signature from the switch's identity, `kind`
/// keeping each certificate's apart.
fn certificate(
    d: &crate::device::Device,
    kind: &str,
    algorithm: &[u8],
    name: &[u8],
    validity: [String; 2],
    extensions: &[Vec<u8>],
) -> Vec<u8> {
    let random = identity_bytes(d, kind, 256 + 256);
    let mut modulus = vec![0];
    modulus.extend(&random[..256]);
    // a 2048-bit modulus has its top bit set
    modulus[1] |= 0x80;
    let alg = der(0x30, &[&der(0x06, &[algorithm]), &[0x05, 0x00]]);
    let key = der(
        0x30,
        &[&der(0x02, &[&modulus]), &der(0x02, &[&[0x01, 0x00, 0x01]])],
    );
    let spki = der(
        0x30,
        &[
            &der(0x30, &[&der(0x06, &[RSA]), &[0x05, 0x00]]),
            &der(0x03, &[&[0], &key]),
        ],
    );
    let time = |t: &String| der(0x17, &[t.as_bytes()]);
    let tbs = der(
        0x30,
        &[
            &der(0xa0, &[&der(0x02, &[&[2]])]),
            &der(0x02, &[&[1]]),
            &alg,
            name,
            &der(0x30, &[&time(&validity[0]), &time(&validity[1])]),
            name,
            &spki,
            &der(0xa3, &[&der(0x30, &[&extensions.concat()])]),
        ],
    );
    der(0x30, &[&tbs, &alg, &der(0x03, &[&[0], &random[256..]])])
}

/// A name of one common name, as IOS writes the self-signed certificate's.
fn common_name(cn: &str) -> Vec<u8> {
    der(
        0x30,
        &[&der(
            0x31,
            &[&der(
                0x30,
                &[
                    &der(0x06, &[&[0x55, 0x04, 0x03]]),
                    &der(0x13, &[cn.as_bytes()]),
                ],
            )],
        )],
    )
}

/// A UTC time as a certificate holds it: `YYMMDDHHMMSSZ`.
fn utc(secs: u64) -> String {
    let stamp = crate::shell::config_stamp(secs);
    let w: Vec<&str> = stamp.split_whitespace().collect();
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| *m == w[3])
    .unwrap_or(0)
        + 1;
    let day: u32 = w[4].parse().unwrap_or(1);
    let year: u32 = w[5].parse().unwrap_or(1970);
    format!(
        "{:02}{month:02}{day:02}{}Z",
        year % 100,
        w[0].replace(':', "")
    )
}

/// The trustpoint's self-signed certificate, made at boot: valid from two seconds before the
/// configuration's last change for 3652 days (the lab C9300, 17.6.4, at two boots; the two seconds
/// inferred, two and three measured), SHA-1 signed, a CA's basic constraints and key identifiers.
fn self_signed(d: &crate::device::Device, tp: u32, at: u64) -> Vec<u8> {
    let name = common_name(&format!("IOS-Self-Signed-Certificate-{tp}"));
    let start = at.saturating_sub(2);
    let id = identity_bytes(d, "self-signed key id", 20);
    let extensions = [
        der(
            0x30,
            &[
                &der(0x06, &[&[0x55, 0x1d, 0x13]]),
                &[0x01, 0x01, 0xff],
                &der(0x04, &[&der(0x30, &[&[0x01, 0x01, 0xff]])]),
            ],
        ),
        der(
            0x30,
            &[
                &der(0x06, &[&[0x55, 0x1d, 0x23]]),
                &der(0x04, &[&der(0x30, &[&der(0x80, &[&id])])]),
            ],
        ),
        der(
            0x30,
            &[
                &der(0x06, &[&[0x55, 0x1d, 0x0e]]),
                &der(0x04, &[&der(0x04, &[&id])]),
            ],
        ),
    ];
    certificate(
        d,
        "self-signed",
        SHA1_RSA,
        &name,
        [utc(start), utc(start + 3652 * 86_400)],
        &extensions,
    )
}

/// The licensing root CA's certificate as the image carries it: its names, dates and extensions as
/// IOS XE 17.6 prints them; its key, key identifier and signature stand-ins from the switch's identity.
fn licensing_root(d: &crate::device::Device) -> Vec<u8> {
    let attr = |oid: u8, v: &str| {
        der(
            0x31,
            &[&der(
                0x30,
                &[
                    &der(0x06, &[&[0x55, 0x04, oid]]),
                    &der(0x13, &[v.as_bytes()]),
                ],
            )],
        )
    };
    let name = der(
        0x30,
        &[&attr(0x0a, "Cisco"), &attr(0x03, "Cisco Licensing Root CA")],
    );
    let id = identity_bytes(d, "licensing root key id", 20);
    let extensions = [
        der(
            0x30,
            &[
                &der(0x06, &[&[0x55, 0x1d, 0x0f]]),
                &[0x01, 0x01, 0xff],
                &der(0x04, &[&[0x03, 0x02, 0x01, 0x06]]),
            ],
        ),
        der(
            0x30,
            &[
                &der(0x06, &[&[0x55, 0x1d, 0x13]]),
                &[0x01, 0x01, 0xff],
                &der(0x04, &[&der(0x30, &[&[0x01, 0x01, 0xff]])]),
            ],
        ),
        der(
            0x30,
            &[
                &der(0x06, &[&[0x55, 0x1d, 0x0e]]),
                &der(0x04, &[&der(0x04, &[&id])]),
            ],
        ),
    ];
    certificate(
        d,
        "licensing root",
        SHA256_RSA,
        &name,
        ["130530194847Z".into(), "380530194847Z".into()],
        &extensions,
    )
}

/// A certificate's hex as the running configuration prints it: 32 bytes a line in groups of four,
/// each group followed by a space but the last, then `quit` after a tab.
fn cert_lines(der: &[u8]) -> Vec<String> {
    let groups: Vec<String> = der
        .chunks(4)
        .map(|g| g.iter().map(|b| format!("{b:02X}")).collect())
        .collect();
    let mut out: Vec<String> = groups
        .chunks(8)
        .map(|line| format!("  {} ", line.join(" ")))
        .collect();
    if let Some(last) = out.last_mut() {
        last.pop();
    }
    out.push("  \tquit".into());
    out
}

/// A DER element: its tag, its contents and what follows it.
fn tlv(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = b.split_first()?;
    let (&first, mut rest) = rest.split_first()?;
    let len = if first & 0x80 == 0 {
        usize::from(first)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        let len = rest[..n]
            .iter()
            .fold(0usize, |a, &x| (a << 8) | usize::from(x));
        rest = &rest[n..];
        len
    };
    (rest.len() >= len).then(|| (tag, &rest[..len], &rest[len..]))
}

/// The common name of a DER certificate's subject.
fn subject_name(der: &[u8]) -> Option<String> {
    let (0x30, cert, _) = tlv(der)? else {
        return None;
    };
    let (0x30, tbs, _) = tlv(cert)? else {
        return None;
    };
    let mut fields = vec![];
    let mut rest = tbs;
    while let Some((tag, value, after)) = tlv(rest) {
        fields.push((tag, value));
        rest = after;
    }
    // [0] version when present, then serial, signature, issuer, validity, subject
    let skip = usize::from(fields.first()?.0 == 0xa0);
    let (_, mut sets) = *fields.get(skip + 4)?;
    while let Some((0x31, set, after)) = tlv(sets) {
        let mut pairs = set;
        while let Some((0x30, pair, next)) = tlv(pairs) {
            if let Some((0x06, oid, value)) = tlv(pair)
                && oid == [0x55, 0x04, 0x03]
                && let Some((_, name, _)) = tlv(value)
            {
                return Some(String::from_utf8_lossy(name).into_owned());
            }
            pairs = next;
        }
        sets = after;
    }
    None
}

/// The bytes a certificate's hex lines hold.
fn hex_bytes(lines: &[String]) -> Option<Vec<u8>> {
    let digits: String = lines.iter().flat_map(|l| l.split_whitespace()).collect();
    if !digits.len().is_multiple_of(2) || !digits.is_ascii() {
        return None;
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).ok())
        .collect()
}

/// The configuration as NVRAM keeps it: each certificate, its hex and `quit`, as one line naming
/// its file (`certificate ca 01 nvram:CiscoLicensi#1CA.cer`): the subject's common name without
/// spaces cut to 12 characters, `#` and its place among the certificates, `CA` for a CA's. A
/// certificate whose hex names no subject is left as it is.
pub fn saved(config: &[String]) -> Vec<String> {
    let mut out = vec![];
    let mut n = 0;
    let mut i = 0;
    while i < config.len() {
        let line = &config[i];
        let words: Vec<&str> = line.split_whitespace().collect();
        let header = line.starts_with(' ')
            && !line.starts_with("  ")
            && words.first() == Some(&"certificate");
        if !header || words.len() < 2 {
            out.push(line.clone());
            i += 1;
            continue;
        }
        let end = config[i + 1..]
            .iter()
            .position(|l| l.trim() == "quit")
            .map(|p| i + 1 + p);
        let name = end
            .and_then(|e| hex_bytes(&config[i + 1..e]))
            .and_then(|der| subject_name(&der));
        let (Some(end), Some(name)) = (end, name) else {
            out.push(line.clone());
            i += 1;
            continue;
        };
        n += 1;
        let short: String = name
            .chars()
            .filter(|c| !c.is_whitespace())
            .take(12)
            .collect();
        let ca = if words[1] == "ca" { "CA" } else { "" };
        out.push(format!("{} nvram:{short}#{n}{ca}.cer", line.trim_end()));
        i = end + 1;
    }
    out
}

/// `show startup-config` for a saved configuration, its lines from the first `!` to `end`.
pub fn show(config: &[String], book: &str) -> Vec<String> {
    let body = saved(config);
    let bytes = body.iter().map(|l| l.len() + 1).sum::<usize>() + 1;
    let mut out = vec![format!("Using {bytes} out of {} bytes", nvram(book))];
    out.extend(body);
    out.push(String::new());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(test, captures))]
    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/startup-tests.rs"));

    #[test]
    fn the_header_counts_the_saved_text_and_names_the_platform_s_nvram() {
        let config: Vec<String> = ["!", "version 15.0", "hostname x", "end"]
            .map(String::from)
            .to_vec();
        // 2 + 13 + 11 + 4, plus one
        assert_eq!(show(&config, "c2960")[0], "Using 31 out of 65536 bytes");
        assert_eq!(show(&config, "c9300")[0], "Using 31 out of 2097152 bytes");
        assert_eq!(show(&config, "c2960x")[0], "Using 31 out of 524288 bytes");
        assert_eq!(show(&config, "ie3300")[0], "Using 31 out of 33554432 bytes");
        assert_eq!(show(&config, "c9400")[0], "Using 31 out of 2097152 bytes");
        assert_eq!(show(&config, "c2960").last().unwrap(), "");
    }

    /// A C9300-24T at 17.6 with base MAC address 0011.2233.4401.
    fn switch() -> crate::device::Device {
        let mut d = crate::device::Device::new(
            crate::platform::parse_stack("C9300-24T+C9300-NM-4G").unwrap(),
        );
        d.release = "17.6".into();
        d.mac = crate::stp::Mac::parse("0011.2233.4401").unwrap();
        d
    }

    /// Each certificate's hex lines in `config`, as bytes and as the lines' lengths.
    fn certificates(config: &[String]) -> Vec<(Vec<u8>, Vec<usize>)> {
        let mut out = vec![];
        let mut at = 0;
        while let Some(i) = config[at..]
            .iter()
            .position(|l| l.starts_with(" certificate "))
        {
            let start = at + i + 1;
            let end = start
                + config[start..]
                    .iter()
                    .position(|l| l.trim() == "quit")
                    .unwrap();
            let lines = &config[start..end];
            out.push((
                hex_bytes(lines).unwrap(),
                lines.iter().map(String::len).collect(),
            ));
            at = end;
        }
        out
    }

    #[test]
    fn the_factory_default_prints_its_certificates_as_ios_xe_lays_them_out() {
        let d = switch();
        // 2026-03-05 12:34:56 UTC
        let at = 1_772_714_096;
        let config = factory(&d, FACTORY_LEVEL, at).unwrap();
        let tp = trustpoint_number(&d);
        assert!((1_000_000_000..=u32::MAX).contains(&tp));
        assert!(config.contains(&format!("crypto pki trustpoint TP-self-signed-{tp}")));
        let certs = certificates(&config);
        assert_eq!(certs.len(), 2);
        let (own, own_lines) = &certs[0];
        let (root, root_lines) = &certs[1];
        // 820 and 805 bytes: 25 full lines of 32 bytes in groups of four, each with a space after
        // it, then the rest with none after its last group
        assert_eq!((own.len(), root.len()), (820, 805));
        assert!(
            own_lines[..25]
                .iter()
                .chain(&root_lines[..25])
                .all(|n| *n == 74)
        );
        assert_eq!((own_lines[25], root_lines[25]), (46, 13));
        assert_eq!(
            subject_name(own).as_deref(),
            Some(format!("IOS-Self-Signed-Certificate-{tp}").as_str())
        );
        assert_eq!(
            subject_name(root).as_deref(),
            Some("Cisco Licensing Root CA")
        );
        // valid from two seconds before boot, for 3652 days
        let own_text = String::from_utf8_lossy(own);
        assert!(own_text.contains("260305123454Z") && own_text.contains("360304123454Z"));
        // the same switch makes the same certificate; another makes its own
        assert_eq!(factory(&d, FACTORY_LEVEL, at).unwrap(), config);
        let mut other = switch();
        other.mac = crate::stp::Mac::parse("0011.2233.4402").unwrap();
        assert_ne!(
            certificates(&factory(&other, FACTORY_LEVEL, at).unwrap())[0].0,
            *own
        );
        // NVRAM names the root by its common name
        assert!(
            saved(&config).contains(&" certificate ca 01 nvram:CiscoLicensi#2CA.cer".to_string())
        );
    }

    #[test]
    fn the_factory_default_spaces_its_lines_by_the_licence_level() {
        let d = switch();
        let essentials = factory(&d, "network-essentials addon dna-essentials", 0).unwrap();
        let advantage = factory(&d, FACTORY_LEVEL, 0).unwrap();
        let bangs = |c: &[String]| c.iter().filter(|l| l.as_str() == "!").count();
        assert_eq!(bangs(&advantage) - bangs(&essentials), 11);
        assert!(
            essentials.contains(
                &"license boot level network-essentials addon dna-essentials".to_string()
            )
        );
        assert_eq!(advantage.first().map(String::as_str), Some("!"));
        assert_eq!(advantage.last().map(String::as_str), Some("end"));
        let mut other = switch();
        other.release = "17.15".into();
        assert!(factory(&other, FACTORY_LEVEL, 0).is_none());
    }
}
