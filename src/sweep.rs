//! Removes passwords and other secrets from switch configuration text (Cisco IOS and IOS-XE, HPE Onyx)
//! and from session logs, where each typed command follows its prompt. The value goes and the rest of
//! the line stays, so a cleaned file still reads and still loads; a value that runs to the end of its
//! line (a password with spaces) goes whole, and a changed line is rewritten with single spaces.
//! Keywords count abbreviated as IOS takes them. Reports name the kind of secret, never the value.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const REMOVED: &str = "<removed>";

#[derive(Debug, Default, PartialEq)]
pub struct Swept {
    pub text: String,
    /// kind of secret -> lines cleaned
    pub found: BTreeMap<&'static str, usize>,
    /// removed values still present anywhere in the text (zero unless a value repeats in a place no
    /// rule covers)
    pub left: usize,
}

impl Swept {
    pub fn lines(&self) -> usize {
        self.found.values().sum()
    }

    pub fn summary(&self) -> String {
        if self.found.is_empty() {
            return "nothing to clean".into();
        }
        let kinds: Vec<String> = self.found.iter().map(|(k, n)| format!("{k} {n}")).collect();
        format!("{} line(s) cleaned ({})", self.lines(), kinds.join(", "))
    }
}

/// A value a rule finds: the words `from..to` of the line's command, and the kind of secret.
type Hit = (usize, usize, &'static str);

/// An encryption type between a keyword and its value: `secret 9 <hash>`, `password 7 <string>`.
fn is_type(t: &str) -> bool {
    matches!(t, "0" | "5" | "6" | "7" | "8" | "9")
}

/// Hashes IOS and Linux write (`$9$...`, `$1$...`), wherever they stand.
fn is_hash(t: &str) -> bool {
    ["$1$", "$5$", "$6$", "$8$", "$9$", "$14$"]
        .iter()
        .any(|p| t.starts_with(p))
}

/// The index of a value after `from`, past an optional encryption type and the modifiers given.
fn value_after(lw: &[String], from: usize, modifiers: &[&str]) -> usize {
    let mut j = from;
    while j < lw.len() && (is_type(&lw[j]) || modifiers.contains(&lw[j].as_str())) {
        j += 1;
    }
    j
}

/// Whether a typed word stands for `keyword`: IOS takes any prefix that names one keyword, and `min`
/// is the shortest prefix that names this one where the rule reads it.
fn abbreviates(word: &str, keyword: &str, min: usize) -> bool {
    word.len() >= min && keyword.starts_with(word)
}

/// Whether the word after `password` is a keyword of the command rather than its value: `password
/// encryption aes`, `system disable password recovery`, `cts sxp connection peer ... password
/// default`, `cip security password window`, `ip ssh server algorithm authentication password
/// keyboard`. `first` is the command's first word, `second` the one after it.
fn names_no_password(first: &str, second: &str, next: &str) -> bool {
    match next {
        "encryption" | "recovery" => true,
        "default" | "none" | "key-chain" => first == "cts",
        "window" => first == "cip",
        "keyboard" | "publickey" => first == "ip" && second == "ssh",
        _ => false,
    }
}

/// Where a line's command stands: inside a block or not, the block's header (lowercase), the mode its
/// prompt names when the line was typed at one, and whether the line is free text (a banner's), where
/// only a command's own leading keywords name a secret.
struct Place<'a> {
    indented: bool,
    block: &'a str,
    mode: Option<&'a str>,
    text: bool,
}

/// The prompt a command typed in a session log follows (`sw1(config-if)#`, `sw1#`, `sw1>`): its length
/// and the mode it names (`config-if`; `privileged` for `#`, `exec` for `>`).
fn prompt(body: &str) -> Option<(usize, &str)> {
    let host = body.find(|c: char| !(c.is_ascii_alphanumeric() || "._-".contains(c)))?;
    if host == 0 {
        return None;
    }
    let rest = &body[host..];
    if let Some(r) = rest.strip_prefix('(') {
        let close = r.find(')')?;
        let mode = &r[..close];
        let named = !mode.is_empty()
            && mode
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_+".contains(c));
        (named && r[close + 1..].starts_with('#')).then_some((host + close + 3, mode))
    } else if rest.starts_with('#') {
        Some((host + 1, "privileged"))
    } else if rest.starts_with('>') {
        Some((host + 1, "exec"))
    } else {
        None
    }
}

