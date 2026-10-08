//! The hardware builder: a stack member by member, each a model and its network module, and the
//! release it runs, with what Cisco does not build, stack or run marked as it is chosen. Each value
//! is picked from a list filtered as it is typed, every choice marked with Cisco's reason against it.

use crate::grammars;
use crate::platform::{
    self, CHASSIS, Chassis, Family, MODELS, MODULES, Poe, PoeKind, Ports, Stack, Uplinks,
};
use crate::release::{self, Release};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use std::cell::Cell;

const PICK: Style = Style::new().fg(Color::Black).bg(Color::Cyan);
/// the focused cell of a member with a problem, red as the rest of its row
const PICK_BAD: Style = Style::new().fg(Color::Black).bg(Color::Red);
const HINT: Style = Style::new().fg(Color::Gray);
const BAD: Style = Style::new().fg(Color::Red);
const NOTE: Style = Style::new().fg(Color::Yellow);
const GOOD: Style = Style::new().fg(Color::Green);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
const PLAIN: Style = Style::new();
/// The width of a label in the panel below the members (`IOS XE  17.15`).
const LABEL: usize = 8;

/// What Tab moves between: the selected member's number, model and network module, then a Catalyst
/// 9400's chassis and the release.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Field {
    Number,
    Model,
    Module,
    Chassis,
    Release,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub number: u32,
    /// index into MODELS
    pub model: usize,
    /// index into MODULES
    pub module: Option<usize>,
    /// the model a design or configuration named that vswitch does not know, which the first one
    /// offered stands in for until a model is chosen
    pub unknown_model: Option<String>,
    /// the network module it named that vswitch does not know, left out until a module is chosen
    pub unknown_module: Option<String>,
}

/// A value a list offers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Choice {
    Number(u32),
    Model(usize),
    Module(Option<usize>),
    /// index into CHASSIS
    Chassis(usize),
    Release(Release),
}

#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Heading(String),
    Item {
        choice: Choice,
        name: String,
        info: String,
        /// what Cisco does not build, stack or run once it is chosen
        reasons: Vec<String>,
        /// what choosing it changes besides itself
        note: Option<String>,
        /// lower case without dashes, as the filter is matched
        search: String,
    },
}

/// A list of choices for one field, open over the builder.
#[derive(Clone, Debug, PartialEq)]
struct Picker {
    field: Field,
    entries: Vec<Entry>,
    filter: String,
    /// the highlighted item's index in `entries`
    cursor: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tone {
    Note,
    Problem,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Builder {
    pub members: Vec<Member>,
    pub row: usize,
    pub field: Field,
    pub release: Release,
    /// boxed: the builder lives inline in the switch list's mode
    picker: Option<Box<Picker>>,
    /// the key list, open over the builder or a list, and the first of its lines shown
    help: Option<usize>,
    /// from the last draw: the key list's lines, and how many of them it shows
    help_lines: Cell<(usize, usize)>,
    /// what the last key did or refused, until the next key
    message: Option<(Tone, String)>,
    /// the 9400 chassis last chosen, which added and changed members are seated in while it fits
    chosen: Option<usize>,
}

/// Whom a problem is about, as a list's choices are compared: the stack, the member the list is for,
/// or another member, by its model and module, which a choice for this one leaves as they were.
#[derive(Clone, Debug, PartialEq)]
enum Whose {
    Stack,
    Own,
    Other(usize, Option<usize>),
}

/// A problem found: whom it is about, what it says, and how the builder names it.
type Found = (Whose, String, String);

/// A choice as a list offers it, before its reasons are known.
struct Offer {
    choice: Choice,
    name: String,
    info: String,
    group: String,
    search: String,
}

fn model_index(name: &str) -> Option<usize> {
    MODELS
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(name))
}

fn module_index(name: &str) -> Option<usize> {
    MODULES
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(name))
}

fn is_sup(model: usize) -> bool {
    platform::is_supervisor(MODELS[model].name)
}

/// The models Left and Right step through: the model's family, a 9400's supervisors and line cards
/// apart.
fn kin(model: usize) -> Vec<usize> {
    let family = MODELS[model].family;
    (0..MODELS.len())
        .filter(|&k| MODELS[k].family == family && is_sup(k) == is_sup(model))
        .collect()
}

/// The network modules a model takes, in MODULES order.
fn fits(model: usize) -> Vec<usize> {
    match MODELS[model].uplinks {
        Uplinks::Modules(list) => (0..MODULES.len())
            .filter(|&i| list.iter().any(|n| n.eq_ignore_ascii_case(MODULES[i].name)))
            .collect(),
        _ => vec![],
    }
}

/// A model's heading in the model list: its family, a 9400's supervisors and line cards apart.
fn group(model: usize) -> String {
    match MODELS[model].family {
        Family::C9400 if is_sup(model) => "Catalyst 9400 supervisors".into(),
        Family::C9400 => "Catalyst 9400 line cards".into(),
        f => f.name().into(),
    }
}

/// Text as the filter matches it: lower case, dashes dropped, so `2960x48` finds WS-C2960X-48TS-L.
fn norm(text: &str) -> String {
    text.to_lowercase().replace('-', "")
}

/// The next or previous of `len` values, wrapping.
fn wrap_step(i: usize, len: usize, up: bool) -> usize {
    if up {
        (i + 1) % len
    } else {
        (i + len - 1) % len
    }
}

/// An interface type as its speed: `TwentyFiveGigE` is 25G.
fn speed(ty: &str) -> &str {
    match ty {
        "FastEthernet" => "100M",
        "GigabitEthernet" => "1G",
        "TwoGigabitEthernet" => "2.5G",
        "FiveGigabitEthernet" => "5G",
        "TenGigabitEthernet" => "10G",
        "TwentyFiveGigE" => "25G",
        "FortyGigabitEthernet" => "40G",
        "HundredGigE" => "100G",
        other => other,
    }
}

/// Ports counted by speed, `24x 1G + 24x 10G`; the application hosting port is left out.
fn counts(p: Ports) -> String {
    p.iter()
        .filter(|(ty, _, _)| *ty != "AppGigabitEthernet")
        .map(|&(ty, lo, hi)| format!("{}x {}", hi - lo + 1, speed(ty)))
        .collect::<Vec<_>>()
        .join(" + ")
}

fn poe_kind(k: PoeKind) -> &'static str {
    match k {
        PoeKind::PoePlus => "PoE+",
        PoeKind::Upoe => "UPOE",
        PoeKind::UpoePlus => "UPOE+",
    }
}

fn poe_text(p: Option<Poe>) -> String {
    match p {
        None => "none".into(),
        Some(Poe { kind, watts }) => {
            let each = format!("{} {} W a port", poe_kind(kind), kind.max_mw() / 1000);
            match watts {
                Some(w) => format!("{each}, {w} W budget"),
                None => format!("{each}, the budget the chassis power supplies give"),
            }
        }
    }
}

/// A model in a list: its ports, PoE and uplinks.
fn model_info(model: usize) -> String {
    let m = &MODELS[model];
    let mut s = counts(m.ports);
    if let Some(p) = m.poe {
        s.push(' ');
        s.push_str(poe_kind(p.kind));
    }
    match m.uplinks {
        Uplinks::Modules(_) => s.push_str(", network module"),
        Uplinks::Fixed(p) | Uplinks::Slot0(p) => s.push_str(&format!(", {} uplinks", counts(p))),
        Uplinks::None => {}
    }
    s
}

/// A member's ports in the members table: the front panel, its PoE, then the uplinks.
fn member_ports(m: &Member) -> String {
    let model = &MODELS[m.model];
    let mut s = counts(model.ports);
    if let Some(p) = model.poe {
        s.push(' ');
        s.push_str(poe_kind(p.kind));
    }
    let up = match model.uplinks {
        Uplinks::Fixed(p) | Uplinks::Slot0(p) => Some(p),
        _ => m.module.map(|i| MODULES[i].ports),
    };
    if let Some(p) = up {
        s.push_str(", ");
        s.push_str(&counts(p));
    }
    s
}

/// The releases hardware runs, from the release notes: `IOS XE 17.5 and later`.
fn runs(since: Option<&str>, until: Option<&str>) -> Option<String> {
    let named = |t: &str| Release::parse(t).map_or(t.to_string(), Release::named);
    let bare = |t: &str| Release::parse(t).map_or(t.to_string(), |r| r.to_string());
    match (since, until) {
        (Some(s), Some(u)) => Some(format!("{} to {}", named(s), bare(u))),
        (Some(s), None) => Some(format!("{} and later", named(s))),
        (None, Some(u)) => Some(format!("up to {}", named(u))),
        (None, None) => None,
    }
}

/// The interfaces ports name on a member: `TwentyFiveGigE1/0/1-24`; a two-part family's
/// `FastEthernet0/1-24`.
fn interfaces(p: Ports, family: Family, member: u32, slot: u32) -> String {
    p.iter()
        .map(|&(ty, lo, hi)| {
            let at = if family.two_part() {
                format!("{ty}{}/", family.front_slot())
            } else {
                format!("{ty}{member}/{slot}/")
            };
            if lo == hi {
                format!("{at}{lo}")
            } else {
                format!("{at}{lo}-{hi}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `1`, `1 and 4`, `1, 2 and 5`.
fn and_list(v: &[u32]) -> String {
    match v {
        [] => String::new(),
        [one] => one.to_string(),
        [init @ .., last] => {
            let head: Vec<String> = init.iter().map(u32::to_string).collect();
            format!("{} and {last}", head.join(", "))
        }
    }
}

/// `C9404R`, `C9404R or C9407R`, `C9404R, C9407R or C9410R`.
fn or_list(v: &[&str]) -> String {
    match v {
        [] => String::new(),
        [one] => one.to_string(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}

/// What a Catalyst 9400 slot holds in each chassis (supervisor data sheet).
fn slot_roles(n: u32) -> String {
    let named = |pick: &dyn Fn(&Chassis) -> bool| -> Vec<&str> {
        CHASSIS.iter().filter(|c| pick(c)).map(|c| c.name).collect()
    };
    let sup = named(&|c| c.supervisors.contains(&n));
    let card = named(&|c| c.cards.contains(&n));
    let mut parts = vec![];
    if !sup.is_empty() {
        parts.push(format!("supervisor in a {}", or_list(&sup)));
    }
    if !card.is_empty() {
        parts.push(format!("line card in a {}", or_list(&card)));
    }
    if parts.is_empty() {
        "no chassis has it".into()
    } else {
        parts.join("; ")
    }
}

fn chassis_info(c: &Chassis) -> String {
    format!(
        "{} slots: supervisors {}, line cards {}",
        c.supervisors.len() + c.cards.len(),
        and_list(c.supervisors),
        c.slots_text
    )
}

/// `text` cut to `w` columns with an ellipsis, or padded to them.
fn cell(text: &str, w: usize) -> String {
    let n = text.chars().count();
    if n <= w {
        format!("{text:<w$}")
    } else if w == 0 {
        String::new()
    } else {
        text.chars().take(w - 1).collect::<String>() + "…"
    }
}

/// Words to lines of `w` columns; a word longer than a line is cut.
fn wrap(text: &str, w: usize) -> Vec<String> {
    let w = w.max(1);
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let len = line.chars().count();
        if len > 0 && len + 1 + word.chars().count() > w {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
        while line.chars().count() > w {
            out.push(line.chars().take(w).collect());
            line = line.chars().skip(w).collect();
        }
    }
    out.push(line);
    out
}

fn wrapped(text: &str, style: Style, w: usize) -> Vec<Line<'static>> {
    wrap(text, w)
        .into_iter()
        .map(|l| Line::styled(l, style))
        .collect()
}

/// Text wrapped to `w` columns, the lines after the first indented by `indent`.
fn hanging(text: &str, style: Style, w: usize, indent: usize) -> Vec<Line<'static>> {
    let first = wrap(text, w);
    let Some(head) = first.first() else {
        return vec![];
    };
    let rest = text.chars().skip(head.chars().count()).collect::<String>();
    let mut out = vec![Line::styled(head.clone(), style)];
    if !rest.trim().is_empty() {
        out.extend(
            wrap(rest.trim_start(), w.saturating_sub(indent).max(1))
                .into_iter()
                .map(|l| Line::styled(format!("{}{l}", " ".repeat(indent)), style)),
        );
    }
    out
}

/// A label and its value, the value wrapped under itself.
fn labeled(label: &str, value: &str, style: Style, w: usize) -> Vec<Line<'static>> {
    wrap(value, w.saturating_sub(LABEL))
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            let head = if i == 0 {
                format!("{label:<LABEL$}")
            } else {
                " ".repeat(LABEL)
            };
            Line::from(vec![Span::styled(head, HINT), Span::styled(part, style)])
        })
        .collect()
}

/// `--stack` text broken after its commas, so each line holds whole members.
fn spec_lines(spec: &str, w: usize) -> Vec<Line<'static>> {
    let room = w.saturating_sub(LABEL).max(1);
    let mut lines: Vec<String> = vec![String::new()];
    let parts: Vec<&str> = spec.split(',').collect();
    for (i, part) in parts.iter().enumerate() {
        let piece = if i + 1 < parts.len() {
            format!("{part},")
        } else {
            part.to_string()
        };
        let last = lines.last_mut().expect("one line");
        if !last.is_empty() && last.chars().count() + piece.chars().count() > room {
            lines.push(piece);
        } else {
            last.push_str(&piece);
        }
    }
    lines
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            let head = if i == 0 {
                format!("{:<LABEL$}", "Stack")
            } else {
                " ".repeat(LABEL)
            };
            Line::from(vec![Span::styled(head, HINT), Span::raw(cell(&part, room))])
        })
        .collect()
}

