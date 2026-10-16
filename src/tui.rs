//! `vswitch` with no arguments: the switches loaded before, by hostname, with their hardware, release
//! and last save. Enter opens the selected switch's console with its configuration loaded; `exit`
//! there comes back to the list. A switch's hardware is chosen in the builder, which marks what Cisco
//! does not build or stack.

mod browser;
mod input;
mod viewer;

use crate::builder::Builder;
use crate::cli::Engine;
use crate::library::{self, Import, Library, Prefs, Removed, Saved};
use crate::platform::Family;
use crate::release::Release;
use crate::{platform, shell};
use browser::{Browser, Goal, Step};
use input::Input;
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Cell as TableCell, Clear, HighlightSpacing, Padding, Paragraph, Row, Table, TableState,
    Wrap,
};
use std::cell::Cell;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use viewer::Viewer;

// The palette every screen of vswitch keeps to: a pick black on cyan, hints gray, problems red, notes
// yellow, success green, headers bold.
const PICK: Style = Style::new().fg(Color::Black).bg(Color::Cyan);
const HINT: Style = Style::new().fg(Color::Gray);
/// files the list does not load, beside those it does
const DIM: Style = Style::new().fg(Color::DarkGray);
const PROBLEM: Style = Style::new().fg(Color::Red);
const NOTE: Style = Style::new().fg(Color::Yellow);
const DONE: Style = Style::new().fg(Color::Green);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);

/// The smallest terminal the list draws in; a smaller one is told so instead.
const MIN_WIDTH: u16 = 60;
const MIN_HEIGHT: u16 = 15;
/// From this width the selected switch's details stand beside the list, else below it.
const SIDE_BY_SIDE: u16 = 110;

#[derive(Debug, PartialEq)]
pub enum Mode {
    List,
    /// typing the filter; the list narrows as it is typed
    Filter,
    /// the file browser, to load a switch or to export one's kept configuration
    Browse(Box<Browser>),
    /// a name typed: a new switch's, a listed switch's new name, a copy's
    Name {
        naming: Naming,
        input: Input,
        error: Option<String>,
    },
    /// the hardware builder, for a configuration being loaded, a new switch or a listed one
    Hardware {
        name: String,
        purpose: Purpose,
        builder: Builder,
    },
    /// asked before a switch leaves the list
    Remove,
    /// asked before a switch of the same name is replaced
    Replace(Box<Pending>),
    /// a listed switch's kept configuration
    View(Box<Viewer>),
}

#[derive(Debug, PartialEq)]
pub enum Purpose {
    /// a configuration whose file does not name its hardware, or names hardware Cisco does not build
    Import { source: String, config: String },
    /// a switch with no configuration but its name
    New,
    /// a listed switch's hardware, its configuration kept
    Edit(usize),
}

/// A switch ready to keep, held while its name is asked about.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub name: String,
    pub stack: String,
    pub release: Release,
    pub source: String,
    pub config: String,
}

/// What a typed name is for; a listed switch by its place in the list.
#[derive(Debug, PartialEq)]
pub enum Naming {
    New,
    Rename(usize),
    Copy(usize),
    /// a switch being loaded under a name another has, kept under its own
    KeepAs(Box<Pending>),
}

#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    Quit,
    Open(usize),
}

/// How the status line reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tone {
    Plain,
    Done,
    Problem,
}

pub struct App {
    /// the selected row among the switches shown
    pub selected: usize,
    pub mode: Mode,
    pub status: Option<String>,
    pub tone: Tone,
    /// shows only the switches whose name, hardware or release hold each word of it
    pub filter: Input,
    /// the list ordered by when each switch was last opened, newest first, rather than by name
    pub by_opened: bool,
    /// the keys of the screen in use, over it, and how far they are scrolled
    pub help: Option<usize>,
    /// the switches removed since vswitch started, the last last: `u` brings back each in turn
    removed: Vec<Removed>,
    /// where the file browser opens
    folder: Option<PathBuf>,
    /// the switch selected when the filter came to show none, which clearing it selects again
    hidden: Option<String>,
    /// from the last draw: the first switch row shown, the rows shown, the help's rows and the rows
    /// each key takes in it, too small
    offset: Cell<usize>,
    page: Cell<usize>,
    help_page: Cell<usize>,
    help_rows: Cell<Vec<usize>>,
    small: Cell<bool>,
}

/// Keys and what they do, for the help and the README; a list's fit one line at 80 columns.
type Keys = &'static [(&'static [&'static str], &'static str)];

const LIST_KEYS: Keys = &[
    (&["Up", "Down", "k", "j"], "select a switch"),
    (
        &["PgUp", "PgDn", "Home", "End"],
        "a page up or down, the first, the last",
    ),
    (&["Enter"], "open its console; `exit` there comes back"),
    (&["/"], "filter by name, model or release as it is typed"),
    (&["Esc"], "clear the filter; with none, quit"),
    (
        &["a", "Insert"],
        "load a design or a configuration from a file",
    ),
    (&["n"], "a new switch: its hostname, then its hardware"),
    (&["h"], "its hardware and release in the builder"),
    (&["r"], "rename it; the configuration's hostname follows"),
    (&["c"], "copy it under another name"),
    (&["v"], "read its kept configuration"),
    (&["e"], "export its kept configuration to a file"),
    (&["d", "Delete"], "remove it from the list, asked first"),
    (&["u"], "bring back the switch removed last"),
    (&["o"], "order by name or by last opened"),
    (&["s"], "sweep the sweep folder of secrets"),
    (&["?", "F1"], "the keys of the screen in use"),
    (&["q", "Ctrl-C"], "quit"),
];

const FILTER_KEYS: Keys = &[
    (
        &["type"],
        "narrows the list to the switches whose name, hardware or release hold each word",
    ),
    (&["Up", "Down", "PgUp", "PgDn"], "select a switch"),
    (&["Left", "Right", "Home", "End"], "move in the filter"),
    (&["Enter"], "keep the filter and go back to the list"),
    (&["Esc"], "clear the filter"),
];

const LOAD_KEYS: Keys = &[
    (
        &["type"],
        "narrow the folder's entries; a path lists its own folder, `~` is home",
    ),
    (&["Up", "Down", "PgUp", "PgDn"], "pick an entry"),
    (
        &["Enter"],
        "open the folder or load the file picked, or the path typed",
    ),
    (&["Tab"], "complete the name typed"),
    (
        &["Backspace", "Left"],
        "with nothing typed: up a folder (above a drive, the drives)",
    ),
    (&["Right"], "with nothing typed: into the folder picked"),
    (
        &["paste"],
        "a path pasted or dropped on the terminal goes in the field",
    ),
    (&["Esc"], "back to the list"),
];

const EXPORT_KEYS: Keys = &[
    (&["type"], "the name of the file to write, or a path"),
    (
        &["Up", "Down"],
        "pick a folder to open, or a file to take its name",
    ),
    (
        &["Enter"],
        "write the file; one already there is asked about first",
    ),
    (&["Tab"], "complete the name typed"),
    (&["Backspace"], "with nothing typed: up a folder"),
    (&["Esc"], "back to the list"),
];

const NAME_KEYS: Keys = &[
    (&["type"], "the name: one word"),
    (&["Left", "Right", "Home", "End"], "move in the name"),
    (&["Enter"], "keep it"),
    (&["Esc"], "back to the list"),
];

const REMOVE_KEYS: Keys = &[
    (
        &["y"],
        "remove the switch; u brings it back until vswitch closes",
    ),
    (&["any other key"], "keep it"),
];

const REPLACE_KEYS: Keys = &[
    (
        &["y"],
        "replace the listed switch with the one being loaded",
    ),
    (&["k"], "keep both, naming the one being loaded"),
    (&["any other key"], "leave the list as it is"),
];

const VIEW_KEYS: Keys = &[
    (&["Up", "Down", "k", "j"], "a line"),
    (&["PgUp", "PgDn", "b", "Space"], "a page"),
    (&["Home", "End", "g", "G"], "the first line, the last"),
    (&["Left", "Right"], "sideways"),
    (&["/"], "search, case ignored; Enter finds, Esc cancels"),
    (&["n", "N"], "the next match, the previous one"),
    (&["Esc", "q"], "back to the list"),
];

/// `IOS` for the families that run classic IOS, else `IOS XE`; the release decides for hardware
/// vswitch does not know.
fn os(stack: &str, release: Release) -> &'static str {
    let classic = platform::parse_stack(stack)
        .ok()
        .and_then(|s| s.family())
        .map_or(release.classic(), Family::classic_ios);
    if classic { "IOS" } else { "IOS XE" }
}

/// The release with its system: `IOS XE 17.15`, `IOS 15.2(7)E`.
pub fn os_release(stack: &str, release: Release) -> String {
    format!("{} {release}", os(stack, release))
}

/// The first member's model, a 9400's supervisor, and how many members besides: `C9300X-24Y +1`.
pub fn summary(stack: &str) -> String {
    match platform::parse_stack(stack) {
        Ok(s) if !s.members.is_empty() => {
            let lead = (s.family() == Some(Family::C9400))
                .then(|| s.members.iter().find(|m| m.model.contains("-SUP-")))
                .flatten()
                .unwrap_or(&s.members[0]);
            match s.members.len() {
                1 => lead.model.clone(),
                n => format!("{} +{}", lead.model, n - 1),
            }
        }
        _ => stack.to_string(),
    }
}

/// The members in one line, alike ones counted: `2 x C9300X-24Y+C9300X-NM-4C`; a 9400's by slot.
fn hardware_line(stack: &str) -> String {
    let Ok(s) = platform::parse_stack(stack) else {
        return stack.to_string();
    };
    let one = |m: &platform::Member| match &m.module {
        Some(md) => format!("{}+{md}", m.model),
        None => m.model.clone(),
    };
    if s.family() == Some(Family::C9400) {
        let slots: Vec<String> = s
            .members
            .iter()
            .map(|m| format!("{}={}", m.number, one(m)))
            .collect();
        return slots.join(", ");
    }
    let mut groups: Vec<(String, usize)> = Vec::new();
    for m in &s.members {
        let t = one(m);
        match groups.last_mut() {
            Some((g, n)) if *g == t => *n += 1,
            _ => groups.push((t, 1)),
        }
    }
    let parts: Vec<String> = groups
        .into_iter()
        .map(|(t, n)| if n == 1 { t } else { format!("{n} x {t}") })
        .collect();
    parts.join(", ")
}

/// The source of a switch built in vswitch, which no file holds.
const BUILT: &str = "built in vswitch";

/// What opening a switch says once its configuration is typed: how many lines, and from where.
fn loaded(s: &Saved, taken: usize) -> String {
    let lines = if taken == 1 { "line" } else { "lines" };
    if s.source == BUILT {
        format!("{}, {BUILT}: {taken} configuration {lines} loaded.", s.name)
    } else {
        format!(
            "{}: {taken} configuration {lines} loaded from {}.",
            s.name, s.source
        )
    }
}

/// What `Opening` prints before the grammar is read and the configuration typed.
pub fn opening(s: &Saved) -> String {
    format!(
        "Opening {} ({}, {})...",
        s.name,
        summary(&s.stack),
        os_release(&s.stack, s.release())
    )
}

/// The end of `text` that fits `width` columns, `…` marking what was cut.
fn fit_left(text: &str, width: usize) -> String {
    match width {
        _ if Span::raw(text).width() <= width => text.to_string(),
        0 => String::new(),
        w => format!("…{}", columns(text, w - 1, true)),
    }
}

/// The start of `text` that fits `width` columns, `…` marking what was cut.
fn fit_right(text: &str, width: usize) -> String {
    match width {
        _ if Span::raw(text).width() <= width => text.to_string(),
        0 => String::new(),
        w => format!("{}…", columns(text, w - 1, false)),
    }
}

/// The first `width` columns of `text`, or with `from_end` the last.
fn columns(text: &str, width: usize, from_end: bool) -> String {
    let mut kept: Vec<char> = Vec::new();
    let mut used = 0;
    let chars: Box<dyn Iterator<Item = char>> = if from_end {
        Box::new(text.chars().rev())
    } else {
        Box::new(text.chars())
    };
    for c in chars {
        let w = Span::raw(c.to_string()).width();
        if used + w > width {
            break;
        }
        used += w;
        kept.push(c);
    }
    if from_end {
        kept.reverse();
    }
    kept.into_iter().collect()
}

/// A popup `width` by `height` at most, centered in `area`.
fn popup_area(area: Rect, width: u16, height: u16) -> Rect {
    let [_, mid, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height.min(area.height.saturating_sub(2)).max(3)),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, c, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(width.min(area.width.saturating_sub(4)).max(10)),
        Constraint::Fill(1),
    ])
    .areas(mid);
    c
}

/// A name no listed switch has: `<base>-copy`, else `-copy2` and on.
fn free_name(lib: &Library, base: &str) -> String {
    (1..)
        .map(|n| match n {
            1 => format!("{base}-copy"),
            n => format!("{base}-copy{n}"),
        })
        .find(|n| lib.taken(n, None).is_none())
        .unwrap_or_else(|| base.to_string())
}

impl App {
    pub fn new() -> Self {
        App {
            selected: 0,
            mode: Mode::List,
            status: None,
            tone: Tone::Plain,
            filter: Input::default(),
            by_opened: false,
            help: None,
            removed: Vec::new(),
            folder: None,
            hidden: None,
            offset: Cell::new(0),
            page: Cell::new(10),
            help_page: Cell::new(10),
            help_rows: Cell::new(Vec::new()),
            small: Cell::new(false),
        }
    }

    /// The order and folder the list kept from the last run.
    pub fn remember(&mut self, prefs: &Prefs) {
        self.by_opened = prefs.by_opened;
        self.folder = prefs.folder.clone();
    }

    fn save_prefs(&mut self, lib: &Library) {
        let prefs = Prefs {
            folder: self.folder.clone(),
            by_opened: self.by_opened,
        };
        if let Err(e) = lib.set_prefs(&prefs) {
            self.fail(format!(
                "The list's order and folder could not be kept: {e}"
            ));
        }
    }

    fn note(&mut self, text: impl Into<String>) {
        self.status = Some(text.into());
        self.tone = Tone::Plain;
    }

    fn done(&mut self, text: impl Into<String>) {
        self.status = Some(text.into());
        self.tone = Tone::Done;
    }

    fn fail(&mut self, text: impl Into<String>) {
        self.status = Some(text.into());
        self.tone = Tone::Problem;
    }

    /// The switches shown, by place in the list: those the filter keeps, by name or by last opened.
    pub fn rows(&self, lib: &Library) -> Vec<usize> {
        let words: Vec<String> = self
            .filter
            .text
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let mut rows: Vec<usize> = (0..lib.switches.len())
            .filter(|&i| {
                let s = &lib.switches[i];
                words.is_empty() || {
                    let hay = format!("{} {} {}", s.name, s.stack, s.release()).to_lowercase();
                    words.iter().all(|w| hay.contains(w))
                }
            })
            .collect();
        if self.by_opened {
            // never opened last; the sort is stable, so ties stay by name
            rows.sort_by_key(|&i| std::cmp::Reverse(lib.switches[i].opened_at));
        }
        rows
    }

    /// The selected switch, by place in the list.
    pub fn current(&self, lib: &Library) -> Option<usize> {
        let rows = self.rows(lib);
        rows.get(self.selected.min(rows.len().saturating_sub(1)))
            .copied()
    }