/// The delimiter a `banner` line opens its text with, and the index of the word it starts: IOS prints
/// the Ctrl-C a banner was typed with as `^C`, and takes any other first character as typed.
fn banner_opens(words: &[&str], lw: &[String]) -> Option<(String, usize)> {
    if !lw.first().is_some_and(|w| abbreviates(w, "banner", 3)) {
        return None;
    }
    const KINDS: &[&str] = &[
        "motd",
        "login",
        "exec",
        "incoming",
        "prompt-timeout",
        "slip-ppp",
        "config-save",
    ];
    let at = if lw
        .get(1)
        .is_some_and(|w| KINDS.iter().any(|k| abbreviates(w, k, 1)))
    {
        2
    } else {
        1
    };
    let word = words.get(at)?;
    let mut chars = word.chars();
    let first = chars.next()?;
    let delimiter = match (first, chars.next()) {
        ('^', Some(c)) if c.is_ascii_uppercase() => format!("^{c}"),
        _ => first.to_string(),
    };
    Some((delimiter, at))
}

/// The byte range of the password in a URL word (`ftp://user:<password>@host/file`).
fn url_password(word: &str) -> Option<(usize, usize)> {
    let start = word.find("://")? + 3;
    let authority_end = word[start..].find('/').map_or(word.len(), |e| start + e);
    let at = start + word[start..authority_end].rfind('@')?;
    let colon = start + word[start..at].find(':')? + 1;
    (colon < at && &word[colon..at] != REMOVED).then_some((colon, at))
}

/// `pre-shared-key`'s value: after `local` or `remote` (IKEv2), after `key` in a keyring's `address
/// <ip> [<mask>] key <value>` or `hostname <name> key <value>`, else the next word; MACsec's
/// `pre-shared-key key-chain <name>` names none.
fn pre_shared(lw: &[String], i: usize) -> Option<Hit> {
    let at = |j: usize| lw.get(j).map(String::as_str).unwrap_or("");
    let from = match at(i + 1) {
        "key-chain" => return None,
        "local" | "remote" => i + 2,
        "address" | "hostname" => (i + 3..lw.len()).find(|&j| at(j) == "key")? + 1,
        _ => i + 1,
    };
    let v = value_after(lw, from, &["ascii", "hex", "encrypted"]);
    Some((v, lw.len(), "pre-shared-key"))
}

