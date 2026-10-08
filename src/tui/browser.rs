//! The switch list's file browser: a folder's entries, folders first, narrowed as a name is typed, to
//! load a design or a configuration or to pick where a kept configuration is written. A typed or
//! pasted path works as well as picking.

use super::input::Input;
use super::{BOLD, DIM, HINT, NOTE, PICK, PROBLEM, fit_left, fit_right, when, wrap};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Cell as TableCell, Clear, HighlightSpacing, Padding, Paragraph, Row, Table, TableState,
};
use std::cell::Cell;
use std::path::{Component, Path, PathBuf};

/// What the browser is for.
#[derive(Clone, Debug, PartialEq)]
pub enum Goal {
    Load,
    /// a switch's kept configuration, by the switch's name, and the file name offered for it
    Export {
        name: String,
        file: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub dir: bool,
    /// `..`, the folder above
    pub up: bool,
    pub modified: Option<u64>,
    pub size: Option<u64>,
}

/// What a key asks of the list.
#[derive(Debug, PartialEq)]
pub enum Step {
    Stay,
    Cancel,
    Load(PathBuf),
    Write(PathBuf),
}

/// A folder read: which one (none: the drives), whether names starting with a dot were kept, and
/// its entries or why it could not be read.
type Listing = (Option<PathBuf>, bool, Result<Vec<Entry>, String>);

#[derive(Debug, PartialEq)]
pub struct Browser {
    pub goal: Goal,
    /// the folder shown; none for the computer's drives
    pub dir: Option<PathBuf>,
    pub query: Input,
    /// the entry picked among those shown; in an export, none until an arrow key picks one
    pub pick: Option<usize>,
    pub error: Option<String>,
    /// an export's file that is there, asked about before it is written over
    pub ask: Option<PathBuf>,
    /// none when the folder must be read again
    listing: Option<Listing>,
    offset: Cell<usize>,
    page: Cell<usize>,
}

/// Designs and configurations: the files the list loads.
pub fn loadable(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            ["toml", "cfg", "txt", "log"]
                .iter()
                .any(|x| e.eq_ignore_ascii_case(x))
        })
}

fn separators() -> &'static [char] {
    if cfg!(windows) { &['/', '\\'] } else { &['/'] }
}

/// A path with `.` and `..` worked out from its text alone.
fn normal(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() && !out.has_root() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn home() -> Option<PathBuf> {
    let k = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(k)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// A path as the screens show it: `~` for the home folder it is in, as a typed path's `~` is read
/// (`~\.vswitch\configs\core1.cfg`).
pub fn shown(path: &Path) -> String {
    match home().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

/// The quotes a terminal puts around a dropped path that holds a space, and spaces around it, left
/// out.
fn unquote(text: &str) -> &str {
    text.trim().trim_matches(['"', '\'']).trim()
}

/// A folder's entries, folders first, each group by name. What Windows marks hidden or system and,
/// elsewhere, names starting with a dot are left out unless `dots`.
fn read(dir: &Path, dots: bool) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // a link is shown as what it leads to; the entry's own metadata costs no extra call
        let md = match e.file_type() {
            Ok(t) if t.is_symlink() => std::fs::metadata(e.path()).or_else(|_| e.metadata()),
            _ => e.metadata(),
        }
        .ok();
        if !dots && hidden(&name, md.as_ref()) {
            continue;
        }
        out.push(Entry {
            path: e.path(),
            dir: md.as_ref().is_some_and(|m| m.is_dir()),
            up: false,
            modified: md
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            size: md.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
            name,
        });
    }
    out.sort_by_cached_key(|e| (!e.dir, e.name.to_lowercase()));
    Ok(out)
}

#[cfg(windows)]
fn hidden(_name: &str, md: Option<&std::fs::Metadata>) -> bool {
    use std::os::windows::fs::MetadataExt;
    // FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM
    md.is_some_and(|m| m.file_attributes() & 0x6 != 0)
}

#[cfg(not(windows))]
fn hidden(name: &str, _md: Option<&std::fs::Metadata>) -> bool {
    name.starts_with('.')
}

/// The drives of a Windows computer: the folder above each drive's root.
fn drives() -> Result<Vec<Entry>, String> {
    if !cfg!(windows) {
        return Err("no folder".into());
    }
    Ok((b'A'..=b'Z')
        .map(|c| format!("{}:\\", c as char))
        .filter(|d| Path::new(d).is_dir())
        .map(|d| Entry {
            path: PathBuf::from(&d),
            name: d,
            dir: true,
            up: false,
            modified: None,
            size: None,
        })
        .collect())
}

/// Why a file did not load, in two lines of `width` columns. A report of several lines (a TOML error
/// quotes the line it stopped at) keeps its first line, which names the file and where, and its last,
/// the cause; one line keeps its end.
fn error_lines(e: &str, width: usize) -> Vec<String> {
    let lines: Vec<&str> = e
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    match lines.as_slice() {
        [] => vec![],
        [one] => {
            let mut wrapped = wrap(one, width);
            if wrapped.len() > 2 {
                let first = format!(
                    "…{}",
                    fit_left(&wrapped[wrapped.len() - 2], width.saturating_sub(1))
                );
                wrapped.drain(..wrapped.len() - 2);
                wrapped[0] = first;
            }
            wrapped
        }
        [first, .., last] => vec![fit_left(first, width), fit_right(last.trim(), width)],
    }
}

/// `512 B`, `12 KB`, `3.4 MB`.
fn size_text(n: u64) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{} KB", n.div_ceil(1024)),
        n => format!("{:.1} MB", n as f64 / (1024.0 * 1024.0)),
    }
}

