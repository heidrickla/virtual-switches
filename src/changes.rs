//! What a backup of a captured switch changes (`vswitch changes`, and `capture --folder` when a
//! backup tool's configuration differs from the kept one): the items it adds, removes and edits, the
//! lines its release refuses that the kept configuration's did not, and the lint and link findings
//! it adds or clears. The kept capture stays: its configuration goes with its show output.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::check::{self, Typed};
use crate::release::{self, Release};
use crate::topology::{self, Captured};
use crate::{grammars, lab, lint, platform};

/// A line and the lines under it: every one in order, and those directly under it, each with its
/// own.
#[derive(Debug, PartialEq)]
struct Item {
    header: String,
    lines: Vec<String>,
    under: Vec<Item>,
}

/// Lines under a header as items: each line with the lines indented further below it, up to the
/// next line no deeper.
fn nest(lines: &[String]) -> Vec<Item> {
    let depth = |l: &str| l.len() - l.trim_start().len();
    let mut out = vec![];
    let mut i = 0;
    while i < lines.len() {
        let d = depth(&lines[i]);
        let end = (i + 1..lines.len())
            .find(|&j| depth(&lines[j]) <= d)
            .unwrap_or(lines.len());
        out.push(Item {
            header: lines[i].clone(),
            lines: lines[i + 1..end].to_vec(),
            under: nest(&lines[i + 1..end]),
        });
        i = end;
    }
    out
}

/// Lines that change with every save, not with the configuration.
fn noise(line: &str) -> bool {
    let t = line.trim();
    t.is_empty()
        || t.starts_with('!')
        || t == "end"
        || t.starts_with("Building configuration")
        || t.starts_with("Current configuration :")
        || t.starts_with("ntp clock-period ")
}

/// A configuration's items in order; a banner is one item through its closing delimiter, its text
/// lines each one of its own.
fn items(text: &str) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    // the delimiter an open banner closes with
    let mut banner: Option<String> = None;
    let mut text_of = vec![];
    for raw in text.lines() {
        let line = raw.trim_end();
        if let Some(d) = &banner {
            if let Some(i) = out.last_mut() {
                i.lines.push(line.to_string());
            }
            if line.contains(d.as_str()) {
                banner = None;
            }
            continue;
        }
        if noise(line) {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            if let Some(i) = out.last_mut() {
                i.lines.push(line.to_string());
            }
            continue;
        }
        banner = crate::capture::banner_opens(line);
        if banner.is_some() {
            text_of.push(out.len());
        }
        out.push(Item {
            header: line.to_string(),
            lines: vec![],
            under: vec![],
        });
    }
    for (i, item) in out.iter_mut().enumerate() {
        item.under = if text_of.contains(&i) {
            nest_flat(&item.lines)
        } else {
            nest(&item.lines)
        };
    }
    out
}

/// Text lines as items of their own, none under another.
fn nest_flat(lines: &[String]) -> Vec<Item> {
    lines
        .iter()
        .map(|l| Item {
            header: l.clone(),
            lines: vec![],
            under: vec![],
        })
        .collect()
}

/// One item a backup changes: added or removed whole, or edited: its lines that differ, in order,
/// `-` kept and gone, `+` new, and ` ` a line both hold that differing lines sit under.
#[derive(Debug, PartialEq)]
pub enum Change {
    Added(String, Vec<String>),
    Removed(String, Vec<String>),
    Edited {
        header: String,
        lines: Vec<(char, String)>,
    },
}