    /// Selects a listed switch, clearing a filter that hides it.
    fn select_index(&mut self, lib: &Library, i: usize) {
        let mut rows = self.rows(lib);
        if !rows.contains(&i) {
            self.filter = Input::default();
            rows = self.rows(lib);
        }
        self.selected = rows.iter().position(|&j| j == i).unwrap_or(0);
    }

    pub fn select_name(&mut self, lib: &Library, name: &str) {
        if let Some(i) = lib.position(name) {
            self.select_index(lib, i);
        }
    }

    /// The selection on the same switch after the rows change, else on the first row. While the
    /// filter shows no switch, the one selected before is remembered and selected once it shows.
    fn reselect(&mut self, lib: &Library, was: Option<usize>) {
        let rows = self.rows(lib);
        let want = was.or_else(|| self.hidden.as_deref().and_then(|n| lib.position(n)));
        if rows.is_empty() {
            if let Some(i) = want {
                self.hidden = Some(lib.switches[i].name.clone());
            }
            self.selected = 0;
            return;
        }
        self.hidden = None;
        self.selected = want
            .and_then(|i| rows.iter().position(|&j| j == i))
            .unwrap_or(0);
    }

    fn move_by(&mut self, lib: &Library, by: isize) {
        let n = self.rows(lib).len() as isize;
        self.selected = (self.selected.min(n.max(1) as usize - 1) as isize + by)
            .clamp(0, (n - 1).max(0)) as usize;
    }

    fn keys(&self) -> (&'static str, Keys) {
        match &self.mode {
            Mode::List | Mode::Hardware { .. } => ("the switch list", LIST_KEYS),
            Mode::Filter => ("the filter", FILTER_KEYS),
            Mode::Browse(b) if b.goal == Goal::Load => ("the file browser", LOAD_KEYS),
            Mode::Browse(_) => ("the export", EXPORT_KEYS),
            Mode::Name { .. } => ("the name", NAME_KEYS),
            Mode::Remove => ("removing a switch", REMOVE_KEYS),
            Mode::Replace(_) => ("replacing a switch", REPLACE_KEYS),
            Mode::View(_) => ("the configuration", VIEW_KEYS),
        }
    }

    pub fn on_key(&mut self, key: KeyEvent, lib: &mut Library) -> Action {
        // Ctrl-C quits from the list and cancels anything open over it, as Esc does, so a typed path
        // or a stack being built is not lost with the program
        let key = if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
        {
            if self.on_list() || self.small.get() {
                return Action::Quit;
            }
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        } else {
            key
        };
        // nothing acts unseen while the terminal is too small to draw the list; Ctrl-C quits above
        if self.small.get() {
            return Action::None;
        }
        if let Some(scroll) = self.help {
            let rows = self.help_rows.take();
            self.help = help_scroll(scroll, key.code, &rows, self.help_page.get().max(1));
            self.help_rows.set(rows);
            return Action::None;
        }
        // the builder reads its own keys
        if !matches!(self.mode, Mode::Hardware { .. })
            && matches!(key.code, KeyCode::F(1) | KeyCode::Char('?'))
        {
            self.help = Some(0);
            return Action::None;
        }
        if let Mode::Hardware { .. } = self.mode {
            self.tone = Tone::Plain;
        }
        match &mut self.mode {
            Mode::List => self.list_key(key, lib),
            Mode::Filter => {
                let was = self.current(lib);
                match key.code {
                    KeyCode::Enter => self.mode = Mode::List,
                    KeyCode::Esc => {
                        self.filter = Input::default();
                        self.mode = Mode::List;
                        self.reselect(lib, was);
                    }
                    KeyCode::Up => self.move_by(lib, -1),
                    KeyCode::Down => self.move_by(lib, 1),
                    KeyCode::PageUp => self.move_by(lib, -(self.page.get().max(1) as isize)),
                    KeyCode::PageDown => self.move_by(lib, self.page.get().max(1) as isize),
                    _ => {
                        if self.filter.key(key) {
                            self.offset.set(0);
                            self.reselect(lib, was);
                        }
                    }
                }
                Action::None
            }
            Mode::Browse(b) => {
                let step = b.key(key);
                self.browsed(step, lib)
            }
            Mode::Name { .. } => self.name_key(key, lib),
            Mode::Remove => {
                if key.code == KeyCode::Char('y')
                    && let Some(i) = self.current(lib)
                {
                    match lib.remove(i) {
                        Ok(r) => {
                            self.note(format!(
                                "Removed {} from the list. u brings it back.",
                                r.saved.name
                            ));
                            self.removed.push(r);
                        }
                        Err(e) => self.fail(e),
                    }
                }
                self.mode = Mode::List;
                Action::None
            }
            Mode::Replace(_) => {
                let Mode::Replace(p) = std::mem::replace(&mut self.mode, Mode::List) else {
                    return Action::None;
                };
                match key.code {
                    KeyCode::Char('y') => self.add_kept(lib, &p),
                    KeyCode::Char('k') => {
                        let input = Input::new(&free_name(lib, &p.name));
                        self.mode = Mode::Name {
                            naming: Naming::KeepAs(p),
                            input,
                            error: None,
                        };
                        Action::None
                    }
                    _ => {
                        self.note(format!("Nothing was loaded; {} stays as it was.", p.name));
                        Action::None
                    }
                }
            }
            Mode::View(v) => {
                if !v.key(key) {
                    self.mode = Mode::List;
                }
                Action::None
            }
            Mode::Hardware { builder, .. } => {
                // an open list takes Enter and Esc, and the builder takes Enter while a problem is
                // marked, saying so; what it leaves keeps the hardware or cancels
                if builder.key(key) {
                    return Action::None;
                }
                match key.code {
                    KeyCode::Esc => {
                        self.mode = Mode::List;
                        Action::None
                    }
                    KeyCode::Enter if builder.problems().is_empty() => {
                        let Mode::Hardware {
                            name,
                            purpose,
                            builder,
                        } = std::mem::replace(&mut self.mode, Mode::List)
                        else {
                            return Action::None;
                        };
                        let spec = builder.spec();
                        let r = builder.release;
                        match purpose {
                            Purpose::Import { source, config } => {
                                self.keep(lib, &name, &spec, r, &source, &config)
                            }
                            Purpose::New => self.keep(
                                lib,
                                &name,
                                &spec,
                                r,
                                BUILT,
                                &format!("hostname {name}\n"),
                            ),
                            Purpose::Edit(i) => {
                                let s = &lib.switches[i];
                                let had = platform::parse_stack(&s.stack)
                                    .map_or(s.stack.clone(), |st| st.spec());
                                if had == spec && s.release() == r {
                                    self.note(format!("{name} stays {spec} on {}.", r.named()));
                                } else {
                                    match lib.set_hardware(i, &spec, r) {
                                        Ok(()) => self.done(format!(
                                            "{name} is now {spec} on {}.",
                                            r.named()
                                        )),
                                        Err(e) => self.fail(e),
                                    }
                                }
                                Action::None
                            }
                        }
                    }
                    _ => Action::None,
                }
            }
        }
    }

    /// Whether keys go to the list itself: no popup, no help, the terminal large enough.
    fn on_list(&self) -> bool {
        matches!(self.mode, Mode::List) && self.help.is_none() && !self.small.get()
    }

    /// Whether a key on the list opens a field to type into.
    fn opens_field(&self, c: char, lib: &Library) -> bool {
        match c {
            'a' | 'n' | '/' => true,
            'r' | 'c' | 'e' => self.current(lib).is_some(),
            _ => false,
        }
    }

    fn list_key(&mut self, key: KeyEvent, lib: &mut Library) -> Action {
        // a list command is a plain key: Alt-q is no q (AltGr, Ctrl and Alt, types into fields)
        if key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
        {
            return Action::None;
        }
        self.status = None;
        self.tone = Tone::Plain;
        let n = self.rows(lib).len();
        self.selected = self.selected.min(n.saturating_sub(1));
        let page = self.page.get().max(1) as isize;
        let current = self.current(lib);
        let named = current.map(|i| lib.switches[i].name.clone());
        match (key.code, current, named) {
            (KeyCode::Up | KeyCode::Char('k'), ..) => self.move_by(lib, -1),
            (KeyCode::Down | KeyCode::Char('j'), ..) => self.move_by(lib, 1),
            (KeyCode::PageUp, ..) => self.move_by(lib, -page),
            (KeyCode::PageDown, ..) => self.move_by(lib, page),
            (KeyCode::Home, ..) => self.selected = 0,
            (KeyCode::End, ..) => self.selected = n.saturating_sub(1),
            (KeyCode::Enter, Some(i), _) => return Action::Open(i),
            (KeyCode::Char('/'), ..) => self.mode = Mode::Filter,
            (KeyCode::Esc, ..) if !self.filter.is_empty() => {
                self.filter = Input::default();
                self.reselect(lib, current);
            }
            (KeyCode::Char('q') | KeyCode::Esc, ..) => return Action::Quit,
            (KeyCode::Char('a') | KeyCode::Insert, ..) => {
                self.mode = Mode::Browse(Box::new(Browser::new(Goal::Load, self.folder.clone())))
            }
            (KeyCode::Char('n'), ..) => {
                self.mode = Mode::Name {
                    naming: Naming::New,
                    input: Input::default(),
                    error: None,
                }
            }
            (KeyCode::Char('h'), Some(i), _) => {
                let s = &lib.switches[i];
                self.mode = Mode::Hardware {
                    name: s.name.clone(),
                    purpose: Purpose::Edit(i),
                    builder: Builder::from_spec(&s.stack, s.release()),
                }
            }
            (KeyCode::Char('r'), Some(i), Some(name)) => {
                self.mode = Mode::Name {
                    naming: Naming::Rename(i),
                    input: Input::new(&name),
                    error: None,
                }
            }
            (KeyCode::Char('c'), Some(i), Some(name)) => {
                self.mode = Mode::Name {
                    naming: Naming::Copy(i),
                    input: Input::new(&free_name(lib, &name)),
                    error: None,
                }
            }
            (KeyCode::Char('v'), _, Some(name)) => match lib.config(&name) {
                Ok(text) => {
                    let path = browser::shown(&lib.config_path(&name));
                    self.mode = Mode::View(Box::new(Viewer::new(&name, &path, &text)));
                }
                Err(e) => self.fail(e),
            },
            (KeyCode::Char('e'), _, Some(name)) => {
                let goal = Goal::Export {
                    file: library::file_name(&name),
                    name,
                };
                self.mode = Mode::Browse(Box::new(Browser::new(goal, self.folder.clone())));
            }
            (KeyCode::Char('d') | KeyCode::Delete, Some(_), _) => self.mode = Mode::Remove,
            (KeyCode::Char('u'), ..) => self.undo(lib),
            (KeyCode::Char('o'), ..) => {
                self.by_opened = !self.by_opened;
                self.reselect(lib, current);
                self.offset.set(0);
                self.note(if self.by_opened {
                    "Ordered by last opened, newest first."
                } else {
                    "Ordered by name."
                });
                self.save_prefs(lib);
            }
            (KeyCode::Char('s'), ..) => {
                let (text, tone) = sweep_status(&lib.dir.join("sweep"));
                self.status = Some(text);
                self.tone = tone;
            }
            // a key for the selected switch, with none shown
            (
                KeyCode::Enter | KeyCode::Delete | KeyCode::Char('h' | 'd' | 'r' | 'c' | 'v' | 'e'),
                None,
                _,
            ) => self.note(if lib.switches.is_empty() {
                "No switch yet: a loads one, n builds one."
            } else {
                "No switch matches the filter: Esc clears it."
            }),
            _ => {}
        }
        Action::None
    }

    fn undo(&mut self, lib: &mut Library) {
        let Some(r) = self.removed.pop() else {
            self.note("No removed switch is left to bring back.");
            return;
        };
        let name = r.saved.name.clone();
        match lib.restore(r.clone()) {
            Ok(i) => {
                self.select_index(lib, i);
                self.done(format!("{name} is back in the list."));
            }
            Err(e) => {
                self.removed.push(r);
                self.fail(e);
            }
        }
    }

    fn name_key(&mut self, key: KeyEvent, lib: &mut Library) -> Action {
        let Mode::Name {
            naming,
            mut input,
            mut error,
        } = std::mem::replace(&mut self.mode, Mode::List)
        else {
            return Action::None;
        };
        if key.code == KeyCode::Esc {
            return Action::None;
        }
        if key.code != KeyCode::Enter {
            if input.key(key) {
                error = None;
            }
            self.mode = Mode::Name {
                naming,
                input,
                error,
            };
            return Action::None;
        }
        let name = input.text.trim().to_string();
        let outcome = match &naming {
            Naming::New => match library::hostname_problem(&name) {
                Some(why) => Err(why.to_string()),
                None => {
                    self.mode = Mode::Hardware {
                        name,
                        purpose: Purpose::New,
                        builder: Builder::new(),
                    };
                    return Action::None;
                }
            },
            Naming::Rename(i) => {
                let old = lib.switches[*i].name.clone();
                lib.rename(*i, &name).map(|j| {
                    self.select_index(lib, j);
                    if old != name {
                        self.done(format!("Renamed {old} to {name}."));
                    }
                })
            }
            Naming::Copy(i) => {
                let from = lib.switches[*i].name.clone();
                lib.duplicate(*i, &name).map(|j| {
                    self.select_index(lib, j);
                    self.done(format!("Copied {from} to {name}."));
                })
            }
            Naming::KeepAs(p) => match lib.check_name(&name, None) {
                Ok(()) => {
                    let kept = Pending {
                        config: library::with_hostname(&p.config, &name),
                        name,
                        ..(**p).clone()
                    };
                    return self.add_kept(lib, &kept);
                }
                Err(e) => Err(e),
            },
        };
        if let Err(e) = outcome {
            self.mode = Mode::Name {
                naming,
                input,
                error: Some(e),
            };
        }
        Action::None
    }

    /// What the file browser's key asks for: a file loaded, a configuration exported.
    fn browsed(&mut self, step: Step, lib: &mut Library) -> Action {
        match step {
            Step::Stay => Action::None,
            Step::Cancel => {
                self.mode = Mode::List;
                Action::None
            }
            Step::Load(path) => match crate::library::import(&path) {
                Ok(read) => {
                    self.folder = path.parent().map(Path::to_path_buf);
                    self.save_prefs(lib);
                    self.mode = Mode::List;
                    self.imported(lib, read, path.display().to_string())
                }
                Err(e) => {
                    if let Mode::Browse(b) = &mut self.mode {
                        b.error = Some(e);
                    }
                    Action::None
                }
            },
            Step::Write(path) => {
                let Mode::Browse(b) = &mut self.mode else {
                    return Action::None;
                };
                let Goal::Export { name, .. } = &b.goal else {
                    return Action::None;
                };
                let name = name.clone();
                match lib.config(&name).and_then(|t| {
                    std::fs::write(&path, t).map_err(|e| format!("{}: {e}", path.display()))
                }) {
                    Ok(()) => {
                        self.folder = path.parent().map(Path::to_path_buf);
                        self.mode = Mode::List;
                        self.done(format!(
                            "Wrote {name}'s kept configuration to {}.",
                            browser::shown(&path)
                        ));
                        self.save_prefs(lib);
                    }
                    Err(e) => b.error = Some(e),
                }
                Action::None
            }
        }
    }

    /// A file read for loading: kept when its hardware is known and built, else to the builder.
    fn imported(&mut self, lib: &mut Library, read: Import, source: String) -> Action {
        match read {
            Import::Ready {
                name,
                stack,
                release,
                config,
            } => {
                let r = first_release(release.as_deref(), &stack);
                self.keep(lib, &name, &stack, r, &source, &config)
            }
            Import::NeedsStack {
                name,
                inferred,
                release,
                config,
            } => {
                let builder = match inferred.as_deref() {
                    Some(spec) => Builder::from_spec(spec, first_release(release.as_deref(), spec)),
                    None => Builder::new(),
                };
                self.mode = Mode::Hardware {
                    name,
                    purpose: Purpose::Import { source, config },
                    builder,
                };
                Action::None
            }
        }
    }

