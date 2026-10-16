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

/// The configuration a switch boots with no startup configuration, as lines that type it: the C9300
/// at 17.6 (the lab C9300-24T, 17.6.4, after `write erase` and a reload). The factory's own lines a
/// running configuration prints beside these (call-home, the control-plane policy, the self-signed
/// trustpoint, whose key each switch makes) are left out; the license line is the factory's level
/// (console-bridge record, 2026-10-14). None elsewhere: no capture holds another platform's or
/// release's default.
pub fn factory(d: &crate::device::Device) -> Option<Vec<String>> {
    let book = crate::grammars::book(d.stack.family());
    let train = crate::release::Release::parse(&d.release)?.config_version();
    if book != "c9300" || train != "17.6" {
        return None;
    }
    let mut lines: Vec<String> = [
        "service timestamps debug datetime msec",
        "service timestamps log datetime msec",
        "hostname Switch",
        "vrf definition Mgmt-vrf",
        " address-family ipv4",
        " exit-address-family",
        " address-family ipv6",
        " exit-address-family",
        "no aaa new-model",
        "license boot level network-advantage addon dna-advantage",
        "spanning-tree mode rapid-pvst",
        "spanning-tree extend system-id",
    ]
    .iter()
    .map(|l| l.to_string())
    .collect();
    if let Some(m) = d.stack.management() {
        lines.extend([
            format!("interface {m}"),
            " vrf forwarding Mgmt-vrf".into(),
            " no ip address".into(),
            " shutdown".into(),
            " negotiation auto".into(),
        ]);
    }
    lines.extend(
        [
            "interface Vlan1",
            " no ip address",
            " shutdown",
            "ip forward-protocol nd",
            "ip http server",
            "ip http authentication local",
            "ip http secure-server",
            "line con 0",
            " stopbits 1",
            "line vty 0 4",
            " login",
            " transport input ssh",
            "line vty 5 31",
            " login",
            " transport input ssh",
        ]
        .iter()
        .map(|l| l.to_string()),
    );
    Some(lines)
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
}