/// A reason as a list row shows it beside the name: without the name it repeats or the document
/// it cites, which the highlighted choice shows in full.
fn brief(reason: &str, name: &str) -> String {
    let text = reason
        .strip_prefix(&format!("the {name} "))
        .or_else(|| reason.strip_prefix(&format!("{name} ")))
        .unwrap_or(reason);
    match text.find(" (") {
        Some(i) => text[..i].to_string(),
        None => text.to_string(),
    }
}

/// A bold name and what follows it on one line, cut to the width.
fn titled(name: &str, rest: &str, w: usize) -> Line<'static> {
    let n = name.chars().count();
    if n >= w {
        return Line::styled(cell(name, w), BOLD);
    }
    let rest = if n + rest.chars().count() > w {
        cell(rest, w - n)
    } else {
        rest.to_string()
    };
    Line::from(vec![Span::styled(name.to_string(), BOLD), Span::raw(rest)])
}

/// Key hints in order, as many as fit: the key plain, what it does gray.
fn hints(pairs: &[(&str, &str)], w: usize) -> Line<'static> {
    let mut spans = vec![];
    let mut used = 0;
    for (key, what) in pairs {
        let len = key.chars().count() + 1 + what.chars().count() + if used > 0 { 2 } else { 0 };
        if used + len > w {
            break;
        }
        if used > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::raw(key.to_string()));
        spans.push(Span::styled(format!(" {what}"), HINT));
        used += len;
    }
    Line::from(spans)
}

/// Whether a key types its character: plain, with Shift, or through AltGr, which arrives as Control
/// and Alt together.
fn types(key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    ctrl == alt
}