    /// Pasted text (a path dropped onto the terminal) goes into the field being typed; on the list it
    /// opens the file browser with it.
    pub fn on_paste(&mut self, text: &str) {
        if self.help.is_some() || self.small.get() {
            return;
        }
        match &mut self.mode {
            Mode::Browse(b) => b.paste(text),
            Mode::Name { input, error, .. } => {
                input.paste(text.trim());
                *error = None;
            }
            Mode::Filter => self.filter.paste(text),
            Mode::View(v) => v.paste(text),
            Mode::List => {
                let mut b = Browser::new(Goal::Load, self.folder.clone());
                b.paste(text);
                self.mode = Mode::Browse(Box::new(b));
            }
            Mode::Hardware { .. } | Mode::Remove | Mode::Replace(_) => {}
        }
    }

    fn keep(
        &mut self,
        lib: &mut Library,
        name: &str,
        stack: &str,
        release: Release,
        source: &str,
        config: &str,
    ) -> Action {
        // a design may name hardware Cisco does not build or a release it does not run: the builder
        // shows why
        let builder = Builder::from_spec(stack, release);
        if platform::parse_stack(stack).is_err() || !builder.problems().is_empty() {
            self.mode = Mode::Hardware {
                name: name.to_string(),
                purpose: Purpose::Import {
                    source: source.to_string(),
                    config: config.to_string(),
                },
                builder,
            };
            return Action::None;
        }
        let pending = Pending {
            name: name.to_string(),
            stack: stack.to_string(),
            release,
            source: source.to_string(),
            config: config.to_string(),
        };
        // a listed switch is replaced only once that is confirmed
        if lib.taken(name, None).is_some() {
            self.mode = Mode::Replace(Box::new(pending));
            return Action::None;
        }
        self.add_kept(lib, &pending)
    }

    fn add_kept(&mut self, lib: &mut Library, p: &Pending) -> Action {
        self.mode = Mode::List;
        match lib.add(&p.name, &p.stack, p.release, &p.source, &p.config) {
            Ok(i) => {
                self.select_index(lib, i);
                Action::Open(i)
            }
            Err(e) => {
                self.fail(e);
                Action::None
            }
        }
    }

    pub fn draw(&self, f: &mut Frame, lib: &Library) {
        let area = f.area();
        self.small
            .set(area.width < MIN_WIDTH || area.height < MIN_HEIGHT);
        if self.small.get() {
            let text = vec![
                Line::from(format!("The terminal is {}x{}.", area.width, area.height)),
                Line::from(format!("The switch list needs {MIN_WIDTH}x{MIN_HEIGHT}.")),
                Line::styled("Ctrl-C quits.", HINT),
            ];
            let [_, mid, _] = Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(3),
                Constraint::Fill(1),
            ])
            .areas(area);
            f.render_widget(
                Paragraph::new(text)
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: true }),
                mid,
            );
            return;
        }
        // a long status takes a second line
        let room = area.width.saturating_sub(2) as usize;
        let status_lines = self
            .status
            .as_ref()
            .map_or(1, |s| wrap(s.trim_start(), room).len().clamp(1, 2) as u16);
        let [title, body, status, hints] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(status_lines),
            Constraint::Length(1),
        ])
        .areas(area);
        self.draw_title(f, title, lib);
        match &self.mode {
            Mode::View(v) => v.draw(f, body),
            Mode::Browse(b) => b.draw(f, body),
            // the builder takes the list's place
            Mode::Hardware { .. } => {}
            _ => self.draw_list(f, body, lib),
        }
        if let Some(s) = &self.status {
            let style = match self.tone {
                Tone::Problem => PROBLEM,
                Tone::Done => DONE,
                Tone::Plain => Style::new(),
            };
            // what happened is at a status's start, the file or the reason at its end
            let lines: Vec<Line> = wrapped(s.trim_start(), room, status_lines as usize)
                .into_iter()
                .map(|l| Line::styled(l, style))
                .collect();
            f.render_widget(
                Paragraph::new(lines).block(Block::new().padding(Padding::horizontal(1))),
                status,
            );
        }
        f.render_widget(Paragraph::new(self.hint_line(lib, hints.width)), hints);
        match &self.mode {
            Mode::Name {
                naming,
                input,
                error,
            } => self.draw_name(f, body, lib, naming, input, error.as_deref()),
            Mode::Hardware { name, builder, .. } => {
                builder.draw(f, body, &format!(" Hardware for {name} "))
            }
            Mode::Remove => {
                if let Some(i) = self.current(lib) {
                    let s = &lib.switches[i];
                    confirm(
                        f,
                        body,
                        &format!(" Remove {}? ", s.name),
                        vec![Line::from(format!(
                            "{} leaves the list with its kept configuration; the file it came from stays. u brings it back until vswitch closes.",
                            s.name
                        ))],
                        &[("y", "remove it"), ("any other key", "keep it")],
                    );
                }
            }
            Mode::Replace(p) => {
                let listed = lib.taken(&p.name, None).map(|j| &lib.switches[j]);
                let mut lines = Vec::new();
                if let Some(s) = listed {
                    let saved = s
                        .saved_at
                        .map(|t| format!(", saved {}", when(t)))
                        .unwrap_or_default();
                    lines.push(Line::from(format!(
                        "{} is in the list: {}, {}{saved}.",
                        s.name,
                        summary(&s.stack),
                        os_release(&s.stack, s.release())
                    )));
                }
                let what = if p.source == BUILT {
                    "The new switch".to_string()
                } else {
                    Path::new(&p.source)
                        .file_name()
                        .map_or(p.source.clone(), |n| n.to_string_lossy().into_owned())
                };
                lines.push(Line::from(format!(
                    "{what} replaces it, kept configuration and all."
                )));
                confirm(
                    f,
                    body,
                    &format!(" Replace {}? ", p.name),
                    lines,
                    &[
                        ("y", "replace it"),
                        ("k", "keep both"),
                        ("any other key", "leave the list as it is"),
                    ],
                );
            }
            Mode::List | Mode::Filter | Mode::View(_) | Mode::Browse(_) => {}
        }
        if let Some(scroll) = self.help {
            self.draw_help(f, body, scroll);
        }
    }

    fn draw_title(&self, f: &mut Frame, area: Rect, lib: &Library) {
        let left = Line::from(vec![
            Span::styled(" vswitch ", PICK.add_modifier(Modifier::BOLD)),
            Span::raw("  virtual Catalyst switches"),
        ]);
        let room = (area.width as usize).saturating_sub(left.width() + 4);
        f.render_widget(Paragraph::new(left), area);
        if room > 12 {
            let place = fit_left(&browser::shown(&lib.dir), room);
            f.render_widget(
                Paragraph::new(Line::styled(format!("{place} "), HINT).right_aligned()),
                area,
            );
        }
    }

    fn draw_list(&self, f: &mut Frame, area: Rect, lib: &Library) {
        let rows = self.rows(lib);
        let current = self.current(lib);
        let side = area.width >= SIDE_BY_SIDE;
        // a popup over the list stands in the list's frame, with no details box for it to cross
        let popup = matches!(
            self.mode,
            Mode::Name { .. } | Mode::Remove | Mode::Replace(_)
        );
        let (list, detail) = match current {
            None => (area, None),
            Some(_) if popup => (area, None),
            Some(i) if side => {
                let [l, r] =
                    Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
                        .areas(area);
                let lines = details(lib, &lib.switches[i], true, r.width.saturating_sub(2));
                (l, Some((r, lines)))
            }
            Some(i) => {
                let lines = details(lib, &lib.switches[i], false, area.width.saturating_sub(2));
                // the list keeps room for four switches
                let want = lines.len() as u16 + 2;
                let room = area.height.saturating_sub(7);
                if room < 3 {
                    (area, None)
                } else {
                    let [l, r] =
                        Layout::vertical([Constraint::Min(7), Constraint::Length(want.min(room))])
                            .areas(area);
                    (l, Some((r, lines)))
                }
            }
        };
        let total = lib.switches.len();
        let order = if self.by_opened {
            "by last opened"
        } else {
            "by name"
        };
        let count = if self.filter.is_empty() {
            format!(" {total}, {order} ")
        } else {
            format!(" {} of {total}, {order} ", rows.len())
        };
        let block = Block::bordered()
            .title(Span::styled(" Switches ", BOLD))
            .title_top(Line::from(count).right_aligned());
        let inner = block.inner(list);
        f.render_widget(block, list);
        let filtering = self.mode == Mode::Filter || !self.filter.is_empty();
        let (table, filter) = if filtering {
            let [t, b] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
            (t, Some(b))
        } else {
            (inner, None)
        };
        if lib.switches.is_empty() {
            let text = vec![
                Line::from("No switches yet."),
                Line::default(),
                Line::from(
                    "a loads a design (.toml) or a configuration file; n builds a new switch from its hostname.",
                ),
                Line::styled(
                    "Configurations are swept of passwords and secrets before they are kept.",
                    HINT,
                ),
            ];
            f.render_widget(
                Paragraph::new(text)
                    .wrap(Wrap { trim: true })
                    .block(Block::new().padding(Padding::horizontal(1))),
                table,
            );
        } else if rows.is_empty() {
            f.render_widget(
                Paragraph::new(Line::styled(
                    format!(" No switch matches {}.", self.filter.text.trim()),
                    HINT,
                )),
                table,
            );
        } else {
            self.draw_rows(f, table, lib, &rows);
        }
        if let Some(b) = filter {
            let typing = self.mode == Mode::Filter;
            let room = (b.width as usize).saturating_sub(10);
            let (text, col) = self.filter.window(room);
            let mut spans = vec![Span::styled(" Filter  ", HINT), Span::raw(text)];
            if typing {
                f.set_cursor_position((b.x + 9 + col as u16, b.y));
            } else {
                spans.push(Span::styled("   / changes it, Esc clears it", HINT));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), b);
        }
        if let (Some((area, lines)), Some(i)) = (detail, current) {
            let title = Span::styled(format!(" {} ", lib.switches[i].name), BOLD);
            f.render_widget(
                Paragraph::new(lines).block(Block::bordered().title(title)),
                area,
            );
        }
    }

    fn draw_rows(&self, f: &mut Frame, area: Rect, lib: &Library, rows: &[usize]) {
        let date = |t: Option<u64>| t.map(|t| when(t)[..10].to_string()).unwrap_or_default();
        let cells: Vec<[String; 5]> = rows
            .iter()
            .map(|&i| {
                let s = &lib.switches[i];
                [
                    s.name.clone(),
                    summary(&s.stack),
                    s.release().to_string(),
                    date(s.saved_at),
                    date(s.opened_at),
                ]
            })
            .collect();
        let widest = |k: usize, least: usize, most: usize| {
            cells
                .iter()
                .map(|c| Span::raw(c[k].as_str()).width())
                .max()
                .unwrap_or(0)
                .clamp(least, most) as u16
        };
        // the columns that fit, dropped from the last, a date no switch has before any; the date the
        // list is ordered by goes first
        let (saved, opened) = (3, 4);
        let mut columns: Vec<(usize, &str, u16)> = vec![
            (1, "Hardware", widest(1, 8, 24)),
            (2, "Release", widest(2, 7, 12)),
        ];
        if self.by_opened {
            columns.push((opened, "Opened", 10));
            columns.push((saved, "Saved", 10));
        } else {
            columns.push((saved, "Saved", 10));
            columns.push((opened, "Opened", 10));
        }
        let room = area.width.saturating_sub(2);
        let name_width = widest(0, 4, 32);
        let blank = |k: usize| cells.iter().all(|c| c[k].is_empty());
        while !columns.is_empty() {
            let used: u16 = columns.iter().map(|c| c.2 + 1).sum();
            if used + name_width.min(16) <= room {
                break;
            }
            match columns.iter().rposition(|c| blank(c.0)) {
                Some(at) => {
                    columns.remove(at);
                }
                None => {
                    columns.pop();
                }
            }
        }
        let used: u16 = columns.iter().map(|c| c.2 + 1).sum();
        let name_width = name_width.min(room.saturating_sub(used));
        let mut widths = vec![Constraint::Length(name_width)];
        widths.extend(columns.iter().map(|c| Constraint::Length(c.2)));
        let mut header = vec![TableCell::from("Name")];
        header.extend(columns.iter().map(|c| TableCell::from(c.1)));
        let body: Vec<Row> = cells
            .iter()
            .map(|c| {
                let mut r = vec![TableCell::from(fit_right(&c[0], name_width as usize))];
                r.extend(columns.iter().map(|col| TableCell::from(c[col.0].clone())));
                Row::new(r)
            })
            .collect();
        self.page.set(area.height.saturating_sub(1) as usize);
        let mut state = TableState::default()
            .with_offset(self.offset.get())
            .with_selected(Some(self.selected.min(rows.len() - 1)));
        f.render_stateful_widget(
            Table::new(body, widths)
                .header(Row::new(header).style(BOLD))
                .row_highlight_style(PICK)
                .highlight_symbol("> ")
                .highlight_spacing(HighlightSpacing::Always)
                .flex(Flex::Start),
            area,
            &mut state,
        );
        self.offset.set(state.offset());
    }

    fn hint_line(&self, lib: &Library, width: u16) -> Line<'static> {
        let mut items: Vec<(&str, &str)> = match &self.mode {
            Mode::List if lib.switches.is_empty() => {
                vec![("a", "load"), ("n", "new"), ("s", "sweep")]
            }
            Mode::List => {
                let mut v = vec![("Enter", "open"), ("/", "filter")];
                if !self.filter.is_empty() {
                    v.push(("Esc", "clear filter"));
                }
                if !self.removed.is_empty() {
                    v.push(("u", "undo"));
                }
                v.extend([
                    ("a", "load"),
                    ("n", "new"),
                    ("h", "hardware"),
                    ("d", "remove"),
                    ("r", "rename"),
                    ("c", "copy"),
                    ("v", "view"),
                    ("e", "export"),
                    ("o", "order"),
                    ("s", "sweep"),
                ]);
                v
            }
            Mode::Filter => vec![("↑↓", "select"), ("Enter", "done"), ("Esc", "clear")],
            Mode::Browse(b) if b.ask.is_some() => {
                vec![("y", "write over it"), ("any other key", "keeps the file")]
            }
            Mode::Browse(b) if b.goal == Goal::Load => vec![
                ("Enter", "open"),
                ("Tab", "complete"),
                ("Backspace", "up a folder"),
                ("Esc", "cancel"),
            ],
            Mode::Browse(_) => vec![
                ("Enter", "write"),
                ("↑↓", "pick"),
                ("Backspace", "up a folder"),
                ("Esc", "cancel"),
            ],
            Mode::Name { .. } => vec![("Enter", "keep"), ("Esc", "cancel")],
            Mode::Remove => vec![("y", "remove"), ("any other key", "keeps it")],
            Mode::Replace(_) => vec![
                ("y", "replace"),
                ("k", "keep both"),
                ("any other key", "cancels"),
            ],
            Mode::View(v) if v.typing.is_some() => vec![("Enter", "find"), ("Esc", "cancel")],
            Mode::View(_) => vec![
                ("↑↓", "scroll"),
                ("/", "search"),
                ("n N", "next, previous"),
                ("Esc", "back"),
            ],
            // the builder shows its own keys
            Mode::Hardware { .. } => return Line::default(),
        };
        let mut tail = vec![("?", "help")];
        if self.mode == Mode::List {
            tail.push(("q", "quit"));
        }
        // each item is its key, a space and what it does, with two spaces before the next
        const GAP: usize = 2;
        let size = |(k, l): &(&str, &str)| Span::raw(*k).width() + 1 + Span::raw(*l).width() + GAP;
        let mut room =
            (width as usize + GAP).saturating_sub(1 + tail.iter().map(size).sum::<usize>());
        let mut shown = Vec::new();
        for item in items.drain(..) {
            if size(&item) > room {
                break;
            }
            room -= size(&item);
            shown.push(item);
        }
        shown.extend(tail);
        let mut spans = vec![Span::raw(" ")];
        for (n, (k, l)) in shown.iter().enumerate() {
            if n > 0 {
                spans.push(Span::raw(" ".repeat(GAP)));
            }
            spans.push(Span::styled(k.to_string(), BOLD));
            spans.push(Span::styled(format!(" {l}"), HINT));
        }
        Line::from(spans)
    }

    fn draw_name(
        &self,
        f: &mut Frame,
        body: Rect,
        lib: &Library,
        naming: &Naming,
        input: &Input,
        error: Option<&str>,
    ) {
        let (title, prompt) = match naming {
            Naming::New => (
                " New switch ".to_string(),
                "Its hostname; the hardware comes next.".to_string(),
            ),
            Naming::Rename(i) => (
                format!(" Rename {} ", lib.switches[*i].name),
                "The new name; the hostname line of its kept configuration follows it.".into(),
            ),
            Naming::Copy(i) => (
                format!(" Copy {} ", lib.switches[*i].name),
                format!(
                    "The copy's name; it takes {}'s hardware, release and configuration.",
                    lib.switches[*i].name
                ),
            ),
            Naming::KeepAs(p) => (
                " Keep both ".to_string(),
                format!(
                    "A name for the switch being loaded; {} stays as it is.",
                    p.name
                ),
            ),
        };
        let except = match naming {
            Naming::Rename(i) => Some(*i),
            _ => None,
        };
        let typed = input.text.trim();
        let warning = match (error, lib.taken(typed, except)) {
            (Some(e), _) => Some(Line::styled(e.to_string(), PROBLEM)),
            (None, Some(j)) if matches!(naming, Naming::New) => Some(Line::styled(
                format!(
                    "{} is in the list: keeping a new switch under this name replaces it.",
                    lib.switches[j].name
                ),
                NOTE,
            )),
            (None, Some(j)) => Some(Line::styled(
                format!(
                    "A switch named {} is already in the list.",
                    lib.switches[j].name
                ),
                NOTE,
            )),
            (None, None) => None,
        };
        let area = popup_area(body, 72, 10);
        f.render_widget(Clear, area);
        let block = Block::bordered()
            .title(Span::styled(title, BOLD))
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [p, field, _, message] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .areas(inner);
        f.render_widget(Paragraph::new(prompt).wrap(Wrap { trim: true }), p);
        let room = (field.width as usize).saturating_sub(7);
        let (text, col) = input.window(room);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Name   ", HINT),
                Span::raw(text),
            ])),
            field,
        );
        f.set_cursor_position((field.x + 7 + col as u16, field.y));
        if let Some(w) = warning {
            f.render_widget(Paragraph::new(w).wrap(Wrap { trim: true }), message);
        }
    }

    /// The keys of the screen in use, over the list.
    fn draw_help(&self, f: &mut Frame, area: Rect, scroll: usize) {
        let (screen, keys) = self.keys();
        f.render_widget(Clear, area);
        let block = Block::bordered()
            .title(Span::styled(format!(" Keys: {screen} "), BOLD))
            .title_bottom(Line::styled(" Esc closes ", HINT).right_aligned())
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let key_width = keys
            .iter()
            .map(|(k, _)| Span::raw(k.join("  ")).width())
            .max()
            .unwrap_or(0)
            .min(24) as u16;
        let what_width = inner.width.saturating_sub(key_width + 3) as usize;
        let page = inner.height as usize;
        let wrapped: Vec<Vec<String>> = keys
            .iter()
            .map(|(_, what)| wrap(&what.replace('`', ""), what_width))
            .collect();
        let heights: Vec<usize> = wrapped.iter().map(Vec::len).collect();
        // a terminal grown since the help was scrolled shows as many keys as it holds
        let scroll = scroll.min(help_most(&heights, page));
        self.help_page.set(page);
        self.help_rows.set(heights);
        let mut used = 0;
        let rows: Vec<Row> = keys
            .iter()
            .zip(wrapped)
            .skip(scroll)
            .map(|((k, _), lines)| {
                let height = lines.len() as u16;
                used += lines.len();
                Row::new(vec![
                    TableCell::from(k.join("  ")).style(BOLD),
                    TableCell::from(lines.into_iter().map(Line::from).collect::<Vec<_>>()),
                ])
                .height(height)
            })
            .collect();
        f.render_widget(
            Table::new(rows, [Constraint::Length(key_width), Constraint::Fill(1)])
                .column_spacing(3),
            inner,
        );
        if used > inner.height as usize {
            let more = Rect {
                y: area.bottom().saturating_sub(1),
                height: 1,
                ..area
            };
            f.render_widget(Line::styled(" Down for more ", HINT).centered(), more);
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

/// The furthest the key list scrolls, its keys taking `rows` rows each: the first key from which
/// the rest fit a `page` of rows.
fn help_most(rows: &[usize], page: usize) -> usize {
    (0..rows.len())
        .find(|&s| rows[s..].iter().sum::<usize>() <= page)
        .unwrap_or(rows.len().saturating_sub(1))
}

/// Where a key scrolls the key list from `scroll`: a key a line, a page the keys a page shows whole,
/// never past where the last key shows; none when the key closes it.
fn help_scroll(scroll: usize, code: KeyCode, rows: &[usize], page: usize) -> Option<usize> {
    let most = help_most(rows, page);
    let scroll = scroll.min(most);
    let mut used = 0;
    let whole = rows[scroll.min(rows.len())..]
        .iter()
        .take_while(|&&r| {
            used += r;
            used <= page
        })
        .count();
    Some(match code {
        KeyCode::Up | KeyCode::Char('k') => scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => (scroll + 1).min(most),
        KeyCode::PageDown => (scroll + whole.max(1)).min(most),
        KeyCode::PageUp => (0..scroll)
            .find(|&s| rows[s..scroll].iter().sum::<usize>() <= page)
            .unwrap_or(scroll.saturating_sub(1)),
        KeyCode::Home => 0,
        KeyCode::End => most,
        KeyCode::Esc | KeyCode::Enter | KeyCode::F(1) | KeyCode::Char('?' | 'q') => return None,
        _ => scroll,
    })
}

/// `text` broken at spaces into lines of `width` columns at most; a word longer than a line, a path
/// most often, is broken where the line ends.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let cols = |s: &str| Span::raw(s).width();
    let mut lines = vec![String::new()];
    for word in text.split(' ') {
        let used = lines.last().map_or(0, |l| cols(l));
        if used > 0 && used + 1 + cols(word) <= width {
            let line = lines.last_mut().unwrap_or_else(|| unreachable!());
            line.push(' ');
            line.push_str(word);
            continue;
        }
        if used > 0 && cols(word) <= width {
            lines.push(word.to_string());
            continue;
        }
        if used > 0 {
            let line = lines.last_mut().unwrap_or_else(|| unreachable!());
            if used + 1 < width {
                line.push(' ');
            } else {
                lines.push(String::new());
            }
        }
        for c in word.chars() {
            let line = lines.last_mut().unwrap_or_else(|| unreachable!());
            if cols(line) + cols(c.encode_utf8(&mut [0; 4])) > width {
                lines.push(c.to_string());
            } else {
                line.push(c);
            }
        }
    }
    lines
}

/// `text` in `rows` lines of `width` columns: its start, then `…` and its end on the last line where
/// the middle is cut.
fn wrapped(text: &str, width: usize, rows: usize) -> Vec<String> {
    let mut lines = wrap(text, width);
    if rows > 0 && lines.len() > rows {
        lines.truncate(rows - 1);
        lines.push(format!("…{}", columns(text, width.saturating_sub(1), true)));
    }
    lines
}

/// A question over the list in `body`, then the keys that answer it.
fn confirm(
    f: &mut Frame,
    body: Rect,
    title: &str,
    mut lines: Vec<Line<'static>>,
    keys: &[(&str, &str)],
) {
    lines.push(Line::default());
    let mut answer = Vec::new();
    for (n, (k, what)) in keys.iter().enumerate() {
        if n > 0 {
            answer.push(Span::raw("   "));
        }
        answer.push(Span::styled(k.to_string(), BOLD));
        answer.push(Span::styled(format!(" {what}"), HINT));
    }
    lines.push(Line::from(answer));
    // the text wraps inside the border and a column of padding each side
    let width = 72.min(body.width.saturating_sub(4));
    let text_width = width.saturating_sub(4) as usize;
    let rows: usize = lines
        .iter()
        .map(|l| wrap(&l.to_string(), text_width).len())
        .sum();
    let area = popup_area(body, width, rows as u16 + 4);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }).block(
            Block::bordered()
                .title(Span::styled(title.to_string(), BOLD))
                .padding(Padding::uniform(1)),
        ),
        area,
    );
}