impl Browser {
    /// A browser in `dir`, else the nearest folder above it that is there, else the working folder.
    pub fn new(goal: Goal, dir: Option<PathBuf>) -> Self {
        let start = dir
            .iter()
            .flat_map(|d| d.ancestors())
            .find(|d| d.is_dir())
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok());
        let mut b = Browser {
            goal,
            dir: None,
            query: Input::default(),
            pick: None,
            error: None,
            ask: None,
            listing: None,
            offset: Cell::new(0),
            page: Cell::new(10),
        };
        b.go(start, None);
        b
    }

    fn loading(&self) -> bool {
        self.goal == Goal::Load
    }

    /// A typed path, relative to the folder shown; `~` is the home folder.
    fn resolve(&self, text: &str) -> Option<PathBuf> {
        let t = unquote(text);
        if t.is_empty() {
            return None;
        }
        let p = match t.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with(separators()) => {
                home()?.join(rest.trim_start_matches(separators()))
            }
            _ => PathBuf::from(t),
        };
        let p = match &self.dir {
            _ if p.is_absolute() => p,
            // joining keeps the drive of the folder shown for a path that starts at a root
            Some(d) => d.join(p),
            None if p.has_root() => p,
            None => return None,
        };
        Some(normal(&p))
    }

    /// The folder the typed text is in, and the part of a name typed in it: a typed or pasted path
    /// lists its own folder.
    fn place(&self) -> (Option<PathBuf>, String) {
        let t = unquote(&self.query.text);
        match t.rfind(separators()) {
            Some(i) => (self.resolve(&t[..=i]), t[i + 1..].to_string()),
            None if t == "~" => (home(), String::new()),
            None => (self.dir.clone(), t.to_string()),
        }
    }

    /// The folder listed and what narrows it: in an export, the folder shown, whole.
    fn listed(&self) -> (Option<PathBuf>, String) {
        if self.loading() {
            self.place()
        } else {
            (self.dir.clone(), String::new())
        }
    }

    fn refresh(&mut self) {
        let (dir, needle) = self.listed();
        let dots = needle.starts_with('.');
        let fresh = matches!(&self.listing, Some((d, k, _)) if *d == dir && *k == dots);
        if !fresh {
            let read = match &dir {
                Some(d) => read(d, dots),
                None => drives(),
            };
            self.listing = Some((dir, dots, read));
        }
        let n = self.shown().len();
        self.pick = self.pick.filter(|_| n > 0).map(|p| p.min(n - 1));
    }

    fn entries(&self) -> Result<&[Entry], &str> {
        match &self.listing {
            Some((_, _, Ok(e))) => Ok(e),
            Some((_, _, Err(e))) => Err(e),
            None => Ok(&[]),
        }
    }

    /// The entries shown: `..` first while nothing is typed, then those whose name holds what is
    /// typed, in any case.
    pub fn shown(&self) -> Vec<Entry> {
        let (dir, needle) = self.listed();
        let Ok(entries) = self.entries() else {
            return vec![];
        };
        let mut out = Vec::new();
        if needle.is_empty()
            && let Some(d) = &dir
            && (d.parent().is_some() || cfg!(windows))
        {
            out.push(Entry {
                name: "..".into(),
                path: d.parent().map(Path::to_path_buf).unwrap_or_default(),
                dir: true,
                up: true,
                modified: None,
                size: None,
            });
        }
        let n = needle.to_lowercase();
        out.extend(
            entries
                .iter()
                .filter(|e| e.name.to_lowercase().contains(&n))
                .cloned(),
        );
        out
    }

    fn picked(&self) -> Option<Entry> {
        self.shown().into_iter().nth(self.pick?)
    }

    /// The first entry other than `..`, when a load has one.
    fn first_pick(&self) -> Option<usize> {
        let shown = self.shown();
        (self.loading() && !shown.is_empty()).then(|| usize::from(shown[0].up && shown.len() > 1))
    }

    /// Into a folder (none: the drives), the entry named `from` picked. A load's typing starts
    /// over; an export keeps the name typed unless it named the folder.
    fn go(&mut self, dir: Option<PathBuf>, from: Option<String>) {
        let named_folder = self.resolve(&self.query.text).is_none_or(|p| p.is_dir());
        self.dir = dir.map(|d| normal(&d));
        match &self.goal {
            Goal::Load => self.query = Input::default(),
            Goal::Export { file, .. } => {
                if named_folder {
                    self.query = Input::new(file);
                }
            }
        }
        self.error = None;
        self.listing = None;
        self.offset.set(0);
        self.pick = None;
        self.refresh();
        let shown = self.shown();
        self.pick = match from {
            Some(name) => shown.iter().position(|e| e.name == name),
            None => self.first_pick(),
        };
    }

    fn up(&mut self) {
        let Some(d) = self.dir.clone() else {
            return;
        };
        let name = d
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| d.display().to_string());
        match d.parent() {
            Some(p) => self.go(Some(p.to_path_buf()), Some(name)),
            None if cfg!(windows) => self.go(None, Some(name)),
            None => {}
        }
    }

    fn open(&mut self, e: Entry) {
        if e.up {
            self.up();
        } else {
            self.go(Some(e.path), None);
        }
    }

    /// A paste at the cursor; a whole path pasted (a file dropped on the terminal) replaces what was
    /// typed.
    pub fn paste(&mut self, text: &str) {
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        let bare = unquote(&clean);
        if self.query.is_empty() || Path::new(bare).is_absolute() {
            self.query = Input::new(bare);
        } else {
            self.query.paste(&clean);
        }
        self.edited();
    }

    fn edited(&mut self) {
        self.error = None;
        self.offset.set(0);
        if self.loading() {
            self.refresh();
            self.pick = self.first_pick();
        } else {
            self.pick = None;
        }
    }

    fn step(&mut self, by: isize) {
        let n = self.shown().len() as isize;
        if n == 0 {
            return;
        }
        let at = match self.pick {
            Some(p) => p as isize + by,
            None if by > 0 => 0,
            None => n - 1,
        };
        self.pick = Some(at.clamp(0, n - 1) as usize);
    }

    /// Completes the name typed to what the entries it begins share; the one folder it begins is
    /// opened.
    fn complete(&mut self) {
        let t = unquote(&self.query.text).to_string();
        let cut = t.rfind(separators()).map_or(0, |i| i + 1);
        let needle = t[cut..].to_lowercase();
        let (dir, _) = self.place();
        let entries = match &self.listing {
            Some((d, _, Ok(e))) if *d == dir => e.clone(),
            _ => match &dir {
                Some(d) => read(d, needle.starts_with('.')).unwrap_or_default(),
                None => drives().unwrap_or_default(),
            },
        };
        let hits: Vec<&Entry> = entries
            .iter()
            .filter(|e| e.name.to_lowercase().starts_with(&needle))
            .collect();
        match hits.as_slice() {
            [] => {}
            // the folder completed is named as if typed whole, so an export offers its file name again
            [one] if one.dir => {
                let path = one.path.clone();
                self.query = Input::new(&format!("{}{}", &t[..cut], one.name));
                self.go(Some(path), None);
            }
            [one] => {
                self.query = Input::new(&format!("{}{}", &t[..cut], one.name));
                self.edited();
            }
            many => {
                let first: Vec<char> = many[0].name.chars().collect();
                let mut len = first.len();
                for h in &many[1..] {
                    len = first
                        .iter()
                        .zip(h.name.chars())
                        .take(len)
                        .take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                        .count();
                }
                if len > needle.chars().count() {
                    let shared: String = first[..len].iter().collect();
                    self.query = Input::new(&format!("{}{shared}", &t[..cut]));
                    self.edited();
                }
            }
        }
    }

    fn enter(&mut self) -> Step {
        if self.loading() {
            if let Some(p) = self.resolve(&self.query.text) {
                if p.is_file() {
                    return Step::Load(p);
                }
                if p.is_dir() {
                    self.go(Some(p), None);
                    return Step::Stay;
                }
            }
            match self.picked() {
                Some(e) if e.dir => self.open(e),
                Some(e) => return Step::Load(e.path),
                None if self.query.is_empty() => self.error = Some("Nothing here to load.".into()),
                None => {
                    self.error = Some(format!(
                        "Nothing here is named {}.",
                        unquote(&self.query.text)
                    ))
                }
            }
            return Step::Stay;
        }
        if let Some(e) = self.picked() {
            if e.dir {
                self.open(e);
            } else {
                self.query = Input::new(&e.name);
                self.pick = None;
            }
            return Step::Stay;
        }
        match self.resolve(&self.query.text) {
            None => self.error = Some("Type the name of the file to write.".into()),
            Some(p) if p.is_dir() => self.go(Some(p), None),
            Some(p) if p.exists() => self.ask = Some(p),
            Some(p) => return Step::Write(p),
        }
        Step::Stay
    }

    pub fn key(&mut self, k: KeyEvent) -> Step {
        if let Some(p) = self.ask.take() {
            // any key but y keeps the file
            return if k.code == KeyCode::Char('y') {
                Step::Write(p)
            } else {
                Step::Stay
            };
        }
        let page = self.page.get().max(1) as isize;
        match k.code {
            KeyCode::Esc => return Step::Cancel,
            KeyCode::Enter => return self.enter(),
            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            KeyCode::PageUp => self.step(-page),
            KeyCode::PageDown => self.step(page),
            KeyCode::Tab => self.complete(),
            KeyCode::Backspace | KeyCode::Left if self.query.is_empty() => self.up(),
            KeyCode::Right if self.query.is_empty() => {
                if let Some(e) = self.picked().filter(|e| e.dir) {
                    self.open(e);
                }
            }
            _ => {
                if self.query.key(k) {
                    self.edited();
                }
            }
        }
        Step::Stay
    }

    /// The browser, filling `area`.
    pub fn draw(&self, f: &mut Frame, area: Rect) {
        f.render_widget(Clear, area);
        let title = match &self.goal {
            Goal::Load => " Load a switch ".to_string(),
            Goal::Export { name, .. } => format!(" Export {name}'s configuration "),
        };
        let block = Block::bordered().title(Span::styled(title, BOLD));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [folder, field, _, list, message] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .areas(inner);
        let label = |t: &str| Span::styled(format!(" {t:<7}"), HINT);
        let room = folder.width.saturating_sub(8) as usize;
        // the folder the entries come from: a typed path lists its own
        let shown_dir = match &self.listed().0 {
            Some(d) => shown(d),
            None => "the computer's drives".into(),
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                label("Folder"),
                Span::raw(fit_left(&shown_dir, room)),
            ])),
            folder,
        );
        let (text, col) = self.query.window(room);
        let field_line = if self.query.is_empty() && self.loading() {
            // a column clear of the cursor
            Line::from(vec![
                label("Find"),
                Span::styled(fit_right(" type to narrow the list, or a path", room), HINT),
            ])
        } else {
            Line::from(vec![
                label(if self.loading() { "Find" } else { "Name" }),
                Span::raw(text),
            ])
        };
        f.render_widget(Paragraph::new(field_line), field);
        if self.ask.is_none() {
            f.set_cursor_position((field.x + 8 + col as u16, field.y));
        }
        self.draw_entries(f, list);
        // two lines at most; a path keeps its end and a line of its own
        let line = message.width.saturating_sub(2) as usize;
        let with_path = |before: &str, p: &Path| {
            let fits = line.saturating_sub(before.len());
            format!("{before}{}", fit_left(&shown(p), fits))
        };
        let lines = if let Some(p) = &self.ask {
            vec![
                Line::styled(with_path("", p), NOTE),
                Line::styled("is there: y writes over it, any other key keeps it", NOTE),
            ]
        } else if let Some(e) = self.error.as_deref().or(self.entries().err()) {
            error_lines(e, line)
                .into_iter()
                .map(|l| Line::styled(l, PROBLEM))
                .collect()
        } else {
            let text = match (&self.goal, self.picked()) {
                (Goal::Load, _) => {
                    "A design (.toml) or a configuration (.cfg, .txt, .log)".to_string()
                }
                (Goal::Export { .. }, Some(e)) if e.up => "Enter goes up a folder".into(),
                (Goal::Export { .. }, Some(e)) if e.dir => format!("Enter opens {}", e.name),
                (Goal::Export { .. }, Some(e)) => format!("Enter takes the name {}", e.name),
                (Goal::Export { .. }, None) => match self.resolve(&self.query.text) {
                    Some(p) => with_path("Enter writes ", &p),
                    None => "Type the name of the file to write".into(),
                },
            };
            vec![Line::styled(text, HINT)]
        };
        f.render_widget(
            Paragraph::new(lines).block(Block::new().padding(Padding::horizontal(1))),
            message,
        );
    }

    fn draw_entries(&self, f: &mut Frame, area: Rect) {
        let shown = self.shown();
        if shown.is_empty() {
            let why = match self.entries() {
                Err(_) => "",
                Ok(_) if self.listed().1.is_empty() => " This folder is empty.",
                Ok(_) => " Nothing here matches.",
            };
            f.render_widget(Paragraph::new(Line::styled(why, HINT)), area);
            return;
        }
        let wide = area.width >= 56;
        let sep = std::path::MAIN_SEPARATOR;
        let rows: Vec<Row> = shown
            .iter()
            .map(|e| {
                let (name, style) = if e.up {
                    ("..".to_string(), BOLD)
                } else if e.dir {
                    let n = if e.name.ends_with(sep) {
                        e.name.clone()
                    } else {
                        format!("{}{sep}", e.name)
                    };
                    (n, BOLD)
                } else if loadable(&e.name) {
                    (e.name.clone(), Style::new())
                } else {
                    (e.name.clone(), DIM)
                };
                let mut cells = vec![TableCell::from(name).style(style)];
                if wide {
                    let size = e.size.map(size_text).unwrap_or_default();
                    cells.push(TableCell::from(Line::from(size).right_aligned()).style(HINT));
                    let at = e.modified.map(|t| when(t)[..16].to_string());
                    cells.push(TableCell::from(at.unwrap_or_default()).style(HINT));
                }
                Row::new(cells)
            })
            .collect();
        let widths = if wide {
            vec![
                Constraint::Fill(1),
                Constraint::Length(8),
                Constraint::Length(16),
            ]
        } else {
            vec![Constraint::Fill(1)]
        };
        let mut table = Table::new(rows, widths)
            .row_highlight_style(PICK)
            .highlight_symbol("> ")
            .highlight_spacing(HighlightSpacing::Always);
        let mut body = area.height as usize;
        if wide {
            table = table.header(
                Row::new(vec![
                    TableCell::from("Name"),
                    TableCell::from(Line::from("Size").right_aligned()),
                    TableCell::from("Modified (UTC)"),
                ])
                .style(BOLD),
            );
            body = body.saturating_sub(1);
        }
        self.page.set(body);
        let mut state = TableState::default()
            .with_offset(self.offset.get())
            .with_selected(self.pick);
        f.render_stateful_widget(table, area, &mut state);
        self.offset.set(state.offset());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::TempDir;
    use ratatui::crossterm::event::KeyModifiers;

    fn press(b: &mut Browser, code: KeyCode) -> Step {
        b.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typed(b: &mut Browser, text: &str) {
        for c in text.chars() {
            press(b, KeyCode::Char(c));
        }
    }

    fn names(b: &Browser) -> Vec<String> {
        b.shown().iter().map(|e| e.name.clone()).collect()
    }

    fn tree(tag: &str) -> TempDir {
        let t = TempDir::new(tag);
        for d in ["captures", "designs", "captures/old"] {
            std::fs::create_dir_all(t.0.join(d)).unwrap();
        }
        for f in ["core1.txt", "Notes.docx", "idf3.log", "captures/idf9.cfg"] {
            std::fs::write(t.0.join(f), "hostname x\n").unwrap();
        }
        t
    }

    #[test]
    fn folders_come_first_and_typing_narrows_the_entries() {
        let t = tree("browse");
        let mut b = Browser::new(Goal::Load, Some(t.0.clone()));
        assert_eq!(
            names(&b),
            [
                "..",
                "captures",
                "designs",
                "core1.txt",
                "idf3.log",
                "Notes.docx"
            ]
        );
        assert_eq!(b.pick, Some(1), "the first entry, not the folder above");
        typed(&mut b, "ID");
        assert_eq!(names(&b), ["idf3.log"]);
        assert_eq!(
            press(&mut b, KeyCode::Enter),
            Step::Load(t.0.join("idf3.log"))
        );
        // Enter opens a folder; Backspace with nothing typed goes back up, that folder picked
        press(&mut b, KeyCode::Backspace);
        press(&mut b, KeyCode::Backspace);
        typed(&mut b, "capt");
        assert_eq!(press(&mut b, KeyCode::Enter), Step::Stay);
        assert_eq!(b.dir.as_deref(), Some(t.0.join("captures").as_path()));
        assert_eq!(names(&b), ["..", "old", "idf9.cfg"]);
        press(&mut b, KeyCode::Backspace);
        assert_eq!(b.dir.as_deref(), Some(t.0.as_path()));
        assert_eq!(b.picked().unwrap().name, "captures");
    }

    #[test]
    fn a_typed_or_pasted_path_loads_and_tab_completes() {
        let t = tree("path");
        let mut b = Browser::new(Goal::Load, Some(t.0.join("designs")));
        // a path lists its own folder as it is typed
        typed(&mut b, "../captures/");
        assert_eq!(names(&b), ["..", "old", "idf9.cfg"]);
        let mut b = Browser::new(Goal::Load, Some(t.0.join("designs")));
        b.paste(&format!("\"{}\"", t.0.join("core1.txt").display()));
        assert_eq!(
            press(&mut b, KeyCode::Enter),
            Step::Load(t.0.join("core1.txt"))
        );
        // Tab opens the one folder a name begins, else grows the name to what the entries share
        let mut b = Browser::new(Goal::Load, Some(t.0.clone()));
        typed(&mut b, "cap");
        press(&mut b, KeyCode::Tab);
        assert_eq!(b.dir.as_deref(), Some(t.0.join("captures").as_path()));
        typed(&mut b, "i");
        press(&mut b, KeyCode::Tab);
        assert_eq!(b.query.text, "idf9.cfg");
        let mut b = Browser::new(Goal::Load, Some(t.0.clone()));
        typed(&mut b, "c");
        press(&mut b, KeyCode::Tab);
        assert_eq!(b.query.text, "c", "captures and core1.txt share only c");
        press(&mut b, KeyCode::Char('z'));
        assert!(names(&b).is_empty(), "{:?}", names(&b));
        assert_eq!(press(&mut b, KeyCode::Enter), Step::Stay);
        assert_eq!(b.error.as_deref(), Some("Nothing here is named cz."));
    }

    #[test]
    fn an_export_asks_before_writing_over_a_file() {
        let t = tree("export");
        let goal = Goal::Export {
            name: "idf3".into(),
            file: "idf3.cfg".into(),
        };
        let mut b = Browser::new(goal, Some(t.0.clone()));
        assert_eq!(
            b.pick, None,
            "Enter writes the name until an arrow picks an entry"
        );
        assert_eq!(
            press(&mut b, KeyCode::Enter),
            Step::Write(t.0.join("idf3.cfg"))
        );
        // into a folder with the arrows; the name stays
        press(&mut b, KeyCode::Down);
        press(&mut b, KeyCode::Down);
        assert_eq!(press(&mut b, KeyCode::Enter), Step::Stay);
        assert_eq!(b.dir.as_deref(), Some(t.0.join("captures").as_path()));
        assert_eq!(b.query.text, "idf3.cfg");
        // a file picked lends its name, and writing over it is asked first
        for _ in 0..3 {
            press(&mut b, KeyCode::Down);
        }
        assert_eq!(press(&mut b, KeyCode::Enter), Step::Stay);
        assert_eq!(b.query.text, "idf9.cfg");
        assert_eq!(press(&mut b, KeyCode::Enter), Step::Stay);
        let there = t.0.join("captures").join("idf9.cfg");
        assert_eq!(b.ask.as_deref(), Some(there.as_path()));
        assert_eq!(
            press(&mut b, KeyCode::Char('n')),
            Step::Stay,
            "any key but y keeps it"
        );
        assert_eq!(b.ask, None);
        press(&mut b, KeyCode::Enter);
        assert_eq!(press(&mut b, KeyCode::Char('y')), Step::Write(there));
        // a folder typed as the name opens it, and the name offered comes back
        let mut b = Browser::new(
            Goal::Export {
                name: "idf3".into(),
                file: "idf3.cfg".into(),
            },
            Some(t.0.clone()),
        );
        b.query = Input::new("designs");
        assert_eq!(press(&mut b, KeyCode::Enter), Step::Stay);
        assert_eq!(b.dir.as_deref(), Some(t.0.join("designs").as_path()));
        assert_eq!(b.query.text, "idf3.cfg");
    }

    #[test]
    fn tab_into_a_folder_in_an_export_offers_the_file_name_again() {
        let t = tree("export-tab");
        let goal = Goal::Export {
            name: "idf3".into(),
            file: "idf3.cfg".into(),
        };
        let mut b = Browser::new(goal, Some(t.0.clone()));
        b.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        typed(&mut b, "cap");
        press(&mut b, KeyCode::Tab);
        assert_eq!(b.dir.as_deref(), Some(t.0.join("captures").as_path()));
        assert_eq!(b.query.text, "idf3.cfg");
        assert_eq!(
            press(&mut b, KeyCode::Enter),
            Step::Write(t.0.join("captures").join("idf3.cfg"))
        );
    }

    /// The browser drawn at 80 by 22, a line a row.
    fn screen(b: &Browser) -> Vec<String> {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 22)).unwrap();
        t.draw(|f| {
            let area = f.area();
            b.draw(f, area)
        })
        .unwrap();
        let buf = t.backend().buffer().clone();
        (0..22)
            .map(|y| {
                let row: String = (0..80).map(|x| buf[(x, y)].symbol()).collect();
                row.trim_end_matches([' ', '│'])
                    .trim_start_matches('│')
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn the_folder_line_names_the_folder_listed_and_an_error_keeps_its_cause() {
        let t = tree("draw");
        let mut b = Browser::new(Goal::Load, Some(t.0.join("designs")));
        typed(&mut b, "../captures/");
        let s = screen(&b);
        let folder = s
            .iter()
            .find(|l| l.contains("Folder"))
            .expect("a Folder line");
        assert!(folder.ends_with("captures"), "{s:#?}");
        // a design with no hostname, and a TOML file that is no design: where, and why
        let cases = [
            (
                "nohost.toml",
                "stack = [\"C9300-48T\"]\n",
                "missing field `hostname`",
            ),
            (
                "Cargo.toml",
                "[package]\nname = \"x\"\n",
                "unknown field `package`",
            ),
        ];
        for (file, text, cause) in cases {
            let path = t.0.join(file);
            std::fs::write(&path, text).unwrap();
            b.error = Some(crate::library::import(&path).unwrap_err());
            let s = screen(&b);
            let at = s
                .iter()
                .position(|l| l.contains("TOML parse error at line 1"))
                .unwrap_or_else(|| panic!("{s:#?}"));
            assert!(s[at].contains(file), "{s:#?}");
            assert!(s[at + 1].trim_start().starts_with(cause), "{s:#?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn above_a_drive_are_the_drives() {
        let root = PathBuf::from(format!("{}\\", std::env::var("SystemDrive").unwrap()));
        let mut b = Browser::new(Goal::Load, Some(root.clone()));
        press(&mut b, KeyCode::Backspace);
        assert_eq!(b.dir, None);
        let here = root.display().to_string();
        assert_eq!(
            b.picked().map(|e| e.name),
            Some(here.clone()),
            "{:?}",
            names(&b)
        );
        press(&mut b, KeyCode::Enter);
        assert_eq!(b.dir.as_ref().map(|d| d.display().to_string()), Some(here));
    }

    #[test]
    fn a_path_in_the_home_folder_is_shown_from_tilde() {
        let Some(h) = home() else {
            return;
        };
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(shown(&h), "~");
        assert_eq!(
            shown(&h.join(".vswitch").join("configs")),
            format!("~{sep}.vswitch{sep}configs")
        );
        // a folder whose name only begins with the home folder's is no folder in it
        let mut beside = h.clone().into_os_string();
        beside.push("x");
        let beside = PathBuf::from(beside);
        assert_eq!(shown(&beside), beside.display().to_string());
    }
}