/// The builder's box: most of the screen, all but a margin of a small one.
fn frame(area: Rect) -> Rect {
    let part = |n: u16, pct: u32| (u32::from(n) * pct / 100) as u16;
    let w = part(area.width, 92)
        .max(area.width.min(78))
        .min(area.width)
        .min(120);
    let h = part(area.height, 82)
        .max(area.height.min(22))
        .min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// A box's inside, a column in from each border.
fn pad(r: Rect) -> Rect {
    Rect {
        x: r.x + 1,
        width: r.width.saturating_sub(2),
        ..r
    }
}

impl Picker {
    fn matches(&self, search: &str) -> bool {
        norm(&self.filter)
            .split_whitespace()
            .all(|term| search.contains(term))
    }

    /// The entries the filter leaves: each item it matches, under its heading.
    fn visible(&self) -> Vec<usize> {
        let mut out = vec![];
        let mut heading = None;
        for (i, e) in self.entries.iter().enumerate() {
            match e {
                Entry::Heading(_) => heading = Some(i),
                Entry::Item { search, .. } if self.matches(search) => {
                    if let Some(h) = heading.take() {
                        out.push(h);
                    }
                    out.push(i);
                }
                Entry::Item { .. } => {}
            }
        }
        out
    }

    fn items(&self) -> Vec<usize> {
        self.visible()
            .into_iter()
            .filter(|&i| matches!(self.entries[i], Entry::Item { .. }))
            .collect()
    }

    /// After the filter changes: the item named exactly as typed, else the highlighted one while it
    /// still matches, else the first that does.
    fn refilter(&mut self) {
        let items = self.items();
        let typed = norm(&self.filter);
        let exact = items.iter().copied().find(|&i| {
            matches!(&self.entries[i], Entry::Item { name, .. } if !typed.is_empty() && norm(name) == typed)
        });
        self.cursor = exact
            .or(self.cursor.filter(|c| items.contains(c)))
            .or(items.first().copied());
    }

    fn step(&mut self, by: isize) {
        let items = self.items();
        let Some(at) = self.cursor.and_then(|c| items.iter().position(|&i| i == c)) else {
            self.cursor = items.first().copied();
            return;
        };
        let to = (at as isize)
            .saturating_add(by)
            .clamp(0, items.len() as isize - 1);
        self.cursor = Some(items[to as usize]);
    }

    fn choice(&self) -> Option<Choice> {
        match self.entries.get(self.cursor?)? {
            Entry::Item { choice, .. } => Some(*choice),
            Entry::Heading(_) => None,
        }
    }
}

impl Builder {
    /// One member: the first model offered.
    pub fn new() -> Self {
        Builder::of(
            vec![Member {
                number: 1,
                model: 0,
                module: None,
                unknown_model: None,
                unknown_module: None,
            }],
            release::DEFAULT,
        )
    }

    fn of(members: Vec<Member>, release: Release) -> Self {
        Builder {
            members,
            row: 0,
            field: Field::Model,
            release,
            picker: None,
            help: None,
            help_lines: Cell::new((0, 0)),
            message: None,
            chosen: None,
        }
    }

    /// The builder for a `--stack` text. A model it does not know is stood in for by the first one
    /// offered and a module it does not know is left out, each named as a problem until a model, or
    /// a module, is chosen for that member (a design's `C9999-48Z` is never kept as a C9300X-24Y
    /// unseen).
    pub fn from_spec(spec: &str, release: Release) -> Self {
        if let Ok(s) = platform::parse_stack(spec)
            && !s.members.is_empty()
        {
            let members = s
                .members
                .iter()
                .map(|m| Member {
                    number: m.number,
                    model: model_index(&m.model).unwrap_or(0),
                    module: m.module.as_deref().and_then(module_index),
                    unknown_model: None,
                    unknown_module: None,
                })
                .collect();
            return Builder::of(members, release);
        }
        let mut members: Vec<Member> = Vec::new();
        for (i, part) in spec.split(',').enumerate() {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (number, m) = match part.split_once('=') {
                Some((n, rest)) => (n.trim().parse().unwrap_or(i as u32 + 1), rest.trim()),
                None => (i as u32 + 1, part),
            };
            // as `platform::parse_stack` reads it: a model's own name may hold a `+`
            let cut = std::iter::once(m.len())
                .chain(m.rmatch_indices('+').map(|(i, _)| i))
                .find(|&i| model_index(m[..i].trim()).is_some())
                .or_else(|| m.find('+'))
                .unwrap_or(m.len());
            let (model, module) = (m[..cut].trim(), m.get(cut + 1..).map(str::trim));
            let module = module.filter(|md| !md.is_empty());
            members.push(Member {
                number,
                model: model_index(model).unwrap_or(0),
                module: module.and_then(module_index),
                unknown_model: model_index(model).is_none().then(|| model.to_string()),
                unknown_module: module
                    .filter(|md| module_index(md).is_none())
                    .map(str::to_string),
            });
        }
        if members.is_empty() {
            return Builder {
                release,
                ..Builder::new()
            };
        }
        Builder::of(members, release)
    }

    pub fn stack(&self) -> Stack {
        Stack {
            members: self
                .members
                .iter()
                .map(|m| platform::Member {
                    number: m.number,
                    model: MODELS[m.model].name.to_string(),
                    module: m.module.map(|i| MODULES[i].name.to_string()),
                })
                .collect(),
        }
    }

    pub fn spec(&self) -> String {
        self.stack().spec()
    }

    /// What Cisco does not build, stack or run, by member (0: the whole stack).
    pub fn problems(&self) -> Vec<(u32, String)> {
        let stack = self.stack();
        let book = grammars::book(stack.family());
        let mut out: Vec<(u32, String)> = self
            .members
            .iter()
            .flat_map(|m| {
                [&m.unknown_model, &m.unknown_module]
                    .into_iter()
                    .flatten()
                    .map(|named| {
                        (
                            m.number,
                            format!("vswitch knows no {named}: choose what it is"),
                        )
                    })
            })
            .collect();
        out.extend(stack.problems(self.release));
        if let Err(e) = grammars::resolve(&self.release.to_string(), book) {
            out.push((0, e));
        }
        out
    }

    /// The family the first member names, which every member stacks with.
    fn family(&self) -> Family {
        MODELS[self.members[0].model].family
    }

    fn is_chassis(&self) -> bool {
        self.family() == Family::C9400
    }

    fn book(&self) -> &'static str {
        grammars::book(Some(self.family()))
    }

    /// What a member is called: a 9400 card's number is its slot.
    fn unit(&self) -> &'static str {
        if self.is_chassis() { "slot" } else { "member" }
    }

    fn os(&self) -> &'static str {
        if self.family().classic_ios() {
            "IOS"
        } else {
            "IOS XE"
        }
    }

    /// The network module column, left out of a 9400 whose cards hold none.
    fn modules_shown(&self) -> bool {
        !self.is_chassis() || self.members.iter().any(|m| m.module.is_some())
    }

    /// The field with the focus, moved off one this stack does not show.
    fn focus(&self) -> Field {
        match self.field {
            Field::Chassis if !self.is_chassis() => Field::Release,
            Field::Module if !self.modules_shown() => Field::Model,
            f => f,
        }
    }

    fn next_field(&self, forward: bool) -> Field {
        let mut order = vec![Field::Number, Field::Model];
        if self.modules_shown() {
            order.push(Field::Module);
        }
        if self.is_chassis() {
            order.push(Field::Chassis);
        }
        order.push(Field::Release);
        let at = order.iter().position(|f| *f == self.focus()).unwrap_or(0);
        order[wrap_step(at, order.len(), forward)]
    }

    /// The highest member number: a C9410R's ten slots, else the family's stack size.
    fn max_number(&self) -> u32 {
        if self.is_chassis() {
            CHASSIS
                .iter()
                .map(|c| (c.supervisors.len() + c.cards.len()) as u32)
                .max()
                .unwrap_or(10)
        } else {
            self.family().max_members() as u32
        }
    }

    /// The chassis that hold every member where it is: supervisors in its supervisor slots, line
    /// cards in its line card slots.
    fn chassis(&self) -> Vec<usize> {
        (0..CHASSIS.len())
            .filter(|&c| {
                self.members.iter().all(|m| {
                    if is_sup(m.model) {
                        CHASSIS[c].supervisors.contains(&m.number)
                    } else {
                        CHASSIS[c].cards.contains(&m.number)
                    }
                })
            })
            .collect()
    }

    /// The chassis to show: the one chosen while the members fit it, else every one they fit.
    fn shown_chassis(&self) -> Vec<usize> {
        let fit = self.chassis();
        match self.chosen {
            Some(c) if fit.contains(&c) => vec![c],
            _ => fit,
        }
    }

    /// Whether every member sits where a chassis takes it: the chosen one, else any.
    fn seated(&self) -> bool {
        match self.chosen {
            Some(c) => self.chassis().contains(&c),
            None => !self.chassis().is_empty(),
        }
    }

    /// The lowest free slot in which a model leaves every member seated, in the chosen chassis when
    /// one can hold it, else in any: member `i` moved there, or a new member added.
    fn free_slot(&self, model: usize, i: Option<usize>) -> Option<u32> {
        let taken: Vec<u32> = self
            .members
            .iter()
            .enumerate()
            .filter(|(j, _)| Some(*j) != i)
            .map(|(_, m)| m.number)
            .collect();
        let with = |s: u32| {
            let mut b = self.clone();
            b.picker = None;
            match i {
                Some(i) => {
                    b.members[i].number = s;
                    b.members[i].model = model;
                }
                None => b.members.push(Member {
                    number: s,
                    model,
                    module: None,
                    unknown_model: None,
                    unknown_module: None,
                }),
            }
            b
        };
        let free: Vec<u32> = (1..=self.max_number())
            .filter(|s| !taken.contains(s))
            .collect();
        free.iter()
            .copied()
            .find(|&s| with(s).seated())
            .or_else(|| {
                free.iter()
                    .copied()
                    .find(|&s| !with(s).chassis().is_empty())
            })
    }

    /// A member's new model. A network module the model does not take comes out, a 9400 member moves
    /// to a slot that keeps one chassis, and a release the new family's book does not document moves
    /// to one the hardware runs; what moved is said.
    fn set_model(&mut self, i: usize, model: usize) -> Option<String> {
        if self.members[i].model == model {
            return None;
        }
        let before = self.book();
        let mut notes = vec![];
        self.members[i].model = model;
        if let Some(md) = self.members[i].module
            && !fits(model).contains(&md)
        {
            self.members[i].module = None;
            notes.push(format!(
                "{} taken out: the {} does not take it.",
                MODULES[md].name, MODELS[model].name
            ));
        }
        if self.is_chassis()
            && !self.seated()
            && let Some(s) = self.free_slot(model, Some(i))
        {
            notes.push(format!(
                "Moved from slot {} to slot {s}.",
                self.members[i].number
            ));
            self.members[i].number = s;
        }
        let book = self.book();
        if book != before
            && grammars::resolve(&self.release.to_string(), book).is_err()
            && let Some(r) = self.fitting_release()
        {
            notes.push(format!(
                "Release now {}: no {} command reference documents {}.",
                r.named(),
                grammars::book_name(book),
                self.release
            ));
            self.release = r;
        }
        self.sort_slots();
        (!notes.is_empty()).then(|| notes.join(" "))
    }

    /// For a stack moved to another book: the book's default release, else its newest, that the
    /// hardware runs; the one leaving fewest problems when none is clear.
    fn fitting_release(&self) -> Option<Release> {
        let book = self.book();
        (!grammars::documented(book).is_empty())
            .then(|| grammars::fitting_release(book, &self.stack()))
    }

    /// The release before or after this one among those the stack's book documents, going round from
    /// the last to the first and saying so.
    fn step_release(&mut self, up: bool) {
        let list = grammars::documented(self.book());
        let near = if up {
            list.iter().find(|r| **r > self.release)
        } else {
            list.iter().rev().find(|r| **r < self.release)
        };
        let round = if up { list.first() } else { list.last() };
        match near.or(round) {
            Some(r) => {
                self.release = *r;
                if near.is_none() {
                    let end = if up { "oldest" } else { "newest" };
                    let text = format!("The release went round to the {end}, {} {r}.", self.os());
                    self.message = Some((Tone::Note, text));
                }
            }
            None => {
                self.message = Some((
                    Tone::Problem,
                    format!(
                        "vswitch holds no {} command reference.",
                        grammars::book_name(self.book())
                    ),
                ))
            }
        }
    }

    /// Left and Right on the focused field: the next number that is free, the next model of the
    /// family, the next module the model takes, chassis or release.
    fn step(&mut self, up: bool) {
        let i = self.row;
        match self.focus() {
            Field::Number => {
                let max = self.max_number();
                let taken: Vec<u32> = self
                    .members
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, m)| m.number)
                    .collect();
                let mut n = self.members[i].number.clamp(1, max);
                for _ in 0..max {
                    n = if up {
                        n % max + 1
                    } else {
                        (n + max - 2) % max + 1
                    };
                    if !taken.contains(&n) {
                        break;
                    }
                }
                self.set_number(i, n);
            }
            Field::Model => {
                let kin = kin(self.members[i].model);
                let at = kin
                    .iter()
                    .position(|&k| k == self.members[i].model)
                    .unwrap_or(0);
                let next = kin[wrap_step(at, kin.len(), up)];
                self.members[i].unknown_model = None;
                self.message = self.set_model(i, next).map(|t| (Tone::Note, t));
            }
            Field::Module => {
                self.members[i].unknown_module = None;
                let mut options = vec![None];
                options.extend(fits(self.members[i].model).into_iter().map(Some));
                self.members[i].module =
                    match options.iter().position(|o| *o == self.members[i].module) {
                        Some(at) => options[wrap_step(at, options.len(), up)],
                        // from a module the model does not take, to the first or last one it does
                        None if up => options[1.min(options.len() - 1)],
                        None => options[options.len() - 1],
                    };
            }
            Field::Chassis => {
                let next = match self.shown_chassis().first() {
                    Some(&c) => wrap_step(c, CHASSIS.len(), up),
                    None => 0,
                };
                self.renumber(next);
            }
            Field::Release => self.step_release(up),
        }
    }

    /// A member like the selected one: in a 9400 the lowest free slot that keeps one chassis, else
    /// numbered after the last.
    fn add(&mut self) {
        let family = self.family();
        let max = family.max_members();
        if self.members.len() >= max {
            let why = match max {
                1 => format!("A {} is one switch: it does not stack.", family.name()),
                _ if self.is_chassis() => "Every slot of a C9410R is taken.".to_string(),
                _ => format!("A {} stack takes at most {max} members.", family.name()),
            };
            self.message = Some((Tone::Problem, why));
            return;
        }
        let mut m = self.members[self.row].clone();
        m.unknown_model = None;
        m.unknown_module = None;
        m.number = if self.is_chassis() {
            let Some(s) = self.free_slot(m.model, None) else {
                let chassis = match self.shown_chassis().as_slice() {
                    [c] => format!(" of the {}", CHASSIS[*c].name),
                    _ => String::new(),
                };
                self.message = Some((
                    Tone::Problem,
                    format!(
                        "No free slot{chassis} takes another {}.",
                        MODELS[m.model].name
                    ),
                ));
                return;
            };
            s
        } else {
            // the lowest number free: a stack numbers its members 1 to 8
            (1..)
                .find(|n| !self.members.iter().any(|m| m.number == *n))
                .unwrap_or(1)
        };
        self.members.push(m);
        self.row = self.members.len() - 1;
        self.sort_slots();
    }

    fn remove(&mut self) {
        if self.members.len() == 1 {
            self.message = Some((
                Tone::Problem,
                "The last member stays: change its model instead.".into(),
            ));
            return;
        }
        self.members.remove(self.row);
        self.row = self.row.min(self.members.len() - 1);
    }

    /// Puts the members in a chassis: supervisors in its supervisor slots and line cards in its line
    /// card slots, each in slot order; one it has no room for takes the lowest number left.
    fn renumber(&mut self, c: usize) {
        self.chosen = Some(c);
        let ch = &CHASSIS[c];
        let mut order: Vec<usize> = (0..self.members.len()).collect();
        order.sort_by_key(|&i| self.members[i].number);
        let (mut sups, mut cards) = (ch.supervisors.iter(), ch.cards.iter());
        let mut used = vec![];
        let mut spare = vec![];
        for &i in &order {
            let slot = if is_sup(self.members[i].model) {
                sups.next()
            } else {
                cards.next()
            };
            match slot {
                Some(&s) => {
                    self.members[i].number = s;
                    used.push(s);
                }
                None => spare.push(i),
            }
        }
        for i in spare {
            let s = (1..).find(|s| !used.contains(s)).unwrap_or(1);
            self.members[i].number = s;
            used.push(s);
        }
        self.sort_slots();
    }

    /// A 9400's members in slot order, as the chassis holds them; the selection stays on its member.
    fn sort_slots(&mut self) {
        if !self
            .members
            .iter()
            .all(|m| MODELS[m.model].family == Family::C9400)
        {
            return;
        }
        let mut order: Vec<usize> = (0..self.members.len()).collect();
        order.sort_by_key(|&i| self.members[i].number);
        self.row = order.iter().position(|&i| i == self.row).unwrap_or(0);
        self.members = order.iter().map(|&i| self.members[i].clone()).collect();
    }

    /// A member's number set by hand; a chosen chassis the slots no longer fit is forgotten.
    fn set_number(&mut self, i: usize, n: u32) {
        self.members[i].number = n;
        if self.chosen.is_some_and(|c| !self.chassis().contains(&c)) {
            self.chosen = None;
        }
        self.sort_slots();
    }

    fn apply(&mut self, c: Choice) -> Option<String> {
        let i = self.row;
        match c {
            Choice::Number(n) => self.set_number(i, n),
            Choice::Model(m) => {
                self.members[i].unknown_model = None;
                return self.set_model(i, m);
            }
            Choice::Module(md) => {
                self.members[i].unknown_module = None;
                self.members[i].module = md;
            }
            Choice::Chassis(c) => self.renumber(c),
            Choice::Release(r) => self.release = r,
        }
        None
    }

    /// A problem as the builder shows it: after the member or slot it names, unless that is `own`
    /// or its text already starts with it.
    fn named(&self, n: u32, own: u32, why: String) -> String {
        let unit = self.unit();
        if n == 0
            || n == own
            || why.starts_with(&format!("slot {n} "))
            || why.starts_with(&format!("member {n} "))
        {
            why
        } else {
            format!("{unit} {n}: {why}")
        }
    }

    /// The problems by whom each is about, each with how the builder names it: the stack's, the
    /// selected member's, and each other member's by its model and module. A number two members share
    /// names either, so its problem counts as each one's.
    fn problems_by_whom(&self) -> Vec<Found> {
        let own = self.members[self.row].number;
        let mut found = vec![];
        for (n, why) in self.problems() {
            let named = self.named(n, own, why.clone());
            if n == 0 {
                found.push((Whose::Stack, why, named));
                continue;
            }
            let others: Vec<Whose> = self
                .members
                .iter()
                .enumerate()
                .filter(|(j, m)| *j != self.row && m.number == n)
                .map(|(_, m)| Whose::Other(m.model, m.module))
                .collect();
            if n == own || others.is_empty() {
                found.push((Whose::Own, why.clone(), named.clone()));
            }
            for w in others {
                found.push((w, why.clone(), named.clone()));
            }
        }
        found
    }

    /// Each choice's reasons and what it changes besides itself. A reason is a problem the choice
    /// would leave that is its doing: another member's that the stack does not have now, or the
    /// member's own or the stack's that not every choice leaves. Problems are told apart by whom they
    /// are about, not by how they read, which a renumbered member or a 9400's slots change.
    fn outcomes(&self, choices: &[Choice]) -> Vec<(Vec<String>, Option<String>)> {
        let key = |(w, y, _): &Found| (w.clone(), y.clone());
        let other = |k: &(Whose, String)| matches!(k.0, Whose::Other(..));
        let now: Vec<(Whose, String)> = self
            .problems_by_whom()
            .iter()
            .map(key)
            .filter(other)
            .collect();
        let all: Vec<(Vec<Found>, Option<String>)> = choices
            .iter()
            .map(|&c| {
                let mut b = self.clone();
                b.picker = None;
                let note = b.apply(c);
                (b.problems_by_whom(), note)
            })
            .collect();
        // the member's own and the stack's that every choice leaves, as often as each leaves them
        let mut common: Vec<(Whose, String)> = vec![];
        if let Some((first, _)) = all.first() {
            for k in first.iter().map(key).filter(|k| !other(k)) {
                let least = all
                    .iter()
                    .map(|(l, _)| l.iter().filter(|p| key(p) == k).count())
                    .min()
                    .unwrap_or(0);
                if common.iter().filter(|c| **c == k).count() < least {
                    common.push(k);
                }
            }
        }
        all.into_iter()
            .map(|(found, note)| {
                let (mut now, mut common) = (now.clone(), common.clone());
                let mut reasons: Vec<String> = vec![];
                for p in found {
                    let k = key(&p);
                    let pool = if other(&k) { &mut now } else { &mut common };
                    match pool.iter().position(|c| *c == k) {
                        Some(at) => {
                            pool.remove(at);
                        }
                        None if !reasons.contains(&p.2) => reasons.push(p.2),
                        None => {}
                    }
                }
                (reasons, note)
            })
            .collect()
    }

    /// Every value the focused field takes, as its list offers them.
    fn offers(&self) -> Vec<Offer> {
        let m = &self.members[self.row];
        let offer =
            |choice: Choice, name: String, info: String, group: String, search: String| Offer {
                choice,
                name,
                info,
                group,
                search,
            };
        match self.focus() {
            Field::Number => (1..=self.max_number())
                .map(|n| {
                    let (info, group) = if self.is_chassis() {
                        (slot_roles(n), "Slots".to_string())
                    } else {
                        (String::new(), "Member numbers".to_string())
                    };
                    offer(Choice::Number(n), n.to_string(), info, group, n.to_string())
                })
                .collect(),
            Field::Model => (0..MODELS.len())
                .map(|k| {
                    let (name, info, group) = (MODELS[k].name, model_info(k), group(k));
                    let search = norm(&format!("{name} {group} {info}"));
                    offer(Choice::Model(k), name.into(), info, group, search)
                })
                .collect(),
            Field::Module => std::iter::once(None)
                .chain((0..MODULES.len()).map(Some))
                .map(|md| {
                    let (name, info) = match md {
                        Some(i) => (MODULES[i].name.to_string(), counts(MODULES[i].ports)),
                        None => (
                            "none".to_string(),
                            match MODELS[m.model].uplinks {
                                Uplinks::Modules(_) => "the slot left empty",
                                Uplinks::Fixed(_) | Uplinks::Slot0(_) => "built-in uplinks",
                                Uplinks::None => "no uplink slot",
                            }
                            .to_string(),
                        ),
                    };
                    let search = norm(&format!("{name} {info}"));
                    offer(Choice::Module(md), name, info, String::new(), search)
                })
                .collect(),
            Field::Chassis => CHASSIS
                .iter()
                .enumerate()
                .map(|(c, ch)| {
                    let info = chassis_info(ch);
                    let search = norm(&format!("{} {info}", ch.name));
                    offer(
                        Choice::Chassis(c),
                        ch.name.into(),
                        info,
                        "Catalyst 9400 chassis".into(),
                        search,
                    )
                })
                .collect(),
            Field::Release => {
                let book = self.book();
                let default = grammars::default_release(book);
                grammars::documented(book)
                    .into_iter()
                    .map(|r| {
                        let group = if r.classic() {
                            format!("IOS {}", r.config_version())
                        } else {
                            format!("IOS XE {}", r.major())
                        };
                        let info = if r == default { "default" } else { "" };
                        offer(
                            Choice::Release(r),
                            r.to_string(),
                            info.into(),
                            group,
                            norm(&r.to_string()),
                        )
                    })
                    .collect()
            }
        }
    }

    /// Whether a choice is what the stack has now: every chassis its slots fit.
    fn is_current(&self, c: Choice) -> bool {
        let m = &self.members[self.row];
        match c {
            Choice::Number(n) => m.number == n,
            Choice::Model(k) => m.model == k,
            Choice::Module(md) => m.module == md,
            Choice::Chassis(c) => self.shown_chassis().contains(&c),
            Choice::Release(r) => self.release == r,
        }
    }

    /// Opens the focused field's list, filtered by what was typed to open it.
    fn open(&mut self, filter: String) {
        let field = self.focus();
        self.field = field;
        let offers = self.offers();
        let choices: Vec<Choice> = offers.iter().map(|o| o.choice).collect();
        let outcomes = self.outcomes(&choices);
        let model = MODELS[self.members[self.row].model].name;
        let mut groups: Vec<String> = vec![];
        let mut items: Vec<(String, Entry)> = vec![];
        for (o, (mut reasons, note)) in offers.into_iter().zip(outcomes) {
            // a number another member holds is named by what holds it, whose problems it would share
            if let Choice::Number(n) = o.choice
                && let Some(other) = self
                    .members
                    .iter()
                    .enumerate()
                    .find(|(j, m)| *j != self.row && m.number == n)
            {
                reasons = vec![format!("taken by the {}", MODELS[other.1.model].name)];
            }
            // modules that fit first, then the rest with Cisco's reason
            let group = match field {
                Field::Module if reasons.is_empty() => format!("Fits the {model}"),
                Field::Module => format!("Does not fit the {model}"),
                _ => o.group,
            };
            if !groups.contains(&group) {
                groups.push(group.clone());
            }
            let entry = Entry::Item {
                choice: o.choice,
                name: o.name,
                info: o.info,
                reasons,
                note,
                search: o.search,
            };
            items.push((group, entry));
        }
        let mut entries = vec![];
        for g in groups {
            let under: Vec<Entry> = items
                .iter()
                .filter(|(ig, _)| *ig == g)
                .map(|(_, e)| e.clone())
                .collect();
            entries.push(Entry::Heading(g));
            entries.extend(under);
        }
        let cursor = entries.iter().position(|e| match e {
            Entry::Item { choice, .. } => {
                self.is_current(*choice) && !matches!(choice, Choice::Chassis(_))
                    || matches!(choice, Choice::Chassis(c) if self.shown_chassis().first() == Some(c))
            }
            Entry::Heading(_) => false,
        });
        let mut p = Picker {
            field,
            entries,
            filter,
            cursor,
        };
        p.refilter();
        self.picker = Some(Box::new(p));
    }

    fn picker_key(&mut self, key: KeyEvent) {
        let typed = types(key);
        let Some(p) = self.picker.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Enter => {
                if let Some(c) = p.choice() {
                    self.picker = None;
                    self.message = self.apply(c).map(|t| (Tone::Note, t));
                }
            }
            // Left and Right step through the choices as they step a field's value
            KeyCode::Up | KeyCode::Left => p.step(-1),
            KeyCode::Down | KeyCode::Right => p.step(1),
            KeyCode::PageUp => p.step(-10),
            KeyCode::PageDown => p.step(10),
            KeyCode::Home => p.step(isize::MIN),
            KeyCode::End => p.step(isize::MAX),
            KeyCode::Backspace => {
                p.filter.pop();
                p.refilter();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                p.filter.clear();
                p.refilter();
            }
            // no choice holds a `?`: it lists the keys, over the list
            KeyCode::Char('?') if typed => self.help = Some(0),
            KeyCode::Char(c) if typed => {
                p.filter.push(c);
                p.refilter();
            }
            _ => {}
        }
    }

    /// Enter keeps the hardware once nothing is marked. Until then the builder takes it, says so and
    /// selects the first member a problem names.
    fn refuse_enter(&mut self) -> bool {
        let problems = self.problems();
        if problems.is_empty() {
            return false;
        }
        if let Some(i) = problems
            .iter()
            .find_map(|(n, _)| self.members.iter().position(|m| m.number == *n))
        {
            self.row = i;
        }
        self.message = Some((
            Tone::Problem,
            "Nothing is kept while a problem is marked: fix the red lines first.".into(),
        ));
        true
    }

    /// A key, and whether the builder took it: Enter while nothing is marked and Esc while no list is
    /// open are left to the caller, to keep the hardware or cancel.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if let Some(top) = self.help {
            // the arrows scroll a key list the box cuts; any other key closes it, so Enter and Esc
            // there neither keep nor cancel
            let (lines, shown) = self.help_lines.get();
            let most = lines.saturating_sub(shown);
            self.help = match key.code {
                KeyCode::Up if most > 0 => Some(top.saturating_sub(1)),
                KeyCode::Down if most > 0 => Some((top + 1).min(most)),
                KeyCode::PageUp if most > 0 => Some(top.saturating_sub(shown.max(1))),
                KeyCode::PageDown if most > 0 => Some((top + shown.max(1)).min(most)),
                _ => None,
            };
            return true;
        }
        if self.picker.is_some() {
            // Tab leaves a list for the next field, choosing nothing
            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                self.picker = None;
            } else {
                self.picker_key(key);
                return true;
            }
        }
        self.message = None;
        self.row = self.row.min(self.members.len() - 1);
        let typed = types(key);
        let last = self.members.len() - 1;
        match key.code {
            KeyCode::Enter => return self.refuse_enter(),
            KeyCode::Up => self.row = self.row.saturating_sub(1),
            KeyCode::Down => self.row = (self.row + 1).min(last),
            // no model, module or release has a k or a j, so they move as in the switch list
            KeyCode::Char('k') if typed => self.row = self.row.saturating_sub(1),
            KeyCode::Char('j') if typed => self.row = (self.row + 1).min(last),
            KeyCode::Home => self.row = 0,
            KeyCode::End => self.row = last,
            KeyCode::BackTab => self.field = self.next_field(false),
            KeyCode::Tab => {
                self.field = self.next_field(!key.modifiers.contains(KeyModifiers::SHIFT))
            }
            KeyCode::Left => self.step(false),
            KeyCode::Right => self.step(true),
            KeyCode::Char('+') | KeyCode::Insert => self.add(),
            KeyCode::Char('-') | KeyCode::Delete => self.remove(),
            KeyCode::Char(']') => self.step_release(true),
            KeyCode::Char('[') => self.step_release(false),
            KeyCode::Char('?') => self.help = Some(0),
            KeyCode::Char(' ') => self.open(String::new()),
            KeyCode::Char(c) if typed && c.is_ascii_alphanumeric() => self.open(c.to_string()),
            _ => return false,
        }
        true
    }

    /// The builder in a box titled `title` within `area`, as tall as its content up to most of it; an
    /// open list or the key list takes the whole box.
    pub fn draw(&self, f: &mut Frame, area: Rect, title: &str) {
        let full = frame(area);
        if let Some(top) = self.help {
            let shown = draw_help(f, full, &format!("{}: keys ", title.trim_end()), top);
            self.help_lines.set(shown);
            return;
        }
        if let Some(p) = &self.picker {
            let title = format!("{}: {} ", title.trim_end(), self.picker_title(p.field));
            return self.draw_picker(f, p, full, &title);
        }
        let w = full.width.saturating_sub(4) as usize;
        let most = full.height.saturating_sub(2) as usize;
        if w == 0 || most == 0 {
            return;
        }
        let problems = self.problems();
        let footer = self.footer(w, most);
        let mut status = self.status(&problems, w);
        let mut panel = self.panel(w);
        panel.insert(0, Line::raw(""));
        let table_full = self.members.len() + 1;
        // the box grows down from the frame's top with its content; past the frame the panel is cut,
        // then the problems, never the members' header and the selected one, or the keys
        let h = (table_full + panel.len() + 1 + status.len() + footer.len()).min(most);
        let room = h.saturating_sub(footer.len() + 2);
        if status.len() > room {
            let more = status.len() + 1 - room;
            status.truncate(room.saturating_sub(1));
            status.push(Line::styled(
                format!("and {more} more lines: a taller terminal shows them"),
                BAD,
            ));
        }
        let table_h = table_full.min(h.saturating_sub(footer.len() + status.len()));
        let panel_h = h.saturating_sub(footer.len() + status.len() + table_h);
        // a panel the box cuts says so on its last line shown
        if panel.len() > panel_h && panel_h > 1 {
            let more = panel.len() + 1 - panel_h;
            panel.truncate(panel_h - 1);
            panel.push(Line::styled(
                format!("and {more} more lines: a taller terminal shows them"),
                HINT,
            ));
        }
        let area = Rect {
            height: h as u16 + 2,
            ..full
        };
        f.render_widget(Clear, area);
        let block = Block::bordered().title(title.to_string());
        let inner = pad(block.inner(area));
        f.render_widget(block, area);
        let rect = |y: usize, height: usize| Rect {
            x: inner.x,
            y: inner.y + y as u16,
            width: inner.width,
            height: height as u16,
        };
        f.render_widget(
            Paragraph::new(self.table(&problems, w, table_h)),
            rect(0, table_h),
        );
        f.render_widget(Paragraph::new(panel), rect(table_h, panel_h));
        let status_h = status.len();
        let status_y = h.saturating_sub(footer.len() + status_h);
        f.render_widget(Paragraph::new(status), rect(status_y, status_h));
        let footer_h = footer.len().min(h);
        f.render_widget(Paragraph::new(footer), rect(h - footer_h, footer_h));
    }

    /// The members, one a row: number or slot, model, network module, ports, and the family where the
    /// width allows; the focused cell highlighted, a member with a problem red.
    fn table(&self, problems: &[(u32, String)], w: usize, rows: usize) -> Vec<Line<'static>> {
        let focus = self.focus();
        let modules = self.modules_shown();
        let mw = MODELS.iter().map(|m| m.name.len()).max().unwrap_or(0);
        let dw = MODULES
            .iter()
            .map(|m| m.name.len())
            .max()
            .unwrap_or(0)
            .max("Network module".len());
        let number = if self.is_chassis() { "Slot" } else { "#" };
        let nw = number.len().max(2);
        let fixed = 2 + nw + 2 + mw + 2 + if modules { dw + 2 } else { 0 };
        let rest = w.saturating_sub(fixed);
        let fw = MODELS
            .iter()
            .map(|m| m.family.name().len())
            .max()
            .unwrap_or(0);
        // the widest ports text (40x 5G + 8x 10G UPOE+, 8x 25G) is 29 columns
        let family = rest >= 30 + 2 + fw;
        let ports_w = if family { 30 } else { rest };
        let mut head = format!("  {}  {}  ", cell(number, nw), cell("Model", mw));
        if modules {
            head.push_str(&format!("{}  ", cell("Network module", dw)));
        }
        head.push_str(&cell("Ports", ports_w));
        if family {
            head.push_str(&format!("  {}", cell("Family", fw)));
        }
        let mut out = vec![Line::styled(cell(&head, w), BOLD)];
        let shown = rows.saturating_sub(1);
        let first = (self.row + 1).saturating_sub(shown.max(1));
        for (r, m) in self.members.iter().enumerate().skip(first).take(shown) {
            let model = &MODELS[m.model];
            let marked = problems.iter().any(|(n, _)| *n == m.number);
            let base = if marked { BAD } else { PLAIN };
            let at = |f: Field| match (r == self.row && focus == f, marked) {
                (true, true) => PICK_BAD,
                (true, false) => PICK,
                (false, _) => base,
            };
            let gutter = if r == self.row {
                Span::styled("> ", Style::new().fg(Color::Cyan))
            } else {
                Span::raw("  ")
            };
            let mut spans = vec![
                gutter,
                Span::styled(cell(&m.number.to_string(), nw), at(Field::Number)),
                Span::raw("  "),
                Span::styled(cell(model.name, mw), at(Field::Model)),
                Span::raw("  "),
            ];
            if modules {
                let module = match (m.module, model.uplinks) {
                    (Some(i), _) => MODULES[i].name,
                    (None, Uplinks::Fixed(_) | Uplinks::Slot0(_)) => "built in",
                    (None, Uplinks::Modules(_)) => "none",
                    (None, Uplinks::None) => "-",
                };
                spans.push(Span::styled(cell(module, dw), at(Field::Module)));
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(cell(&member_ports(m), ports_w), base));
            if family {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(cell(model.family.name(), fw), HINT));
            }
            out.push(Line::from(spans));
        }
        out
    }

    /// The fields below the members: a 9400's chassis and the release, each with what it allows;
    /// then the selected member in full and the stack as `--stack` takes it.
    fn panel(&self, w: usize) -> Vec<Line<'static>> {
        let focus = self.focus();
        let mut out = vec![];
        // a field's hint runs on under itself when the line is short
        let field = |label: &str, value: String, style: Style, on: bool, hint: String| {
            let value_style = if on { PICK } else { style };
            let used = LABEL + value.chars().count() + 3;
            let mut parts = wrap(&hint, w.saturating_sub(used).max(1)).into_iter();
            let mut lines = vec![Line::from(vec![
                Span::styled(format!("{label:<LABEL$}"), HINT),
                Span::styled(value, value_style),
                Span::raw("   "),
                Span::styled(parts.next().unwrap_or_default(), HINT),
            ])];
            lines.extend(parts.map(|p| Line::styled(format!("{}{p}", " ".repeat(used)), HINT)));
            lines
        };
        if self.is_chassis() {
            let fit = self.shown_chassis();
            let names: Vec<&str> = fit.iter().map(|&c| CHASSIS[c].name).collect();
            let (value, style, hint) = match fit.as_slice() {
                [] => (
                    "none".to_string(),
                    BAD,
                    "no chassis has these slots; Space lists the three".to_string(),
                ),
                [c] => {
                    let ch = &CHASSIS[*c];
                    let free: Vec<u32> = ch
                        .supervisors
                        .iter()
                        .chain(ch.cards)
                        .copied()
                        .filter(|s| self.members.iter().all(|m| m.number != *s))
                        .collect();
                    let mut free = free;
                    free.sort_unstable();
                    let free = if free.is_empty() {
                        "every slot taken".to_string()
                    } else {
                        format!("free {}", and_list(&free))
                    };
                    (
                        ch.name.to_string(),
                        PLAIN,
                        format!(
                            "supervisors in {}, line cards in {}; {free}",
                            and_list(ch.supervisors),
                            ch.slots_text
                        ),
                    )
                }
                _ => (
                    or_list(&names),
                    PLAIN,
                    "each fits these slots; Space chooses one".to_string(),
                ),
            };
            out.extend(field(
                "Chassis",
                value,
                style,
                focus == Field::Chassis,
                hint,
            ));
        }
        let list = grammars::documented(self.book());
        let hint = match (list.first(), list.last()) {
            (Some(a), Some(b)) if list.len() > 1 => {
                format!("{} documented, {a} to {b}", list.len())
            }
            (Some(a), _) => format!("the one documented release is {a}"),
            _ => format!(
                "no {} command reference in vswitch",
                grammars::book_name(self.book())
            ),
        };
        out.extend(field(
            self.os(),
            self.release.to_string(),
            PLAIN,
            focus == Field::Release,
            hint,
        ));
        out.push(Line::raw(""));
        let m = &self.members[self.row];
        let who = format!(
            "{} {}: {}",
            if self.is_chassis() { "Slot" } else { "Member" },
            m.number,
            MODELS[m.model].name
        );
        out.extend(self.model_lines(m.model, m.module, m.number, &who, w));
        out.push(Line::raw(""));
        out.extend(spec_lines(&self.spec(), w));
        out
    }

    /// A model as fitted in member `number`: family and releases, ports by name, uplinks, PoE, and
    /// what the documents do not settle.
    fn model_lines(
        &self,
        model: usize,
        module: Option<usize>,
        number: u32,
        title: &str,
        w: usize,
    ) -> Vec<Line<'static>> {
        let m = &MODELS[model];
        let mut rest = format!(", {}", m.family.name());
        if let Some(r) = runs(m.since, m.until) {
            rest.push_str(&format!(", {r}"));
        }
        let mut out = vec![titled(title, &rest, w)];
        out.extend(labeled(
            "Ports",
            &interfaces(m.ports, m.family, number, 0),
            PLAIN,
            w,
        ));
        let uplinks = match m.uplinks {
            Uplinks::Modules(list) => Some(match module {
                Some(md) => format!(
                    "{} on the {}",
                    interfaces(MODULES[md].ports, m.family, number, 1),
                    MODULES[md].name
                ),
                None => format!("network module slot, empty; takes {}", list.join(", ")),
            }),
            Uplinks::Fixed(p) => Some(format!("{}, built in", interfaces(p, m.family, number, 1))),
            Uplinks::Slot0(p) => Some(format!("{}, built in", interfaces(p, m.family, number, 0))),
            Uplinks::None => None,
        };
        if let Some(u) = uplinks {
            out.extend(labeled("Uplinks", &u, PLAIN, w));
        }
        out.extend(labeled("PoE", &poe_text(m.poe), PLAIN, w));
        if let Some(note) = m.note {
            out.extend(wrapped(note, NOTE, w));
        }
        out
    }

    /// What Enter does now, then the problems, or that Cisco builds the hardware.
    fn status(&self, problems: &[(u32, String)], w: usize) -> Vec<Line<'static>> {
        let mut out = vec![];
        if let Some((tone, text)) = &self.message {
            let style = match tone {
                Tone::Note => NOTE,
                Tone::Problem => BAD,
            };
            out.extend(wrapped(text, style, w));
        }
        if problems.is_empty() {
            let what = match self.family() {
                Family::C9400 => "Cisco builds this chassis",
                f if f.max_members() == 1 => "Cisco builds this switch",
                _ => "Cisco builds and stacks this hardware",
            };
            out.extend(wrapped(&format!("{what}: Enter keeps it."), GOOD, w));
        }
        for (n, why) in problems {
            out.extend(hanging(&self.named(*n, 0, why.clone()), BAD, w, 2));
        }
        out
    }

    /// The keys for the focused field on one line and the rest on another, or what fits one line on
    /// a short terminal.
    fn footer(&self, w: usize, h: usize) -> Vec<Line<'static>> {
        let choose = match self.focus() {
            Field::Number if self.is_chassis() => "choose slot",
            Field::Number => "choose number",
            Field::Model => "choose model",
            Field::Module => "choose module",
            Field::Chassis => "choose chassis",
            Field::Release => "choose release",
        };
        let local = [
            ("Space/type", choose),
            ("Tab", "field"),
            ("↑↓", "member"),
            ("←→", "step"),
        ];
        let global = [
            ("Enter", "keep"),
            ("Esc", "cancel"),
            ("?", "keys"),
            ("+", "add"),
            ("-", "remove"),
            ("[ ]", "release"),
        ];
        let all: Vec<(&str, &str)> = local.iter().chain(global.iter()).copied().collect();
        let width: usize = all
            .iter()
            .map(|(k, d)| k.chars().count() + d.len() + 3)
            .sum();
        if width <= w + 2 {
            vec![hints(&all, w)]
        } else if h >= 12 {
            vec![hints(&local, w), hints(&global, w)]
        } else {
            vec![hints(&[local[0], global[0], global[1], global[2]], w)]
        }
    }

    fn picker_title(&self, field: Field) -> String {
        let m = &self.members[self.row];
        let who = format!("{} {}", self.unit(), m.number);
        match field {
            Field::Number if self.is_chassis() => format!("Slot for the {}", MODELS[m.model].name),
            Field::Number => format!("Number for member {}", m.number),
            Field::Model => format!("Model for {who}"),
            Field::Module => format!("Network module for {who}"),
            Field::Chassis => "Catalyst 9400 chassis".into(),
            Field::Release => format!("{} release", self.os()),
        }
    }

    /// The highlighted choice in full: what it is, Cisco's reasons against it, what choosing it
    /// changes, then the detail.
    fn choice_lines(&self, p: &Picker, w: usize) -> Vec<Line<'static>> {
        let Some(Entry::Item {
            choice,
            reasons,
            note,
            ..
        }) = p.cursor.and_then(|c| p.entries.get(c))
        else {
            return vec![Line::styled(
                format!(
                    "Nothing matches \"{}\": Backspace widens the list.",
                    p.filter
                ),
                HINT,
            )];
        };
        let m = &self.members[self.row];
        let mut body = match *choice {
            Choice::Model(k) => {
                let module = m.module.filter(|md| fits(k).contains(md));
                self.model_lines(k, module, m.number, MODELS[k].name, w)
            }
            Choice::Module(Some(md)) => {
                let x = &MODULES[md];
                let mut rest = format!(": {}", counts(x.ports));
                if let Some(r) = runs(x.since, None) {
                    rest.push_str(&format!(", {r}"));
                }
                let takers: Vec<&str> = MODELS
                    .iter()
                    .filter(|mo| {
                        matches!(mo.uplinks, Uplinks::Modules(l) if l.iter().any(|n| n.eq_ignore_ascii_case(x.name)))
                    })
                    .map(|mo| mo.name)
                    .collect();
                let mut v = vec![titled(x.name, &rest, w)];
                v.extend(labeled("Fits", &takers.join(", "), PLAIN, w));
                v.extend(labeled(
                    "Ports",
                    &interfaces(x.ports, Family::C9300, m.number, 1),
                    PLAIN,
                    w,
                ));
                if x.breakout {
                    v.extend(wrapped(
                        "its 40G ports take a 4x10G breakout cable (hardware installation guide)",
                        HINT,
                        w,
                    ));
                }
                v
            }
            Choice::Module(None) => {
                let model = MODELS[m.model].name;
                let rest = match MODELS[m.model].uplinks {
                    Uplinks::Modules(_) => {
                        format!(": the {model}'s network module slot left empty")
                    }
                    Uplinks::Fixed(_) | Uplinks::Slot0(_) => {
                        format!(": the {model}'s uplinks are built in")
                    }
                    Uplinks::None => format!(": the {model} has no network module slot"),
                };
                vec![titled("none", &rest, w)]
            }
            Choice::Number(n) if self.is_chassis() => {
                vec![titled(
                    &format!("Slot {n}"),
                    &format!(": {}", slot_roles(n)),
                    w,
                )]
            }
            Choice::Number(n) => {
                let model = &MODELS[m.model];
                let mut v = vec![titled(&format!("Member {n}"), "", w)];
                v.extend(labeled(
                    "Ports",
                    &interfaces(model.ports, model.family, n, 0),
                    PLAIN,
                    w,
                ));
                v
            }
            Choice::Chassis(c) => {
                let mut b = self.clone();
                b.picker = None;
                b.apply(*choice);
                let mut v = vec![titled(
                    CHASSIS[c].name,
                    &format!(", {}", chassis_info(&CHASSIS[c])),
                    w,
                )];
                v.extend(labeled("Slots", &b.spec(), PLAIN, w));
                v
            }
            Choice::Release(r) => {
                let rest = if r == grammars::default_release(self.book()) {
                    format!(
                        ", the release a {} switch runs when none is named",
                        grammars::book_name(self.book())
                    )
                } else {
                    String::new()
                };
                vec![titled(&r.named(), &rest, w)]
            }
        };
        let mut out = vec![body.remove(0)];
        for r in reasons {
            out.extend(wrapped(r, BAD, w));
        }
        if let Some(n) = note {
            out.extend(wrapped(n, NOTE, w));
        }
        out.extend(body);
        out
    }

    /// The open list in the builder's box: the filter, the choices under their headings, the
    /// highlighted one in full, and the keys.
    fn draw_picker(&self, f: &mut Frame, p: &Picker, area: Rect, title: &str) {
        f.render_widget(Clear, area);
        let block = Block::bordered().title(title.to_string());
        let inner = pad(block.inner(area));
        f.render_widget(block, area);
        let (w, h) = (inner.width as usize, inner.height as usize);
        if w == 0 || h < 3 {
            return;
        }
        let visible = p.visible();
        let shown = visible
            .iter()
            .filter(|&&i| matches!(p.entries[i], Entry::Item { .. }))
            .count();
        let total = p
            .entries
            .iter()
            .filter(|e| matches!(e, Entry::Item { .. }))
            .count();
        let count = format!("{shown} of {total}");
        let mut filter = vec![
            Span::styled("Filter  ", HINT),
            Span::raw(p.filter.clone()),
            Span::styled("█", Style::new().fg(Color::Cyan)),
        ];
        let used = LABEL + p.filter.chars().count() + 1;
        let tail = if p.filter.is_empty() {
            " type to narrow the list"
        } else {
            ""
        };
        let gap = w.saturating_sub(used + tail.len() + count.len());
        filter.push(Span::styled(tail, HINT));
        // the star the gutter marks the value now with, explained beside the count: a legend, in
        // the gutter's colour, where the keys below are keys
        let legend = " the value now";
        let star = 1 + legend.len() + 3;
        if gap >= star + 2 {
            filter.push(Span::raw(" ".repeat(gap - star)));
            filter.push(Span::styled("*", Style::new().fg(Color::Cyan)));
            filter.push(Span::styled(legend, HINT));
            filter.push(Span::raw("   "));
        } else {
            filter.push(Span::raw(" ".repeat(gap)));
        }
        filter.push(Span::styled(count, HINT));
        let footer = hints(
            &[
                ("Enter", "choose"),
                ("Esc", "back"),
                ("↑↓", "move"),
                ("PgUp PgDn", "page"),
                ("Backspace", "erase"),
                ("?", "keys"),
            ],
            w,
        );
        // the detail takes what a short list leaves, and a third of a long one's
        let mut details = self.choice_lines(p, w);
        details.truncate((h / 3).max(3).max(h.saturating_sub(3 + visible.len())));
        let mut list_h = h.saturating_sub(3 + details.len());
        if list_h < 3 {
            details.clear();
            list_h = h.saturating_sub(2);
        }
        let nw = p
            .entries
            .iter()
            .filter_map(|e| match e {
                Entry::Item { name, .. } => Some(name.chars().count()),
                Entry::Heading(_) => None,
            })
            .max()
            .unwrap_or(0)
            .min(18);
        let at = p
            .cursor
            .and_then(|c| visible.iter().position(|&i| i == c))
            .unwrap_or(0);
        let top = at
            .saturating_sub(list_h / 2)
            .min(visible.len().saturating_sub(list_h));
        let mut lines = vec![];
        for &i in visible.iter().skip(top).take(list_h) {
            lines.push(match &p.entries[i] {
                Entry::Heading(t) => Line::styled(cell(t, w), BOLD),
                Entry::Item {
                    choice,
                    name,
                    info,
                    reasons,
                    ..
                } => {
                    let now = self.is_current(*choice);
                    let gutter = if now { "* " } else { "  " };
                    // a slot that does not fit keeps its roles in red; the reason is shown below
                    let (text, style) = match (reasons.first(), choice) {
                        (Some(r), Choice::Number(_))
                            if self.is_chassis() && !r.starts_with("taken by") =>
                        {
                            (info.clone(), BAD)
                        }
                        (Some(r), _) => (brief(r, name), BAD),
                        (None, _) => (info.clone(), HINT),
                    };
                    if Some(i) == p.cursor {
                        Line::styled(cell(&format!("{gutter}{name:<nw$}  {text}"), w), PICK)
                    } else {
                        let rest = w.saturating_sub(2 + nw + 2);
                        Line::from(vec![
                            Span::styled(gutter, Style::new().fg(Color::Cyan)),
                            Span::styled(
                                cell(name, nw),
                                if reasons.is_empty() { PLAIN } else { BAD },
                            ),
                            Span::raw("  "),
                            Span::styled(cell(&text, rest), style),
                        ])
                    }
                }
            });
        }
        if visible.is_empty() {
            lines.push(Line::styled("  nothing matches", HINT));
        }
        let rect = |y: usize, height: usize| Rect {
            x: inner.x,
            y: inner.y + y as u16,
            width: inner.width,
            height: height as u16,
        };
        f.render_widget(Paragraph::new(Line::from(filter)), rect(0, 1));
        f.render_widget(Paragraph::new(lines), rect(1, list_h));
        // the list runs on above or below what is shown
        if top > 0 {
            f.render_widget(
                Paragraph::new(Span::styled("↑", HINT)),
                Rect {
                    x: inner.x + inner.width - 1,
                    ..rect(1, 1)
                },
            );
        }
        if top + list_h < visible.len() {
            f.render_widget(
                Paragraph::new(Span::styled("↓", HINT)),
                Rect {
                    x: inner.x + inner.width - 1,
                    ..rect(list_h, 1)
                },
            );
        }
        if !details.is_empty() {
            f.render_widget(
                Paragraph::new(Line::styled("─".repeat(w), HINT)),
                rect(1 + list_h, 1),
            );
            let details_h = details.len();
            f.render_widget(Paragraph::new(details), rect(2 + list_h, details_h));
        }
        f.render_widget(Paragraph::new(footer), rect(h - 1, 1));
    }
}