/// The selected switch in `width` columns: beside the list every member on its own line and every
/// date, below it what fits in five lines. A path keeps its end.
fn details(lib: &Library, s: &Saved, side: bool, width: u16) -> Vec<Line<'static>> {
    let room = (width as usize).saturating_sub(11);
    let label = |l: &str| Span::styled(format!(" {l:<9} "), HINT);
    let line = |l: &str, v: String| Line::from(vec![label(l), Span::raw(fit_right(&v, room))]);
    let path = |l: &str, v: String| Line::from(vec![label(l), Span::raw(fit_left(&v, room))]);
    let mut out = Vec::new();
    match (side, platform::parse_stack(&s.stack)) {
        (true, Ok(stack)) => {
            for (k, m) in stack.members.iter().enumerate() {
                let module = m.module.as_deref().unwrap_or("");
                let member = format!("{:>2}  {:<18}{module}", m.number, m.model);
                out.push(Line::from(vec![
                    label(if k == 0 { "Hardware" } else { "" }),
                    Span::raw(fit_right(&member, room)),
                ]));
            }
        }
        _ => out.push(line("Hardware", hardware_line(&s.stack))),
    }
    out.push(line("Release", os_release(&s.stack, s.release())));
    out.push(path("Source", s.source.clone()));
    let at = |t: u64| when(t);
    if side {
        out.push(path("Kept", browser::shown(&lib.config_path(&s.name))));
        out.push(line("Loaded", at(s.loaded_at)));
        if let Some(o) = s.opened_at {
            out.push(line("Opened", at(o)));
        }
        if let Some(w) = s.saved_at {
            out.push(line("Saved", format!("{}, by write memory", at(w))));
        }
    } else {
        out.push(match (s.saved_at, s.opened_at) {
            (Some(w), _) => line("Saved", format!("{}, by write memory", at(w))),
            (None, Some(o)) => line("Opened", at(o)),
            (None, None) => line("Loaded", at(s.loaded_at)),
        });
    }
    if s.swept > 0 {
        out.push(line("Secrets", format!("{} line(s) removed", s.swept)));
    }
    out
}

/// The release a file names, as the stack's book documents it: its own train or `<major>.x` book, else the
/// newest documented release before it, else 17.15. The builder shows it before anything is kept.
fn first_release(text: Option<&str>, spec: &str) -> Release {
    let book = crate::grammars::book(platform::parse_stack(spec).ok().and_then(|s| s.family()));
    let Some(text) = text else {
        return crate::release::DEFAULT;
    };
    let documented = crate::grammars::documented(book);
    Release::resolve(text, &documented)
        .or_else(|| {
            let r = Release::parse(text)?;
            documented.iter().rev().find(|d| **d < r).copied()
        })
        .unwrap_or(crate::release::DEFAULT)
}

/// The folder configuration files are dropped into to be swept of secrets, created when missing.
pub fn sweep_folder() -> Result<std::path::PathBuf, String> {
    let dir = Library::default_dir()
        .ok_or("no data directory: set VSWITCH_HOME")?
        .join("sweep");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// Sweeps the folder in place and says what it did, in one line for the status bar.
fn sweep_status(dir: &Path) -> (String, Tone) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        return (format!("{}: {e}", dir.display()), Tone::Problem);
    }
    match crate::sweep::sweep_path(dir, None) {
        Ok(r) if r.is_empty() => (
            format!(
                "Sweep folder {} is empty: put configuration files there.",
                browser::shown(dir)
            ),
            Tone::Plain,
        ),
        Ok(r) => {
            let lines: usize = r
                .iter()
                .filter_map(|f| f.outcome.as_ref().ok())
                .map(|s| s.lines())
                .sum();
            let skipped = r.iter().filter(|f| f.outcome.is_err()).count();
            let left: usize = r
                .iter()
                .filter_map(|f| f.outcome.as_ref().ok())
                .map(|s| s.left)
                .sum();
            let mut msg = format!(
                "Swept {}: {} file(s), {lines} secret line(s) removed",
                browser::shown(dir),
                r.len()
            );
            if skipped > 0 {
                msg.push_str(&format!(", {skipped} not text and left"));
            }
            if left > 0 {
                msg.push_str(&format!(
                    ", {left} removed value(s) still appear: check with vswitch sweep"
                ));
                return (msg, Tone::Problem);
            }
            (msg, Tone::Done)
        }
        Err(e) => (e, Tone::Problem),
    }
}

/// Seconds since the epoch as `2026-10-05 21:34 UTC`.
pub fn when(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let (h, m) = ((secs % 86_400) / 3600, (secs % 3600) / 60);
    // civil date from days since 1970-01-01 (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02} UTC")
}

/// The switch after boot: its configuration typed in configuration mode, then back at the user prompt
/// as a console shows it. Returns the session, the lines it took, and each line it refused with the
/// switch's error.
pub fn boot<'a>(
    engine: &'a Engine,
    stack: platform::Stack,
    config: &str,
) -> (shell::Session<'a>, usize, Vec<String>) {
    let (mut s, typed) = crate::check::type_config(engine, stack, config);
    let taken = typed.iter().filter(|t| t.error.is_none()).count();
    let refused = typed
        .iter()
        .filter_map(|t| t.error.as_ref().map(|why| format!("{}   {why}", t.line)))
        .collect();
    s.type_line("end");
    s.type_line("disable");
    (s, taken, refused)
}