/// The longest run of entries two lists hold in the same order, as pairs of their places.
fn common(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let mut t = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            t[i][j] = if a[i] == b[j] {
                t[i + 1][j + 1] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, vec![]);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            out.push((i, j));
            (i, j) = (i + 1, j + 1);
        } else if t[i + 1][j] >= t[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// The lines two items hold under them that differ, in order: the lines under each pair alike
/// compared in turn, so a line moved between sections, or among its own, shows.
fn edits(kept: &[Item], new: &[Item], out: &mut Vec<(char, String)>) {
    fn headers(v: &[Item]) -> Vec<&str> {
        v.iter().map(|i| i.header.as_str()).collect()
    }
    let (mut k, mut n) = (0, 0);
    for (a, b) in common(&headers(kept), &headers(new))
        .into_iter()
        .chain([(kept.len(), new.len())])
    {
        for i in &kept[k..a] {
            out.push(('-', i.header.clone()));
            out.extend(i.lines.iter().map(|l| ('-', l.clone())));
        }
        for i in &new[n..b] {
            out.push(('+', i.header.clone()));
            out.extend(i.lines.iter().map(|l| ('+', l.clone())));
        }
        if a < kept.len() && b < new.len() {
            let mut inner = vec![];
            edits(&kept[a].under, &new[b].under, &mut inner);
            if !inner.is_empty() {
                out.push((' ', kept[a].header.clone()));
                out.extend(inner);
            }
        }
        (k, n) = (a + 1, b + 1);
    }
}

/// The list a top-level line is an entry of where IOS reads its entries in order, first match
/// taken: a numbered access list (`access-list 101`), an AS-path or community list.
fn ordered_list(header: &str) -> Option<String> {
    let w: Vec<&str> = header.split_whitespace().collect();
    let n = match w[..] {
        ["access-list", ..] => 2,
        ["ip", "as-path", "access-list", ..] => 4,
        ["ip", "community-list", "standard" | "expanded", ..] => 4,
        ["ip", "community-list", ..] => 3,
        _ => return None,
    };
    (w.len() > n).then(|| w[..n].join(" "))
}

/// The backup's items against the kept configuration's, in the backup's order, then the items it
/// no longer has. Items pair by header, a header held twice pairing in order; the entries of a list
/// IOS reads in order pair in order, so an entry moved is one removed and one added.
pub fn diff(kept: &str, new: &str) -> Vec<Change> {
    let (k, n) = (items(kept), items(new));
    let mut partner: Vec<Option<usize>> = vec![None; n.len()];
    let mut paired = vec![false; k.len()];
    let lists: BTreeSet<String> = k
        .iter()
        .chain(&n)
        .filter_map(|i| ordered_list(&i.header))
        .collect();
    for list in &lists {
        let of = |v: &[Item]| -> Vec<usize> {
            (0..v.len())
                .filter(|&i| ordered_list(&v[i].header).as_ref() == Some(list))
                .collect()
        };
        let (ki, ni) = (of(&k), of(&n));
        let kh: Vec<&str> = ki.iter().map(|&i| k[i].header.as_str()).collect();
        let nh: Vec<&str> = ni.iter().map(|&i| n[i].header.as_str()).collect();
        for (a, b) in common(&kh, &nh) {
            partner[ni[b]] = Some(ki[a]);
            paired[ki[a]] = true;
        }
    }
    let mut by_header: BTreeMap<&str, VecDeque<usize>> = BTreeMap::new();
    for (i, item) in k.iter().enumerate() {
        if ordered_list(&item.header).is_none() {
            by_header.entry(&item.header).or_default().push_back(i);
        }
    }
    for (j, item) in n.iter().enumerate() {
        if ordered_list(&item.header).is_none()
            && let Some(i) = by_header
                .get_mut(item.header.as_str())
                .and_then(VecDeque::pop_front)
        {
            partner[j] = Some(i);
            paired[i] = true;
        }
    }
    let mut out = vec![];
    for (item, at) in n.iter().zip(partner) {
        match at {
            Some(i) => {
                let mut lines = vec![];
                edits(&k[i].under, &item.under, &mut lines);
                if !lines.is_empty() {
                    out.push(Change::Edited {
                        header: item.header.clone(),
                        lines,
                    });
                }
            }
            None => out.push(Change::Added(item.header.clone(), item.lines.clone())),
        }
    }
    for (item, _) in k.into_iter().zip(paired).filter(|(_, p)| !p) {
        out.push(Change::Removed(item.header, item.lines));
    }
    out
}

/// The changes as a unified diff prints them: `+` added, `-` removed, a space before an edited
/// item's header and the lines both hold that its changes sit under.
pub fn render(changes: &[Change]) -> Vec<String> {
    let mut out = vec![];
    for c in changes {
        match c {
            Change::Added(h, lines) | Change::Removed(h, lines) => {
                let sign = if matches!(c, Change::Added(..)) {
                    '+'
                } else {
                    '-'
                };
                out.push(format!("{sign}{h}"));
                out.extend(lines.iter().map(|l| format!("{sign}{l}")));
            }
            Change::Edited { header, lines } => {
                out.push(format!(" {header}"));
                out.extend(lines.iter().map(|(sign, l)| format!("{sign}{l}")));
            }
        }
    }
    out
}

/// What a backup changes on a captured switch.
pub struct Report {
    pub host: String,
    /// the kept configuration the backup is read against
    pub kept: PathBuf,
    pub release: Release,
    /// the release the backup's version line names, where it differs from the capture's
    pub upgraded: Option<Release>,
    pub changes: Vec<Change>,
    /// the backup's lines its release refuses that the kept configuration's typing took, each with
    /// the item it sits under ("" for a top-level line)
    pub refused: Vec<(String, Typed)>,
    /// lint findings and notes (`note: `) the backup adds, and those it clears
    pub lint: (Vec<String>, Vec<String>),
    /// cables the backup's descriptions make and those they no longer make
    pub cables: (Vec<String>, Vec<String>),
    /// link findings and notes on the switch's cables the backup adds, and those it clears
    pub links: (Vec<String>, Vec<String>),
}

/// The entries of `after` not in `before`, and of `before` not in `after`.
fn delta(before: &[String], after: &[String]) -> (Vec<String>, Vec<String>) {
    let b: BTreeSet<&String> = before.iter().collect();
    let a: BTreeSet<&String> = after.iter().collect();
    (
        after.iter().filter(|x| !b.contains(x)).cloned().collect(),
        before.iter().filter(|x| !a.contains(x)).cloned().collect(),
    )
}

fn host_of(end: &str) -> &str {
    end.split(':').next().unwrap_or(end)
}

/// The cables at `host`, as a lab takes them.
fn cables_at(linked: &topology::Linked, host: &str) -> Vec<lab::Cable> {
    linked
        .cables
        .iter()
        .filter(|(a, b, _)| host_of(a) == host || host_of(b) == host)
        .map(|(a, b, optic)| lab::Cable {
            a: a.clone(),
            b: b.clone(),
            optic: optic.clone(),
            speed: None,
            half: false,
        })
        .collect()
}

/// The lab's findings and notes over `cables`: each switch they name built from its capture, `host`
/// from `text` at `release`.
fn lab_findings(
    engines: &mut lab::Engines,
    caps: &[Captured],
    host: &str,
    text: &str,
    release: Release,
    cables: &[lab::Cable],
) -> Result<Vec<String>, String> {
    let names: BTreeSet<&str> = cables
        .iter()
        .flat_map(|c| [host_of(&c.a), host_of(&c.b)])
        .collect();
    let mut switches = vec![];
    for c in caps.iter().filter(|c| names.contains(c.host.as_str())) {
        let file = c.config.to_string_lossy().to_string();
        let (body, train) = if c.host == host {
            (text.to_string(), release.to_string())
        } else {
            let body = std::fs::read_to_string(&c.config).map_err(|e| format!("{file}: {e}"))?;
            (body, c.release.clone())
        };
        let spec = lab::SwitchSpec {
            name: c.host.clone(),
            files: vec![file.clone()],
            stack: c.stack.clone(),
            release: Some(train),
        };
        switches.push(lab::ready(&spec, vec![(file, body)], engines)?);
    }
    let mut built: Vec<lab::Built> = crate::live::par_map(&switches, lab::build);
    let r = lab::connect(&mut built, cables)?;
    let mut out = r.findings;
    out.extend(r.notes.into_iter().map(|n| format!("note: {n}")));
    Ok(out)
}

/// A backup's configuration `new` of the switch `host` read against its capture under `live`: the
/// one at the release the backup's version line names (any train of a classic `version 15.2`),
/// else the newest.
pub fn report(live: &Path, host: &str, new: &str) -> Result<Report, String> {
    let caps = topology::read(live)?;
    let release = |c: &Captured| Release::parse(&c.release);
    let named = release::version_line(new).and_then(Release::parse);
    let fits = |r: Release| {
        named.is_some_and(|v| {
            r == v || (v.major() < 16 && r.classic() && r.config_version() == v.to_string())
        })
    };
    let mine = || caps.iter().filter(|c| c.host.eq_ignore_ascii_case(host));
    let chosen = mine()
        .filter(|c| release(c).is_some_and(fits))
        .max_by_key(|c| release(c))
        .or_else(|| mine().max_by_key(|c| release(c)))
        .map(|c| c.config.clone())
        .ok_or(format!("{host}: no capture under {}", live.display()))?;
    report_on(caps, &chosen, new)
}

/// A backup's configuration `new` read against the capture whose configuration is `kept`, under
/// `live`.
pub fn report_at(live: &Path, kept: &Path, new: &str) -> Result<Report, String> {
    report_on(topology::read(live)?, kept, new)
}

/// `report` against the capture whose configuration is `kept`: the plant around it is every other
/// switch at its newest capture, and the switch's captures at other releases are left out.
fn report_on(caps: Vec<Captured>, kept: &Path, new: &str) -> Result<Report, String> {
    let host = caps
        .iter()
        .find(|c| c.config == kept)
        .map(|c| c.host.clone())
        .ok_or(format!("{}: no such capture", kept.display()))?;
    let (caps, _) = topology::current(
        caps.into_iter()
            .filter(|c| !c.host.eq_ignore_ascii_case(&host) || c.config == kept)
            .collect(),
    );
    let at = caps
        .iter()
        .position(|c| c.config == kept)
        .ok_or(format!("{}: no such capture", kept.display()))?;
    let c = &caps[at];
    let host = c.host.clone();
    let shown = c.config.display().to_string();
    let kept = std::fs::read_to_string(&c.config).map_err(|e| format!("{shown}: {e}"))?;
    let spec = c
        .stack
        .clone()
        .or_else(|| platform::infer_stack(&kept))
        .ok_or(format!("{host}: its hardware is unknown"))?;
    let stack = platform::parse_stack(&spec).map_err(|e| format!("{host}: {e}"))?;
    let book = grammars::book(stack.family());
    let from = grammars::resolve(&c.release, book)?;
    // a backup taken after an upgrade names its own release
    let to = release::version_line(new)
        .filter(|v| Some(*v) != release::version_line(&kept))
        .and_then(|v| grammars::resolve(v, book).ok())
        .unwrap_or(from);
    let mut engines = lab::Engines::new();
    let (at_from, at_to) = (
        lab::engine(&mut engines, book, from)?,
        lab::engine(&mut engines, book, to)?,
    );
    let inferred = c.stack.is_none();
    let typed_at = |engine: &crate::cli::Engine, text: &str| -> (Vec<String>, Vec<Typed>) {
        let (s, typed) = check::type_config(engine, stack.clone(), text);
        // hardware read from the configuration says nothing of the module fitted
        let l = if inferred {
            lint::lint_inferred(&s.device)
        } else {
            lint::lint(&s.device)
        };
        let mut found = l.findings;
        found.extend(l.notes.into_iter().map(|n| format!("note: {n}")));
        (found, typed)
    };
    // the kept configuration and the backup are typed side by side
    let ((lint_before, typed_before), (lint_after, typed_after)) = std::thread::scope(|s| {
        let before = s.spawn(|| typed_at(&at_from, &kept));
        let after = typed_at(&at_to, new);
        (
            before
                .join()
                .unwrap_or_else(|e| std::panic::resume_unwind(e)),
            after,
        )
    });
    let refused_before: BTreeSet<&str> = typed_before
        .iter()
        .filter(|t| t.error.is_some())
        .map(|t| t.line.trim())
        .collect();
    // a refused line is named by its item, not its number: the backup was read out of a file, a
    // session or a show tech, whose lines number apart
    let file: Vec<&str> = new.lines().collect();
    let item_of = |t: &Typed| -> String {
        let line = file.get(t.number.wrapping_sub(1)).copied().unwrap_or("");
        if !line.starts_with(char::is_whitespace) {
            return String::new();
        }
        file[..t.number - 1]
            .iter()
            .rev()
            .find(|l| !l.starts_with(char::is_whitespace) && !noise(l))
            .map_or(String::new(), |l| l.trim_end().to_string())
    };
    let refused = typed_after
        .into_iter()
        .filter(|t| t.error.is_some() && !refused_before.contains(t.line.trim()))
        .map(|t| (item_of(&t), t))
        .collect();
    // the switch's cables as its kept configuration and as the backup describe its ports
    let mut after = caps.clone();
    after[at] = topology::reconfigured(c, new);
    let (cables_before, cables_after) = (
        cables_at(&topology::link(&caps), &host),
        cables_at(&topology::link(&after), &host),
    );
    let named = |cs: &[lab::Cable]| -> Vec<String> {
        cs.iter().map(|c| format!("{} - {}", c.a, c.b)).collect()
    };
    let links_before = lab_findings(&mut engines, &caps, &host, &kept, from, &cables_before)?;
    let links_after = lab_findings(&mut engines, &after, &host, new, to, &cables_after)?;
    Ok(Report {
        kept: c.config.clone(),
        release: from,
        upgraded: (to != from).then_some(to),
        changes: diff(&kept, new),
        refused,
        lint: delta(&lint_before, &lint_after),
        cables: delta(&named(&cables_before), &named(&cables_after)),
        links: delta(&links_before, &links_after),
        host,
    })
}

impl Report {
    /// Whether the backup brings a refused line or a finding the kept configuration did not have.
    pub fn clean(&self) -> bool {
        let finding = |v: &[String]| v.iter().any(|f| !f.starts_with("note: "));
        self.refused.is_empty() && !finding(&self.lint.0) && !finding(&self.links.0)
    }

    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![format!(
            "{}: a backup read against {}, {}",
            self.host,
            self.kept.display(),
            self.release.named()
        )];
        if let Some(r) = self.upgraded {
            out.push(format!("the backup runs {}", r.named()));
        }
        out.extend(render(&self.changes));
        let count = |f: fn(&Change) -> bool| self.changes.iter().filter(|c| f(c)).count();
        out.push(format!(
            "{} item(s) changed: {} added, {} removed, {} edited",
            self.changes.len(),
            count(|c| matches!(c, Change::Added(..))),
            count(|c| matches!(c, Change::Removed(..))),
            count(|c| matches!(c, Change::Edited { .. }))
        ));
        let at = self.upgraded.unwrap_or(self.release).named();
        for (item, t) in &self.refused {
            let under = if item.is_empty() {
                String::new()
            } else {
                format!(", under {item}")
            };
            out.push(format!(
                "refused at {at}: {}{under} ({})",
                t.line.trim(),
                t.error.as_deref().unwrap_or("")
            ));
        }
        for (what, (new, gone)) in [
            ("lint", &self.lint),
            ("cable", &self.cables),
            ("link", &self.links),
        ] {
            out.extend(new.iter().map(|x| format!("{what} new: {x}")));
            out.extend(gone.iter().map(|x| format!("{what} cleared: {x}")));
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn a_backup_s_items_pair_by_header_and_its_save_noise_is_not_a_change() {
        let kept = "Building configuration...\n\nCurrent configuration : 100 bytes\n! Last configuration change at 1\nversion 17.15\nhostname a\n!\ninterface Gi1/0/1\n description x\n shutdown\n!\nip route 0.0.0.0 0.0.0.0 10.0.0.1\nntp clock-period 36\nbanner motd ^C\nhello\n^C\nend\n";
        let new = "Building configuration...\n\nCurrent configuration : 120 bytes\n! Last configuration change at 2\nversion 17.15\nhostname a\n!\ninterface Gi1/0/1\n description y\n shutdown\n!\ninterface Vlan9\n ip address 10.9.0.1 255.255.255.0\nntp clock-period 99\nbanner motd ^C\nhello\n^C\nend\n";
        let d = diff(kept, new);
        assert_eq!(
            d,
            vec![
                Change::Edited {
                    header: "interface Gi1/0/1".into(),
                    lines: vec![
                        ('-', " description x".into()),
                        ('+', " description y".into())
                    ],
                },
                Change::Added(
                    "interface Vlan9".into(),
                    vec![" ip address 10.9.0.1 255.255.255.0".into()]
                ),
                Change::Removed("ip route 0.0.0.0 0.0.0.0 10.0.0.1".into(), vec![]),
            ]
        );
        assert_eq!(
            render(&d),
            [
                " interface Gi1/0/1",
                "- description x",
                "+ description y",
                "+interface Vlan9",
                "+ ip address 10.9.0.1 255.255.255.0",
                "-ip route 0.0.0.0 0.0.0.0 10.0.0.1",
            ]
        );
        assert!(diff(kept, kept).is_empty());
    }

    #[test]
    fn a_banner_is_one_item_and_a_line_held_twice_counts_twice() {
        let kept = "banner motd ^C\nhostname b\n^C\nbanner login ^Cone^C\nhostname a\n";
        let i = items(kept);
        assert_eq!(i.len(), 3);
        assert_eq!(i[0].lines, ["hostname b", "^C"]);
        assert_eq!(i[1].header, "banner login ^Cone^C");
        assert_eq!(i[2].header, "hostname a");
        let once = "interface Gi1/0/1\n x\n";
        let twice = "interface Gi1/0/1\n x\n x\n";
        assert_eq!(render(&diff(once, twice)), [" interface Gi1/0/1", "+ x"]);
        assert_eq!(render(&diff(twice, once)), [" interface Gi1/0/1", "- x"]);
    }

    #[test]
    fn a_line_moved_between_sections_or_reordered_is_a_change() {
        // c2960 15.2(6)E capture 1's policy map with the two classes' DSCP values swapped
        let kept = "policy-map qos_policy\n class qos_urgent\n  set dscp ef\n class qos_high\n  set dscp cs4\n";
        let new = "policy-map qos_policy\n class qos_urgent\n  set dscp cs4\n class qos_high\n  set dscp ef\n";
        assert_eq!(
            render(&diff(kept, new)),
            [
                " policy-map qos_policy",
                "  class qos_urgent",
                "-  set dscp ef",
                "+  set dscp cs4",
                "  class qos_high",
                "-  set dscp cs4",
                "+  set dscp ef",
            ]
        );
        // a named list without sequence numbers, its entries swapped
        let kept = "ip access-list extended A\n deny   tcp any any eq 22\n permit ip any any\n";
        let new = "ip access-list extended A\n permit ip any any\n deny   tcp any any eq 22\n";
        assert_eq!(
            render(&diff(kept, new)),
            [
                " ip access-list extended A",
                "- deny   tcp any any eq 22",
                "+ deny   tcp any any eq 22",
            ]
        );
    }

    #[test]
    fn a_numbered_access_list_s_entries_keep_their_order() {
        let kept = "access-list 101 deny   tcp any any eq 22\naccess-list 101 permit ip any any\nntp server 10.0.0.1\nntp server 10.0.0.2\n";
        // SSH no longer denied: the permit comes first
        let new = "access-list 101 permit ip any any\naccess-list 101 deny   tcp any any eq 22\nntp server 10.0.0.2\nntp server 10.0.0.1\n";
        assert_eq!(
            render(&diff(kept, new)),
            [
                "+access-list 101 deny   tcp any any eq 22",
                "-access-list 101 deny   tcp any any eq 22",
            ]
        );
    }

    #[cfg(all(test, captures))]
    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/changes-tests.rs"));
}