/// Every key of the builder and its lists, and what the colours mean.
fn draw_help(f: &mut Frame, area: Rect, title: &str, top: usize) -> (usize, usize) {
    const KEYS: &[(&str, &str)] = &[
        ("↑ ↓  k j", "select a member"),
        ("Tab  Shift-Tab", "next or previous field"),
        ("Space, or type", "open the field's list; typing filters it"),
        ("← →", "step the field's value"),
        ("+  Insert", "add a member like the selected one"),
        ("-  Delete", "remove the selected member"),
        ("[ ]", "previous or next release"),
        ("Enter", "keep the hardware once nothing is marked red"),
        ("Esc", "cancel"),
        ("In a list", "↑↓ ←→ PgUp PgDn Home End move, Enter chooses"),
        ("", "Esc goes back; Tab goes on to the next field"),
        ("", "Backspace erases the filter, Ctrl-U clears it"),
        (
            "Red",
            "what Cisco does not build, stack or run, with its reason",
        ),
        (
            "Yellow",
            "what a choice changed, or a fact the documents leave open",
        ),
        ("*", "in a list, the value chosen now"),
    ];
    f.render_widget(Clear, area);
    let block = Block::bordered().title(title.to_string());
    let inner = pad(block.inner(area));
    f.render_widget(block, area);
    let w = inner.width as usize;
    let kw = 16;
    let mut lines = vec![];
    for (key, what) in KEYS {
        let style = match *key {
            "Red" => BAD,
            "Yellow" => NOTE,
            _ => PLAIN,
        };
        for (i, part) in wrap(what, w.saturating_sub(kw)).into_iter().enumerate() {
            let head = if i == 0 {
                cell(key, kw)
            } else {
                " ".repeat(kw)
            };
            lines.push(Line::from(vec![
                Span::styled(head, style),
                Span::styled(part, HINT),
            ]));
        }
    }
    let h = inner.height as usize;
    let (all, shown) = (lines.len(), h.saturating_sub(1));
    let top = top.min(all.saturating_sub(shown));
    let page: Vec<Line> = lines.into_iter().skip(top).take(shown).collect();
    f.render_widget(Paragraph::new(page), inner);
    if h > 0 {
        let note = if all > shown {
            "↑↓ more keys; any other key closes this list"
        } else {
            "any key closes this list"
        };
        f.render_widget(
            Paragraph::new(Line::styled(note, HINT)),
            Rect {
                y: inner.y + inner.height - 1,
                height: 1,
                ..inner
            },
        );
    }
    (all, shown)
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

/// True when every model and module the family offers is reachable from the builder.
#[cfg(test)]
fn family_models(f: platform::Family) -> Vec<&'static str> {
    MODELS
        .iter()
        .filter(|m| m.family == f)
        .map(|m| m.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(b: &mut Builder, model: &str, module: Option<&str>) {
        let m = &mut b.members[b.row];
        m.model = model_index(model).unwrap();
        m.module = module.map(|x| module_index(x).unwrap());
    }

    fn press(b: &mut Builder, code: KeyCode) -> bool {
        b.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typed(b: &mut Builder, text: &str) {
        for c in text.chars() {
            assert!(press(b, KeyCode::Char(c)), "{c}");
        }
    }

    /// The builder as a terminal of that size shows it.
    fn screen(b: &Builder, w: u16, h: u16) -> String {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        t.draw(|f| {
            let area = f.area();
            b.draw(f, area, " Hardware for lab1 ")
        })
        .unwrap();
        let buf = t.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn r(t: &str) -> Release {
        Release::parse(t).unwrap()
    }

    /// What a list shows: headings marked `#`, then each item's name.
    fn shown(b: &Builder) -> Vec<String> {
        let p = b.picker.as_ref().expect("a list is open");
        p.visible()
            .iter()
            .map(|&i| match &p.entries[i] {
                Entry::Heading(h) => format!("# {h}"),
                Entry::Item { name, .. } => name.clone(),
            })
            .collect()
    }

    /// The reasons a list gives against an item.
    fn reasons_for(b: &Builder, item: &str) -> Vec<String> {
        let p = b.picker.as_ref().expect("a list is open");
        p.entries
            .iter()
            .find_map(|e| match e {
                Entry::Item { name, reasons, .. } if name == item => Some(reasons.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{item} is not in the list"))
    }

    #[test]
    fn a_9300x_stack_is_typed_from_a_new_builder() {
        let mut b = Builder::new();
        typed(&mut b, "48hx");
        press(&mut b, KeyCode::Enter);
        press(&mut b, KeyCode::Tab);
        typed(&mut b, "4c");
        press(&mut b, KeyCode::Enter);
        press(&mut b, KeyCode::Char('+'));
        assert_eq!(
            b.spec(),
            "C9300X-48HX+C9300X-NM-4C,C9300X-48HX+C9300X-NM-4C"
        );
        assert!(b.problems().is_empty(), "{:?}", b.problems());
        assert!(!press(&mut b, KeyCode::Enter), "Enter is left to keep it");
    }

    #[test]
    fn typing_opens_the_model_list_grouped_by_family_and_filtered() {
        let mut b = Builder::new();
        typed(&mut b, "48h");
        assert_eq!(b.picker.as_ref().map(|p| p.field), Some(Field::Model));
        assert_eq!(
            shown(&b),
            [
                "# Catalyst 9300X",
                "C9300X-48HX",
                "C9300X-48HXN",
                "# Catalyst 9300",
                "C9300-48H",
                "# Catalyst 9400 line cards",
                "C9400-LC-48H",
                "C9400-LC-48HN",
                "C9400-LC-48HX",
            ]
        );
        // a line card alone is marked with Cisco's reason, and stays choosable
        assert!(
            reasons_for(&b, "C9400-LC-48H")[0].contains("needs a supervisor"),
            "{:?}",
            reasons_for(&b, "C9400-LC-48H")
        );
        let s = screen(&b, 80, 24);
        assert!(
            s.contains("Model for member 1") && s.contains("6 of 103"),
            "{s}"
        );
        press(&mut b, KeyCode::Down);
        press(&mut b, KeyCode::Enter);
        assert!(b.picker.is_none());
        assert_eq!(b.spec(), "C9300X-48HXN");
        // a name typed in full is chosen whatever it is listed after
        typed(&mut b, "c9300-48h");
        press(&mut b, KeyCode::Enter);
        assert_eq!(b.spec(), "C9300-48H");
    }

    #[test]
    fn the_module_list_puts_what_fits_first_and_marks_the_rest_with_cisco_s_reason() {
        let mut b = Builder::from_spec("C9300X-12Y", release::DEFAULT);
        press(&mut b, KeyCode::Tab);
        assert_eq!(b.field, Field::Module);
        press(&mut b, KeyCode::Char(' '));
        let rows = shown(&b);
        assert_eq!(
            rows[..7],
            [
                "# Fits the C9300X-12Y",
                "none",
                "C9300X-NM-2C",
                "C9300X-NM-8Y",
                "C9300X-NM-8M",
                "# Does not fit the C9300X-12Y",
                "C9300X-NM-4C",
            ]
        );
        assert_eq!(rows.len(), 2 + 1 + MODULES.len(), "every module is offered");
        for fits in &rows[1..5] {
            assert!(reasons_for(&b, fits).is_empty(), "{fits}");
        }
        for other in &rows[6..] {
            assert!(!reasons_for(&b, other).is_empty(), "{other}");
        }
        assert!(
            reasons_for(&b, "C9300X-NM-4C")[0].contains(
                "C9300X-NM-4C does not fit the C9300X-12Y, which takes C9300X-NM-2C, C9300X-NM-8Y, C9300X-NM-8M"
            )
        );
        let s = screen(&b, 80, 24);
        assert!(
            s.contains("C9300-NM-4G     is a Catalyst 9300 module; the C9300X-12Y takes"),
            "{s}"
        );
        // one that does not fit is chosen all the same, and marked
        typed(&mut b, "4c");
        press(&mut b, KeyCode::Enter);
        assert_eq!(b.spec(), "C9300X-12Y+C9300X-NM-4C");
        assert!(b.problems()[0].1.contains("does not fit the C9300X-12Y"));
    }

    #[test]
    fn an_open_list_and_the_key_list_take_enter_and_esc() {
        let mut b = Builder::new();
        assert!(press(&mut b, KeyCode::Char(' ')), "Space opens the list");
        assert!(b.picker.is_some());
        assert!(press(&mut b, KeyCode::Esc), "Esc closes the list");
        assert!(b.picker.is_none());
        assert!(!press(&mut b, KeyCode::Esc), "then is left to cancel");
        press(&mut b, KeyCode::Char(' '));
        assert!(press(&mut b, KeyCode::Enter), "Enter chooses in a list");
        assert!(b.picker.is_none());
        assert!(!press(&mut b, KeyCode::Enter), "then is left to keep");
        assert!(press(&mut b, KeyCode::Char('?')));
        let s = screen(&b, 80, 24);
        assert!(
            s.contains("keep the hardware once nothing is marked red")
                && s.contains("Ctrl-U clears it")
                && s.contains("in a list, the value chosen now"),
            "{s}"
        );
        assert!(press(&mut b, KeyCode::Enter), "the key list takes Enter");
        assert!(b.help.is_none());
        press(&mut b, KeyCode::Char('?'));
        assert!(press(&mut b, KeyCode::Char('c')), "and any key");
        assert!(b.help.is_none() && b.picker.is_none(), "doing nothing else");
    }

    #[test]
    fn enter_with_a_problem_marked_says_what_to_fix_first() {
        let mut b = Builder::from_spec("C9300X-24Y,C9200-24T+C9200-NM-2Y", release::DEFAULT);
        assert!(press(&mut b, KeyCode::Enter), "the builder keeps Enter");
        assert_eq!(b.row, 1, "and selects the member the first problem names");
        let s = screen(&b, 80, 24);
        assert!(
            s.contains("Nothing is kept while a problem is marked: fix the red lines first.")
                && s.contains("member 2: a Catalyst 9200 does not stack with a Catalyst 9300X"),
            "{s}"
        );
        press(&mut b, KeyCode::Up);
        assert!(
            !screen(&b, 80, 24).contains("Nothing is kept"),
            "the next key clears it"
        );
    }

    #[test]
    fn hardware_vswitch_does_not_know_is_named_until_it_is_chosen() {
        // a design's model and module vswitch does not know: stood in for, and red until chosen
        let mut b = Builder::from_spec(
            "C9999-48Z,C9300X-24Y+C9300X-NM-99,C9999-24Q+C9999-NM",
            release::DEFAULT,
        );
        assert_eq!(b.members.len(), 3);
        assert_eq!(b.members[1].module, None);
        let named = |b: &Builder| -> Vec<String> {
            b.problems()
                .into_iter()
                .filter_map(|(_, p)| {
                    p.strip_prefix("vswitch knows no ")?
                        .strip_suffix(": choose what it is")
                        .map(str::to_string)
                })
                .collect()
        };
        assert_eq!(
            named(&b),
            ["C9999-48Z", "C9300X-NM-99", "C9999-24Q", "C9999-NM"]
        );
        assert!(press(&mut b, KeyCode::Enter), "Enter keeps nothing");
        assert!(screen(&b, 80, 24).contains("vswitch knows no C9999-48Z"));
        // a module chosen leaves a model vswitch does not know named, and a model chosen a module
        b.row = 0;
        b.field = Field::Module;
        press(&mut b, KeyCode::Char(' '));
        press(&mut b, KeyCode::Enter);
        for row in [1, 2] {
            b.row = row;
            b.field = Field::Model;
            press(&mut b, KeyCode::Right);
        }
        assert_eq!(named(&b), ["C9999-48Z", "C9300X-NM-99", "C9999-NM"]);
        // each goes once its own field is chosen
        for (row, field) in [(0, Field::Model), (1, Field::Module), (2, Field::Module)] {
            b.row = row;
            b.field = field;
            press(&mut b, KeyCode::Right);
        }
        assert!(named(&b).is_empty(), "{:?}", b.problems());
        // a spec vswitch reads whole has nothing to name
        assert!(
            Builder::from_spec("C9300X-24Y", release::DEFAULT)
                .problems()
                .is_empty()
        );
    }

    #[test]
    fn a_list_marks_only_what_its_choices_do() {
        // another member's problem is no number's doing; a number another holds is named by it
        let mut b = Builder::from_spec("C9300X-24Y,C9200-24T+C9200-NM-2Y", release::DEFAULT);
        b.field = Field::Number;
        press(&mut b, KeyCode::Char(' '));
        for free in ["1", "3", "4", "5", "6", "7", "8"] {
            assert!(
                reasons_for(&b, free).is_empty(),
                "{free}: {:?}",
                reasons_for(&b, free)
            );
        }
        assert_eq!(reasons_for(&b, "2"), ["taken by the C9200-24T"]);
        // nor is a supervisor's release any model's doing in another slot, slot or member named
        let mut b = Builder::from_spec("1=C9400-LC-48T,3=C9400-SUP-1XL", r("26.x"));
        assert!(
            b.problems().iter().any(|(_, p)| p.contains("runs up to")),
            "{:?}",
            b.problems()
        );
        press(&mut b, KeyCode::Char(' '));
        assert!(
            reasons_for(&b, "C9400-LC-48U").is_empty(),
            "{:?}",
            reasons_for(&b, "C9400-LC-48U")
        );
        let p = b.picker.as_ref().expect("the model list");
        for e in &p.entries {
            if let Entry::Item { name, reasons, .. } = e {
                assert!(
                    reasons
                        .iter()
                        .all(|r| !r.contains("3: the C9400-SUP-1XL runs up to")),
                    "{name}: {reasons:?}"
                );
            }
        }
        // a model that does not run the release says so for the member it is chosen for
        assert!(
            reasons_for(&b, "C9400-SUP-1XL")
                .iter()
                .any(|r| r.contains("runs up to")),
            "{:?}",
            reasons_for(&b, "C9400-SUP-1XL")
        );
    }

    #[test]
    fn left_and_right_step_within_the_member_s_family() {
        let mut b = Builder::new();
        press(&mut b, KeyCode::Right);
        assert_eq!(b.spec(), "C9300X-12Y");
        press(&mut b, KeyCode::Left);
        press(&mut b, KeyCode::Left);
        assert_eq!(
            b.spec(),
            "C9300X-48TX",
            "wrapping within the Catalyst 9300X"
        );
        // the module column steps through the modules the model takes
        press(&mut b, KeyCode::Tab);
        press(&mut b, KeyCode::Right);
        assert_eq!(b.spec(), "C9300X-48TX+C9300X-NM-4C");
        let mut s = Builder::from_spec("3=C9400X-SUP-2XL", release::DEFAULT);
        press(&mut s, KeyCode::Right);
        assert_eq!(
            s.spec(),
            "3=C9400-SUP-1",
            "a supervisor steps through supervisors"
        );
        // a module the next model does not take comes out, and says so
        let mut m = Builder::from_spec("C9300X-24Y+C9300X-NM-4C", release::DEFAULT);
        press(&mut m, KeyCode::Right);
        assert_eq!(m.spec(), "C9300X-12Y");
        assert!(
            screen(&m, 80, 24).contains("C9300X-NM-4C taken out: the C9300X-12Y does not take it.")
        );
    }

    #[test]
    fn a_family_whose_book_lacks_the_release_moves_to_one_it_runs() {
        let mut b = Builder::new();
        typed(&mut b, "2960g-24");
        press(&mut b, KeyCode::Enter);
        assert_eq!(b.spec(), "WS-C2960G-24TC-L");
        assert_eq!(
            b.release,
            r("15.0(2)SE"),
            "the newest release a plain 2960 runs"
        );
        assert!(b.problems().is_empty(), "{:?}", b.problems());
        let s = screen(&b, 80, 24);
        assert!(
            s.contains("Release now IOS 15.0(2)SE: no Catalyst 2960 command reference")
                && s.contains("IOS     15.0(2)SE")
                && !s.contains("IOS XE"),
            "{s}"
        );
        // a release the new book documents stays
        typed(&mut b, "2960+");
        press(&mut b, KeyCode::Enter);
        assert_eq!(b.spec(), "WS-C2960+24TC-L");
        assert_eq!(b.release, r("15.0(2)SE"));
        // back on IOS XE, the release a switch runs when none is named
        typed(&mut b, "c9300x-24y");
        press(&mut b, KeyCode::Enter);
        assert_eq!(b.release, release::DEFAULT);
        assert!(screen(&b, 80, 24).contains("IOS XE  17.15"));
    }

    #[test]
    fn a_release_is_typed_into_its_list() {
        let mut b = Builder::from_spec("C9300X-24Y", release::DEFAULT);
        press(&mut b, KeyCode::Tab);
        press(&mut b, KeyCode::Tab);
        assert_eq!(b.field, Field::Release);
        press(&mut b, KeyCode::Char(' '));
        assert!(
            reasons_for(&b, "17.4")[0].contains("the C9300X-24Y runs IOS XE 17.5 and later"),
            "{:?}",
            reasons_for(&b, "17.4")
        );
        assert!(reasons_for(&b, "17.5").is_empty());
        press(&mut b, KeyCode::Esc);
        typed(&mut b, "17.9");
        assert_eq!(shown(&b), ["# IOS XE 17", "17.9"]);
        press(&mut b, KeyCode::Enter);
        assert_eq!(b.release, r("17.9"));
    }

    #[test]
    fn a_9400_is_built_by_slot_in_the_chassis_chosen() {
        let mut b = Builder::new();
        typed(&mut b, "sup-1xl");
        press(&mut b, KeyCode::Enter);
        assert_eq!(
            b.spec(),
            "2=C9400-SUP-1XL",
            "a supervisor goes to a supervisor slot"
        );
        let s = screen(&b, 80, 24);
        assert!(
            s.contains("Slot  Model")
                && s.contains("Chassis C9404R")
                && s.contains("Moved from slot 1 to slot 2."),
            "{s}"
        );
        // the chassis is a field of its own; choosing one moves the slots into it
        press(&mut b, KeyCode::Tab);
        assert_eq!(
            b.field,
            Field::Chassis,
            "a 9400 has no module column to stop at"
        );
        press(&mut b, KeyCode::Right);
        assert_eq!(b.spec(), "3=C9400-SUP-1XL");
        press(&mut b, KeyCode::Char('+'));
        assert_eq!(
            b.spec(),
            "3=C9400-SUP-1XL,4=C9400-SUP-1XL",
            "the redundant supervisor"
        );
        // a supervisor made a line card moves to a line card slot of the chassis chosen
        press(&mut b, KeyCode::BackTab);
        typed(&mut b, "lc-48u");
        press(&mut b, KeyCode::Enter);
        assert_eq!(b.spec(), "1=C9400-LC-48U,3=C9400-SUP-1XL");
        press(&mut b, KeyCode::Char('+'));
        assert_eq!(b.spec(), "1=C9400-LC-48U,2=C9400-LC-48U,3=C9400-SUP-1XL");
        assert_eq!(b.row, 1, "the table in slot order, the new card selected");
        assert!(b.problems().is_empty(), "{:?}", b.problems());
        let s = screen(&b, 80, 24);
        assert!(
            s.contains("Chassis C9407R   supervisors in 3 and 4, line cards in 1, 2 and 5 to 7"),
            "{s}"
        );
        // the slot list names what holds a slot and marks those the chassis does not have
        b.field = Field::Number;
        press(&mut b, KeyCode::Char(' '));
        assert_eq!(reasons_for(&b, "3")[0], "taken by the C9400-SUP-1XL");
        assert!(reasons_for(&b, "5").is_empty());
        assert!(reasons_for(&b, "8")[0].contains("slot 8 is not a line card slot"));
    }

    #[test]
    fn line_cards_either_side_of_one_supervisor_fit_no_one_chassis() {
        let b = Builder::from_spec(
            "2=C9400-LC-48T,3=C9400-SUP-1XL,4=C9400-LC-48T",
            release::DEFAULT,
        );
        // each card alone fits a chassis the supervisor fits; the stack's own check names it once
        assert_eq!(
            b.problems(),
            [(
                0,
                "no one chassis has a supervisor in slot 3 and line cards in slots 2 and 4 (supervisor data sheet)"
                    .to_string()
            )]
        );
        let s = screen(&b, 80, 24);
        assert!(s.contains("Chassis none"), "{s}");
    }

    #[test]
    fn adding_stops_at_what_the_family_stacks() {
        let mut one = Builder::from_spec("WS-C2960G-24TC-L", r("15.0(2)SE"));
        press(&mut one, KeyCode::Char('+'));
        assert_eq!(one.members.len(), 1);
        assert!(screen(&one, 80, 24).contains("A Catalyst 2960 is one switch: it does not stack."));
        let mut eight = Builder::from_spec(&["C9300-48P"; 8].join(","), release::DEFAULT);
        press(&mut eight, KeyCode::Char('+'));
        assert_eq!(eight.members.len(), 8);
        assert!(screen(&eight, 80, 24).contains("A Catalyst 9300 stack takes at most 8 members."));
        press(&mut eight, KeyCode::Char('-'));
        assert_eq!(eight.members.len(), 7);
        let mut full = Builder::from_spec(
            "1=C9400-LC-48T,2=C9400-SUP-1XL,3=C9400-SUP-1XL,4=C9400-LC-48T",
            release::DEFAULT,
        );
        press(&mut full, KeyCode::Char('+'));
        assert_eq!(full.members.len(), 4);
        assert!(
            screen(&full, 80, 24)
                .contains("No free slot of the C9404R takes another C9400-LC-48T.")
        );
    }

    #[test]
    fn screens_hold_every_member_and_whole_names_at_80_by_24_and_120_by_40() {
        let full_9400 = "1=C9400-LC-48U,2=C9400-LC-48U,3=C9400-LC-48U,4=C9400-LC-48U,5=C9400X-SUP-2,6=C9400X-SUP-2,7=C9400-LC-24XS,8=C9400-LC-48P,9=C9400-LC-48T,10=C9400-LC-48H";
        let eight = ["C9300-48UXM+C3850-NM-2-10G"; 8].join(",");
        for (spec, release, os) in [
            (
                // the LL models and the 24PSQ-L do not stack (2960-X data sheet)
                "WS-C2960X-48FPD-L,WS-C2960X-48LPS-L",
                "15.2(7)E",
                "IOS     15.2(7)E",
            ),
            (eight.as_str(), "17.15", "IOS XE  17.15"),
            (full_9400, "17.15", "IOS XE  17.15"),
        ] {
            let b = Builder::from_spec(spec, r(release));
            assert!(b.problems().is_empty(), "{spec}: {:?}", b.problems());
            for (w, h) in [(80, 24), (120, 40)] {
                let s = screen(&b, w, h);
                let rows: Vec<&str> = s.lines().filter(|l| l.contains("│ ")).collect();
                for m in &b.members {
                    let name = MODELS[m.model].name;
                    let module = m.module.map_or("", |i| MODULES[i].name);
                    assert!(
                        rows.iter()
                            .any(|l| l.contains(&format!(" {name} ")) && l.contains(module)),
                        "{w}x{h} {name} {module}:\n{s}"
                    );
                }
                assert!(s.contains(os) && s.contains("Enter keep"), "{w}x{h}:\n{s}");
                assert!(!s.contains("built in:"), "{s}");
            }
        }
    }

    #[test]
    fn the_focused_cell_is_highlighted_and_a_marked_member_red() {
        let b = Builder::from_spec("C9300X-24Y,C9200-24T", release::DEFAULT);
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        t.draw(|f| {
            let area = f.area();
            b.draw(f, area, " Hardware for lab1 ")
        })
        .unwrap();
        let buf = t.backend().buffer().clone();
        let find = |text: &str, nth: usize| {
            (0..buf.area.height)
                .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    text.chars().enumerate().all(|(i, c)| {
                        x + (i as u16) < buf.area.width
                            && buf[(x + i as u16, y)].symbol() == c.to_string()
                    })
                })
                .nth(nth)
                .unwrap_or_else(|| panic!("{text}"))
        };
        let (x, y) = find("C9300X-24Y", 0);
        assert_eq!(
            (buf[(x, y)].fg, buf[(x, y)].bg),
            (Color::Black, Color::Cyan)
        );
        let (x, y) = find("C9200-24T", 0);
        assert_eq!(
            buf[(x, y)].fg,
            Color::Red,
            "a member Cisco does not stack is red"
        );
        let (x, y) = find("Space/type", 0);
        assert_eq!(buf[(x + 11, y)].fg, Color::Gray, "hints are gray");
        // the focused cell of a marked member is red with the rest of its row
        let mut b = b.clone();
        b.row = 1;
        t.draw(|f| {
            let area = f.area();
            b.draw(f, area, " Hardware for lab1 ")
        })
        .unwrap();
        let buf = t.backend().buffer().clone();
        let (x, y) = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .find(|&(x, y)| {
                "C9200-24T".chars().enumerate().all(|(i, c)| {
                    x + (i as u16) < buf.area.width
                        && buf[(x + i as u16, y)].symbol() == c.to_string()
                })
            })
            .expect("C9200-24T");
        assert_eq!((buf[(x, y)].fg, buf[(x, y)].bg), (Color::Black, Color::Red));
    }

    #[test]
    fn the_key_list_scrolls_where_the_box_cuts_it_and_a_list_takes_the_builder_s_keys() {
        let mut b = Builder::from_spec("C9300X-24Y", release::DEFAULT);
        press(&mut b, KeyCode::Char('?'));
        let s = screen(&b, 60, 15);
        assert!(
            !s.contains("the value chosen now") && s.contains("↑↓ more keys"),
            "{s}"
        );
        for _ in 0..12 {
            assert!(press(&mut b, KeyCode::Down));
            screen(&b, 60, 15);
        }
        let s = screen(&b, 60, 15);
        assert!(s.contains("the value chosen now"), "{s}");
        assert!(press(&mut b, KeyCode::Char('c')), "any other key");
        assert!(b.help.is_none() && b.picker.is_none());
        // in a list: Left and Right move as Up and Down, ? lists the keys, Tab goes on to the next
        // field choosing nothing
        press(&mut b, KeyCode::Char(' '));
        let at = b.picker.as_ref().and_then(|p| p.cursor);
        press(&mut b, KeyCode::Right);
        assert_ne!(b.picker.as_ref().and_then(|p| p.cursor), at);
        press(&mut b, KeyCode::Left);
        assert_eq!(b.picker.as_ref().and_then(|p| p.cursor), at);
        press(&mut b, KeyCode::Char('?'));
        assert!(screen(&b, 80, 24).contains("Tab goes on to the next field"));
        press(&mut b, KeyCode::Esc);
        assert!(
            b.picker.is_some() && b.help.is_none(),
            "the list stays open"
        );
        press(&mut b, KeyCode::Right);
        press(&mut b, KeyCode::Tab);
        assert!(b.picker.is_none());
        assert_eq!(
            (b.field, b.spec()),
            (Field::Module, "C9300X-24Y".to_string())
        );
        // the star is a legend, drawn as the gutter's
        press(&mut b, KeyCode::Char(' '));
        let s = screen(&b, 80, 24);
        assert!(s.contains("* the value now") && s.contains("? keys"), "{s}");
    }

    #[test]
    fn a_panel_the_box_cuts_says_how_much_more_there_is() {
        let b = Builder::from_spec(&["C9300-48P"; 8].join(","), release::DEFAULT);
        let s = screen(&b, 80, 24);
        assert!(
            s.contains("more lines: a taller terminal shows them"),
            "{s}"
        );
        assert!(!screen(&b, 120, 40).contains("more lines"));
    }

    #[test]
    fn a_small_terminal_draws_what_fits() {
        let mut b = Builder::from_spec(
            "1=C9400-LC-48U,2=C9400-LC-48U,3=C9400-LC-48U,4=C9400-LC-48U,5=C9400X-SUP-2,6=C9400X-SUP-2,7=C9400-LC-24XS,8=C9400-LC-48P,9=C9400-LC-48T,10=C9400-LC-48H",
            release::DEFAULT,
        );
        b.row = 9;
        let sizes = [(60, 14), (40, 10), (24, 6), (10, 3), (1, 1)];
        for (w, h) in sizes {
            let s = screen(&b, w, h);
            if h >= 10 {
                assert!(
                    s.contains("C9400-LC-48H"),
                    "the selected member stays in view:\n{s}"
                );
            }
        }
        press(&mut b, KeyCode::Char(' '));
        for (w, h) in sizes {
            screen(&b, w, h);
        }
        press(&mut b, KeyCode::Esc);
        press(&mut b, KeyCode::Char('?'));
        for (w, h) in sizes {
            screen(&b, w, h);
        }
    }

    #[test]
    fn every_9200_model_and_module_lead_to_their_ports() {
        assert_eq!(
            family_models(platform::Family::C9200L),
            [
                "C9200L-24T-4G",
                "C9200L-48T-4G",
                "C9200L-24T-4X",
                "C9200L-48T-4X",
                "C9200L-24P-4G",
                "C9200L-48P-4G",
                "C9200L-24P-4X",
                "C9200L-48P-4X",
                "C9200L-48PL-4G",
                "C9200L-48PL-4X",
                "C9200L-24PXG-4X",
                "C9200L-48PXG-4X",
                "C9200L-24PXG-2Y",
                "C9200L-48PXG-2Y"
            ]
        );
        assert_eq!(
            family_models(platform::Family::C9200B),
            ["C9200-24PB", "C9200-48PB"]
        );
        // a 9200L's 25G uplinks, and the 48-port PXG's twelve mGig ports
        let s = platform::checked_stack("C9200L-48PXG-4X,C9200L-24PXG-2Y", crate::release::DEFAULT)
            .unwrap();
        let p = s.physical();
        for n in [
            "GigabitEthernet1/0/36",
            "TenGigabitEthernet1/0/37",
            "TenGigabitEthernet1/1/4",
            "TwentyFiveGigE2/1/2",
        ] {
            assert!(p.contains(&n.to_string()), "{n}");
        }
        assert!(!p.contains(&"GigabitEthernet1/0/37".to_string()));
        assert!(
            platform::checked_stack("C9200-24PB,C9200-24P", crate::release::DEFAULT).is_err(),
            "a B model stacks only with B models"
        );
        let s = platform::checked_stack(
            "C9200-48T+C9200-NM-4X,C9200-24T+C9200-NM-4G",
            crate::release::DEFAULT,
        )
        .unwrap();
        let p = s.physical();
        assert!(
            p.contains(&"GigabitEthernet1/0/48".to_string())
                && p.contains(&"TenGigabitEthernet1/1/4".to_string())
        );
        assert!(
            p.contains(&"GigabitEthernet2/0/24".to_string())
                && p.contains(&"GigabitEthernet2/1/4".to_string())
        );
        assert!(!p.contains(&"GigabitEthernet2/0/25".to_string()));
        let l = platform::checked_stack("C9200L-48T-4X,C9200L-24T-4G", crate::release::DEFAULT)
            .unwrap()
            .physical();
        assert!(
            l.contains(&"TenGigabitEthernet1/1/4".to_string())
                && l.contains(&"GigabitEthernet2/1/4".to_string())
        );
        let x = platform::checked_stack(
            "C9200-24PXG+C9200-NM-2Y,C9200-48PXG+C9200-NM-2Q",
            crate::release::DEFAULT,
        )
        .unwrap()
        .physical();
        for port in [
            "GigabitEthernet1/0/16",
            "TenGigabitEthernet1/0/17",
            "TenGigabitEthernet1/0/24",
            "TwentyFiveGigE1/1/2",
            "GigabitEthernet2/0/40",
            "TenGigabitEthernet2/0/48",
            "FortyGigabitEthernet2/1/2",
        ] {
            assert!(x.contains(&port.to_string()), "{port}");
        }
        assert!(!x.contains(&"GigabitEthernet1/0/17".to_string()));
    }

    #[test]
    fn incompatible_modules_are_named_with_cisco_s_reason() {
        let mut b = Builder::new();
        set(&mut b, "C9200-24T", Some("C9200-NM-2Y"));
        let p = b.problems();
        assert_eq!(p.len(), 1);
        assert!(
            p[0].1.contains(
                "C9200-NM-2Y does not fit the C9200-24T, which takes C9200-NM-4G, C9200-NM-4X"
            ),
            "{p:?}"
        );
        set(&mut b, "C9200-24PXG", Some("C9200-NM-4G"));
        assert!(b.problems()[0].1.contains("does not fit the C9200-24PXG"));
        set(&mut b, "C9200-24PXG", Some("C9200-NM-2Q"));
        assert!(b.problems().is_empty());
        set(&mut b, "C9200L-24T-4X", Some("C9200-NM-4X"));
        assert!(b.problems()[0].1.contains("built-in uplinks"));
        set(&mut b, "C9200-48T", Some("C9300X-NM-4C"));
        assert!(b.problems()[0].1.contains("is a Catalyst 9300X module"));
        // families do not mix in a stack
        set(&mut b, "C9200-48T", Some("C9200-NM-4X"));
        press(&mut b, KeyCode::Char('+'));
        set(&mut b, "C9200L-48T-4X", None);
        let p = b.problems();
        assert!(
            p.len() == 1 && p[0].0 == 2 && p[0].1.contains("does not stack with"),
            "{p:?}"
        );
        assert!(
            platform::checked_stack(&b.spec(), crate::release::DEFAULT)
                .unwrap_err()
                .starts_with("member 2: ")
        );
    }

    #[test]
    fn brackets_step_through_the_releases_the_book_documents() {
        let r = |t| Release::parse(t).unwrap();
        let mut b = Builder::from_spec("C9200L-24T-4G", r("16.10"));
        press(&mut b, KeyCode::Char('['));
        assert_eq!(b.release, r("16.9"), "the 9200 book starts at 16.9");
        assert!(!screen(&b, 80, 24).contains("went round"));
        press(&mut b, KeyCode::Char('['));
        assert_eq!(b.release, r("26.x"), "and wraps to the newest");
        assert!(
            screen(&b, 80, 24).contains("The release went round to the newest, IOS XE 26.x."),
            "{}",
            screen(&b, 80, 24)
        );
        press(&mut b, KeyCode::Char(']'));
        assert_eq!(b.release, r("16.9"));
        assert!(screen(&b, 80, 24).contains("went round to the oldest, IOS XE 16.9."));
        // a release the book does not document is marked
        let b = Builder::from_spec("C9200L-24T-4G", r("16.5"));
        assert!(
            b.problems()
                .iter()
                .any(|(n, w)| *n == 0 && w.contains("no Catalyst 9200 command reference")),
            "{:?}",
            b.problems()
        );
    }

    #[test]
    fn keys_build_a_stack_and_its_text() {
        let mut b = Builder::from_spec("C9200-24T+C9200-NM-4G", crate::release::DEFAULT);
        assert_eq!(b.spec(), "C9200-24T+C9200-NM-4G");
        press(&mut b, KeyCode::Tab); // to the module column
        press(&mut b, KeyCode::Right); // C9200-NM-4X
        assert_eq!(b.spec(), "C9200-24T+C9200-NM-4X");
        press(&mut b, KeyCode::Char('+'));
        assert_eq!(b.spec(), "C9200-24T+C9200-NM-4X,C9200-24T+C9200-NM-4X");
        press(&mut b, KeyCode::Char('-'));
        assert_eq!(b.members.len(), 1);
        press(&mut b, KeyCode::Char('-'));
        assert_eq!(b.members.len(), 1, "the last member stays");
        // a member added takes the lowest number free, never one past 8
        let mut gap = Builder::from_spec(
            "1=C9200-24T,2=C9200-24T,3=C9200-24T",
            crate::release::DEFAULT,
        );
        gap.row = 1;
        press(&mut gap, KeyCode::Char('-'));
        press(&mut gap, KeyCode::Char('+'));
        let mut numbers: Vec<u32> = gap.members.iter().map(|m| m.number).collect();
        numbers.sort();
        assert_eq!(numbers, [1, 2, 3]);
        // the module column steps through none and the two modules the C9200-24T takes
        let before = b.members[0].module;
        for _ in 0..3 {
            press(&mut b, KeyCode::Right);
        }
        assert_eq!(b.members[0].module, before, "a full turn comes back");
        press(&mut b, KeyCode::Right);
        assert_eq!(b.members[0].module, None, "through none");
        let many = "C9200-24T,".repeat(9);
        assert!(
            Builder::from_spec(many.trim_end_matches(','), crate::release::DEFAULT)
                .problems()
                .iter()
                .any(|(_, w)| w.contains("at most 8"))
        );
    }
}