/// The selector, then the chosen switch's console, until the selector is left.
pub fn run(
    engine_for: impl Fn(&platform::Stack, Release) -> Result<Engine, String>,
    console: impl Fn(&mut shell::Session<'_>) -> Result<(), String>,
) -> Result<(), String> {
    let dir =
        Library::default_dir().ok_or("no data directory for the switch list: set VSWITCH_HOME")?;
    let mut lib = Library::open(dir)?;
    let mut app = App::new();
    app.remember(&lib.prefs());
    // each platform release's grammar, read once a run: a switch opened again opens at once
    let mut engines: std::collections::BTreeMap<(&'static str, Release), Engine> =
        std::collections::BTreeMap::new();
    loop {
        let action = select(&mut app, &mut lib)?;
        let Action::Open(i) = action else {
            return Ok(());
        };
        let sw = lib.switches[i].clone();
        // `write memory` may rename the switch; the list selects it under its name after `exit`
        let name = std::sync::Mutex::new(sw.name.clone());
        let opened = (|| -> Result<(), String> {
            // said before the grammar is read, which takes a moment
            println!("{}", opening(&sw));
            let _ = std::io::stdout().flush();
            let config = lib.config(&sw.name)?;
            let stack = platform::parse_stack(&sw.stack)?;
            let key = (crate::grammars::book(stack.family()), sw.release());
            let engine = match engines.entry(key) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(v) => {
                    v.insert(engine_for(&stack, sw.release())?)
                }
            };
            let (mut s, taken, refused) = boot(engine, stack, &config);
            println!("{}", loaded(&sw, taken));
            if !refused.is_empty() {
                println!(
                    "{} line(s) the virtual switch does not model were skipped:",
                    refused.len()
                );
                for r in refused.iter().take(12) {
                    println!("  {r}");
                }
                if refused.len() > 12 {
                    println!("  ... and {} more", refused.len() - 12);
                }
            }
            println!("Type exit to return to the switch list.\n");
            lib.opened(i)?;
            // `write memory` keeps the configuration in the list, under the hostname it saves
            let list = &mut lib;
            let name = &name;
            s.on_save(move |text: &str| {
                let mut name = name.lock().unwrap_or_else(|e| e.into_inner());
                *name = list.save_running(&name, text)?;
                Ok(())
            });
            console(&mut s)
        })();
        if let Err(e) = opened {
            app.fail(e);
        }
        app.select_name(&lib, &name.into_inner().unwrap_or_else(|e| e.into_inner()));
    }
}

/// The terminal as the list draws it: raw, on the alternate screen. Dropping it restores the
/// terminal, on a return or an error; the panic hook restores it on a panic.
struct Screen {
    terminal: ratatui::DefaultTerminal,
}

/// Whether the list holds the terminal, so a panic during a console leaves that screen alone.
static HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn restore() {
    use ratatui::crossterm::{cursor, execute, terminal};
    if HELD.swap(false, std::sync::atomic::Ordering::SeqCst) {
        let _ = terminal::disable_raw_mode();
        #[cfg(not(windows))]
        let _ = execute!(
            std::io::stdout(),
            ratatui::crossterm::event::DisableBracketedPaste
        );
        let _ = execute!(
            std::io::stdout(),
            terminal::LeaveAlternateScreen,
            cursor::Show
        );
    }
}

impl Screen {
    fn on() -> Result<Self, String> {
        use ratatui::crossterm::{execute, terminal};
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            let before = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                restore();
                before(info);
            }));
        });
        let io = |e: std::io::Error| e.to_string();
        terminal::enable_raw_mode().map_err(io)?;
        HELD.store(true, std::sync::atomic::Ordering::SeqCst);
        let held = (|| {
            execute!(std::io::stdout(), terminal::EnterAlternateScreen)?;
            // crossterm reads a Windows console's input as keys and never makes a paste of it
            #[cfg(not(windows))]
            let _ = execute!(
                std::io::stdout(),
                ratatui::crossterm::event::EnableBracketedPaste
            );
            ratatui::Terminal::new(ratatui::prelude::CrosstermBackend::new(std::io::stdout()))
        })();
        match held {
            Ok(terminal) => Ok(Screen { terminal }),
            Err(e) => {
                restore();
                Err(e.to_string())
            }
        }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
        restore();
    }
}

fn select(app: &mut App, lib: &mut Library) -> Result<Action, String> {
    let mut screen = Screen::on()?;
    let io = |e: std::io::Error| e.to_string();
    loop {
        screen.terminal.draw(|f| app.draw(f, lib)).map_err(io)?;
        let first = event::read().map_err(io)?;
        let read = gather(first, cfg!(windows) && app.on_list(), |wait| {
            let start = Instant::now();
            let next = if event::poll(wait).map_err(io)? {
                Some(event::read().map_err(io)?)
            } else {
                None
            };
            Ok((next, start.elapsed()))
        })?;
        if let Some(a) = act(app, lib, read, cfg!(windows)) {
            return Ok(a);
        }
    }
}

/// How long keys read together are apart at most: crossterm drops the Shift records a pasted capital
/// or colon brings, and a poll of no time ended the paste at each one (`D:\` lost its `D:`); keys a
/// person types are tens of milliseconds apart.
const TOGETHER: Duration = Duration::from_millis(12);

/// How long the list waits for the rest of a paste a busy console types slowly, at least and at most:
/// twice what its last wait took, which a busy machine stretches. With 32 busy processes on 16
/// threads the console's pauses were 7 to 31 ms, and 250 ms while that wait took 250.
const TRICKLE: (Duration, Duration) = (Duration::from_millis(35), Duration::from_millis(500));

/// How long the list waits for a typed key's release, at least and at most, twice what its last wait
/// took: a busy console can type a pasted key's press and its release apart, and a person's release
/// comes later than this.
const RELEASE: (Duration, Duration) = (Duration::from_millis(25), Duration::from_millis(250));

/// The events read together from `first` on: what follows it within `TOGETHER`, and on the list
/// (`on_list`) the rest of a paste a busy console types in pieces, each key of it pressed and
/// released at once. `poll` waits up to the time given for an event and says how long it took.
fn gather(
    first: Event,
    on_list: bool,
    mut poll: impl FnMut(Duration) -> Result<(Option<Event>, Duration), String>,
) -> Result<Vec<Event>, String> {
    let mut read = vec![first];
    let mut wait = TOGETHER;
    loop {
        match poll(wait)? {
            (Some(e), _) => {
                read.push(e);
                wait = TOGETHER;
            }
            (None, took) if on_list && wait == TOGETHER => match typing(&read) {
                Typing::Console => wait = (took * 2).clamp(TRICKLE.0, TRICKLE.1),
                Typing::Pressed => wait = (took * 2).clamp(RELEASE.0, RELEASE.1),
                Typing::Person => return Ok(read),
            },
            (None, _) => return Ok(read),
        }
    }
}

/// Who typed keys read together, as their presses and releases show it.
#[derive(Debug, PartialEq)]
enum Typing {
    /// a Windows console typing a paste: each key pressed and at once released, the last perhaps
    /// still to be released
    Console,
    /// a key pressed and not yet released, which a busy console types apart as a person does
    Pressed,
    /// a person, who holds a key for tens of milliseconds and whose key held down repeats without
    /// being released; or keys that type nothing
    Person,
}

/// The character a key types (Enter a line end, Tab a tab), as a paste delivers it; none for a key
/// that types nothing. AltGr arrives as Ctrl and Alt together.
fn typed_char(k: &KeyEvent) -> Option<char> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        KeyCode::Char(c) if !ctrl || k.modifiers.contains(KeyModifiers::ALT) => Some(c),
        KeyCode::Enter => Some('\n'),
        KeyCode::Tab => Some('\t'),
        _ => None,
    }
}

/// Whether two key codes are one key; a capital is released as its small letter once Shift is up.
fn same_key(a: KeyCode, b: KeyCode) -> bool {
    match (a, b) {
        (KeyCode::Char(x), KeyCode::Char(y)) => x.to_lowercase().eq(y.to_lowercase()),
        _ => a == b,
    }
}

/// Who typed the keys read together (`Typing`).
fn typing(read: &[Event]) -> Typing {
    let (mut pairs, mut pending) = (0, false);
    let mut i = 0;
    while i < read.len() {
        let Event::Key(k) = &read[i] else {
            return Typing::Person;
        };
        if typed_char(k).is_none() {
            return Typing::Person;
        }
        i += 1;
        // a release first is of a key pressed before these
        if k.kind == KeyEventKind::Release {
            continue;
        }
        match read.get(i) {
            None => pending = true,
            Some(Event::Key(r)) if r.kind == KeyEventKind::Release && same_key(r.code, k.code) => {
                pairs += 1;
                i += 1;
            }
            Some(_) => return Typing::Person,
        }
    }
    match (pairs, pending) {
        (0, true) => Typing::Pressed,
        (0, false) => Typing::Person,
        _ => Typing::Console,
    }
}

/// Acts on events read together; the first action that leaves the list ends it. A Windows console
/// delivers a paste or a file dropped on the terminal as keys, faster than anyone types (`bursts`):
/// on the list three or more typed characters read together are a paste, never commands, unless the
/// first opens a field to type into (`a`, `n`, `/`, and `r`, `c` and `e` on a switch), which then
/// takes the rest as typing. A trailing Enter acts after the paste.
fn act(app: &mut App, lib: &mut Library, read: Vec<Event>, bursts: bool) -> Option<Action> {
    let mut events: std::collections::VecDeque<Event> = read.into();
    while let Some(e) = events.pop_front() {
        if bursts && app.on_list() {
            let (text, used) = typed_run(std::iter::once(&e).chain(events.iter()));
            // Enters and spaces alone are keys: a blank paste would open the browser and load its
            // first entry
            if text.chars().count() >= 3
                && !text.trim().is_empty()
                && !text.chars().next().is_some_and(|c| app.opens_field(c, lib))
            {
                events.drain(..used - 1);
                let body = text.trim_end_matches('\n');
                app.on_paste(body);
                if body.len() < text.len() {
                    match app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), lib) {
                        Action::None => {}
                        a => return Some(a),
                    }
                }
                continue;
            }
        }
        match e {
            Event::Key(k) if k.kind != KeyEventKind::Release => match app.on_key(k, lib) {
                Action::None => {}
                a => return Some(a),
            },
            Event::Paste(t) => app.on_paste(&t),
            _ => {}
        }
    }
    None
}

