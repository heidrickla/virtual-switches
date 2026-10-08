//! A listed switch's kept configuration, read in the switch list: scrolled by line and page, searched
//! with `/`, `n` and `N` stepping through the lines that match.

use super::input::Input;
use super::{BOLD, HINT, NOTE, PROBLEM, fit_left};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use std::cell::Cell;

const MATCH: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
const HIT: Style = Style::new().fg(Color::Black).bg(Color::Yellow);

#[derive(Debug, PartialEq)]
pub struct Viewer {
    pub name: String,
    /// where the configuration is kept, for the title
    pub path: String,
    pub lines: Vec<String>,
    /// the first line shown, and the first column
    pub top: usize,
    pub left: usize,
    /// a search being typed
    pub typing: Option<Input>,
    /// the text searched for last, and the line of the match shown
    pub query: String,
    pub hit: Option<usize>,
    /// what the last search said: not found (a problem), or that it went round the end
    pub note: Option<(String, bool)>,
    /// from the last draw: the lines shown and the columns a line shows
    page: Cell<usize>,
    room: Cell<usize>,
}

/// Where `needle` is in `line`, ignoring ASCII case, as byte ranges.
fn matches(line: &str, needle: &str) -> Vec<(usize, usize)> {
    let (h, n) = (line.as_bytes(), needle.as_bytes());
    if n.is_empty() || n.len() > h.len() {
        return vec![];
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + n.len() <= h.len() {
        if h[i..i + n.len()].eq_ignore_ascii_case(n) {
            out.push((i, i + n.len()));
            i += n.len();
        } else {
            i += 1;
        }
    }
    out
}

impl Viewer {
    pub fn new(name: &str, path: &str, text: &str) -> Self {
        Viewer {
            name: name.to_string(),
            path: path.to_string(),
            // a tab has no width of its own on screen
            lines: text.lines().map(|l| l.replace('\t', "    ")).collect(),
            top: 0,
            left: 0,
            typing: None,
            query: String::new(),
            hit: None,
            note: None,
            page: Cell::new(20),
            room: Cell::new(72),
        }
    }

    fn last_top(&self) -> usize {
        self.lines.len().saturating_sub(self.page.get().max(1))
    }

    fn scroll(&mut self, by: isize) {
        self.top = (self.top as isize + by).clamp(0, self.last_top() as isize) as usize;
    }

    /// The first line matching from `start` on (back from it, `back`), going round the end.
    fn find(&mut self, start: isize, back: bool) {
        let n = self.lines.len() as isize;
        if self.query.is_empty() || n == 0 {
            return;
        }
        let found = (0..n)
            .map(|k| if back { start - k } else { start + k })
            .find(|raw| !matches(&self.lines[raw.rem_euclid(n) as usize], &self.query).is_empty());
        match found {
            Some(raw) => {
                let i = raw.rem_euclid(n) as usize;
                let wrapped = raw < 0 || raw >= n;
                self.hit = Some(i);
                self.note = wrapped.then(|| ("The search went round the end.".into(), false));
                let page = self.page.get().max(1);
                if i < self.top || i >= self.top + page {
                    self.top = i.saturating_sub(page / 3).min(self.last_top());
                }
                // the match in view sideways, a little of what precedes it shown
                let (at, end) = matches(&self.lines[i], &self.query)[0];
                let col = self.lines[i][..at].chars().count();
                let len = self.lines[i][at..end].chars().count();
                let room = self.room.get().max(1);
                if col < self.left || col + len > self.left + room {
                    self.left = col.saturating_sub(8);
                }
            }
            None => {
                self.hit = None;
                self.note = Some((format!("Not found: {}", self.query), true));
            }
        }
    }

    /// Whether the viewer stays open after the key.
    pub fn key(&mut self, k: KeyEvent) -> bool {
        if let Some(input) = &mut self.typing {
            match k.code {
                KeyCode::Esc => self.typing = None,
                KeyCode::Enter => {
                    self.query = input.text.clone();
                    self.typing = None;
                    self.find(self.top as isize, false);
                }
                _ => {
                    input.key(k);
                }
            }
            return true;
        }
        let page = self.page.get().max(1) as isize;
        self.note = None;
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return false,
            KeyCode::Up | KeyCode::Char('k') => self.scroll(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll(1),
            KeyCode::PageUp | KeyCode::Char('b') => self.scroll(-page),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll(page),
            KeyCode::Home | KeyCode::Char('g') => self.top = 0,
            KeyCode::End | KeyCode::Char('G') => self.top = self.last_top(),
            KeyCode::Left => self.left = self.left.saturating_sub(8),
            KeyCode::Right => {
                let widest = self.lines.iter().map(|l| l.chars().count()).max();
                self.left = (self.left + 8).min(widest.unwrap_or(0).saturating_sub(1));
            }
            KeyCode::Char('/') => self.typing = Some(Input::default()),
            KeyCode::Char('n') => {
                let start = self.hit.map_or(self.top as isize, |h| h as isize + 1);
                self.find(start, false)
            }
            KeyCode::Char('N') => {
                let start = self.hit.map_or(self.top as isize, |h| h as isize) - 1;
                self.find(start, true)
            }
            _ => {}
        }
        true
    }

    pub fn paste(&mut self, text: &str) {
        if let Some(input) = &mut self.typing {
            input.paste(text);
        }
    }

    /// The configuration in `area`, line numbers on the left, matches marked.
    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let block = Block::bordered();
        let [text, bar] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(block.inner(area));
        // the lines this frame shows, which the title counts
        self.page.set(text.height as usize);
        let digits = self.lines.len().max(1).to_string().len();
        let room = (text.width as usize).saturating_sub(digits + 2);
        self.room.set(room);
        let last = (self.top + self.page.get()).min(self.lines.len());
        let block = block
            .title(Span::styled(format!(" {} ", self.name), BOLD))
            .title_top(
                Line::from(format!(
                    " lines {}-{} of {} ",
                    (self.top + 1).min(self.lines.len()),
                    last,
                    self.lines.len()
                ))
                .right_aligned(),
            )
            .title_bottom(Line::styled(
                format!(
                    " {} ",
                    fit_left(&self.path, (area.width as usize).saturating_sub(6))
                ),
                HINT,
            ));
        f.render_widget(block, area);
        let shown: Vec<Line> = self
            .lines
            .iter()
            .enumerate()
            .skip(self.top)
            .take(text.height as usize)
            .map(|(i, l)| {
                let mut spans = vec![Span::styled(format!("{:>digits$}  ", i + 1), HINT)];
                spans.extend(self.marked(l, Some(i) == self.hit, room));
                Line::from(spans)
            })
            .collect();
        f.render_widget(Paragraph::new(shown), text);
        let line = match (&self.typing, &self.note) {
            (Some(input), _) => {
                let (t, col) = input.window((bar.width as usize).saturating_sub(2));
                f.set_cursor_position((bar.x + 1 + col as u16, bar.y));
                Line::from(vec![Span::raw("/"), Span::raw(t)])
            }
            (None, Some((note, true))) => Line::styled(note.clone(), PROBLEM),
            (None, Some((note, false))) => Line::styled(note.clone(), NOTE),
            (None, None) if !self.query.is_empty() => {
                let count: usize = self
                    .lines
                    .iter()
                    .map(|l| matches(l, &self.query).len())
                    .sum();
                Line::styled(
                    format!("/{}   {count} match(es)   n next   N previous", self.query),
                    HINT,
                )
            }
            (None, None) => Line::default(),
        };
        f.render_widget(Paragraph::new(line), bar);
    }

    /// A line from column `left`, `room` columns of it, its matches marked.
    fn marked(&self, line: &str, hit: bool, room: usize) -> Vec<Span<'static>> {
        let found = matches(line, &self.query);
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut col = 0;
        let mut taken = 0;
        let mut push = |text: &str, style: Style| {
            let mut out = String::new();
            for c in text.chars() {
                if col >= self.left && taken < room {
                    out.push(c);
                    taken += 1;
                }
                col += 1;
            }
            if !out.is_empty() {
                spans.push(Span::styled(out, style));
            }
        };
        let mut at = 0;
        for (s, e) in found {
            push(&line[at..s], Style::new());
            push(&line[s..e], if hit { HIT } else { MATCH });
            at = e;
        }
        push(&line[at..], Style::new());
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn press(v: &mut Viewer, code: KeyCode) -> bool {
        v.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn a_search_steps_through_the_matching_lines_and_goes_round() {
        let text: String = (1..=60)
            .map(|i| {
                if i % 20 == 0 {
                    format!("interface Vlan{i}\n")
                } else {
                    format!("! line {i}\n")
                }
            })
            .collect();
        let mut v = Viewer::new("idf1", "configs/idf1.cfg", &text);
        press(&mut v, KeyCode::Char('/'));
        for c in "VLAN".chars() {
            press(&mut v, KeyCode::Char(c));
        }
        press(&mut v, KeyCode::Enter);
        assert_eq!(v.hit, Some(19), "line 20, case ignored");
        press(&mut v, KeyCode::Char('n'));
        assert_eq!(v.hit, Some(39));
        press(&mut v, KeyCode::Char('n'));
        press(&mut v, KeyCode::Char('n'));
        assert_eq!(v.hit, Some(19), "round the end");
        assert!(v.note.as_ref().is_some_and(|(n, _)| n.contains("round")));
        press(&mut v, KeyCode::Char('N'));
        assert_eq!(v.hit, Some(59));
        press(&mut v, KeyCode::Char('/'));
        press(&mut v, KeyCode::Char('x'));
        press(&mut v, KeyCode::Enter);
        assert_eq!(v.hit, None);
        assert_eq!(v.note, Some(("Not found: x".into(), true)));
        assert!(!press(&mut v, KeyCode::Esc), "Esc closes the viewer");
    }

    /// The viewer drawn in `w` by `h`, as text.
    fn shown(v: &Viewer, w: u16, h: u16) -> String {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        t.draw(|f| {
            let area = f.area();
            v.draw(f, area)
        })
        .unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_title_counts_the_lines_the_first_frame_shows() {
        let text: String = (1..=60).map(|i| format!("! line {i}\n")).collect();
        let v = Viewer::new("idf1", "configs/idf1.cfg", &text);
        // 21 rows: two borders and the search line leave 18
        let s = shown(&v, 80, 21);
        assert!(s.contains("lines 1-18 of 60"), "{s}");
        assert!(
            s.contains("18  ! line 18") && !s.contains("19  ! line 19"),
            "{s}"
        );
    }

    #[test]
    fn a_match_past_the_right_edge_scrolls_into_view() {
        let long = format!("snmp-server location {} rack-42-east", "x".repeat(90));
        let text = format!("hostname idf1\n{long}\nend\n");
        let mut v = Viewer::new("idf1", "configs/idf1.cfg", &text);
        shown(&v, 80, 21);
        press(&mut v, KeyCode::Char('/'));
        for c in "rack-42".chars() {
            press(&mut v, KeyCode::Char(c));
        }
        press(&mut v, KeyCode::Enter);
        assert_eq!(v.hit, Some(1));
        let s = shown(&v, 80, 21);
        assert!(s.contains("rack-42-east"), "{s}");
        // and back to the left for one before it
        press(&mut v, KeyCode::Char('/'));
        for c in "hostname".chars() {
            press(&mut v, KeyCode::Char(c));
        }
        press(&mut v, KeyCode::Enter);
        assert!(shown(&v, 80, 21).contains("hostname idf1"));
    }

    #[test]
    fn matches_ignore_case_and_do_not_overlap() {
        assert_eq!(matches("aAaa", "aa"), vec![(0, 2), (2, 4)]);
        assert_eq!(matches("Vlan10 vlan20", "VLAN"), vec![(0, 4), (7, 11)]);
        assert!(matches("abc", "").is_empty());
    }
}