/// The secret values in one command's words (lowercase), each with the kind of secret.
fn secrets(lw: &[String], place: &Place) -> Vec<Hit> {
    let n = lw.len();
    let at = |i: usize| lw.get(i).map(String::as_str).unwrap_or("");
    let is = |i: usize, keyword: &str, min: usize| {
        lw.get(i).is_some_and(|w| abbreviates(w, keyword, min))
    };
    let mut out: Vec<Hit> = Vec::new();
    // a `no` or `default` form names the same value: `no snmp-server community <string>`
    let k = usize::from(matches!(at(0), "no" | "default"));
    let enable = is(k, "enable", 3);
    let username = is(k, "username", 3);
    if enable {
        let mut j = k + 1;
        while is(j, "algorithm-type", 2) || is(j, "common-criteria-policy", 2) {
            j += 2;
        }
        if is(j, "secret", 1) || is(j, "password", 1) {
            let v = if is(j + 1, "level", 2) { j + 3 } else { j + 1 };
            out.push((value_after(lw, v, &[]), n, "enable secret"));
        }
    } else if username {
        if let Some(j) = (k + 2..n).find(|&j| is(j, "secret", 3) || is(j, "password", 3)) {
            out.push((value_after(lw, j + 1, &[]), n, "username"));
        }
    } else if is(k, "snmp-server", 4) {
        if is(k + 1, "community", 3) {
            out.push((k + 2, k + 3, "snmp-server community"));
        } else if is(k + 1, "host", 1) {
            // the options before the community are whole words, so no community is read as one
            let mut j = k + 3;
            loop {
                match at(j) {
                    "vrf" => j += 2,
                    "informs" | "traps" => j += 1,
                    "version" => {
                        j += 2;
                        if matches!(at(j), "auth" | "noauth" | "priv") {
                            j += 1;
                        }
                    }
                    _ => break,
                }
            }
            out.push((j, j + 1, "snmp-server host"));
        } else if is(k + 1, "user", 1)
            && let Some(v3) = (k + 2..n).find(|&j| at(j) == "v3")
        {
            // `auth` and `priv` follow the version; `sha-2` and `aes` name a length before the password
            let mut j = v3 + 1;
            while j < n {
                let v = if at(j) == "auth" {
                    j + 2 + usize::from(at(j + 1).starts_with("sha-"))
                } else if at(j) == "priv" {
                    j + 2 + usize::from(at(j + 1) == "aes")
                } else {
                    j += 1;
                    continue;
                };
                out.push((v, v + 1, "snmp-server user"));
                j = v + 1;
            }
        }
    } else if is(k, "tacacs-server", 7) || is(k, "radius-server", 7) {
        // a server key may hold spaces and is the last thing on its line
        if is(k + 1, "key", 1) {
            out.push((value_after(lw, k + 2, &[]), n, "server key"));
        } else if is(k + 1, "host", 1)
            && let Some(j) = (k + 3..n).find(|&j| at(j) == "key")
        {
            out.push((value_after(lw, j + 1, &[]), n, "server key"));
        }
    } else if is(k, "server-private", 8) {
        if let Some(j) = (k + 2..n).find(|&j| at(j) == "key") {
            out.push((value_after(lw, j + 1, &[]), n, "server key"));
        }
    } else if is(k, "crypto", 2) && is(k + 1, "isakmp", 2) && is(k + 2, "key", 3) {
        let v = value_after(lw, k + 3, &[]);
        out.push((v, v + 1, "isakmp key"));
    } else if is(k, "key", 3) && is(k + 1, "config-key", 2) && is(k + 2, "password-encrypt", 1) {
        out.push((value_after(lw, k + 3, &[]), n, "config key"));
    }
    let header: Vec<&str> = place.block.split_whitespace().collect();
    let mode = place.mode.unwrap_or("");
    let server = (header.len() >= 2
        && (header[0] == "tacacs" || header[0] == "radius")
        && abbreviates(header[1], "server", 3))
        || ["server-tacacs", "radius-server", "server-radius"]
            .iter()
            .any(|m| mode.contains(m));
    let line = mode == "config-line"
        || (place.mode.is_none() && header.first().is_some_and(|w| abbreviates(w, "line", 3)));
    if place.indented && server {
        if is(k, "key", 1) {
            out.push((value_after(lw, k + 1, &[]), n, "server key"));
        } else if is(k, "pac", 2) && is(k + 1, "key", 1) {
            out.push((value_after(lw, k + 2, &[]), n, "server key"));
        }
    }
    if place.indented && line && is(k, "password", 3) {
        out.push((value_after(lw, k + 1, &[]), n, "password"));
    }
    if mode == "config-keychain-key" && is(k, "key-string", 1) {
        out.push((value_after(lw, k + 1, &[]), n, "key-string"));
    }
    // free text names a secret only by its leading keywords, which a line of it holds when a banner's
    // end was misread
    if place.text {
        return out;
    }
    let neighbor = is(k, "neighbor", 3);
    for i in k..n {
        let w = at(i);
        let first = i == k;
        let hit = match w {
            // enable and username lines name theirs above
            "password"
                if !enable && !username && !names_no_password(at(k), at(k + 1), at(i + 1)) =>
            {
                if at(i + 1) == "peer" {
                    // `ip msdp password peer <address> [<type>] <password>`
                    Some((value_after(lw, i + 3, &[]), n, "password"))
                } else {
                    let v = value_after(lw, i + 1, &["ascii", "hex", "encrypted"]);
                    // a line's or a BGP neighbour's password may hold spaces and ends its line
                    Some((v, if first || neighbor { n } else { v + 1 }, "password"))
                }
            }
            "secret" if !enable && !username => {
                let v = value_after(lw, i + 1, &["ascii", "hex", "encrypted"]);
                Some((v, v + 1, "secret"))
            }
            "area-password" | "domain-password" => {
                let v = value_after(lw, i + 1, &[]);
                Some((v, v + 1, "password"))
            }
            "wpa-psk" => Some((
                value_after(lw, i + 1, &["ascii", "hex", "encrypted"]),
                n,
                "wpa-psk",
            )),
            "md5" if i >= 2 && matches!(at(i - 2), "message-digest-key" | "authentication-key") => {
                let v = value_after(lw, i + 1, &["ascii", "hex", "encrypted"]);
                Some((v, v + 1, "md5 key"))
            }
            // OSPFv3 IPsec: `authentication ipsec spi <n> md5 <key>`
            "md5" | "sha1" if i >= 2 && at(i - 2) == "spi" => {
                let v = value_after(lw, i + 1, &[]);
                Some((v, v + 1, "md5 key"))
            }
            // HSRP's group number may be left out; VRRP and GLBP name theirs
            "authentication"
                if (i >= 2 && matches!(at(i - 2), "standby" | "vrrp" | "glbp")
                    || i >= 1 && at(i - 1) == "standby")
                    && at(i + 1) != "md5" =>
            {
                Some((
                    value_after(lw, i + 1, &["text"]),
                    n,
                    "standby authentication",
                ))
            }
            _ if abbreviates(w, "key-string", 5) => {
                let v = value_after(lw, i + 1, &[]);
                Some((v, if first { n } else { v + 1 }, "key-string"))
            }
            _ if abbreviates(w, "pre-shared-key", 5) => pre_shared(lw, i),
            _ if abbreviates(w, "authentication-key", 16) && at(k) != "ntp" => Some((
                value_after(lw, i + 1, &["ascii", "hex", "encrypted"]),
                n,
                "authentication-key",
            )),
            _ if abbreviates(w, "server-key", 8) => {
                let v = value_after(lw, i + 1, &["ascii", "hex", "encrypted"]);
                Some((v, v + 1, "server key"))
            }
            _ => None,
        };
        out.extend(hit);
    }
    out
}