/// The text the leading run of typed keys spells (Enter as a line end), and how many events the run
/// takes, releases included. A key pressed again while it is held is repeating, as a person holding
/// it makes it, and ends the run.
fn typed_run<'e>(events: impl Iterator<Item = &'e Event>) -> (String, usize) {
    let mut text = String::new();
    let mut used = 0;
    let mut held: Vec<KeyCode> = Vec::new();
    for e in events {
        match e {
            Event::Key(k) if k.kind == KeyEventKind::Release => {
                held.retain(|h| !same_key(*h, k.code));
            }
            Event::Key(k) => {
                let Some(c) = typed_char(k) else {
                    break;
                };
                if held.iter().any(|h| same_key(*h, k.code)) {
                    break;
                }
                held.push(k.code);
                text.push(c);
            }
            _ => break,
        }
        used += 1;
    }
    (text, used)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::TempDir;
    use ratatui::Terminal;
    use ratatui::backend::{Backend, TestBackend};

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn typed(app: &mut App, lib: &mut Library, text: &str) {
        for c in text.chars() {
            assert_eq!(app.on_key(key(KeyCode::Char(c)), lib), Action::None);
        }
    }

    fn render(app: &App, lib: &Library, w: u16, h: u16) -> Terminal<TestBackend> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| app.draw(f, lib)).unwrap();
        t
    }

    /// The screen at `w` by `h` as text, the cursor a block where the terminal shows it.
    fn screen_at(app: &App, lib: &Library, w: u16, h: u16) -> String {
        let mut t = render(app, lib, w, h);
        let cursor = t
            .backend()
            .cursor_visible()
            .then(|| t.backend_mut().get_cursor_position().unwrap());
        let buf = t.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| match cursor {
                        Some(p) if (p.x, p.y) == (x, y) => "█".to_string(),
                        _ => buf[(x, y)].symbol().to_string(),
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn screen(app: &App, lib: &Library) -> String {
        screen_at(app, lib, 100, 20)
    }

    /// The status at 80 by 24, one line or two above the footer, and the colour it is drawn in.
    fn status_line(app: &App, lib: &Library) -> (String, Color) {
        let t = render(app, lib, 80, 24);
        let buf = t.backend().buffer();
        let row = |y: u16| (0..80).map(|x| buf[(x, y)].symbol()).collect::<String>();
        let top = if row(21).starts_with(['└', '│']) {
            22
        } else {
            21
        };
        let text: Vec<String> = (top..23).map(|y| row(y).trim().to_string()).collect();
        let first = (0..80)
            .find(|&x| buf[(x, top)].symbol() != " ")
            .unwrap_or(0);
        (text.join(" "), buf[(first, top)].fg)
    }

    /// Three switches, sorted core1, dc1-stack-sw01, lab2960x, with fixed times.
    fn fleet(tag: &str) -> (TempDir, Library) {
        let t = TempDir::new(tag);
        let mut lib = Library::open(t.0.join("lib")).unwrap();
        let r = |x| Release::parse(x).unwrap();
        lib.add(
            "dc1-stack-sw01",
            "C9300X-24Y+C9300X-NM-4C,C9300X-24Y+C9300X-NM-4C",
            crate::release::DEFAULT,
            "tests/example-design.toml",
            "hostname dc1-stack-sw01\n",
        )
        .unwrap();
        lib.add(
            "lab2960x",
            "WS-C2960X-24PS-L",
            r("15.2(7)E"),
            "D:/captures/lab2960x.log",
            "hostname lab2960x\nenable secret 9 $9$FFFFexampleHASH\n",
        )
        .unwrap();
        lib.add(
            "core1",
            "2=C9400-LC-24XS,3=C9400-SUP-1XL",
            r("17.9"),
            "C:/captures/core1-running-config.txt",
            "hostname core1\n",
        )
        .unwrap();
        for (i, s) in lib.switches.iter_mut().enumerate() {
            s.loaded_at = 1_791_200_000 + i as u64 * 60;
        }
        lib.switches[1].saved_at = Some(1_791_235_200);
        lib.switches[1].opened_at = Some(1_791_235_000);
        lib.switches[2].opened_at = Some(1_791_300_000);
        (t, lib)
    }

    fn names(app: &App, lib: &Library) -> Vec<String> {
        app.rows(lib)
            .iter()
            .map(|&i| lib.switches[i].name.clone())
            .collect()
    }

    #[test]
    fn a_design_loads_from_the_selector_and_opens_its_console() {
        let t = TempDir::new("tui-design");
        let mut lib = Library::open(t.0.clone()).unwrap();
        let mut app = App::new();
        assert!(screen(&app, &lib).contains("No switches yet"));
        assert_eq!(app.on_key(key(KeyCode::Char('a')), &mut lib), Action::None);
        app.on_paste("\"tests/example-design.toml\"");
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::Open(0));
        assert_eq!(lib.switches[0].name, "dc1-stack-sw01");
        // the list shows switch names, and Enter on one opens it
        let shown = screen(&app, &lib);
        assert!(
            shown.contains("dc1-stack-sw01") && shown.contains("C9300X-24Y+C9300X-NM-4C"),
            "{shown}"
        );
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::Open(0));
        // the browser opens next in the folder the design came from
        let tests = std::env::current_dir().unwrap().join("tests");
        assert_eq!(lib.prefs().folder.as_deref(), Some(tests.as_path()));
    }

    #[test]
    fn a_path_pasted_on_the_list_opens_the_file_browser_with_it() {
        let t = TempDir::new("tui-drop");
        let mut lib = Library::open(t.0.clone()).unwrap();
        let mut app = App::new();
        let design = std::env::current_dir()
            .unwrap()
            .join("tests")
            .join("example-design.toml");
        app.on_paste(&format!("\"{}\"", design.display()));
        assert!(
            matches!(&app.mode, Mode::Browse(b) if b.query.text == design.display().to_string()),
            "{:?}",
            app.mode
        );
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::Open(0));
        assert_eq!(lib.switches[0].source, design.display().to_string());
    }

    #[test]
    fn a_configuration_opens_the_builder_with_the_inferred_hardware() {
        let t = TempDir::new("tui-config");
        let p = t.0.join("core.txt");
        std::fs::write(&p, "hostname core1\ninterface TenGigabitEthernet2/0/3\n description x\n!\ninterface FortyGigabitEthernet3/0/9\nend\n").unwrap();
        let mut lib = Library::open(t.0.join("lib")).unwrap();
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('a')), &mut lib);
        typed(&mut app, &mut lib, &p.to_string_lossy());
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::None);
        match &app.mode {
            Mode::Hardware { name, builder, .. } => {
                assert_eq!(name, "core1");
                assert_eq!(builder.spec(), "2=C9400-LC-24XS,3=C9400-SUP-1XL");
            }
            m => panic!("{m:?}"),
        }
        let shown = screen(&app, &lib);
        assert!(
            shown.contains("Hardware for core1") && shown.contains("C9400-SUP-1XL"),
            "{shown}"
        );
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::Open(0));
        assert_eq!(lib.switches[0].stack, "2=C9400-LC-24XS,3=C9400-SUP-1XL");
    }

    #[test]
    fn a_configuration_s_version_line_is_the_release_it_runs() {
        let t = TempDir::new("tui-version");
        let p = t.0.join("idf.txt");
        std::fs::write(
            &p,
            "!\nversion 17.4\nhostname idf9\nswitch 1 provision c9300x-24y\ninterface TwentyFiveGigE1/0/1\nend\n",
        )
        .unwrap();
        let mut lib = Library::open(t.0.join("lib")).unwrap();
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('a')), &mut lib);
        typed(&mut app, &mut lib, &p.to_string_lossy());
        app.on_key(key(KeyCode::Enter), &mut lib);
        let shown = screen(&app, &lib);
        assert!(
            shown.contains("IOS XE  17.4")
                && shown.contains("the C9300X-24Y runs IOS XE 17.5 and later"),
            "{shown}"
        );
        // nothing is kept on a release the hardware does not run; ] moves to the next one
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::None);
        assert!(lib.switches.is_empty());
        app.on_key(key(KeyCode::Char(']')), &mut lib);
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::Open(0));
        assert_eq!(lib.switches[0].release(), Release::parse("17.5").unwrap());
        assert!(screen(&app, &lib).contains("17.5"));
    }

    #[test]
    fn the_builder_marks_an_incompatible_module_and_keeps_nothing_until_it_is_fixed() {
        let t = TempDir::new("tui-new");
        let mut lib = Library::open(t.0.clone()).unwrap();
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('n')), &mut lib);
        typed(&mut app, &mut lib, "idf1");
        app.on_key(key(KeyCode::Enter), &mut lib);
        let Mode::Hardware { builder, .. } = &mut app.mode else {
            panic!("{:?}", app.mode)
        };
        *builder = Builder::from_spec("C9200-24T+C9200-NM-2Y", crate::release::DEFAULT);
        let shown = screen(&app, &lib);
        assert!(
            shown.contains("C9200-NM-2Y does not fit the C9200-24T"),
            "{shown}"
        );
        assert_eq!(
            app.on_key(key(KeyCode::Enter), &mut lib),
            Action::None,
            "not kept with a problem"
        );
        assert!(lib.switches.is_empty());
        let Mode::Hardware { builder, .. } = &mut app.mode else {
            panic!("{:?}", app.mode)
        };
        *builder = Builder::from_spec("C9200-24T+C9200-NM-4X", crate::release::DEFAULT);
        assert!(screen(&app, &lib).contains("Cisco builds and stacks this hardware"));
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::Open(0));
        assert_eq!(lib.switches[0].stack, "C9200-24T+C9200-NM-4X");
        assert_eq!(lib.config("idf1").unwrap(), "hostname idf1\n");
        // h changes a listed switch's hardware and keeps its configuration
        app.on_key(key(KeyCode::Char('h')), &mut lib);
        let Mode::Hardware { builder, .. } = &mut app.mode else {
            panic!("{:?}", app.mode)
        };
        // a listed switch keeps its hardware while the builder shows a problem
        *builder = Builder::from_spec("C9200-48T+C9200-NM-2Q", crate::release::DEFAULT);
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::None);
        assert_eq!(lib.switches[0].stack, "C9200-24T+C9200-NM-4X");
        let Mode::Hardware { builder, .. } = &mut app.mode else {
            panic!("{:?}", app.mode)
        };
        *builder = Builder::from_spec("C9200-48T+C9200-NM-4G", crate::release::DEFAULT);
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::None);
        assert_eq!(lib.switches[0].stack, "C9200-48T+C9200-NM-4G");
        assert_eq!(lib.config("idf1").unwrap(), "hostname idf1\n");
    }

    #[test]
    fn a_design_naming_hardware_cisco_does_not_build_opens_the_builder() {
        let t = TempDir::new("tui-baddesign");
        let d = t.0.join("bad.toml");
        std::fs::write(
            &d,
            "hostname = \"lab9\"\nstack = [\"C9200L-24T-4G+C9200-NM-4X\"]\n",
        )
        .unwrap();
        let mut lib = Library::open(t.0.join("lib")).unwrap();
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('a')), &mut lib);
        typed(&mut app, &mut lib, &d.to_string_lossy());
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::None);
        assert!(
            matches!(&app.mode, Mode::Hardware { builder, .. } if builder.problems()[0].1.contains("built-in uplinks"))
        );
        assert!(lib.switches.is_empty());
    }

    #[test]
    fn a_missing_file_is_named_and_removing_asks_first() {
        let t = TempDir::new("tui-missing");
        let mut lib = Library::open(t.0.clone()).unwrap();
        lib.add(
            "sw1",
            "C9300X-24Y",
            crate::release::DEFAULT,
            "x.cfg",
            "hostname sw1\n",
        )
        .unwrap();
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('a')), &mut lib);
        typed(&mut app, &mut lib, "no/such/file.cfg");
        app.on_key(key(KeyCode::Enter), &mut lib);
        assert!(
            matches!(&app.mode, Mode::Browse(b) if b.error.as_deref().is_some_and(|e| e.contains("no/such/file.cfg"))),
            "{:?}",
            app.mode
        );
        app.on_key(key(KeyCode::Esc), &mut lib);
        app.on_key(key(KeyCode::Char('d')), &mut lib);
        assert_eq!(app.mode, Mode::Remove);
        app.on_key(key(KeyCode::Char('n')), &mut lib);
        assert_eq!(lib.switches.len(), 1, "anything but y keeps it");
        app.on_key(key(KeyCode::Char('d')), &mut lib);
        app.on_key(key(KeyCode::Char('y')), &mut lib);
        assert!(lib.switches.is_empty());
        assert_eq!(app.on_key(key(KeyCode::Char('q')), &mut lib), Action::Quit);
    }

    #[test]
    fn the_release_reads_ios_on_classic_families_and_ios_xe_on_the_rest() {
        let (_t, lib) = fleet("os");
        let mut app = App::new();
        app.select_name(&lib, "lab2960x");
        for (w, h) in [(80, 24), (120, 40)] {
            let shown = screen_at(&app, &lib, w, h);
            assert!(
                shown.contains("IOS 15.2(7)E") && !shown.contains("IOS XE 15.2(7)E"),
                "{shown}"
            );
        }
        app.select_name(&lib, "core1");
        assert!(screen_at(&app, &lib, 120, 40).contains("IOS XE 17.9"));
        assert_eq!(
            opening(&lib.switches[2]),
            "Opening lab2960x (WS-C2960X-24PS-L, IOS 15.2(7)E)..."
        );
        assert_eq!(
            opening(&lib.switches[1]),
            "Opening dc1-stack-sw01 (C9300X-24Y +1, IOS XE 17.15)..."
        );
    }

    #[test]
    fn the_list_shows_hardware_release_and_last_save_as_the_width_allows() {
        let (_t, lib) = fleet("columns");
        let mut app = App::new();
        let narrow = screen_at(&app, &lib, 80, 24);
        let row = |s: &str, name: &str| {
            s.lines()
                .find(|l| l.contains(&format!(" {name} ")))
                .unwrap_or_else(|| panic!("{name}:\n{s}"))
                .to_string()
        };
        let fw = row(&narrow, "dc1-stack-sw01");
        assert!(
            fw.contains("C9300X-24Y +1") && fw.contains("17.15") && fw.contains("2026-10-05"),
            "{narrow}"
        );
        assert!(
            row(&narrow, "core1").contains("C9400-SUP-1XL +1"),
            "{narrow}"
        );
        assert!(
            narrow.contains("Saved") && narrow.contains("Opened"),
            "{narrow}"
        );
        // below the list, the members in a line; beside it at 120 columns, a member a line
        assert!(
            narrow.contains("Hardware  2=C9400-LC-24XS, 3=C9400-SUP-1XL"),
            "{narrow}"
        );
        let wide = screen_at(&app, &lib, 120, 40);
        assert!(
            wide.contains(" 2  C9400-LC-24XS") && wide.contains(" 3  C9400-SUP-1XL"),
            "{wide}"
        );
        assert!(
            wide.contains("Kept") && wide.contains("core1.cfg"),
            "{wide}"
        );
        // the narrowest terminal keeps the name and the hardware and drops the dates
        let small = screen_at(&app, &lib, 60, 15);
        assert!(
            row(&small, "lab2960x").contains("WS-C2960X-24PS-L"),
            "{small}"
        );
        assert!(!small.contains("Opened"), "{small}");
        app.select_name(&lib, "dc1-stack-sw01");
        let narrow = screen_at(&app, &lib, 80, 24);
        assert!(narrow.contains("2 x C9300X-24Y+C9300X-NM-4C"), "{narrow}");
    }

    #[test]
    fn slash_filters_as_it_is_typed_and_esc_clears_it_before_quitting() {
        let (_t, mut lib) = fleet("filter");
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('/')), &mut lib);
        typed(&mut app, &mut lib, "2960");
        assert_eq!(names(&app, &lib), ["lab2960x"]);
        let shown = screen_at(&app, &lib, 80, 24);
        assert!(
            shown.contains("1 of 3") && shown.contains("Filter  2960█"),
            "{shown}"
        );
        // every word must match, each in the name, the hardware or the release
        for _ in 0..4 {
            app.on_key(key(KeyCode::Backspace), &mut lib);
        }
        typed(&mut app, &mut lib, "sup 17.9");
        assert_eq!(names(&app, &lib), ["core1"]);
        app.on_key(key(KeyCode::Enter), &mut lib);
        assert_eq!(app.mode, Mode::List);
        assert_eq!(app.current(&lib), Some(0));
        assert_eq!(app.on_key(key(KeyCode::Esc), &mut lib), Action::None);
        assert_eq!(names(&app, &lib).len(), 3);
        assert_eq!(app.current(&lib), Some(0), "the switch stays selected");
        assert_eq!(app.on_key(key(KeyCode::Esc), &mut lib), Action::Quit);
    }

    #[test]
    fn a_listed_name_is_replaced_only_once_that_is_confirmed() {
        let (t, mut lib) = fleet("replace");
        let mut app = App::new();
        // a new switch under a listed name: warned as it is typed, asked after the builder
        app.on_key(key(KeyCode::Char('n')), &mut lib);
        typed(&mut app, &mut lib, "core1");
        let shown = screen_at(&app, &lib, 80, 24);
        assert!(
            shown.contains("core1 is in the list: keeping a new switch under this"),
            "{shown}"
        );
        app.on_key(key(KeyCode::Enter), &mut lib);
        let Mode::Hardware { builder, .. } = &mut app.mode else {
            panic!("{:?}", app.mode)
        };
        *builder = Builder::from_spec("C9300X-24Y", crate::release::DEFAULT);
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::None);
        assert!(matches!(&app.mode, Mode::Replace(p) if p.name == "core1"));
        let shown = screen_at(&app, &lib, 80, 24);
        assert!(
            shown.contains("Replace core1?") && shown.contains("C9400-SUP-1XL +1, IOS XE 17.9"),
            "{shown}"
        );
        app.on_key(key(KeyCode::Esc), &mut lib);
        assert_eq!(app.mode, Mode::List);
        assert_eq!(lib.config("core1").unwrap(), "hostname core1\n");
        assert_eq!(lib.switches[0].stack, "2=C9400-LC-24XS,3=C9400-SUP-1XL");
        // a file loaded under a listed name: y replaces it
        let p = t.0.join("core1.txt");
        std::fs::write(
            &p,
            "hostname core1\nswitch 1 provision c9300x-24y\ninterface TwentyFiveGigE1/0/1\nend\n",
        )
        .unwrap();
        let load = |app: &mut App, lib: &mut Library| {
            app.on_key(key(KeyCode::Char('a')), lib);
            app.on_paste(&p.display().to_string());
            app.on_key(key(KeyCode::Enter), lib);
            app.on_key(key(KeyCode::Enter), lib)
        };
        assert_eq!(load(&mut app, &mut lib), Action::None);
        assert!(matches!(&app.mode, Mode::Replace(_)), "{:?}", app.mode);
        assert_eq!(
            app.on_key(key(KeyCode::Char('y')), &mut lib),
            Action::Open(0)
        );
        assert_eq!(lib.switches[0].stack, "C9300X-24Y");
        assert_eq!(lib.switches.len(), 3);
        // k keeps both, the one loaded under a name of its own
        assert_eq!(load(&mut app, &mut lib), Action::None);
        app.on_key(key(KeyCode::Char('k')), &mut lib);
        assert!(matches!(&app.mode, Mode::Name { input, .. } if input.text == "core1-copy"));
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::Open(1));
        assert_eq!(lib.switches.len(), 4);
        assert!(
            lib.config("core1-copy")
                .unwrap()
                .starts_with("hostname core1-copy\n")
        );
    }

    #[test]
    fn rename_copy_remove_and_undo_keep_the_list_whole() {
        let (_t, mut lib) = fleet("edit");
        let mut app = App::new();
        app.select_name(&lib, "core1");
        app.on_key(key(KeyCode::Char('r')), &mut lib);
        app.on_key(ctrl('u'), &mut lib);
        typed(&mut app, &mut lib, "mdf1");
        app.on_key(key(KeyCode::Enter), &mut lib);
        assert_eq!(app.status.as_deref(), Some("Renamed core1 to mdf1."));
        assert_eq!(lib.config("mdf1").unwrap(), "hostname mdf1\n");
        assert_eq!(lib.switches[app.current(&lib).unwrap()].name, "mdf1");
        // a name another switch has is refused in the popup
        app.on_key(key(KeyCode::Char('r')), &mut lib);
        app.on_key(ctrl('u'), &mut lib);
        typed(&mut app, &mut lib, "LAB2960X");
        app.on_key(key(KeyCode::Enter), &mut lib);
        assert!(
            matches!(&app.mode, Mode::Name { error: Some(e), .. } if e == "A switch named lab2960x is already in the list."),
            "{:?}",
            app.mode
        );
        app.on_key(key(KeyCode::Esc), &mut lib);
        app.on_key(key(KeyCode::Char('c')), &mut lib);
        app.on_key(key(KeyCode::Enter), &mut lib);
        assert_eq!(app.status.as_deref(), Some("Copied mdf1 to mdf1-copy."));
        assert!(
            lib.config("mdf1-copy")
                .unwrap()
                .starts_with("hostname mdf1-copy\n")
        );
        app.on_key(key(KeyCode::Char('d')), &mut lib);
        app.on_key(key(KeyCode::Char('y')), &mut lib);
        assert!(lib.position("mdf1-copy").is_none());
        assert!(screen_at(&app, &lib, 80, 24).contains("u brings it back"));
        app.on_key(key(KeyCode::Char('u')), &mut lib);
        assert_eq!(
            app.status.as_deref(),
            Some("mdf1-copy is back in the list.")
        );
        assert!(
            lib.config("mdf1-copy")
                .unwrap()
                .starts_with("hostname mdf1-copy\n")
        );
        assert_eq!(lib.switches[app.current(&lib).unwrap()].name, "mdf1-copy");
        app.on_key(key(KeyCode::Char('u')), &mut lib);
        assert_eq!(
            app.status.as_deref(),
            Some("No removed switch is left to bring back.")
        );
    }

    #[test]
    fn every_switch_removed_comes_back_with_u_in_turn() {
        let (_t, mut lib) = fleet("undo-all");
        let mut app = App::new();
        for name in ["core1", "lab2960x"] {
            app.select_name(&lib, name);
            app.on_key(key(KeyCode::Char('d')), &mut lib);
            app.on_key(key(KeyCode::Char('y')), &mut lib);
            assert!(lib.position(name).is_none(), "{name}");
        }
        assert_eq!(lib.switches.len(), 1);
        app.on_key(key(KeyCode::Char('u')), &mut lib);
        assert_eq!(app.status.as_deref(), Some("lab2960x is back in the list."));
        app.on_key(key(KeyCode::Char('u')), &mut lib);
        assert_eq!(app.status.as_deref(), Some("core1 is back in the list."));
        assert_eq!(lib.config("core1").unwrap(), "hostname core1\n");
        assert_eq!(lib.switches.len(), 3);
    }

    #[test]
    fn v_reads_the_kept_configuration_and_e_exports_it() {
        let (t, mut lib) = fleet("view");
        let mut app = App::new();
        app.select_name(&lib, "lab2960x");
        app.on_key(key(KeyCode::Char('v')), &mut lib);
        let shown = screen_at(&app, &lib, 80, 24);
        assert!(
            shown.contains("2  enable secret 9 <removed>") && shown.contains("lines 1-2 of 2"),
            "{shown}"
        );
        app.on_key(key(KeyCode::Char('/')), &mut lib);
        typed(&mut app, &mut lib, "SECRET");
        app.on_key(key(KeyCode::Enter), &mut lib);
        assert!(matches!(&app.mode, Mode::View(v) if v.hit == Some(1)));
        app.on_key(key(KeyCode::Esc), &mut lib);
        assert_eq!(app.mode, Mode::List);
        let out = t.0.join("out");
        std::fs::create_dir(&out).unwrap();
        app.folder = Some(out.clone());
        app.on_key(key(KeyCode::Char('e')), &mut lib);
        app.on_key(key(KeyCode::Enter), &mut lib);
        let file = out.join("lab2960x.cfg");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            lib.config("lab2960x").unwrap()
        );
        assert_eq!(app.tone, Tone::Done);
        assert_eq!(lib.prefs().folder.as_deref(), Some(out.as_path()));
        // a file there is asked about, and kept unless y
        std::fs::write(&file, "mine\n").unwrap();
        app.on_key(key(KeyCode::Char('e')), &mut lib);
        app.on_key(key(KeyCode::Enter), &mut lib);
        assert!(screen_at(&app, &lib, 80, 24).contains("is there: y writes over it"));
        app.on_key(key(KeyCode::Char('n')), &mut lib);
        app.on_key(key(KeyCode::Esc), &mut lib);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "mine\n");
        assert_eq!(app.mode, Mode::List);
    }

    #[test]
    fn a_problem_reads_red_and_information_plain() {
        let (_t, mut lib) = fleet("tone");
        let mut app = App::new();
        app.select_name(&lib, "core1");
        std::fs::remove_file(lib.config_path("core1")).unwrap();
        app.on_key(key(KeyCode::Char('v')), &mut lib);
        let (text, colour) = status_line(&app, &lib);
        assert!(text.contains("core1.cfg"), "{text}");
        assert_eq!(colour, Color::Red);
        app.on_key(key(KeyCode::Char('o')), &mut lib);
        let (text, colour) = status_line(&app, &lib);
        assert_eq!(text, "Ordered by last opened, newest first.");
        assert_eq!(colour, Color::Reset);
        // every status starts in the same column
        let t = render(&app, &lib, 80, 24);
        assert_eq!(t.backend().buffer()[(1, 22)].symbol(), "O");
    }

    #[test]
    fn o_orders_by_last_opened_and_the_order_is_remembered() {
        let (_t, mut lib) = fleet("order");
        let mut app = App::new();
        app.select_name(&lib, "core1");
        app.on_key(key(KeyCode::Char('o')), &mut lib);
        assert_eq!(names(&app, &lib), ["lab2960x", "dc1-stack-sw01", "core1"]);
        assert_eq!(lib.switches[app.current(&lib).unwrap()].name, "core1");
        assert!(screen_at(&app, &lib, 80, 24).contains("3, by last opened"));
        let mut again = App::new();
        again.remember(&lib.prefs());
        assert!(again.by_opened);
    }

    #[test]
    fn question_mark_lists_the_keys_of_the_screen_in_use() {
        let (_t, mut lib) = fleet("help");
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('?')), &mut lib);
        let shown = screen_at(&app, &lib, 80, 24);
        assert!(
            shown.contains("Keys: the switch list")
                && shown.contains("bring back the switch removed last"),
            "{shown}"
        );
        app.on_key(key(KeyCode::Esc), &mut lib);
        assert_eq!(app.help, None);
        app.on_key(key(KeyCode::Char('a')), &mut lib);
        app.on_key(key(KeyCode::F(1)), &mut lib);
        assert!(screen_at(&app, &lib, 80, 24).contains("Keys: the file browser"));
        app.on_key(key(KeyCode::Char('?')), &mut lib);
        assert!(
            matches!(app.mode, Mode::Browse(_)),
            "closing the help leaves the browser"
        );
        // the footer names the keys, ending with help and quit
        app.on_key(key(KeyCode::Esc), &mut lib);
        let shown = screen_at(&app, &lib, 80, 24);
        let footer = shown.lines().last().unwrap();
        assert!(
            footer.starts_with(" Enter open") && footer.ends_with("? help  q quit"),
            "{footer}"
        );
    }

    #[test]
    fn a_terminal_too_small_says_so_and_no_key_acts_unseen() {
        let (_t, mut lib) = fleet("small");
        let mut app = App::new();
        let shown = screen_at(&app, &lib, 50, 12);
        assert!(
            shown.contains("The terminal is 50x12.")
                && shown.contains("The switch list needs 60x15."),
            "{shown}"
        );
        assert!(!shown.contains('┌'), "no box drawn: {shown}");
        app.on_key(key(KeyCode::Char('d')), &mut lib);
        assert_eq!(app.mode, Mode::List);
        // q and Esc too: a filter Esc would clear is kept, and nothing quits but Ctrl-C
        app.filter = Input::new("9300");
        for k in [KeyCode::Char('q'), KeyCode::Esc] {
            assert_eq!(app.on_key(key(k), &mut lib), Action::None);
        }
        assert_eq!(app.filter.text, "9300");
        assert_eq!(app.on_key(ctrl('c'), &mut lib), Action::Quit);
        screen_at(&app, &lib, 80, 24);
        app.on_key(key(KeyCode::Char('d')), &mut lib);
        assert_eq!(app.mode, Mode::Remove);
    }

    #[test]
    fn the_terminal_page_lists_the_keys_the_help_lists() {
        let page = include_str!("../docs/terminal.md").replace("\r\n", "\n");
        let section = page
            .split("\n### List keys\n")
            .nth(1)
            .and_then(|s| s.split("\n#").next())
            .expect("docs/terminal.md has a List keys section");
        let mut table = String::from("| Key | Action |\n|---|---|\n");
        for (keys, what) in LIST_KEYS {
            table.push_str(&format!("| {} | {what} |\n", keys.join(", ")));
        }
        assert!(
            section.contains(&table),
            "docs/terminal.md's List keys table should read:\n{table}"
        );
    }

    #[test]
    fn boot_loads_the_configuration_and_leaves_the_user_prompt() {
        let engine = Engine::new(
            serde_json::from_str(&crate::grammars::text("c9300", crate::release::DEFAULT).unwrap())
                .unwrap(),
        );
        let stack =
            platform::parse_stack("C9300X-24Y+C9300X-NM-4C,C9300X-24Y+C9300X-NM-4C").unwrap();
        let config = "Building configuration...\n\nCurrent configuration : 99 bytes\n!\nversion 17.15\nhostname lab1\n!\ninterface TwentyFiveGigE1/0/1\n description up\n frobnicate now\n!\nend\n";
        let (s, taken, refused) = boot(&engine, stack, config);
        assert_eq!(s.prompt(), "lab1>");
        assert_eq!(taken, 3);
        assert_eq!(refused.len(), 1);
        assert!(
            refused[0].starts_with("frobnicate now") && refused[0].contains('%'),
            "{refused:?}"
        );
    }

    #[test]
    fn s_sweeps_the_sweep_folder_and_says_what_it_did() {
        let t = TempDir::new("tui-sweep");
        let mut lib = Library::open(t.0.clone()).unwrap();
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('s')), &mut lib);
        assert!(app.status.as_deref().unwrap().contains("is empty"));
        std::fs::write(
            t.0.join("sweep").join("core.txt"),
            "username a secret 9 $9$FFFFexampleHASH\n",
        )
        .unwrap();
        app.on_key(key(KeyCode::Char('s')), &mut lib);
        assert!(
            app.status
                .as_deref()
                .unwrap()
                .contains("1 file(s), 1 secret line(s) removed"),
            "{:?}",
            app.status
        );
        assert_eq!(app.tone, Tone::Done);
        assert_eq!(
            std::fs::read_to_string(t.0.join("sweep").join("core.txt")).unwrap(),
            "username a secret 9 <removed>\n"
        );
    }

    #[test]
    fn the_builder_s_lists_take_enter_and_esc_before_the_switch_list() {
        let t = TempDir::new("tui-lists");
        let mut lib = Library::open(t.0.clone()).unwrap();
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('n')), &mut lib);
        typed(&mut app, &mut lib, "lab1");
        app.on_key(key(KeyCode::Enter), &mut lib);
        // Esc closes an open list and leaves the builder open
        app.on_key(key(KeyCode::Char(' ')), &mut lib);
        assert_eq!(app.on_key(key(KeyCode::Esc), &mut lib), Action::None);
        assert!(matches!(app.mode, Mode::Hardware { .. }), "{:?}", app.mode);
        // Enter in a list chooses and keeps nothing
        typed(&mut app, &mut lib, "2960g-24");
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::None);
        assert!(lib.switches.is_empty());
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::Open(0));
        assert_eq!(lib.switches[0].stack, "WS-C2960G-24TC-L");
        assert_eq!(
            lib.switches[0].release(),
            Release::parse("15.0(2)SE").unwrap()
        );
        // a listed switch's new hardware is reported with its own operating system
        app.on_key(key(KeyCode::Char('h')), &mut lib);
        typed(&mut app, &mut lib, "2960+");
        app.on_key(key(KeyCode::Enter), &mut lib);
        assert_eq!(app.on_key(key(KeyCode::Enter), &mut lib), Action::None);
        assert_eq!(
            app.status.as_deref(),
            Some("lab1 is now WS-C2960+24TC-L on IOS 15.0(2)SE.")
        );
        // with no list open, Esc cancels
        app.on_key(key(KeyCode::Char('h')), &mut lib);
        app.on_key(key(KeyCode::Esc), &mut lib);
        assert_eq!(app.mode, Mode::List);
    }

    #[test]
    fn a_hardware_change_reads_green_one_not_made_plain_and_one_not_kept_red() {
        let (t, mut lib) = fleet("hardware-tone");
        let mut app = App::new();
        app.select_name(&lib, "lab2960x");
        app.on_key(key(KeyCode::Char('h')), &mut lib);
        app.on_key(key(KeyCode::Enter), &mut lib);
        assert_eq!(
            status_line(&app, &lib),
            (
                "lab2960x stays WS-C2960X-24PS-L on IOS 15.2(7)E.".to_string(),
                Color::Reset
            )
        );
        let step = |app: &mut App, lib: &mut Library| {
            app.on_key(key(KeyCode::Char('h')), lib);
            app.on_key(key(KeyCode::Right), lib);
            app.on_key(key(KeyCode::Enter), lib);
        };
        step(&mut app, &mut lib);
        let (text, colour) = status_line(&app, &lib);
        assert!(text.starts_with("lab2960x is now "), "{text}");
        assert_eq!(colour, Color::Green);
        // a list that cannot be written keeps the hardware and says why in red
        let index = t.0.join("lib").join("switches.json");
        std::fs::remove_file(&index).unwrap();
        std::fs::create_dir(&index).unwrap();
        let before = lib.switches[app.current(&lib).unwrap()].stack.clone();
        step(&mut app, &mut lib);
        let (text, colour) = status_line(&app, &lib);
        assert!(text.contains("switches.json"), "{text}");
        assert_eq!(colour, Color::Red);
        assert_eq!(lib.switches[app.current(&lib).unwrap()].stack, before);
    }

    #[test]
    fn times_read_as_dates() {
        assert_eq!(when(0), "1970-01-01 00:00 UTC");
        assert_eq!(when(1_791_235_200), "2026-10-05 21:20 UTC");
    }

    #[test]
    fn keys_read_together_are_a_paste_and_never_commands() {
        let press = |c| Event::Key(key(KeyCode::Char(c)));
        // a Windows console's paste: each key pressed and released
        let keys = |s: &str| -> Vec<Event> {
            s.chars()
                .flat_map(|c| {
                    let code = if c == '\n' {
                        KeyCode::Enter
                    } else {
                        KeyCode::Char(c)
                    };
                    [
                        Event::Key(key(code)),
                        Event::Key(KeyEvent::new_with_kind(
                            code,
                            KeyModifiers::NONE,
                            KeyEventKind::Release,
                        )),
                    ]
                })
                .collect()
        };
        let (_t, mut lib) = fleet("paste");
        // on the list a paste opens the file browser with it instead of acting on d and y
        let mut app = App::new();
        let mut read = keys("C:\\dy.cfg");
        read.push(Event::Key(key(KeyCode::Down)));
        assert!(act(&mut app, &mut lib, read, true).is_none());
        assert!(
            matches!(&app.mode, Mode::Browse(b) if b.query.text == "C:\\dy.cfg"),
            "{:?}",
            app.mode
        );
        app.on_key(key(KeyCode::Esc), &mut lib);
        let mut app = App::new();
        assert!(act(&mut app, &mut lib, keys("dy\n"), true).is_none());
        assert_eq!(lib.switches.len(), 3);
        // a path pasted with its line end loads, as typed in the browser and Enter
        let mut app = App::new();
        let a = act(&mut app, &mut lib, keys("tests/c9200-design.toml\n"), true);
        assert!(matches!(a, Some(Action::Open(_))), "{:?}", app.mode);
        assert_eq!(lib.switches.len(), 4);
        // Enters alone are keys: the first opens the selected switch, no browser loads anything
        let mut app = App::new();
        let a = act(&mut app, &mut lib, keys("\n\n\n"), true);
        assert!(
            matches!(a, Some(Action::Open(_))) && matches!(app.mode, Mode::List),
            "{:?}",
            app.mode
        );
        assert_eq!(lib.switches.len(), 4);
        // two keys read together are typing
        let mut app = App::new();
        act(&mut app, &mut lib, vec![press('j'), press('j')], true);
        assert_eq!(app.selected, 2);
        // a key that opens a field takes the rest as typing, Enter included: typed ahead, as a script
        // or a fast typist does
        let mut app = App::new();
        act(&mut app, &mut lib, keys("/9300\n"), true);
        assert!(matches!(app.mode, Mode::List) && app.filter.text == "9300");
        let mut app = App::new();
        act(&mut app, &mut lib, keys("nlab9\n"), true);
        assert!(
            matches!(&app.mode, Mode::Hardware { name, .. } if name == "lab9"),
            "{:?}",
            app.mode
        );
        // off Windows keys act one by one, and a paste is one event
        let mut app = App::new();
        act(&mut app, &mut lib, keys("jj"), false);
        assert_eq!(app.selected, 2);
        // Ctrl-C ends a run and is no text; AltGr's Ctrl and Alt type
        let altgr = Event::Key(KeyEvent::new(
            KeyCode::Char('\\'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ));
        let run = [press('a'), altgr, Event::Key(ctrl('c')), press('b')];
        assert_eq!(typed_run(run.iter()), ("a\\".to_string(), 2));
    }

    fn key_of(c: char, kind: KeyEventKind) -> Event {
        let code = if c == '\n' {
            KeyCode::Enter
        } else {
            KeyCode::Char(c)
        };
        Event::Key(KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind))
    }

    /// A console that delivers each event at its time (ms after the first), as `gather` polls it.
    fn console(
        timed: Vec<(u64, Event)>,
    ) -> impl FnMut(Duration) -> Result<(Option<Event>, Duration), String> {
        let mut now = Duration::ZERO;
        let mut queue: std::collections::VecDeque<(Duration, Event)> = timed
            .into_iter()
            .map(|(ms, e)| (Duration::from_millis(ms), e))
            .collect();
        move |wait| match queue.front() {
            Some((at, _)) if *at <= now + wait => {
                let (at, e) = queue.pop_front().unwrap_or_else(|| unreachable!());
                let took = at.saturating_sub(now);
                now = now.max(at);
                Ok((Some(e), took))
            }
            _ => {
                now += wait;
                Ok((None, wait))
            }
        }
    }

    /// `text` as a busy Windows console types a paste: each key pressed and at once released, a key
    /// every `apart` ms.
    fn typed_slowly(text: &str, apart: u64) -> (Event, Vec<(u64, Event)>) {
        let mut timed = Vec::new();
        for (n, c) in text.chars().enumerate() {
            let at = n as u64 * apart;
            timed.push((at, key_of(c, KeyEventKind::Press)));
            timed.push((at, key_of(c, KeyEventKind::Release)));
        }
        let first = timed.remove(0).1;
        (first, timed)
    }

    #[test]
    fn a_paste_a_busy_console_types_in_pieces_stays_one_burst() {
        let (_t, mut lib) = fleet("trickle");
        // keys 30 ms apart, as the console typed a paste with every thread busy; read a piece at a
        // time, its d and y would remove a switch
        let path = "C:\\dy\\core.cfg";
        let (first, rest) = typed_slowly(path, 30);
        let read = gather(first, true, console(rest)).unwrap();
        assert_eq!(read.len(), 2 * path.len());
        let mut app = App::new();
        assert!(act(&mut app, &mut lib, read, true).is_none());
        assert!(
            matches!(&app.mode, Mode::Browse(b) if b.query.text == path),
            "{:?}",
            app.mode
        );
        assert_eq!(lib.switches.len(), 3);
        // each key released 20 ms after it is pressed, which a busy console also does
        let path = "dy.cfg";
        let mut timed: Vec<(u64, Event)> = path
            .chars()
            .enumerate()
            .flat_map(|(n, c)| {
                let at = n as u64 * 40;
                [
                    (at, key_of(c, KeyEventKind::Press)),
                    (at + 20, key_of(c, KeyEventKind::Release)),
                ]
            })
            .collect();
        let first = timed.remove(0).1;
        let read = gather(first, true, console(timed)).unwrap();
        assert_eq!(read.len(), 2 * path.len());
        let mut app = App::new();
        assert!(act(&mut app, &mut lib, read, true).is_none());
        assert!(
            matches!(&app.mode, Mode::Browse(b) if b.query.text == path),
            "{:?}",
            app.mode
        );
        assert_eq!(lib.switches.len(), 3);
        // a pause longer than the busy machine makes ends it
        let (first, mut rest) = typed_slowly("ab", 30);
        rest.push((900, key_of('c', KeyEventKind::Press)));
        assert_eq!(gather(first, true, console(rest)).unwrap().len(), 4);
        // only on the list: a field takes keys one at a time as well
        let (first, rest) = typed_slowly("abc", 30);
        assert_eq!(gather(first, false, console(rest)).unwrap().len(), 2);
    }

    #[test]
    fn a_person_s_keys_and_a_key_held_down_are_never_one_burst() {
        // a key held for 90 ms acts as soon as it is read
        let press = |c| key_of(c, KeyEventKind::Press);
        let release = |c| key_of(c, KeyEventKind::Release);
        let read = gather(press('d'), true, console(vec![(90, release('d'))])).unwrap();
        assert_eq!(read, [press('d')]);
        // a fast typist's keys, 100 ms apart and each held 50 ms
        let typed: Vec<(u64, Event)> = (0..6)
            .flat_map(|n| [(n * 100 + 50, release('j')), (n * 100 + 100, press('j'))])
            .collect();
        assert_eq!(gather(press('j'), true, console(typed)).unwrap().len(), 1);
        // a key held down repeats every 33 ms with no release between: a person's keys
        let repeats: Vec<(u64, Event)> = (1..6).map(|n| (n * 33, press('j'))).collect();
        let read = gather(press('j'), true, console(repeats)).unwrap();
        assert!(read.len() < 3, "{read:?}");
        assert_eq!(typing(&read), Typing::Person);
        // repeats read together, as a busy machine reads them, are keys each
        let (_t, mut lib) = fleet("repeat");
        let mut app = App::new();
        assert!(act(&mut app, &mut lib, vec![press('j'); 3], true).is_none());
        assert!(matches!(app.mode, Mode::List), "{:?}", app.mode);
        assert_eq!(app.selected, 2);
        assert_eq!(typed_run([press('j'), press('j')].iter()), ("j".into(), 1));
    }

    #[test]
    fn ctrl_c_cancels_what_is_open_and_quits_only_from_the_list() {
        let (_t, mut lib) = fleet("ctrl-c");
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('n')), &mut lib);
        app.on_key(key(KeyCode::Char('x')), &mut lib);
        assert!(matches!(app.on_key(ctrl('c'), &mut lib), Action::None));
        assert!(matches!(app.mode, Mode::List), "{:?}", app.mode);
        assert!(matches!(app.on_key(ctrl('c'), &mut lib), Action::Quit));
        // Alt and a letter is no list command
        let alt_q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT);
        assert!(matches!(app.on_key(alt_q, &mut lib), Action::None));
    }

    #[test]
    fn every_status_starts_in_the_same_column() {
        let (_t, lib) = fleet("column");
        let mut app = App::new();
        // a status that starts with a space
        app.status = Some(" lab2960x is now WS-C2960X-24PS-L on IOS 15.2(7)E".into());
        let (text, _) = status_line(&app, &lib);
        assert!(text.starts_with("lab2960x is now"), "{text}");
        let t = render(&app, &lib, 80, 24);
        assert_eq!(t.backend().buffer()[(1, 22)].symbol(), "l");
    }

    #[test]
    fn a_long_status_keeps_what_happened_and_the_file_it_names() {
        let (_t, lib) = fleet("long-status");
        let mut app = App::new();
        let path = format!("D:\\{}\\lab2960x.cfg", "exports\\by-site\\".repeat(12));
        app.done(format!("Wrote lab2960x's kept configuration to {path}."));
        let (text, colour) = status_line(&app, &lib);
        assert!(
            text.starts_with("Wrote lab2960x's kept configuration to D:\\exports")
                && text.ends_with("\\lab2960x.cfg.")
                && text.contains('…'),
            "{text}"
        );
        assert_eq!(colour, Color::Green);
        // a problem keeps its reason
        app.fail(format!(
            "{path}: The system cannot find the path specified."
        ));
        let (text, _) = status_line(&app, &lib);
        assert!(
            text.starts_with("D:\\exports") && text.ends_with("cannot find the path specified."),
            "{text}"
        );
    }

    #[test]
    fn a_key_for_a_switch_says_why_when_none_is_shown() {
        let t = TempDir::new("tui-none");
        let mut lib = Library::open(t.0.clone()).unwrap();
        let mut app = App::new();
        for k in [
            KeyCode::Enter,
            KeyCode::Delete,
            KeyCode::Char('h'),
            KeyCode::Char('d'),
            KeyCode::Char('r'),
            KeyCode::Char('c'),
            KeyCode::Char('v'),
            KeyCode::Char('e'),
        ] {
            app.status = None;
            assert_eq!(app.on_key(key(k), &mut lib), Action::None);
            assert_eq!(app.mode, Mode::List);
            assert_eq!(
                app.status.as_deref(),
                Some("No switch yet: a loads one, n builds one."),
                "{k:?}"
            );
        }
        let (_t, mut lib) = fleet("none-shown");
        app.filter = Input::new("no-such-switch");
        app.on_key(key(KeyCode::Enter), &mut lib);
        assert_eq!(
            app.status.as_deref(),
            Some("No switch matches the filter: Esc clears it.")
        );
    }

    #[test]
    fn a_filter_that_shows_no_switch_leaves_the_selection_where_it_was() {
        let (_t, mut lib) = fleet("filter-none");
        let mut app = App::new();
        app.select_name(&lib, "lab2960x");
        app.on_key(key(KeyCode::Char('/')), &mut lib);
        typed(&mut app, &mut lib, "labq");
        assert!(app.rows(&lib).is_empty());
        app.on_key(key(KeyCode::Esc), &mut lib);
        assert_eq!(lib.switches[app.current(&lib).unwrap()].name, "lab2960x");
        // a filter kept while it shows nothing, then cleared from the list
        app.on_key(key(KeyCode::Char('/')), &mut lib);
        typed(&mut app, &mut lib, "labq");
        app.on_key(key(KeyCode::Enter), &mut lib);
        app.on_key(key(KeyCode::Esc), &mut lib);
        assert_eq!(lib.switches[app.current(&lib).unwrap()].name, "lab2960x");
    }

    #[test]
    fn a_narrow_list_drops_a_date_no_switch_has_first() {
        let (_t, mut lib) = fleet("blank-column");
        lib.switches[1].saved_at = None;
        let app = App::new();
        let small = screen_at(&app, &lib, 60, 15);
        assert!(
            small.contains("Opened") && !small.contains("Saved"),
            "{small}"
        );
    }

    #[test]
    fn the_help_pages_by_the_keys_a_page_shows_and_stops_at_the_last() {
        let (_t, mut lib) = fleet("help-pages");
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('?')), &mut lib);
        // at 80 by 24 every key shows: nothing scrolls
        screen_at(&app, &lib, 80, 24);
        app.on_key(key(KeyCode::PageDown), &mut lib);
        app.on_key(key(KeyCode::Down), &mut lib);
        assert_eq!(app.help, Some(0));
        // at 60 by 20 a page down shows the key after the last one shown, and every key is seen:
        // a row shows its keys, three spaces and more, then what they do
        let mut seen = std::collections::BTreeSet::new();
        let mut last = None;
        loop {
            let shown = screen_at(&app, &lib, 60, 20);
            for l in shown.lines() {
                let Some((k, what)) = l.trim_start_matches(['│', ' ']).split_once("   ") else {
                    continue;
                };
                for (keys, does) in LIST_KEYS {
                    let first = does.split(' ').next().unwrap_or_default();
                    if k.trim() == keys.join("  ") && what.trim_start().starts_with(first) {
                        seen.insert(keys[0]);
                    }
                }
            }
            if app.help == last {
                break;
            }
            last = app.help;
            app.on_key(key(KeyCode::PageDown), &mut lib);
        }
        assert_eq!(seen.len(), LIST_KEYS.len(), "{seen:?}");
        let end = app.help;
        app.on_key(key(KeyCode::Down), &mut lib);
        assert_eq!(app.help, end, "the last page stays");
        app.on_key(key(KeyCode::PageUp), &mut lib);
        assert!(app.help < end);
        app.on_key(key(KeyCode::Home), &mut lib);
        assert_eq!(app.help, Some(0));
    }

    #[test]
    fn opening_says_how_many_lines_loaded_and_from_where() {
        let (_t, mut lib) = fleet("loaded");
        assert_eq!(
            loaded(&lib.switches[0], 1),
            "core1: 1 configuration line loaded from C:/captures/core1-running-config.txt."
        );
        lib.switches[0].source = BUILT.into();
        assert_eq!(
            loaded(&lib.switches[0], 2),
            "core1, built in vswitch: 2 configuration lines loaded."
        );
    }

    #[test]
    fn a_switch_another_vswitch_kept_outlives_the_list_s_next_save() {
        let (t, mut lib) = fleet("two-lists");
        // `vswitch load`, or a second list, keeps a switch while this one is open
        let mut other = Library::open(t.0.join("lib")).unwrap();
        other
            .add(
                "idf7",
                "C9300X-24Y",
                crate::release::DEFAULT,
                "idf7.cfg",
                "hostname idf7\n",
            )
            .unwrap();
        let mut app = App::new();
        app.select_name(&lib, "core1");
        app.on_key(key(KeyCode::Char('r')), &mut lib);
        app.on_key(ctrl('u'), &mut lib);
        typed(&mut app, &mut lib, "mdf1");
        app.on_key(key(KeyCode::Enter), &mut lib);
        let kept = Library::open(t.0.join("lib")).unwrap();
        let names: Vec<&str> = kept.switches.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"idf7") && names.contains(&"mdf1"),
            "{names:?}"
        );
        assert!(lib.position("idf7").is_some(), "the list shows it too");
    }

    #[test]
    fn the_builder_and_the_popups_stay_inside_the_list_s_frame() {
        let (_t, mut lib) = fleet("frames");
        let mut app = App::new();
        app.select_name(&lib, "lab2960x");
        for (w, h) in [(80, 24), (60, 15), (132, 43)] {
            // the builder takes the list's place, leaving the status and footer rows to them
            app.on_key(key(KeyCode::Char('h')), &mut lib);
            let s = screen_at(&app, &lib, w, h);
            for joined in ["┌┌", "┐┐", "└│", "│┘", "└└", "┘┘"] {
                assert!(!s.contains(joined), "{w}x{h} {joined}:\n{s}");
            }
            let rows: Vec<&str> = s.lines().collect();
            for r in &rows[usize::from(h) - 2..] {
                assert!(!r.contains(['│', '└', '┘']), "{w}x{h}:\n{s}");
            }
            assert!(!s.contains("Source"), "another switch's details:\n{s}");
            app.on_key(key(KeyCode::Esc), &mut lib);
            // a popup stands inside the list's frame, crossing no other box
            for open in ['d', 'r'] {
                app.on_key(key(KeyCode::Char(open)), &mut lib);
                let s = screen_at(&app, &lib, w, h);
                let rows: Vec<&str> = s.lines().collect();
                let top = rows
                    .iter()
                    .position(|r| r.starts_with("┌ Switches"))
                    .unwrap();
                let bottom = rows.iter().rposition(|r| r.starts_with('└')).unwrap();
                for r in &rows[top + 1..bottom] {
                    assert!(r.starts_with('│') && r.ends_with('│'), "{w}x{h}:\n{s}");
                }
                assert!(!s.contains("Source"), "{w}x{h}:\n{s}");
                app.on_key(key(KeyCode::Esc), &mut lib);
                assert_eq!(app.mode, Mode::List);
            }
        }
    }
}
