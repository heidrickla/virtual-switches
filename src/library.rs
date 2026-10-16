//! The switches loaded before, by name: each one's hardware, the file its configuration came from, and
//! a copy of that configuration, kept in the user's data directory so the list outlives the files.

use crate::release::Release;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Saved {
    /// the switch's hostname
    pub name: String,
    /// its members as `--stack` takes them
    pub stack: String,
    /// the IOS XE release it runs; 17.15 for a switch kept before releases were recorded
    #[serde(default)]
    pub release: Option<String>,
    /// the file it was loaded from
    pub source: String,
    /// seconds since the epoch
    pub loaded_at: u64,
    #[serde(default)]
    pub opened_at: Option<u64>,
    /// when `write memory` in its console last saved it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<u64>,
    /// secret lines the sweep removed from the kept copy
    #[serde(default)]
    pub swept: usize,
    /// its base MAC address, made once from its name and hardware and kept through renames
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
}

/// The base MAC address a switch takes when it joins the list: from its name and hardware.
pub fn new_mac(name: &str, stack: &str) -> String {
    crate::stp::Mac::derive(&format!("{name}\n{stack}")).to_string()
}

impl Saved {
    /// The release it runs: the one kept, else 17.15, which every switch ran before releases were kept.
    pub fn release(&self) -> Release {
        self.release
            .as_deref()
            .and_then(Release::parse)
            .unwrap_or(crate::release::DEFAULT)
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Index {
    switches: Vec<Saved>,
}

/// A switch taken out of the list with its kept configuration, so it can be put back.
#[derive(Clone, Debug, PartialEq)]
pub struct Removed {
    pub saved: Saved,
    /// none when the kept copy could not be read, which is then left where it was
    pub config: Option<String>,
}

/// What the list remembers between runs besides its switches, in `prefs.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    /// the folder a file was last loaded from or exported to
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<PathBuf>,
    /// the list ordered by when each switch was last opened, newest first, rather than by name
    #[serde(default)]
    pub by_opened: bool,
}

pub struct Library {
    pub dir: PathBuf,
    pub switches: Vec<Saved>,
    /// the list as this process last read or wrote it: what differs from it on disk another vswitch
    /// changed meanwhile
    base: Vec<Saved>,
}

/// A file read for loading: a design names its own hardware; a configuration's is inferred from its
/// interfaces and confirmed before it is kept.
#[derive(Debug, PartialEq)]
pub enum Import {
    Ready {
        name: String,
        stack: String,
        /// the release the design names
        release: Option<String>,
        config: String,
    },
    NeedsStack {
        name: String,
        inferred: Option<String>,
        /// the configuration's `version` line
        release: Option<String>,
        config: String,
    },
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `c9400 17.5 capture 1` -> `c9400 17.5 capture 1.cfg`; anything a file name cannot hold becomes `_`.
pub fn file_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.cfg")
}

/// Whether two names are one switch's: the same name in any case, or names kept in the same file
/// (compared without case, as Windows compares file names).
pub fn same_switch(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || file_name(a).eq_ignore_ascii_case(&file_name(b))
}

/// Why a name cannot be a switch's hostname; none when it can.
pub fn hostname_problem(name: &str) -> Option<&'static str> {
    (name.is_empty() || name.contains(char::is_whitespace)).then_some("A hostname is one word.")
}

/// The configuration with its top-level `hostname` line naming `name`, or one put first when it has
/// none, so a renamed or copied switch boots under its own name and `write memory` keeps it.
pub fn with_hostname(config: &str, name: &str) -> String {
    let mut out = String::with_capacity(config.len() + name.len() + 10);
    let mut done = false;
    for line in config.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        if !done && body.starts_with("hostname ") {
            out.push_str("hostname ");
            out.push_str(name);
            out.push_str(&line[body.len()..]);
            done = true;
        } else {
            out.push_str(line);
        }
    }
    if done {
        out
    } else {
        format!("hostname {name}\n{config}")
    }
}

/// Written beside and renamed over, so a crash never leaves half a file. The new text reaches the disk
/// before the rename, since a file system may keep the rename and lose the data on a power cut.
fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let written = std::fs::File::create(&tmp).and_then(|mut f| {
        f.write_all(text.as_bytes())?;
        f.sync_all()
    });
    written.map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// A file's text: UTF-8, or UTF-16 with the byte order mark Windows PowerShell 5.1 redirection writes.
fn decode(bytes: &[u8]) -> Option<String> {
    let wide = |unit: fn([u8; 2]) -> u16| {
        let (pairs, odd) = bytes[2..].as_chunks::<2>();
        if !odd.is_empty() {
            return None;
        }
        let units: Vec<u16> = pairs.iter().map(|p| unit(*p)).collect();
        String::from_utf16(&units).ok()
    };
    match bytes {
        [0xff, 0xfe, ..] => wide(u16::from_le_bytes),
        [0xfe, 0xff, ..] => wide(u16::from_be_bytes),
        _ => String::from_utf8(bytes.to_vec()).ok(),
    }
}