/// Where a command's free text starts, which names no secret: an `alias`, `snmp-server location` or
/// `contact`, and the first `description` or `remark` word no value takes (`access-list 10 remark
/// ...`, a BGP neighbour's description); the command's length when it has none.
fn free_text(lw: &[String], hits: &[Hit]) -> usize {
    let k = usize::from(lw.first().is_some_and(|w| w == "no" || w == "default"));
    let is = |i: usize, keyword: &str, min: usize| {
        lw.get(i).is_some_and(|w| abbreviates(w, keyword, min))
    };
    if is(k, "alias", 3) || is(k, "description", 4) || is(k, "remark", 3) {
        return k;
    }
    if is(k, "snmp-server", 4) && (is(k + 1, "location", 3) || is(k + 1, "contact", 3)) {
        return k + 2;
    }
    (0..lw.len())
        .find(|&i| {
            matches!(lw[i].as_str(), "description" | "remark")
                && !hits.iter().any(|h| h.0 <= i && i < h.1)
        })
        .unwrap_or(lw.len())
}

pub fn sweep(text: &str) -> Swept {
    let mut found: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut removed: Vec<String> = Vec::new();
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut block = String::new();
    // the last line of a banner's text, and whether the lines read answer a `?`
    let mut banner_end: Option<usize> = None;
    let mut help = false;
    for (n, line) in lines.iter().enumerate() {
        let body = line.trim_end_matches(['\r', '\n']);
        let ending = &line[body.len()..];
        // a byte order mark opens the text; it is no part of the first command
        let bom = if n == 0 && body.starts_with('\u{feff}') {
            '\u{feff}'.len_utf8()
        } else {
            0
        };
        let in_banner = banner_end.is_some_and(|e| n <= e);
        let typed = if in_banner {
            None
        } else {
            prompt(&body[bom..])
        };
        let (lead, mode) = match typed {
            Some((len, mode)) => (&body[..bom + len], Some(mode)),
            None => (&body[..bom], None),
        };
        let command = &body[lead.len()..];
        let indent = &command[..command.len() - command.trim_start().len()];
        let words: Vec<&str> = command.split_whitespace().collect();
        let lw: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
        let indented = !indent.is_empty() || mode.is_some_and(crate::cli::is_submode);
        if typed.is_some() {
            help = false;
        }
        // a `?` answer's lines list keywords with their help, and name no value
        let answer = typed.is_none() && help;
        if !answer && !command.starts_with('!') && !words.is_empty() {
            match mode {
                None | Some("config") if !indented => block = command.trim().to_lowercase(),
                Some("privileged" | "exec") => block.clear(),
                _ => {}
            }
        }
        let place = Place {
            indented,
            block: &block,
            mode,
            text: in_banner,
        };
        let mut hits = Vec::new();
        if in_banner {
            hits = secrets(&lw, &place);
        } else if !answer {
            hits = secrets(&lw, &place);
            let mut free = free_text(&lw, &hits);
            if let Some((delimiter, at)) = banner_opens(&words, &lw) {
                free = free.min(at);
                // the text runs to the delimiter's next appearance. A banner that never closes, or
                // one typed at a prompt that the next prompt finds open, leaves the lines after it
                // to the rules.
                let from = words[at].as_ptr() as usize - command.as_ptr() as usize;
                let opened = &command[from + delimiter.len()..];
                if !opened.contains(&delimiter) {
                    banner_end = lines[n + 1..]
                        .iter()
                        .take_while(|l| typed.is_none() || prompt(l).is_none())
                        .position(|l| {
                            l.contains(&delimiter) && !l.starts_with("Enter TEXT message")
                        })
                        .map(|p| n + 1 + p);
                }
            }
            hits.retain(|h| h.0 < free);
        }
        if typed.is_some_and(|_| command.trim_end().ends_with('?')) {
            help = true;
        }
        for (i, w) in words.iter().enumerate() {
            if is_hash(w) && !hits.iter().any(|h| h.0 <= i && i < h.1) {
                hits.push((i, i + 1, "hash"));
            }
        }
        // `secret masked-secret` asks for the value at the prompt, and `<name>` is a placeholder; a
        // value removed before keeps no word after it on its line
        let mut hits: Vec<Hit> = hits
            .into_iter()
            .map(|(a, b, kind)| (a, b.min(words.len()), kind))
            .filter(|&(a, b, _)| {
                a < b
                    && if words[a] == REMOVED {
                        b - a > 1
                    } else {
                        !lw[a].starts_with("masked-")
                            && !(words[a].starts_with('<') && words[a].ends_with('>'))
                    }
            })
            .collect();
        hits.sort();
        let mut kinds: Vec<&'static str> = Vec::new();
        let mut new_words: Vec<String> = Vec::with_capacity(words.len());
        let mut i = 0;
        for (a, b, kind) in &hits {
            if !kinds.contains(kind) {
                kinds.push(kind);
            }
            if *a < i {
                // a second rule naming a value already taken, or more of it
                if *b > i {
                    removed.push(words[i..*b].join(" "));
                    i = *b;
                }
                continue;
            }
            new_words.extend(words[i..*a].iter().map(|w| w.to_string()));
            let value = if words[*a] == REMOVED {
                &words[a + 1..*b]
            } else {
                &words[*a..*b]
            };
            removed.push(value.join(" "));
            new_words.push(REMOVED.to_string());
            i = *b;
        }
        new_words.extend(words[i..].iter().map(|w| w.to_string()));
        // a URL's password, wherever the URL stands
        for w in new_words.iter_mut() {
            if let Some((a, b)) = url_password(w) {
                removed.push(w[a..b].to_string());
                w.replace_range(a..b, REMOVED);
                if !kinds.contains(&"url password") {
                    kinds.push("url password");
                }
            }
        }
        if kinds.is_empty() {
            out.push(line.to_string());
            continue;
        }
        for k in kinds {
            *found.entry(k).or_insert(0) += 1;
        }
        out.push(format!("{lead}{indent}{}{ending}", new_words.join(" ")));
    }
    let text: String = out.concat();
    // a value removed in one place must not survive in another; single digits and short words are
    // too common to judge by
    let left = removed
        .iter()
        .filter(|v| v.len() >= 4 && text.contains(v.as_str()))
        .count();
    Swept { text, found, left }
}