/// Reads a design (`.toml`) or a configuration (anything else: a generated `.cfg`, a running
/// configuration capture). The name is the configuration's hostname, else the file's stem with its
/// spaces made dashes and written into the configuration, so the switch boots under the name the list
/// shows. A name that is not one word is refused.
pub fn import(path: &Path) -> Result<Import, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text =
        decode(&bytes).ok_or_else(|| format!("{}: not UTF-8 or UTF-16 text", path.display()))?;
    let text = text.trim_start_matches('\u{feff}');
    let one_word = |name: &str| match hostname_problem(name) {
        Some(why) => Err(format!("{}: hostname {name:?}: {why}", path.display())),
        None => Ok(()),
    };
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
    {
        let spec: crate::generate::Spec =
            toml::from_str(text).map_err(|e| format!("{}: {e}", path.display()))?;
        one_word(&spec.hostname)?;
        return Ok(Import::Ready {
            name: spec.hostname.clone(),
            stack: spec.stack.join(","),
            release: spec.release.clone(),
            config: crate::generate::render(&spec).join("\n") + "\n",
        });
    }
    // a switch's configuration names its hostname, version, an interface or a VLAN at a line's start;
    // notes, a README or an empty file are no switch
    let configured = text.lines().any(|l| {
        ["hostname ", "version ", "interface ", "vlan "]
            .iter()
            .any(|k| l.starts_with(k))
    });
    if !configured {
        return Err(format!(
            "{}: no configuration here (no hostname, version, interface or vlan line)",
            path.display()
        ));
    }
    let hostname = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("hostname "))
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty());
    let (name, config) = match hostname {
        Some(h) => {
            one_word(&h)?;
            (h, text.to_string())
        }
        None => {
            let stem = path
                .file_stem()
                .map(|s| {
                    s.to_string_lossy()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join("-")
                })
                .unwrap_or_default();
            one_word(&stem)?;
            let config = with_hostname(text, &stem);
            (stem, config)
        }
    };
    Ok(Import::NeedsStack {
        name,
        inferred: crate::platform::infer_stack(text),
        release: crate::release::version_line(text).map(str::to_string),
        config,
    })
}