/// One file's result: its path and what was cleaned, or why it was left alone.
pub struct FileReport {
    pub path: PathBuf,
    pub outcome: Result<Swept, String>,
}

/// Cleans each text file under `root` (a file or a folder, read recursively). With `out`, cleaned copies
/// go there under the same relative names and the originals stay; without it, each file is cleaned in
/// place, written beside and renamed over. Binary files, other encodings and files over 32 MB are left.
/// A root that does not exist is an error, so a mistyped path never reads as a clean one.
pub fn sweep_path(root: &Path, out: Option<&Path>) -> Result<Vec<FileReport>, String> {
    if !root.exists() {
        return Err(format!("{}: no such file or folder", root.display()));
    }
    let mut files = Vec::new();
    collect(root, &mut files).map_err(|e| format!("{}: {e}", root.display()))?;
    files.sort();
    let base = if root.is_dir() {
        root.to_path_buf()
    } else {
        root.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    let mut reports = Vec::new();
    for path in files {
        let outcome = (|| -> Result<Swept, String> {
            let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
            if meta.len() > 32 * 1024 * 1024 {
                return Err("over 32 MB, left alone".into());
            }
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            if bytes.contains(&0) {
                return Err("binary, left alone".into());
            }
            let text =
                String::from_utf8(bytes).map_err(|_| "not UTF-8 text, left alone".to_string())?;
            let swept = sweep(&text);
            let dest = match out {
                Some(o) => o.join(path.strip_prefix(&base).unwrap_or(&path)),
                None => path.clone(),
            };
            if out.is_some() || swept.lines() > 0 {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let tmp = dest.with_extension("sweep-tmp");
                std::fs::write(&tmp, &swept.text).map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
            }
            Ok(swept)
        })();
        reports.push(FileReport { path, outcome });
    }
    Ok(reports)
}

fn collect(p: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if p.is_dir() {
        for e in std::fs::read_dir(p)? {
            collect(&e?.path(), files)?;
        }
    } else if p.is_file() {
        files.push(p.to_path_buf());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::TempDir;

    #[cfg(all(test, captures))]
    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/sweep-tests.rs"));

    // Every secret here is invented for the test.
    const CONFIG: &str = "\
hostname core9
service password-encryption
enable secret 9 $9$AAAAexampleHASHvalueXYZ
enable password level 15 7 0822455D0A16
username admin privilege 15 secret 9 $9$BBBBexampleHASHvalueQRS
username ops password 7 094F471A1A0A
username lab algorithm-type scrypt secret 9 $9$CCCCexampleHASH
snmp-server community TestCommunityRO RO
snmp-server host 10.0.0.5 version 2c TestCommunityRO
snmp-server host 10.0.0.6 vrf Mgmt-vrf informs version 2c TestInform
snmp-server user ops grp v3 auth sha AuthPhrase1 priv aes 128 PrivPhrase1
tacacs-server key 7 060506324F41
tacacs server ISE
 address ipv4 10.0.0.9
 key 7 1511021F0725
key chain OSPF
 key 1
  key-string 7 105A0C1D1D1
interface Vlan10
 standby 1 authentication md5 key-string 7 0007421507
 standby 2 authentication PlainHsrp
 ip ospf message-digest-key 1 md5 7 13061E010803
 ip ospf authentication-key 7 01100F175804
router bgp 65000
 neighbor 10.0.0.1 password 7 045802150C2E
ntp authentication-key 1 md5 NtpKeyValue 7
vtp password VtpPass
crypto isakmp key IkeKeyValue address 10.0.0.2
line vty 0 4
 password 7 0822455D0A16
 login local
password encryption aes
";

    #[test]
    fn every_secret_goes_and_the_rest_of_each_line_stays() {
        let s = sweep(CONFIG);
        for secret in [
            "$9$AAAA",
            "0822455D0A16",
            "$9$BBBB",
            "094F471A1A0A",
            "$9$CCCC",
            "TestCommunityRO",
            "TestInform",
            "AuthPhrase1",
            "PrivPhrase1",
            "060506324F41",
            "1511021F0725",
            "105A0C1D1D1",
            "0007421507",
            "PlainHsrp",
            "13061E010803",
            "01100F175804",
            "045802150C2E",
            "NtpKeyValue",
            "VtpPass",
            "IkeKeyValue",
        ] {
            assert!(!s.text.contains(secret), "{secret} survived:\n{}", s.text);
        }
        assert_eq!(s.left, 0);
        for kept in [
            "hostname core9\n",
            "service password-encryption\n",
            "enable secret 9 <removed>\n",
            "enable password level 15 7 <removed>\n",
            "username admin privilege 15 secret 9 <removed>\n",
            "snmp-server community <removed> RO\n",
            "snmp-server host 10.0.0.6 vrf Mgmt-vrf informs version 2c <removed>\n",
            "snmp-server user ops grp v3 auth sha <removed> priv aes 128 <removed>\n",
            " address ipv4 10.0.0.9\n",
            " key 7 <removed>\n",
            " key 1\n",
            "  key-string 7 <removed>\n",
            " ip ospf message-digest-key 1 md5 7 <removed>\n",
            "ntp authentication-key 1 md5 <removed> 7\n",
            "crypto isakmp key <removed> address 10.0.0.2\n",
            " login local\n",
            "password encryption aes\n",
        ] {
            assert!(s.text.contains(kept), "missing {kept:?}:\n{}", s.text);
        }
        assert_eq!(s.lines(), 21);
        assert!(s.summary().starts_with("21 line(s) cleaned"));
    }

    #[test]
    fn a_clean_configuration_is_left_byte_for_byte() {
        let text = "hostname sw1\r\ninterface Gi1/0/1\r\n description  two  spaces\r\n description password reset desk\r\n\
                    !   username <name> algorithm-type scrypt secret masked-secret   (prompts for the password)\r\n";
        let s = sweep(text);
        assert_eq!(s.text, text);
        assert_eq!(s.summary(), "nothing to clean");
    }

    /// Each line's swept text, and whether any of `values` survived.
    fn swept_lines(text: &str, values: &[&str]) -> Vec<String> {
        let s = sweep(text);
        for v in values {
            assert!(!s.text.contains(v), "{v} survived:\n{}", s.text);
        }
        s.text.lines().map(str::to_string).collect()
    }

    #[test]
    fn a_value_with_spaces_goes_to_the_end_of_its_line() {
        let out = swept_lines(
            "username bob password 0 Invented horse battery\nline vty 0 4\n password Invented staple horse\nenable password Invented words\nusername ann secret 0 <removed> left behind\n",
            &[
                "Invented",
                "horse",
                "battery",
                "staple",
                "words",
                "left behind",
            ],
        );
        assert_eq!(
            out,
            [
                "username bob password 0 <removed>",
                "line vty 0 4",
                " password <removed>",
                "enable password <removed>",
                "username ann secret 0 <removed>",
            ]
        );
    }

    #[test]
    fn a_session_log_s_commands_are_swept_after_their_prompts() {
        let log = "sw1#configure terminal\n\
                   sw1(config)#snmp-server community InventedComm1 RO\n\
                   sw1(config)#tacacs-server key InventedTac1\n\
                   sw1(config)#tacacs server ISE\n\
                   sw1(config-server-tacacs)#key 0 InventedTac2\n\
                   sw1(config-server-tacacs)#exit\n\
                   sw1(config)#crypto isakmp key InventedIke1 address 10.0.0.2\n\
                   sw1(config)#snmp-server host 10.0.0.5 version 2c InventedComm2\n\
                   sw1(config)#enable sec 0 InventedEnable1\n\
                   sw1(config)#snmp com InventedComm3 RO\n\
                   sw1(config)#usern bob privilege 15 sec 0 InventedUser1\n\
                   sw1(config)#key chain KC\n\
                   sw1(config-keychain)#key 1\n\
                   sw1(config-keychain-key)#key-str InventedKey1\n\
                   sw1(config-keychain-key)#k 7 InventedKey2\n\
                   sw1(config-keychain-key)#exit\n\
                   sw1(config-keychain)#exit\n\
                   sw1(config)#vtp ?\n  password   VTP password\n  mode       VTP mode\n\
                   sw1(config)#end\n";
        let out = swept_lines(
            log,
            &[
                "InventedComm1",
                "InventedTac1",
                "InventedTac2",
                "InventedIke1",
                "InventedComm2",
                "InventedEnable1",
                "InventedComm3",
                "InventedUser1",
                "InventedKey1",
                "InventedKey2",
            ],
        );
        for kept in [
            "sw1(config)#snmp-server community <removed> RO",
            "sw1(config-server-tacacs)#key 0 <removed>",
            "sw1(config)#crypto isakmp key <removed> address 10.0.0.2",
            "sw1(config)#enable sec 0 <removed>",
            "sw1(config-keychain)#key 1",
            "sw1(config-keychain-key)#k 7 <removed>",
            // a `?` answer lists keywords with their help, no value
            "  password   VTP password",
        ] {
            assert!(out.iter().any(|l| l == kept), "missing {kept:?}: {out:#?}");
        }
    }

    #[test]
    fn per_host_server_keys_and_pac_keys_go() {
        let out = swept_lines(
            "tacacs-server host 10.0.0.1 key InventedTac3\nradius-server host 10.0.0.2 auth-port 1812 acct-port 1813 key 7 InventedRad1\nradius server R1\n pac key 7 InventedPac1\n key 7 InventedRad2\n",
            &[
                "InventedTac3",
                "InventedRad1",
                "InventedPac1",
                "InventedRad2",
            ],
        );
        assert_eq!(
            out[1],
            "radius-server host 10.0.0.2 auth-port 1812 acct-port 1813 key 7 <removed>"
        );
        assert_eq!(out[3], " pac key 7 <removed>");
    }

    #[test]
    fn snmp_v3_sha_2_passwords_go_past_the_hash_length() {
        let out = swept_lines(
            "snmp-server user U G v3 auth sha-2 256 InventedAuth1 priv aes 256 InventedPriv1\n",
            &["InventedAuth1", "InventedPriv1"],
        );
        assert_eq!(
            out,
            ["snmp-server user U G v3 auth sha-2 256 <removed> priv aes 256 <removed>"]
        );
    }

    #[test]
    fn a_value_after_other_keywords_goes_and_the_keywords_stay() {
        let out = swept_lines(
            "ip msdp password peer 10.32.43.144 0 InventedMsdp1\ncrypto keyring KR\n pre-shared-key address 10.0.0.9 key InventedPsk1\n pre-shared-key hostname peer1 key InventedPsk2\ncrypto ikev2 keyring K2\n peer P\n  pre-shared-key local InventedPsk3\n  pre-shared-key remote InventedPsk4\n",
            &[
                "InventedMsdp1",
                "InventedPsk1",
                "InventedPsk2",
                "InventedPsk3",
                "InventedPsk4",
            ],
        );
        assert_eq!(out[0], "ip msdp password peer 10.32.43.144 0 <removed>");
        assert_eq!(out[2], " pre-shared-key address 10.0.0.9 key <removed>");
        assert_eq!(out[6], "  pre-shared-key local <removed>");
    }

    #[test]
    fn keywords_after_password_and_pre_shared_key_stay() {
        let text = "system disable password recovery switch all\ninterface GigabitEthernet1/0/1\n mka pre-shared-key key-chain kc1\ncts sxp connection peer 10.0.0.1 password default mode local listener\nip ssh server algorithm authentication publickey password keyboard\n";
        let s = sweep(text);
        assert_eq!(s.text, text);
        assert_eq!(s.summary(), "nothing to clean");
        // outside the commands that have them, those words are values
        assert_eq!(
            sweep("vtp password default\n").text,
            "vtp password <removed>\n"
        );
    }

    #[test]
    fn remarks_and_banner_text_are_left_alone() {
        let text = "access-list 10 remark password reset server\nip access-list standard X\n 10 remark password reset server\nbanner motd ^C\nEnter your password and secret token at the prompt\n^C\nbanner login #password here#\nsnmp-server location password reset closet\n";
        let s = sweep(text);
        assert_eq!(s.text, text);
        assert_eq!((s.summary().as_str(), s.left), ("nothing to clean", 0));
        // a banner that never closes keeps the lines after it swept, and one whose end is misread
        // keeps each command's own secret swept
        let s = sweep("banner motd ^C\nline vty 0 4\n password InventedVty1\n");
        assert!(!s.text.contains("InventedVty1"), "{}", s.text);
        let s = sweep(
            "banner motd #\nWelcome\nusername admin secret 0 InventedUser2\nsnmp-server community Invented#Comm RO\n",
        );
        assert!(
            !s.text.contains("InventedUser2") && !s.text.contains("Invented#Comm"),
            "{}",
            s.text
        );
    }

    #[test]
    fn is_is_glbp_groupless_standby_and_enable_algorithm_type_secrets_go() {
        swept_lines(
            "router isis\n area-password InventedArea1\n domain-password InventedDomain1 authenticate snp validate\ninterface Vlan10\n glbp 10 authentication text InventedGlbp1\n standby authentication InventedHsrp1\nenable algorithm-type scrypt secret InventedEnable2\n",
            &[
                "InventedArea1",
                "InventedDomain1",
                "InventedGlbp1",
                "InventedHsrp1",
                "InventedEnable2",
            ],
        );
        let s = sweep(" domain-password InventedDomain1 authenticate snp validate\n");
        assert_eq!(
            s.text,
            " domain-password <removed> authenticate snp validate\n"
        );
    }

    #[test]
    fn a_byte_order_mark_hides_no_secret() {
        let s = sweep("\u{feff}snmp-server community InventedBom1 RO\r\n");
        assert_eq!(s.text, "\u{feff}snmp-server community <removed> RO\r\n");
    }

    #[test]
    fn ospfv3_ipsec_keys_the_config_key_and_url_passwords_go() {
        let out = swept_lines(
            "router ospfv3 1\n area 1 authentication ipsec spi 678 md5 InventedSpi1\nkey config-key password-encrypt InventedMaster1\narchive\n path ftp://admin:InventedFtp1@10.0.0.3/$h\n",
            &["InventedSpi1", "InventedMaster1", "InventedFtp1"],
        );
        assert_eq!(out[4], " path ftp://admin:<removed>@10.0.0.3/$h");
    }

    #[test]
    fn no_forms_name_their_values_too() {
        swept_lines(
            "no snmp-server community InventedComm4 RO\nno snmp-server host 10.0.0.5 version 2c InventedComm5\nno crypto isakmp key InventedIke2 address 10.0.0.2\n",
            &["InventedComm4", "InventedComm5", "InventedIke2"],
        );
    }

    #[test]
    fn a_path_that_does_not_exist_is_refused() {
        let t = TempDir::new("sweep-missing");
        assert!(sweep_path(&t.0.join("conifg.txt"), None).is_err());
    }

    #[test]
    fn a_folder_is_cleaned_in_place_or_into_copies() {
        let t = TempDir::new("sweep");
        let inbox = t.0.join("inbox");
        std::fs::create_dir_all(inbox.join("mdf")).unwrap();
        std::fs::write(
            inbox.join("core.txt"),
            "enable secret 9 $9$DDDDexampleHASH\n",
        )
        .unwrap();
        std::fs::write(
            inbox.join("mdf").join("sw.cfg"),
            "snmp-server community Abc123xyz ro\n",
        )
        .unwrap();
        std::fs::write(inbox.join("photo.bin"), [0u8, 1, 2]).unwrap();
        // copies: the originals stay as they were
        let out = t.0.join("clean");
        let r = sweep_path(&inbox, Some(&out)).unwrap();
        assert_eq!(r.len(), 3);
        assert!(
            std::fs::read_to_string(inbox.join("core.txt"))
                .unwrap()
                .contains("$9$DDDD")
        );
        assert_eq!(
            std::fs::read_to_string(out.join("mdf").join("sw.cfg")).unwrap(),
            "snmp-server community <removed> ro\n"
        );
        assert!(
            r.iter()
                .any(|f| f.path.ends_with("photo.bin") && f.outcome.is_err())
        );
        // in place
        sweep_path(&inbox, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(inbox.join("core.txt")).unwrap(),
            "enable secret 9 <removed>\n"
        );
        assert!(!inbox.join("core.sweep-tmp").exists());
    }
}