/// A switch list file's switches: none when there is no file yet. A file that is not a list is an
/// error and is left as it is, since a list rewritten from nothing would lose every switch it names.
fn read_list(index: &Path) -> Result<Vec<Saved>, String> {
    match std::fs::read_to_string(index) {
        Ok(t) => serde_json::from_str::<Index>(t.trim_start_matches('\u{feff}'))
            .map(|i| i.switches)
            .map_err(|e| {
                format!(
                    "{} is not a switch list vswitch can read ({e}). It was left as it is: mend it \
                     or move it aside, and vswitch starts again with it or with an empty list.",
                    index.display()
                )
            }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(format!("{}: {e}", index.display())),
    }
}

/// The list to write: this process's switches (`ours`), with what another process changed on disk
/// since this one last read or wrote the list (`base`). A switch another process added is taken from
/// the disk, and each field of one it changed that this process left as it was; one it removed or
/// renamed goes, unless this process changed it and its kept configuration is still there (`kept`).
fn merge(
    base: &[Saved],
    ours: &[Saved],
    disk: &[Saved],
    kept: impl Fn(&str) -> bool,
) -> Vec<Saved> {
    fn either<T: PartialEq + Clone>(base: &T, ours: &T, disk: &T) -> T {
        if ours == base { disk } else { ours }.clone()
    }
    let find = |list: &[Saved], name: &str| list.iter().position(|s| s.name == name);
    let mut out: Vec<Saved> = Vec::with_capacity(ours.len());
    for o in ours {
        match (find(base, &o.name), find(disk, &o.name)) {
            (Some(b), None) if base[b] == *o || !kept(&o.name) => {}
            (Some(b), Some(d)) => {
                let (b, d) = (&base[b], &disk[d]);
                out.push(Saved {
                    name: o.name.clone(),
                    stack: either(&b.stack, &o.stack, &d.stack),
                    release: either(&b.release, &o.release, &d.release),
                    source: either(&b.source, &o.source, &d.source),
                    loaded_at: either(&b.loaded_at, &o.loaded_at, &d.loaded_at),
                    opened_at: either(&b.opened_at, &o.opened_at, &d.opened_at),
                    saved_at: either(&b.saved_at, &o.saved_at, &d.saved_at),
                    swept: either(&b.swept, &o.swept, &d.swept),
                    mac: either(&b.mac, &o.mac, &d.mac),
                });
            }
            _ => out.push(o.clone()),
        }
    }
    for d in disk {
        if find(base, &d.name).is_none() && !out.iter().any(|s| same_switch(&s.name, &d.name)) {
            out.push(d.clone());
        }
    }
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

impl Library {
    /// `VSWITCH_HOME`, else `%USERPROFILE%\.vswitch` on Windows and the XDG data directory elsewhere.
    /// AppData is redirected per MSIX package: a file written there from Claude or Codex is
    /// invisible to every other process, and a delete removes only the package's copy.
    pub fn default_dir() -> Option<PathBuf> {
        let env = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        if let Some(d) = env("VSWITCH_HOME") {
            return Some(d);
        }
        if cfg!(windows) {
            return env("USERPROFILE").map(|d| d.join(".vswitch"));
        }
        env("XDG_DATA_HOME")
            .or_else(|| env("HOME").map(|h| h.join(".local").join("share")))
            .map(|d| d.join("vswitch"))
    }

    pub fn open(dir: PathBuf) -> Result<Self, String> {
        let switches = read_list(&dir.join("switches.json"))?;
        let mut lib = Library {
            dir,
            base: switches.clone(),
            switches,
        };
        // a switch kept before MACs were kept gets one now, written with the list; the same name and
        // hardware give the same address if the write fails
        let mut missing = false;
        for s in lib.switches.iter_mut().filter(|s| s.mac.is_none()) {
            s.mac = Some(new_mac(&s.name, &s.stack));
            missing = true;
        }
        if missing {
            let _ = lib.save();
        }
        Ok(lib)
    }

    /// Writes the list: this process's switches with what another vswitch (a second selector, `vswitch
    /// load`) changed since this one last read or wrote it, so neither loses the other's changes. The
    /// list in memory is then the one written; when the write fails it is left as it was.
    fn save(&mut self) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let index = self.dir.join("switches.json");
        let disk = read_list(&index)?;
        let merged = merge(&self.base, &self.switches, &disk, |name| {
            self.config_path(name).exists()
        });
        let text = serde_json::to_string_pretty(&Index {
            switches: merged.clone(),
        })
        .map_err(|e| e.to_string())?;
        write_whole(&index, &text)?;
        self.base = merged.clone();
        self.switches = merged;
        Ok(())
    }

    fn sort(&mut self) {
        self.switches.sort_by_key(|s| s.name.to_lowercase());
    }

    /// Where a switch's configuration is kept.
    pub fn config_path(&self, name: &str) -> PathBuf {
        self.dir.join("configs").join(file_name(name))
    }

    pub fn config(&self, name: &str) -> Result<String, String> {
        let p = self.config_path(name);
        std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))
    }

    /// Writes a switch's kept configuration swept of secrets; returns the lines the sweep removed.
    fn keep_config(&self, name: &str, config: &str) -> Result<usize, String> {
        let swept = crate::sweep::sweep(config);
        let configs = self.dir.join("configs");
        std::fs::create_dir_all(&configs).map_err(|e| format!("{}: {e}", configs.display()))?;
        write_whole(&configs.join(file_name(name)), &swept.text)?;
        Ok(swept.lines())
    }

    /// Puts a kept configuration file back as it was before a step that failed: its old bytes, or no
    /// file where there was none.
    fn put_back(path: &Path, old: &Option<Vec<u8>>) {
        match old {
            Some(bytes) => {
                let _ = std::fs::write(path, bytes);
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// The listed switch named exactly `name`.
    pub fn position(&self, name: &str) -> Option<usize> {
        self.switches.iter().position(|s| s.name == name)
    }

    /// The switch other than `except` that `name` collides with: the same name in any case, or a name
    /// kept in the same file (compared without case, as Windows compares file names).
    pub fn taken(&self, name: &str, except: Option<usize>) -> Option<usize> {
        (0..self.switches.len())
            .find(|&j| Some(j) != except && same_switch(&self.switches[j].name, name))
    }

    /// A name a switch can take: one word, and no other switch's.
    pub fn check_name(&self, name: &str, except: Option<usize>) -> Result<(), String> {
        if let Some(why) = hostname_problem(name) {
            return Err(why.into());
        }
        match self.taken(name, except) {
            Some(j) => Err(format!(
                "A switch named {} is already in the list.",
                self.switches[j].name
            )),
            None => Ok(()),
        }
    }

    /// Keeps a switch, its configuration swept of secrets first, replacing one of the same name; returns
    /// its place in the list, which is sorted by name. A list that cannot be written leaves the list
    /// and the kept configurations as they were.
    pub fn add(
        &mut self,
        name: &str,
        stack: &str,
        release: Release,
        source: &str,
        config: &str,
    ) -> Result<usize, String> {
        let path = self.config_path(name);
        let old = std::fs::read(&path).ok();
        let swept = self.keep_config(name, config)?;
        let was = self.switches.clone();
        self.switches.retain(|s| !same_switch(&s.name, name));
        self.switches.push(Saved {
            name: name.to_string(),
            stack: stack.to_string(),
            release: Some(release.to_string()),
            source: source.to_string(),
            loaded_at: now(),
            opened_at: None,
            saved_at: None,
            swept,
            mac: Some(new_mac(name, stack)),
        });
        self.sort();
        if let Err(e) = self.save() {
            self.switches = was;
            Self::put_back(&path, &old);
            return Err(e);
        }
        Ok(self.position(name).unwrap_or(0))
    }

    /// Takes a switch out of the list with its kept configuration; `restore` puts it back.
    pub fn remove(&mut self, i: usize) -> Result<Removed, String> {
        if i >= self.switches.len() {
            return Err("no such switch".into());
        }
        let config = self.config(&self.switches[i].name).ok();
        let saved = self.switches.remove(i);
        // the list is written before the kept copy goes, so a failed write loses nothing
        if let Err(e) = self.save() {
            self.switches.insert(i, saved);
            return Err(e);
        }
        // a kept copy that could not be read stays where it is, for `restore` to find
        if config.is_some() {
            let _ = std::fs::remove_file(self.config_path(&saved.name));
        }
        Ok(Removed { saved, config })
    }

    /// Puts a removed switch back under its own name with its kept configuration, swept as a load is;
    /// refused when another switch has taken the name since. Returns its place in the list.
    pub fn restore(&mut self, r: Removed) -> Result<usize, String> {
        let Removed { mut saved, config } = r;
        if let Some(j) = self.taken(&saved.name, None) {
            return Err(format!(
                "A switch named {} is in the list again.",
                self.switches[j].name
            ));
        }
        let wrote = match config {
            Some(config) => {
                saved.swept += self.keep_config(&saved.name, &config)?;
                true
            }
            None => false,
        };
        let name = saved.name.clone();
        self.switches.push(saved);
        self.sort();
        if let Err(e) = self.save() {
            self.switches.retain(|s| s.name != name);
            if wrote {
                let _ = std::fs::remove_file(self.config_path(&name));
            }
            return Err(e);
        }
        Ok(self.position(&name).unwrap_or(0))
    }

    /// Renames a switch: its kept configuration's hostname line and file follow the new name, its
    /// source and times stay. Returns its place in the list.
    pub fn rename(&mut self, i: usize, new: &str) -> Result<usize, String> {
        let old = self.switches.get(i).ok_or("no such switch")?.name.clone();
        self.check_name(new, Some(i))?;
        let kept = self.config(&old)?;
        let text = with_hostname(&kept, new);
        let (from, to) = (self.config_path(&old), self.config_path(new));
        // moved before it is written: a name differing only in case is the same file on Windows,
        // which writing the new name and deleting the old one would lose
        if from != to {
            std::fs::rename(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        }
        // a later step that fails puts the kept configuration back where and as it was
        let put_back = || {
            let _ = write_whole(&to, &kept);
            if from != to {
                let _ = std::fs::rename(&to, &from);
            }
        };
        let swept = match self.keep_config(new, &text) {
            Ok(n) => n,
            Err(e) => {
                put_back();
                return Err(e);
            }
        };
        let was = self.switches[i].clone();
        let s = &mut self.switches[i];
        s.name = new.to_string();
        s.swept += swept;
        self.sort();
        if let Err(e) = self.save() {
            if let Some(j) = self.position(new) {
                self.switches[j] = was;
            }
            self.sort();
            put_back();
            return Err(e);
        }
        Ok(self.position(new).unwrap_or(0))
    }

    /// A copy of a switch under another name: its hardware and release, and its kept configuration with
    /// the hostname line naming the copy. Returns the copy's place in the list.
    pub fn duplicate(&mut self, i: usize, new: &str) -> Result<usize, String> {
        let s = self.switches.get(i).ok_or("no such switch")?.clone();
        self.check_name(new, None)?;
        let swept = self.keep_config(new, &with_hostname(&self.config(&s.name)?, new))?;
        self.switches.push(Saved {
            name: new.to_string(),
            source: format!("copy of {}", s.name),
            loaded_at: now(),
            opened_at: None,
            saved_at: None,
            swept: s.swept + swept,
            mac: Some(new_mac(new, &s.stack)),
            ..s
        });
        self.sort();
        if let Err(e) = self.save() {
            // the list on disk never named the copy: neither does the one in memory, nor its file
            self.switches.retain(|x| x.name != new);
            let _ = std::fs::remove_file(self.config_path(new));
            return Err(e);
        }
        Ok(self.position(new).unwrap_or(0))
    }

    /// What the list remembers besides its switches; the defaults when `prefs.json` is missing or
    /// unreadable, since it only saves steps.
    pub fn prefs(&self) -> Prefs {
        std::fs::read_to_string(self.dir.join("prefs.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn set_prefs(&self, prefs: &Prefs) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let text = serde_json::to_string_pretty(prefs).map_err(|e| e.to_string())?;
        write_whole(&self.dir.join("prefs.json"), &text)
    }

    /// A listed switch's hardware and release, its configuration kept.
    pub fn set_hardware(&mut self, i: usize, stack: &str, release: Release) -> Result<(), String> {
        let s = self.switches.get_mut(i).ok_or("no such switch")?;
        let was = s.clone();
        s.stack = stack.to_string();
        s.release = Some(release.to_string());
        let saved = self.save();
        if saved.is_err() {
            self.switches[i] = was;
        }
        saved
    }

    /// Keeps the configuration `write memory` saved in the console of the switch named `name`, swept of
    /// secrets. A changed hostname renames the switch, unless another switch has that name. Returns the
    /// switch's name after the save.
    pub fn save_running(&mut self, name: &str, config: &str) -> Result<String, String> {
        let i = self
            .switches
            .iter()
            .position(|s| s.name == name)
            .ok_or(format!("{name} is no longer in the switch list"))?;
        let hostname = config
            .lines()
            .find_map(|l| l.strip_prefix("hostname "))
            .map(str::trim)
            .filter(|h| !h.is_empty() && !h.eq_ignore_ascii_case(name));
        if let Some(h) = hostname
            && self.taken(h, Some(i)).is_some()
        {
            return Err(format!("a switch named {h} is already in the list"));
        }
        let new_name = hostname.unwrap_or(name).to_string();
        let (from, to) = (self.config_path(name), self.config_path(&new_name));
        let old = std::fs::read(&from).ok();
        // moved before it is written, as `rename` does: a name kept in the same file under another
        // case is one file on Windows, which writing the new name and deleting the old one would lose
        let moved = from != to && old.is_some();
        if moved {
            std::fs::rename(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        }
        // a later step that fails puts the kept configuration back where and as it was
        let put_back = || {
            Self::put_back(&to, &old);
            if moved {
                let _ = std::fs::rename(&to, &from);
            }
        };
        let swept = match self.keep_config(&new_name, config) {
            Ok(n) => n,
            Err(e) => {
                put_back();
                return Err(e);
            }
        };
        let was = self.switches[i].clone();
        let s = &mut self.switches[i];
        s.name = new_name.clone();
        s.saved_at = Some(now());
        s.swept = swept;
        self.sort();
        if let Err(e) = self.save() {
            if let Some(j) = self.position(&new_name) {
                self.switches[j] = was;
            }
            self.sort();
            put_back();
            return Err(e);
        }
        Ok(new_name)
    }

    pub fn opened(&mut self, i: usize) -> Result<(), String> {
        let was = self.switches.get(i).cloned();
        if let Some(s) = self.switches.get_mut(i) {
            s.opened_at = Some(now());
        }
        let saved = self.save();
        if let (Err(_), Some(w)) = (&saved, was) {
            self.switches[i] = w;
        }
        saved
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A fresh directory under the system temp directory, removed when dropped.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let d =
                std::env::temp_dir().join(format!("vswitch-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&d).unwrap();
            TempDir(d)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_list_lives_in_the_profile_outside_appdata() {
        if std::env::var_os("VSWITCH_HOME").is_some() {
            return;
        }
        let profile = PathBuf::from(std::env::var_os("USERPROFILE").unwrap());
        assert_eq!(Library::default_dir(), Some(profile.join(".vswitch")));
    }

    #[test]
    fn switches_are_kept_by_name_and_outlive_the_process() {
        let t = TempDir::new("lib");
        let mut lib = Library::open(t.0.clone()).unwrap();
        assert!(lib.switches.is_empty());
        lib.add(
            "core-b",
            "1=C9400-LC-24XS",
            crate::release::DEFAULT,
            "a.txt",
            "hostname core-b\nenable secret 9 $9$EEEEexampleHASH\n",
        )
        .unwrap();
        assert_eq!(
            lib.config("core-b").unwrap(),
            "hostname core-b\nenable secret 9 <removed>\n"
        );
        assert_eq!(lib.switches[0].swept, 1, "the kept copy holds no secret");
        let i = lib
            .add(
                "core-a",
                "2=C9400-LC-24XS",
                crate::release::DEFAULT,
                "b.txt",
                "hostname core-a\n",
            )
            .unwrap();
        assert_eq!(i, 0, "sorted by name");
        // loading the same switch again replaces it
        lib.add(
            "CORE-A",
            "2=C9400-LC-24XS",
            crate::release::DEFAULT,
            "c.txt",
            "hostname CORE-A\n",
        )
        .unwrap();
        let again = Library::open(t.0.clone()).unwrap();
        let names: Vec<&str> = again.switches.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["CORE-A", "core-b"]);
        assert_eq!(again.config("CORE-A").unwrap(), "hostname CORE-A\n");
        assert_eq!(again.switches[0].source, "c.txt");
        let mut again = again;
        assert_eq!(again.remove(0).unwrap().saved.name, "CORE-A");
        assert!(again.config("CORE-A").is_err());
        assert_eq!(Library::open(t.0.clone()).unwrap().switches.len(), 1);
    }

    fn two(tag: &str) -> (TempDir, Library) {
        let t = TempDir::new(tag);
        let mut lib = Library::open(t.0.clone()).unwrap();
        for (n, extra) in [("idf1", "interface Vlan1\n hostname x\n"), ("idf2", "")] {
            let config = format!("!\nversion 17.15\nhostname {n}\n{extra}!\nend\n");
            lib.add(n, "C9300X-24Y", crate::release::DEFAULT, "a.txt", &config)
                .unwrap();
        }
        (t, lib)
    }

    #[test]
    fn a_renamed_switch_s_file_and_hostname_follow_it_and_its_times_stay() {
        let (t, mut lib) = two("rename");
        lib.switches[0].opened_at = Some(5);
        lib.switches[0].saved_at = Some(6);
        assert_eq!(lib.rename(0, "mdf1").unwrap(), 1, "sorted after idf2");
        let s = &lib.switches[1];
        assert_eq!(
            (s.name.as_str(), s.opened_at, s.saved_at, s.source.as_str()),
            ("mdf1", Some(5), Some(6), "a.txt")
        );
        // the top-level hostname line only; an indented one is left
        assert_eq!(
            lib.config("mdf1").unwrap(),
            "!\nversion 17.15\nhostname mdf1\ninterface Vlan1\n hostname x\n!\nend\n"
        );
        assert!(!lib.config_path("idf1").exists());
        assert_eq!(
            lib.rename(1, "IDF2").unwrap_err(),
            "A switch named idf2 is already in the list."
        );
        assert_eq!(
            lib.rename(1, "two words").unwrap_err(),
            "A hostname is one word."
        );
        let again = Library::open(t.0.clone()).unwrap();
        assert_eq!(again.switches[1].name, "mdf1");
    }

    #[test]
    fn a_rename_that_changes_only_case_keeps_the_configuration() {
        // on Windows idf1.cfg and IDF1.cfg are one file: writing the new name and deleting the old
        // one would delete it
        let (_t, mut lib) = two("case");
        let i = lib.rename(0, "IDF1").unwrap();
        assert_eq!(lib.switches[i].name, "IDF1");
        assert_eq!(
            lib.config("IDF1").unwrap(),
            "!\nversion 17.15\nhostname IDF1\ninterface Vlan1\n hostname x\n!\nend\n"
        );
    }

    /// Windows refuses to replace a read-only file, so the list cannot be written.
    #[cfg(windows)]
    #[test]
    fn a_rename_or_copy_the_list_cannot_keep_leaves_everything_as_it_was() {
        let (t, mut lib) = two("unwritable");
        let index = t.0.join("switches.json");
        let (list, kept) = (
            std::fs::read_to_string(&index).unwrap(),
            lib.config("idf1").unwrap(),
        );
        let read_only = |on: bool| {
            let mut p = std::fs::metadata(&index).unwrap().permissions();
            p.set_readonly(on);
            std::fs::set_permissions(&index, p).unwrap();
        };
        read_only(true);
        let renamed = lib.rename(0, "mdf1");
        let copied = lib.duplicate(0, "idf1-lab");
        read_only(false);
        assert!(
            renamed.is_err() && copied.is_err(),
            "{renamed:?} {copied:?}"
        );
        assert_eq!(std::fs::read_to_string(&index).unwrap(), list);
        let names: Vec<&str> = lib.switches.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["idf1", "idf2"]);
        assert_eq!(lib.config("idf1").unwrap(), kept);
        assert!(!lib.config_path("mdf1").exists() && !lib.config_path("idf1-lab").exists());
    }

    #[test]
    fn a_saved_hostname_kept_in_the_same_file_keeps_the_configuration() {
        // `a/b` is kept as a_b.cfg and `A_B` as A_B.cfg: one file on Windows, which writing the new
        // name and deleting the old one would delete
        let t = TempDir::new("save-case");
        let mut lib = Library::open(t.0.clone()).unwrap();
        lib.add(
            "a/b",
            "C9300X-24Y",
            crate::release::DEFAULT,
            "x.cfg",
            "hostname a/b\n",
        )
        .unwrap();
        assert_eq!(
            lib.save_running("a/b", "hostname A_B\nvlan 10\n").unwrap(),
            "A_B"
        );
        assert_eq!(lib.config("A_B").unwrap(), "hostname A_B\nvlan 10\n");
        assert_eq!(Library::open(t.0.clone()).unwrap().switches[0].name, "A_B");
    }

    /// Windows refuses to replace a read-only file, so the list cannot be written.
    #[cfg(windows)]
    #[test]
    fn a_save_add_restore_or_hardware_change_the_list_cannot_keep_leaves_everything_as_it_was() {
        let (t, mut lib) = two("unwritable-save");
        let index = t.0.join("switches.json");
        let gone = lib.remove(1).unwrap();
        let (list, kept, names) = (
            std::fs::read_to_string(&index).unwrap(),
            lib.config("idf1").unwrap(),
            lib.switches.clone(),
        );
        let read_only = |on: bool| {
            let mut p = std::fs::metadata(&index).unwrap().permissions();
            p.set_readonly(on);
            std::fs::set_permissions(&index, p).unwrap();
        };
        read_only(true);
        let saved = lib.save_running("idf1", "hostname mdf1\nvlan 10\n");
        let added = lib.add(
            "core1",
            "C9300X-24Y",
            crate::release::DEFAULT,
            "c.txt",
            "hostname core1\n",
        );
        let restored = lib.restore(gone);
        let hardware = lib.set_hardware(0, "C9300-48P", crate::release::DEFAULT);
        read_only(false);
        assert!(
            saved.is_err() && added.is_err() && restored.is_err() && hardware.is_err(),
            "{saved:?} {added:?} {restored:?} {hardware:?}"
        );
        assert_eq!(std::fs::read_to_string(&index).unwrap(), list);
        assert_eq!(lib.switches, names);
        assert_eq!(lib.config("idf1").unwrap(), kept);
        for name in ["mdf1", "core1", "idf2"] {
            assert!(!lib.config_path(name).exists(), "{name}");
        }
    }

    #[test]
    fn another_process_s_changes_to_the_list_survive_this_one_s_save() {
        let (t, mut lib) = two("two-processes");
        let mut other = Library::open(t.0.clone()).unwrap();
        other
            .add(
                "core1",
                "C9300X-24Y",
                crate::release::DEFAULT,
                "c.txt",
                "hostname core1\n",
            )
            .unwrap();
        other.remove(other.position("idf2").unwrap()).unwrap();
        let i = other.position("idf1").unwrap();
        other
            .set_hardware(i, "C9300-48P", crate::release::DEFAULT)
            .unwrap();
        // this process knew idf1 and idf2 only, and changes nothing the other one did
        lib.opened(0).unwrap();
        lib.add(
            "mdf1",
            "C9300X-24Y",
            crate::release::DEFAULT,
            "m.txt",
            "hostname mdf1\n",
        )
        .unwrap();
        let again = Library::open(t.0.clone()).unwrap();
        let names: Vec<&str> = again.switches.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["core1", "idf1", "mdf1"]);
        let idf1 = &again.switches[1];
        assert_eq!(idf1.stack, "C9300-48P");
        assert!(idf1.opened_at.is_some());
        assert_eq!(lib.switches, again.switches);
    }

    #[test]
    fn an_imported_switch_is_named_with_one_word() {
        let t = TempDir::new("import-name");
        for (file, text) in [
            ("empty.toml", "hostname = \"\"\nstack = [\"C9300X-24Y\"]\n"),
            (
                "spaced.toml",
                "hostname = \"core sw\"\nstack = [\"C9300X-24Y\"]\n",
            ),
            // a design saved under another extension is no configuration
            (
                "design.txt",
                "hostname = \"core1\"\nstack = [\"C9300X-24Y\"]\n",
            ),
        ] {
            let p = t.0.join(file);
            std::fs::write(&p, text).unwrap();
            let e = import(&p).unwrap_err();
            assert!(e.contains("one word"), "{file}: {e}");
        }
        // a file name with a space names the switch with a dash, and its configuration says so
        let p = t.0.join("my switch.cfg");
        std::fs::write(&p, "version 17.15\ninterface GigabitEthernet1/0/1\n").unwrap();
        match import(&p).unwrap() {
            Import::NeedsStack { name, config, .. } => {
                assert_eq!(name, "my-switch");
                assert!(config.starts_with("hostname my-switch\n"), "{config}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_configuration_saved_as_utf_16_loads() {
        // Windows PowerShell 5.1 `>` writes UTF-16 with a byte order mark
        let t = TempDir::new("import-utf16");
        let p = t.0.join("u16.txt");
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(
            "hostname u16\r\ninterface GigabitEthernet1/0/1\r\nend\r\n"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        std::fs::write(&p, &bytes).unwrap();
        match import(&p).unwrap() {
            Import::NeedsStack { name, config, .. } => {
                assert_eq!(name, "u16");
                assert!(config.starts_with("hostname u16\r\n"), "{config:?}");
            }
            other => panic!("{other:?}"),
        }
        // other bytes say what they are
        std::fs::write(&p, [0x68, 0xe9, 0xff, 0x0a]).unwrap();
        assert!(import(&p).unwrap_err().contains("not UTF-8 or UTF-16 text"));
    }

    #[test]
    fn a_removed_switch_whose_copy_cannot_be_read_keeps_its_file() {
        let (_t, mut lib) = two("remove-unreadable");
        let path = lib.config_path("idf2");
        // a banner saved in Latin-1 by a hand edit
        std::fs::write(&path, b"hostname idf2\nbanner motd ^C caf\xe9 ^C\n").unwrap();
        let gone = lib.remove(1).unwrap();
        assert!(gone.config.is_none());
        lib.restore(gone).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"hostname idf2\nbanner motd ^C caf\xe9 ^C\n"
        );
    }

    #[test]
    fn a_copy_names_itself_and_leaves_the_original() {
        let (_t, mut lib) = two("copy");
        lib.switches[0].saved_at = Some(6);
        let i = lib.duplicate(0, "idf1-lab").unwrap();
        let c = &lib.switches[i];
        assert_eq!(
            (
                c.name.as_str(),
                c.stack.as_str(),
                c.source.as_str(),
                c.saved_at
            ),
            ("idf1-lab", "C9300X-24Y", "copy of idf1", None)
        );
        assert!(
            lib.config("idf1-lab")
                .unwrap()
                .contains("\nhostname idf1-lab\n")
        );
        assert!(lib.config("idf1").unwrap().contains("\nhostname idf1\n"));
        assert!(lib.duplicate(0, "idf2").is_err());
        // a name kept in the same file as another switch's is taken
        assert_eq!(lib.taken("IDF1-LAB", None), Some(i));
        assert_eq!(lib.taken("idf1/lab", None), None);
        let j = lib.duplicate(0, "a_b").unwrap();
        assert_eq!(lib.taken("a/b", None), Some(j));
    }

    #[test]
    fn a_removed_switch_comes_back_with_its_configuration() {
        let (_t, mut lib) = two("restore");
        let kept = lib.config("idf1").unwrap();
        let gone = lib.remove(0).unwrap();
        assert_eq!(gone.config.as_deref(), Some(kept.as_str()));
        assert!(lib.config("idf1").is_err());
        let i = lib.restore(gone.clone()).unwrap();
        assert_eq!(lib.switches[i], gone.saved);
        assert_eq!(lib.config("idf1").unwrap(), kept);
        // not over a switch that took its name meanwhile
        let gone = lib.remove(i).unwrap();
        lib.add(
            "IDF1",
            "C9300X-24Y",
            crate::release::DEFAULT,
            "b.txt",
            "hostname IDF1\n",
        )
        .unwrap();
        assert_eq!(
            lib.restore(gone).unwrap_err(),
            "A switch named IDF1 is in the list again."
        );
    }

    #[test]
    fn hostname_lines_are_replaced_in_place_or_put_first() {
        assert_eq!(
            with_hostname("!\r\nhostname a\r\nend\r\n", "b"),
            "!\r\nhostname b\r\nend\r\n"
        );
        assert_eq!(with_hostname("vlan 2\n", "b"), "hostname b\nvlan 2\n");
        assert_eq!(with_hostname("", "b"), "hostname b\n");
    }

    #[test]
    fn a_damaged_list_is_named_and_left_as_it_is() {
        let t = TempDir::new("damaged");
        std::fs::create_dir_all(&t.0).unwrap();
        let index = t.0.join("switches.json");
        std::fs::write(&index, "{switches: [").unwrap();
        let e = Library::open(t.0.clone()).err().unwrap();
        assert!(
            e.contains("is not a switch list vswitch can read") && e.contains("left as it is"),
            "{e}"
        );
        assert_eq!(std::fs::read_to_string(&index).unwrap(), "{switches: [");
        // a list an editor saved with a byte order mark reads
        std::fs::write(&index, "\u{feff}{\"switches\": []}").unwrap();
        assert!(Library::open(t.0.clone()).unwrap().switches.is_empty());
    }

    #[test]
    fn prefs_are_remembered_and_default_when_unreadable() {
        let t = TempDir::new("prefs");
        let lib = Library::open(t.0.clone()).unwrap();
        assert_eq!(lib.prefs(), Prefs::default());
        let p = Prefs {
            folder: Some(t.0.join("captures")),
            by_opened: true,
        };
        lib.set_prefs(&p).unwrap();
        assert_eq!(Library::open(t.0.clone()).unwrap().prefs(), p);
        std::fs::write(t.0.join("prefs.json"), "{not json").unwrap();
        assert_eq!(lib.prefs(), Prefs::default());
    }

    #[test]
    fn a_generated_stack_configuration_infers_its_9300x_members() {
        let spec: crate::generate::Spec =
            toml::from_str(include_str!("../tests/example-design.toml")).unwrap();
        let config = crate::generate::render(&spec).join("\n");
        // it sets only HundredGigE n/1/1, which the smaller module has too; a design names its own
        // hardware, and the selector asks before keeping an inferred one
        assert_eq!(
            crate::platform::infer_stack(&config).as_deref(),
            Some("C9300X-24Y+C9300X-NM-2C,C9300X-24Y+C9300X-NM-2C")
        );
        // a running configuration lists every port
        let full: String = (1..=4)
            .map(|p| format!("interface HundredGigE1/1/{p}\ninterface TwentyFiveGigE1/0/24\n"))
            .collect();
        assert_eq!(
            crate::platform::infer_stack(&full).as_deref(),
            Some("C9300X-24Y+C9300X-NM-4C")
        );
        assert_eq!(crate::platform::infer_stack("hostname x\n"), None);
        // copper 1G downlinks are a Catalyst 9200; its uplinks are taken as a module, the builder
        // offering the 9200L
        assert_eq!(
            crate::platform::infer_stack("interface GigabitEthernet1/0/1\n").as_deref(),
            Some("C9200-24T")
        );
        let c9200 = "interface GigabitEthernet1/0/48\ninterface TenGigabitEthernet1/1/4\n\
                     interface GigabitEthernet2/0/16\ninterface TenGigabitEthernet2/0/24\ninterface TwentyFiveGigE2/1/2\n";
        assert_eq!(
            crate::platform::infer_stack(c9200).as_deref(),
            Some("C9200-48T+C9200-NM-4X,C9200-24PXG+C9200-NM-2Y")
        );
    }

    #[test]
    fn a_design_names_its_hardware_and_a_configuration_has_it_inferred() {
        let design = import(Path::new("tests/example-design.toml")).unwrap();
        match design {
            Import::Ready {
                name,
                stack,
                release,
                config,
            } => {
                assert_eq!(release, None);
                assert_eq!(name, "dc1-stack-sw01");
                assert_eq!(stack, "C9300X-24Y+C9300X-NM-4C,C9300X-24Y+C9300X-NM-4C");
                assert!(config.contains("vtp mode transparent"));
            }
            other => panic!("{other:?}"),
        }
        let t = TempDir::new("import");
        let p = t.0.join("core.txt");
        std::fs::write(
            &p,
            "!\nhostname core1\n!\ninterface TenGigabitEthernet2/0/24\n!\ninterface FortyGigabitEthernet3/0/10\n!\ninterface GigabitEthernet5/0/48\n!\nend\n",
        )
        .unwrap();
        assert_eq!(
            import(&p).unwrap(),
            Import::NeedsStack {
                name: "core1".into(),
                inferred: Some("2=C9400-LC-24XS,3=C9400-SUP-1XL,5=C9400-LC-48U".into()),
                release: None,
                config: std::fs::read_to_string(&p).unwrap(),
            }
        );
        std::fs::write(&p, "!\nversion 17.5\nhostname core1\n").unwrap();
        assert!(matches!(
            import(&p).unwrap(),
            Import::NeedsStack { release: Some(r), .. } if r == "17.5"
        ));
        // notes, a README and an empty file are no switch
        for text in [
            "# Notes\n\nThe core moves on Friday.\n",
            "",
            "hostnames are listed below\n",
        ] {
            std::fs::write(&p, text).unwrap();
            let e = import(&p).unwrap_err();
            assert!(e.contains("no configuration here"), "{text:?}: {e}");
        }
        let old: Saved =
            serde_json::from_str(r#"{"name":"a","stack":"C9300X-24Y","source":"x","loaded_at":1}"#)
                .unwrap();
        assert_eq!(old.release(), crate::release::DEFAULT);
    }

    #[test]
    fn each_switch_keeps_one_mac_made_from_its_name_and_hardware() {
        let t = TempDir::new("lib-mac");
        // a list written before MACs were kept loads with one each, written back once
        std::fs::write(
            t.0.join("switches.json"),
            r#"{"switches":[{"name":"a","stack":"C9300-24T","source":"x","loaded_at":1}]}"#,
        )
        .unwrap();
        let mut lib = Library::open(t.0.clone()).unwrap();
        let mac = new_mac("a", "C9300-24T");
        assert_eq!(lib.switches[0].mac.as_deref(), Some(mac.as_str()));
        assert!(
            std::fs::read_to_string(t.0.join("switches.json"))
                .unwrap()
                .contains(&mac)
        );
        assert_eq!(lib.switches[0].stack, "C9300-24T");
        // kept through a rename; a copy gets its own
        std::fs::create_dir_all(t.0.join("configs")).unwrap();
        std::fs::write(lib.config_path("a"), "hostname a\n").unwrap();
        let i = lib.rename(0, "b").unwrap();
        assert_eq!(lib.switches[i].mac.as_deref(), Some(mac.as_str()));
        let j = lib.duplicate(i, "c").unwrap();
        assert_eq!(
            lib.switches[j].mac.as_deref(),
            Some(new_mac("c", "C9300-24T").as_str())
        );
        assert_ne!(new_mac("c", "C9300-24T"), mac);
    }
}
