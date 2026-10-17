//! The console a switch is typed at. On a terminal it is a line editor as an IOS console: `?` prints
//! help for the line so far and leaves it as typed, Tab completes the last word, Up and Down step
//! through the lines entered, the IOS editing keys move and change words, a kill ring recalls what was
//! deleted, and long output pages at `--More--`. Piped input is read line by line, a line ending in
//! `?` printing help.

use crate::cli;
use crate::shell::Session;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use ratatui::crossterm::{cursor, execute, queue, style::Print, terminal};
use std::io::{BufRead, IsTerminal, Write};

/// What a key did to the line being typed.
#[derive(Debug, PartialEq)]
pub enum Effect {
    None,
    /// the line changed: draw it again
    Redraw,
    /// `?`: the help printed for the line so far, which stays as typed
    Help(Vec<String>),
    /// Enter: the line typed
    Enter(String),
    /// Ctrl-Z: configuration mode ends (`end`)
    End,
    /// Ctrl-C: the line is discarded
    Cancel,
    /// Ctrl-L or Ctrl-R: reprint the prompt and line on a fresh row
    Redisplay,
    /// nothing to do here, or an impossible key: a beep
    Bell,
}

/// Which way a kill took text, so consecutive kills join in reading order.
#[derive(Clone, Copy, Debug, PartialEq)]
enum KillDir {
    Back,
    Fwd,
}

/// Esc C, Esc L, Esc U.
#[derive(Clone, Copy)]
enum Case {
    Capitalize,
    Lower,
    Upper,
}

/// A yank in place, so Esc Y can replace it with the next ring entry.
#[derive(Clone, Copy)]
struct Yank {
    /// where the yanked text starts, in characters
    start: usize,
    /// how many characters it is now
    len: usize,
    /// which ring entry is shown
    idx: usize,
}

/// The line being typed, the cursor in it (in characters), and the lines entered before.
#[derive(Default)]
pub struct Editor {
    pub buf: String,
    pub cursor: usize,
    history: Vec<String>,
    /// where Up and Down are in the history
    back: Option<usize>,
    /// Esc was pressed and the next key is its sequence (crossterm also reports Alt+key directly)
    esc: bool,
    /// Ctrl-V or Esc Q was pressed and the next key is inserted literally
    quote: bool,
    /// what Ctrl-K, Ctrl-U, Ctrl-X, Ctrl-W and Esc D deleted, newest first, at most ten
    ring: Vec<String>,
    /// the previous key, when it was a kill, so the next kill joins its entry
    last_kill: Option<KillDir>,
    /// a yank in place that Esc Y can rotate
    yank: Option<Yank>,
}

impl Editor {
    fn byte(&self, chars: usize) -> usize {
        self.buf
            .char_indices()
            .nth(chars)
            .map_or(self.buf.len(), |(i, _)| i)
    }

    fn len(&self) -> usize {
        self.buf.chars().count()
    }

    fn set(&mut self, text: String) {
        self.buf = text;
        self.cursor = self.len();
    }

    fn chars(&self) -> Vec<char> {
        self.buf.chars().collect()
    }

    /// Clears the state that lasts only until the next key (Esc prefix, quote, kill and yank).
    fn reset_transient(&mut self) {
        self.esc = false;
        self.quote = false;
        self.last_kill = None;
        self.yank = None;
    }

    pub fn key(&mut self, k: KeyEvent, s: &Session<'_>) -> Effect {
        // AltGr arrives from a Windows console as Ctrl and Alt together, and types its character
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let altgr = alt
            && k.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(k.code, KeyCode::Char(c) if !c.is_control());
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL) && !altgr;
        // the key after Ctrl-V or Esc Q is inserted as itself, `?` and Tab included
        if self.quote {
            self.quote = false;
            self.last_kill = None;
            self.yank = None;
            return match k.code {
                KeyCode::Char(c) => self.insert_char(c),
                KeyCode::Tab => self.insert_char('\t'),
                _ => Effect::Bell,
            };
        }
        // Esc or Alt with a key makes an Esc sequence; with Ctrl the key is its Ctrl key
        let meta = (self.esc || alt) && !ctrl && !altgr;
        if k.code == KeyCode::Esc && !meta {
            self.esc = true;
            return Effect::None;
        }
        self.esc = false;
        let prev_kill = self.last_kill;
        let prev_yank = self.yank;
        self.last_kill = None;
        self.yank = None;
        if meta {
            return match k.code {
                KeyCode::Char(c) => match c.to_ascii_lowercase() {
                    'b' => self.esc_b(),
                    'f' => self.esc_f(),
                    'd' => self.esc_d(prev_kill),
                    'c' => self.esc_case(Case::Capitalize),
                    'l' => self.esc_case(Case::Lower),
                    'u' => self.esc_case(Case::Upper),
                    'y' => self.yank_pop(prev_yank),
                    'q' => {
                        self.quote = true;
                        Effect::None
                    }
                    _ => Effect::Bell,
                },
                _ => Effect::Bell,
            };
        }
        // a banner's text is text: IOS takes `?` and Tab in it as typed
        let text = s.in_banner();
        match k.code {
            KeyCode::Char('?') if !ctrl && !text => {
                let at = self.byte(self.cursor);
                Effect::Help(s.help(&self.buf[..at]))
            }
            KeyCode::Tab if text => self.insert_char('\t'),
            KeyCode::Tab => {
                let at = self.byte(self.cursor);
                match s.complete(&self.buf[..at]) {
                    Some(done) => {
                        let rest = self.buf[at..].to_string();
                        let n = done.chars().count();
                        self.buf = done + &rest;
                        self.cursor = n;
                        Effect::Redraw
                    }
                    None => Effect::Bell,
                }
            }
            KeyCode::Enter => {
                let line = std::mem::take(&mut self.buf);
                self.cursor = 0;
                self.back = None;
                self.remember(&line);
                Effect::Enter(line)
            }
            KeyCode::Char('z') if ctrl => {
                self.set(String::new());
                self.back = None;
                Effect::End
            }
            KeyCode::Char('c') if ctrl => {
                self.set(String::new());
                self.back = None;
                Effect::Cancel
            }
            KeyCode::Char('l' | 'r') if ctrl => Effect::Redisplay,
            KeyCode::Char('v') if ctrl => {
                self.quote = true;
                Effect::None
            }
            KeyCode::Char('t') if ctrl => self.transpose(),
            KeyCode::Char('y') if ctrl => self.yank(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete_forward(),
            KeyCode::Char('d') if ctrl => self.delete_forward(),
            KeyCode::Left => self.move_left(),
            KeyCode::Char('b') if ctrl => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Char('f') if ctrl => self.move_right(),
            KeyCode::Home => self.cursor_home(),
            KeyCode::Char('a') if ctrl => self.cursor_home(),
            KeyCode::End => self.cursor_tail(),
            KeyCode::Char('e') if ctrl => self.cursor_tail(),
            KeyCode::Char('k') if ctrl => self.kill_to_end(prev_kill),
            KeyCode::Char('u' | 'x') if ctrl => self.kill_to_start(prev_kill),
            KeyCode::Char('w') if ctrl => self.kill_word_left(prev_kill),
            KeyCode::Up => self.hist_back(),
            KeyCode::Char('p') if ctrl => self.hist_back(),
            KeyCode::Down => self.hist_forward(),
            KeyCode::Char('n') if ctrl => self.hist_forward(),
            KeyCode::Char(c) if !ctrl => self.insert_char(c),
            _ => Effect::None,
        }
    }

    fn insert_char(&mut self, c: char) -> Effect {
        let at = self.byte(self.cursor);
        self.buf.insert(at, c);
        self.cursor += 1;
        Effect::Redraw
    }

    fn backspace(&mut self) -> Effect {
        if self.cursor == 0 {
            return Effect::Bell;
        }
        let at = self.byte(self.cursor);
        let from = self.byte(self.cursor - 1);
        self.buf.replace_range(from..at, "");
        self.cursor -= 1;
        Effect::Redraw
    }

    fn delete_forward(&mut self) -> Effect {
        if self.cursor >= self.len() {
            return Effect::Bell;
        }
        let at = self.byte(self.cursor);
        let to = self.byte(self.cursor + 1);
        self.buf.replace_range(at..to, "");
        Effect::Redraw
    }

    fn move_left(&mut self) -> Effect {
        if self.cursor == 0 {
            return Effect::Bell;
        }
        self.cursor -= 1;
        Effect::Redraw
    }

    fn move_right(&mut self) -> Effect {
        if self.cursor >= self.len() {
            return Effect::Bell;
        }
        self.cursor += 1;
        Effect::Redraw
    }

    fn cursor_home(&mut self) -> Effect {
        self.cursor = 0;
        Effect::Redraw
    }

    fn cursor_tail(&mut self) -> Effect {
        self.cursor = self.len();
        Effect::Redraw
    }

    /// Esc B: back to the start of the word on or before the cursor.
    fn esc_b(&mut self) -> Effect {
        let cs = self.chars();
        let p = word_start_left(&cs, self.cursor);
        if p == self.cursor {
            return Effect::Bell;
        }
        self.cursor = p;
        Effect::Redraw
    }

    /// Esc F: forward to the end of the next word.
    fn esc_f(&mut self) -> Effect {
        let cs = self.chars();
        let p = word_end_right(&cs, self.cursor);
        if p == self.cursor {
            return Effect::Bell;
        }
        self.cursor = p;
        Effect::Redraw
    }

    /// Esc D: delete from the cursor to the end of the next word, onto the ring.
    fn esc_d(&mut self, prev: Option<KillDir>) -> Effect {
        let cs = self.chars();
        let end = word_end_right(&cs, self.cursor);
        if end == self.cursor {
            return Effect::Bell;
        }
        let a = self.byte(self.cursor);
        let b = self.byte(end);
        let removed = self.buf[a..b].to_string();
        self.buf.replace_range(a..b, "");
        self.kill(removed, KillDir::Fwd, prev);
        Effect::Redraw
    }

    /// Esc C, Esc L, Esc U: capitalize, lowercase or uppercase from the cursor to the word's end.
    fn esc_case(&mut self, case: Case) -> Effect {
        let mut cs = self.chars();
        let n = cs.len();
        let start = skip_sep_right(&cs, self.cursor);
        if start >= n {
            return Effect::Bell;
        }
        let mut end = start;
        while end < n && is_word(cs[end]) {
            end += 1;
        }
        match case {
            Case::Capitalize => {
                cs[start] = cs[start].to_ascii_uppercase();
                for c in cs.iter_mut().take(end).skip(start + 1) {
                    *c = c.to_ascii_lowercase();
                }
            }
            Case::Lower => {
                for c in cs.iter_mut().take(end).skip(start) {
                    *c = c.to_ascii_lowercase();
                }
            }
            Case::Upper => {
                for c in cs.iter_mut().take(end).skip(start) {
                    *c = c.to_ascii_uppercase();
                }
            }
        }
        self.buf = cs.into_iter().collect();
        self.cursor = end;
        Effect::Redraw
    }

    /// Ctrl-T: swap the character before the cursor with the one at it, and step right; at the end of
    /// the line, swap the last two.
    fn transpose(&mut self) -> Effect {
        let n = self.len();
        if self.cursor == 0 || n < 2 {
            return Effect::Bell;
        }
        let i = if self.cursor >= n { n - 1 } else { self.cursor };
        let mut cs = self.chars();
        cs.swap(i - 1, i);
        self.buf = cs.into_iter().collect();
        self.cursor = (i + 1).min(n);
        Effect::Redraw
    }

    fn kill_to_end(&mut self, prev: Option<KillDir>) -> Effect {
        let at = self.byte(self.cursor);
        if at >= self.buf.len() {
            return Effect::Bell;
        }
        let removed = self.buf[at..].to_string();
        self.buf.truncate(at);
        self.kill(removed, KillDir::Fwd, prev);
        Effect::Redraw
    }

    fn kill_to_start(&mut self, prev: Option<KillDir>) -> Effect {
        let at = self.byte(self.cursor);
        if at == 0 {
            return Effect::Bell;
        }
        let removed = self.buf[..at].to_string();
        self.buf.replace_range(..at, "");
        self.cursor = 0;
        self.kill(removed, KillDir::Back, prev);
        Effect::Redraw
    }

    fn kill_word_left(&mut self, prev: Option<KillDir>) -> Effect {
        if self.cursor == 0 {
            return Effect::Bell;
        }
        let cs = self.chars();
        let start = word_start_left(&cs, self.cursor);
        let a = self.byte(start);
        let b = self.byte(self.cursor);
        let removed = self.buf[a..b].to_string();
        self.buf.replace_range(a..b, "");
        self.cursor = start;
        self.kill(removed, KillDir::Back, prev);
        Effect::Redraw
    }

    /// Puts deleted text on the ring: a new entry, or joined to the last when the previous key was
    /// also a kill, before it for a backward kill and after it for a forward one.
    fn kill(&mut self, removed: String, dir: KillDir, prev: Option<KillDir>) {
        if removed.is_empty() {
            return;
        }
        if prev.is_some() && !self.ring.is_empty() {
            match dir {
                KillDir::Back => self.ring[0].insert_str(0, &removed),
                KillDir::Fwd => self.ring[0].push_str(&removed),
            }
        } else {
            self.ring.insert(0, removed);
            self.ring.truncate(10);
        }
        self.last_kill = Some(dir);
    }

    /// Ctrl-Y: insert the most recent ring entry at the cursor.
    fn yank(&mut self) -> Effect {
        let Some(text) = self.ring.first().cloned() else {
            return Effect::Bell;
        };
        let at = self.byte(self.cursor);
        self.buf.insert_str(at, &text);
        let start = self.cursor;
        let len = text.chars().count();
        self.cursor = start + len;
        self.yank = Some(Yank { start, len, idx: 0 });
        Effect::Redraw
    }

    /// Esc Y: replace the text a Ctrl-Y (or an earlier Esc Y) just yanked with the next ring entry.
    fn yank_pop(&mut self, prev: Option<Yank>) -> Effect {
        let Some(y) = prev else {
            return Effect::Bell;
        };
        if self.ring.len() < 2 {
            self.yank = Some(y);
            return Effect::Bell;
        }
        let idx = (y.idx + 1) % self.ring.len();
        let text = self.ring[idx].clone();
        let a = self.byte(y.start);
        let b = self.byte(y.start + y.len);
        self.buf.replace_range(a..b, &text);
        let len = text.chars().count();
        self.cursor = y.start + len;
        self.yank = Some(Yank {
            start: y.start,
            len,
            idx,
        });
        Effect::Redraw
    }

    /// A line entered, kept for Up and Down unless it is blank or the one entered last.
    fn remember(&mut self, line: &str) {
        if !line.trim().is_empty() && self.history.last().map(String::as_str) != Some(line) {
            self.history.push(line.to_string());
        }
    }

    /// A paste: inserted at the cursor when it holds no line end; else the lines it enters, each
    /// kept for Up and Down as a typed line is, its last partial line left being typed.
    fn paste(&mut self, text: &str) -> Option<Vec<String>> {
        let at = self.byte(self.cursor);
        let Some((lines, pending)) = paste_split(&self.buf, at, text) else {
            let clean: String = text.chars().filter(|c| *c != '\r').collect();
            self.buf.insert_str(at, &clean);
            self.cursor += clean.chars().count();
            return None;
        };
        for l in &lines {
            self.remember(l);
        }
        self.set(pending);
        self.back = None;
        self.reset_transient();
        Some(lines)
    }

    fn hist_back(&mut self) -> Effect {
        let back = self.back.unwrap_or(self.history.len());
        if back == 0 {
            return Effect::Bell;
        }
        self.back = Some(back - 1);
        self.set(self.history[back - 1].clone());
        Effect::Redraw
    }

    fn hist_forward(&mut self) -> Effect {
        match self.back {
            Some(b) if b + 1 < self.history.len() => {
                self.back = Some(b + 1);
                self.set(self.history[b + 1].clone());
            }
            Some(_) => {
                self.back = None;
                self.set(String::new());
            }
            None => return Effect::Bell,
        }
        Effect::Redraw
    }
}

/// IOS editing treats a run of letters and digits as a word; everything else separates words.
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// Leftward from `pos`: past any separators, then past the word, giving the word's start.
fn word_start_left(chars: &[char], pos: usize) -> usize {
    let mut i = pos.min(chars.len());
    while i > 0 && !is_word(chars[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word(chars[i - 1]) {
        i -= 1;
    }
    i
}

/// Rightward from `pos`: past any separators to the next word's first character.
fn skip_sep_right(chars: &[char], pos: usize) -> usize {
    let mut i = pos.min(chars.len());
    while i < chars.len() && !is_word(chars[i]) {
        i += 1;
    }
    i
}

/// Rightward from `pos`: past any separators, then past the word, giving the word's end.
fn word_end_right(chars: &[char], pos: usize) -> usize {
    let mut i = skip_sep_right(chars, pos);
    while i < chars.len() && is_word(chars[i]) {
        i += 1;
    }
    i
}

/// Raw mode for as long as it lives, so every way out of a line restores the terminal.
struct Raw;

impl Raw {
    fn on() -> Result<Self, String> {
        terminal::enable_raw_mode().map_err(|e| e.to_string())?;
        Ok(Raw)
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// Bracketed paste for as long as it lives, so a pasted configuration arrives as one event.
struct Bracket;

impl Bracket {
    fn on() -> Self {
        let _ = execute!(std::io::stdout(), EnableBracketedPaste);
        Bracket
    }
}

impl Drop for Bracket {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    }
}

/// The prompt and the line, as much of it as fits: IOS shifts a long line and marks the side that
/// runs past the edge with `$`. The width is the terminal's, or `terminal width` when it was set.
fn draw(
    out: &mut impl Write,
    prompt: &str,
    ed: &Editor,
    width_setting: usize,
) -> std::io::Result<()> {
    let width = screen_width(width_setting);
    let chars: Vec<char> = ed.buf.chars().collect();
    let room = width.saturating_sub(prompt.chars().count() + 1).max(8);
    let (shown, col) = if chars.len() < room {
        (ed.buf.clone(), ed.cursor)
    } else {
        let start = ed.cursor.saturating_sub(room - 2);
        let end = (start + room).min(chars.len());
        let mut v: Vec<char> = chars[start..end].to_vec();
        if start > 0 {
            v[0] = '$';
        }
        if end < chars.len() {
            let n = v.len() - 1;
            v[n] = '$';
        }
        (v.into_iter().collect(), ed.cursor - start)
    };
    queue!(
        out,
        cursor::MoveToColumn(0),
        Print(prompt),
        Print(shown),
        terminal::Clear(terminal::ClearType::UntilNewLine),
        cursor::MoveToColumn((prompt.chars().count() + col) as u16)
    )?;
    out.flush()
}

/// What a line of input becomes: one line typed interactively (the prompt and text are already on
/// screen), or lines from a paste (not yet echoed).
enum Input {
    Typed(String),
    Pasted(Vec<String>),
}

/// The lines a paste enters and the partial line it leaves being typed: each newline enters a line, a
/// trailing partial stays. None when the paste holds no newline, so it is inserted like typing.
fn paste_split(buf: &str, at: usize, text: &str) -> Option<(Vec<String>, String)> {
    let clean: String = text.chars().filter(|c| *c != '\r').collect();
    let mut combined = buf.to_string();
    combined.insert_str(at, &clean);
    if !combined.contains('\n') {
        return None;
    }
    let mut parts: Vec<String> = combined.split('\n').map(str::to_string).collect();
    let pending = parts.pop().unwrap_or_default();
    Some((parts, pending))
}

/// The line "end" types when Ctrl-C leaves configuration mode; nothing to run at an EXEC prompt.
fn cancel_line(mode: &str) -> &'static str {
    if cli::is_config(mode) { "end" } else { "" }
}

/// Reads one line of input at the prompt on a terminal; Eof when input ends.
fn read(ed: &mut Editor, s: &mut Session<'_>) -> Result<Input, String> {
    let io = |e: std::io::Error| e.to_string();
    let mut out = std::io::stdout();
    let _raw = Raw::on()?;
    draw(&mut out, &s.prompt(), ed, s.width).map_err(io)?;
    // the line is drawn once no key is queued: a Windows console delivers a paste as keys, and a
    // draw per key made a 730-line paste take 6 s
    let mut stale = false;
    loop {
        if stale && !event::poll(std::time::Duration::ZERO).map_err(io)? {
            draw(&mut out, &s.prompt(), ed, s.width).map_err(io)?;
            stale = false;
        }
        match event::read().map_err(io)? {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                let (effect, typed) = take_key(ed, k, s);
                match effect {
                    Effect::None => {}
                    Effect::Redraw => stale = true,
                    effect => {
                        // whatever prints below the line shows it as typed first
                        if stale {
                            let now = std::mem::replace(&mut ed.buf, typed.0);
                            let at = std::mem::replace(&mut ed.cursor, typed.1);
                            draw(&mut out, &s.prompt(), ed, s.width).map_err(io)?;
                            (ed.buf, ed.cursor) = (now, at);
                            stale = false;
                        }
                        if let Some(input) = act(&mut out, effect, ed, s, &mut terminal_key)? {
                            return Ok(input);
                        }
                    }
                }
            }
            Event::Resize(cols, rows) => {
                if !s.width_set {
                    s.width = cols as usize;
                }
                if !s.length_set {
                    s.length = rows as usize;
                }
                draw(&mut out, &s.prompt(), ed, s.width).map_err(io)?;
                stale = false;
            }
            Event::Paste(t) => {
                stale = false;
                match ed.paste(&t) {
                    None => draw(&mut out, &s.prompt(), ed, s.width).map_err(io)?,
                    Some(lines) => {
                        // wipe the drawn line; interact echoes each pasted line as it is entered
                        queue!(
                            out,
                            cursor::MoveToColumn(0),
                            terminal::Clear(terminal::ClearType::UntilNewLine)
                        )
                        .map_err(io)?;
                        out.flush().map_err(io)?;
                        return Ok(Input::Pasted(lines));
                    }
                }
            }
            _ => {}
        }
    }
}

/// A key typed at the prompt, and the line as the key found it, which Enter, Ctrl-C and Ctrl-Z take
/// from the editor. IOS takes Ctrl-Z as `end` in configuration mode and ignores it at an EXEC prompt,
/// and a banner's text takes no `end` from it: there the line stays as typed.
fn take_key(ed: &mut Editor, k: KeyEvent, s: &Session<'_>) -> (Effect, (String, usize)) {
    let typed = (ed.buf.clone(), ed.cursor);
    match ed.key(k, s) {
        Effect::End if !cli::is_config(s.mode()) || s.in_banner() => {
            (ed.buf, ed.cursor) = typed.clone();
            (Effect::None, typed)
        }
        effect => (effect, typed),
    }
}

/// The next key pressed at the terminal.
fn terminal_key() -> Result<KeyEvent, String> {
    loop {
        if let Event::Key(k) = event::read().map_err(|e| e.to_string())?
            && k.kind == KeyEventKind::Press
        {
            return Ok(k);
        }
    }
}

/// What a key's effect prints, and the input it ends the line with, if any. `key` waits for a key
/// at `--More--`.
fn act(
    out: &mut impl Write,
    effect: Effect,
    ed: &mut Editor,
    s: &mut Session<'_>,
    key: &mut impl FnMut() -> Result<KeyEvent, String>,
) -> Result<Option<Input>, String> {
    let io = |e: std::io::Error| e.to_string();
    match effect {
        Effect::None | Effect::Redraw => {}
        Effect::Bell => {
            write!(out, "\x07").map_err(io)?;
            out.flush().map_err(io)?;
        }
        Effect::Redisplay => {
            write!(out, "\r\n").map_err(io)?;
            draw(out, &s.prompt(), ed, s.width).map_err(io)?;
        }
        // help pages as output does, a description too long for the terminal wrapped under its column
        Effect::Help(lines) => {
            write!(out, "?\r\n").map_err(io)?;
            page_out(out, fit_help(lines, screen_width(s.width)), s.length, key)?;
            draw(out, &s.prompt(), ed, s.width).map_err(io)?;
        }
        Effect::Enter(line) => {
            write!(out, "\r\n").map_err(io)?;
            return Ok(Some(Input::Typed(line)));
        }
        Effect::End => {
            write!(out, "^Z\r\n").map_err(io)?;
            return Ok(Some(Input::Typed("end".into())));
        }
        // a question being asked (`copy`'s destination) is dropped, never answered with its default;
        // a banner's text keeps the lines entered, and the line being typed goes
        Effect::Cancel => {
            write!(out, "\r\n").map_err(io)?;
            s.cancel();
            if s.in_banner() {
                draw(out, &s.prompt(), ed, s.width).map_err(io)?;
                return Ok(None);
            }
            return Ok(Some(Input::Typed(cancel_line(s.mode()).into())));
        }
    }
    Ok(None)
}

/// The width lines are fitted to: `terminal width` when it was set to a number, else the terminal's.
fn screen_width(setting: usize) -> usize {
    if setting == 0 || setting == usize::MAX {
        terminal::size().map_or(80, |(w, _)| w as usize)
    } else {
        setting
    }
}

/// Where a help line's description starts: after its indent, its keyword and two spaces or more.
fn help_column(line: &str) -> Option<usize> {
    let lead = line.len() - line.trim_start().len();
    if lead == 0 {
        return None;
    }
    let word = lead + line[lead..].find(' ')?;
    let gap = line[word..].len() - line[word..].trim_start().len();
    (gap >= 2 && word + gap < line.len()).then_some(word + gap)
}

/// Help as IOS prints it on a terminal `width` columns wide: a description goes on under its own
/// column, broken at a space short of the edge. A description the switch's help already broke, for
/// 80 columns, is joined first; what fits is left as it came.
fn fit_help(lines: Vec<String>, width: usize) -> Vec<String> {
    let most = width.saturating_sub(1);
    let mut out = Vec::new();
    // a description being gathered: its line as it came, its head up to the column, its text, and
    // whether a line under it was joined to it
    let mut entry: Option<(String, String, String, bool)> = None;
    for line in lines {
        let lead = line.len() - line.trim_start().len();
        if let Some((_, head, text, joined)) = entry.as_mut()
            && lead == head.len()
            && !line.trim().is_empty()
        {
            text.push(' ');
            text.push_str(line.trim());
            *joined = true;
            continue;
        }
        if let Some(e) = entry.take() {
            out.extend(under(e, most));
        }
        match help_column(&line) {
            Some(col) => {
                let (head, text) = (line[..col].to_string(), line[col..].to_string());
                entry = Some((line, head, text, false));
            }
            None => out.push(line),
        }
    }
    if let Some(e) = entry {
        out.extend(under(e, most));
    }
    out
}

/// A description under its head, as many words a line as stay within `most` columns.
fn under((line, head, text, joined): (String, String, String, bool), most: usize) -> Vec<String> {
    let col = head.chars().count();
    let fits = col + text.chars().count() <= most;
    if fits && !joined {
        return vec![line];
    }
    // a terminal too narrow for a word beside the column takes the description on one line
    if fits || col + 8 > most {
        return vec![format!("{head}{text}")];
    }
    let mut lines = vec![head];
    for w in text.split(' ').filter(|w| !w.is_empty()) {
        let n = lines.last().map_or(0, |l| l.chars().count());
        if n > col && n + 1 + w.chars().count() > most {
            lines.push(format!("{}{w}", " ".repeat(col)));
        } else if let Some(last) = lines.last_mut() {
            if n > col {
                last.push(' ');
            }
            last.push_str(w);
        }
    }
    lines
}

const MORE: &str = " --More-- ";
const MORE_HELP: &str =
    "Press RETURN for another line, SPACE for another page, anything else to quit";

/// The lines left to print and where in them paging is.
pub struct Pager {
    lines: Vec<String>,
    idx: usize,
    length: usize,
}

impl Pager {
    pub fn new(lines: Vec<String>, length: usize) -> Self {
        Pager {
            lines,
            idx: 0,
            length,
        }
    }

    pub fn done(&self) -> bool {
        self.idx >= self.lines.len()
    }

    /// The next page: up to `length` lines.
    pub fn page(&mut self) -> Vec<String> {
        let end = (self.idx + self.length.max(1)).min(self.lines.len());
        let out = self.lines[self.idx..end].to_vec();
        self.idx = end;
        out
    }

    /// The next single line.
    pub fn line(&mut self) -> Vec<String> {
        if self.done() {
            return vec![];
        }
        let out = vec![self.lines[self.idx].clone()];
        self.idx += 1;
        out
    }
}

/// What a key does at the `--More--` prompt.
#[derive(Debug, PartialEq)]
enum MoreKey {
    Page,
    Line,
    Help,
    Quit,
}

fn more_key(k: KeyEvent) -> MoreKey {
    match k.code {
        KeyCode::Char(' ') => MoreKey::Page,
        KeyCode::Enter => MoreKey::Line,
        KeyCode::Char('?') if !k.modifiers.contains(KeyModifiers::CONTROL) => MoreKey::Help,
        _ => MoreKey::Quit,
    }
}

/// The lines of output a page shows before `--More--`, which takes the page's last row as on IOS;
/// none when the output fits or paging is off (`terminal length 0`).
fn page_rows(lines: usize, length: usize) -> Option<usize> {
    (length != 0 && length != usize::MAX && lines >= length).then(|| (length - 1).max(1))
}

/// Writes lines in raw mode, pausing at `--More--` when they are longer than the page (`page_rows`):
/// Space a page, Return a line, `?` what the keys do, q or any other key stops. `key` waits for a key.
fn page_out(
    out: &mut impl Write,
    lines: Vec<String>,
    length: usize,
    key: &mut impl FnMut() -> Result<KeyEvent, String>,
) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    let rows = page_rows(lines.len(), length).unwrap_or(lines.len());
    let mut p = Pager::new(lines, rows);
    for l in p.page() {
        write!(out, "{l}\r\n").map_err(io)?;
    }
    while !p.done() {
        write!(out, "{MORE}").map_err(io)?;
        out.flush().map_err(io)?;
        let action = more_key(key()?);
        // erase the --More-- prompt, as IOS does before it prints on
        write!(out, "\r{}\r", " ".repeat(MORE.len())).map_err(io)?;
        match action {
            MoreKey::Page => {
                for l in p.page() {
                    write!(out, "{l}\r\n").map_err(io)?;
                }
            }
            MoreKey::Line => {
                for l in p.line() {
                    write!(out, "{l}\r\n").map_err(io)?;
                }
            }
            MoreKey::Help => write!(out, "{MORE_HELP}\r\n").map_err(io)?,
            MoreKey::Quit => break,
        }
    }
    out.flush().map_err(io)
}

/// Prints command output, paging at `--More--` when it is longer than the page: Space a page, Return
/// a line, q or any other key stops, `?` prints what the keys do. Paging off (`terminal length 0`)
/// prints it whole. Only the interactive console pages; pipes, files and scripts never do.
fn paged(lines: Vec<String>, length: usize) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    let mut out = std::io::stdout();
    if page_rows(lines.len(), length).is_none() {
        for l in &lines {
            writeln!(out, "{l}").map_err(io)?;
        }
        return out.flush().map_err(io);
    }
    let _raw = Raw::on()?;
    page_out(&mut out, lines, length, &mut terminal_key)
}

/// Lines from a pipe or a file: each typed at its prompt, a line ending in `?` printing help, except
/// in a banner's text, which takes `?` as typed.
fn plain(s: &mut Session<'_>) -> Result<(), String> {
    plain_from(s, std::io::stdin().lock(), std::io::stdout())
}

/// `plain` on any input and output.
fn plain_from(
    s: &mut Session<'_>,
    mut input: impl BufRead,
    mut out: impl Write,
) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    loop {
        write!(out, "{}", s.prompt()).map_err(io)?;
        out.flush().map_err(io)?;
        let mut line = String::new();
        if input.read_line(&mut line).map_err(io)? == 0 {
            return Ok(());
        }
        let line = line.trim_end_matches(['\r', '\n']);
        // help is fitted to `terminal width` as on a console (the lab C9300, 17.6.4, over the relay)
        let o = match line.strip_suffix('?').filter(|_| !s.in_banner()) {
            Some(before) if matches!(s.width, 0 | usize::MAX) => s.help(before),
            Some(before) => fit_help(s.help(before), s.width),
            None => s.enter(line).0,
        };
        for l in o {
            writeln!(out, "{l}").map_err(io)?;
        }
        if s.closed {
            return Ok(());
        }
    }
}

/// The console: a prompt, a line typed, the switch's answer, until `exit` leaves it or input ends.
pub fn interact(s: &mut Session<'_>) -> Result<(), String> {
    if !std::io::stdin().is_terminal() {
        return plain(s);
    }
    if let Ok((cols, rows)) = terminal::size() {
        if !s.width_set {
            s.width = cols as usize;
        }
        if !s.length_set {
            s.length = rows as usize;
        }
    }
    let _bracket = Bracket::on();
    let mut out = std::io::stdout();
    let mut ed = Editor::default();
    loop {
        match read(&mut ed, s)? {
            Input::Typed(line) => {
                let (o, _) = s.enter(&line);
                paged(o, s.length)?;
                if s.closed {
                    return Ok(());
                }
            }
            Input::Pasted(lines) => {
                for line in lines {
                    writeln!(out, "{}{}", s.prompt(), line).map_err(|e| e.to_string())?;
                    let (o, _) = s.enter(&line);
                    paged(o, s.length)?;
                    if s.closed {
                        return Ok(());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Engine;
    use crate::{device, grammars, platform, release};

    fn engine() -> Engine {
        Engine::new(grammars::grammar("c9300", release::DEFAULT).unwrap())
    }

    fn session(e: &Engine) -> Session<'_> {
        let st = platform::parse_stack("C9300-24T").unwrap();
        let mut s = Session::new(e, device::Device::new(st));
        s.type_line("enable");
        s
    }

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn a_page_leaves_its_last_row_to_more() {
        // 24 rows: 23 lines, then --More--; output that fits, or paging off, prints whole
        assert_eq!(page_rows(30, 24), Some(23));
        assert_eq!(page_rows(24, 24), Some(23));
        assert_eq!(page_rows(23, 24), None);
        assert_eq!(page_rows(500, 0), None);
        assert_eq!(page_rows(500, usize::MAX), None);
        assert_eq!(page_rows(5, 1), Some(1));
    }

    #[test]
    fn ctrl_c_at_copy_s_question_saves_nothing() {
        let e = engine();
        let mut s = session(&e);
        s.type_line("copy running-config startup-config");
        assert!(
            s.prompt().contains("Destination filename"),
            "{}",
            s.prompt()
        );
        s.cancel();
        // what the console then types at the EXEC prompt
        s.type_line(cancel_line(s.mode()));
        assert_eq!(s.startup, None);
        assert!(!s.prompt().contains("Destination"));
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn alt(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }

    fn feed(ed: &mut Editor, s: &Session<'_>, text: &str) {
        for c in text.chars() {
            ed.key(k(KeyCode::Char(c)), s);
        }
    }

    #[test]
    fn esc_f_and_esc_b_move_by_word_over_punctuation() {
        let e = engine();
        let s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "show spanning-tree vlan_10");
        // Esc B steps back over each run of letters and digits, punctuation separating them
        for (presses, cursor) in [(1, 24), (2, 19), (3, 14), (4, 5), (5, 0)] {
            ed.cursor = ed.len();
            for _ in 0..presses {
                ed.key(alt('b'), &s);
            }
            assert_eq!(ed.cursor, cursor, "{presses} Esc B");
        }
        ed.cursor = 0;
        ed.key(alt('f'), &s);
        assert_eq!(ed.cursor, 4, "Esc F to the end of show");
    }

    #[test]
    fn esc_d_deletes_the_next_word_onto_the_ring() {
        let e = engine();
        let s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "show interfaces status");
        ed.key(ctrl('a'), &s);
        ed.key(alt('f'), &s); // cursor after "show"
        assert_eq!(ed.key(alt('d'), &s), Effect::Redraw);
        assert_eq!(ed.buf, "show status");
        ed.key(ctrl('a'), &s); // a move ends the kill run, so the next kill is a new entry
        ed.key(alt('d'), &s);
        assert_eq!(ed.buf, " status");
        // what Esc D removed is on the ring
        ed.key(ctrl('y'), &s);
        assert_eq!(ed.buf, "show status");
    }

    #[test]
    fn esc_case_capitalizes_lowercases_and_uppercases() {
        let e = engine();
        let s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "show interfaces");
        ed.cursor = 0;
        ed.key(alt('c'), &s);
        assert_eq!(ed.buf, "Show interfaces");
        ed.key(alt('c'), &s);
        assert_eq!(ed.buf, "Show Interfaces");
        ed.set("show INTERFACES".into());
        ed.cursor = 5;
        ed.key(alt('l'), &s);
        assert_eq!(ed.buf, "show interfaces");
        ed.set("show interfaces".into());
        ed.cursor = 7;
        ed.key(alt('u'), &s);
        assert_eq!(ed.buf, "show inTERFACES");
    }

    #[test]
    fn ctrl_t_transposes_characters() {
        let e = engine();
        let s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "shwo");
        assert_eq!(ed.key(ctrl('t'), &s), Effect::Redraw);
        assert_eq!(ed.buf, "show");
        feed(&mut ed, &s, "");
        ed.set("shwo".into());
        ed.cursor = 3;
        ed.key(ctrl('t'), &s);
        assert_eq!(ed.buf, "show");
        // at the start of the line there is nothing to transpose
        ed.set("ab".into());
        ed.cursor = 0;
        assert_eq!(ed.key(ctrl('t'), &s), Effect::Bell);
        assert_eq!(ed.buf, "ab");
    }

    #[test]
    fn consecutive_word_kills_join_one_ring_entry() {
        let e = engine();
        let s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "show version");
        ed.key(ctrl('w'), &s);
        assert_eq!(ed.buf, "show ");
        ed.key(ctrl('w'), &s);
        assert_eq!(ed.buf, "");
        ed.key(ctrl('y'), &s);
        assert_eq!(ed.buf, "show version");
    }

    #[test]
    fn esc_y_rotates_through_the_ring() {
        let e = engine();
        let s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "one two three");
        ed.key(ctrl('w'), &s); // kill "three"
        ed.key(ctrl('a'), &s); // a move breaks the kill run
        ed.key(ctrl('k'), &s); // kill "one two "
        feed(&mut ed, &s, "four ");
        ed.key(ctrl('y'), &s);
        assert_eq!(ed.buf, "four one two ");
        ed.key(alt('y'), &s);
        assert_eq!(ed.buf, "four three");
        ed.key(alt('y'), &s);
        assert_eq!(ed.buf, "four one two ");
    }

    #[test]
    fn esc_y_without_a_yank_beeps() {
        let e = engine();
        let s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "show version");
        assert_eq!(ed.key(alt('y'), &s), Effect::Bell);
        assert_eq!(ed.buf, "show version");
    }

    #[test]
    fn an_impossible_move_or_delete_beeps() {
        let e = engine();
        let s = session(&e);
        let mut ed = Editor::default();
        assert_eq!(ed.key(k(KeyCode::Backspace), &s), Effect::Bell);
        assert_eq!(ed.key(k(KeyCode::Left), &s), Effect::Bell);
        assert_eq!(ed.key(ctrl('b'), &s), Effect::Bell);
        feed(&mut ed, &s, "ab");
        assert_eq!(ed.key(k(KeyCode::Right), &s), Effect::Bell);
        assert_eq!(ed.key(ctrl('f'), &s), Effect::Bell);
        assert_eq!(ed.key(ctrl('d'), &s), Effect::Bell);
    }

    #[test]
    fn ctrl_c_discards_and_ctrl_z_ends() {
        let e = engine();
        let mut s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "show ver");
        assert_eq!(ed.key(ctrl('c'), &s), Effect::Cancel);
        assert_eq!(ed.buf, "");
        // at an EXEC prompt the console takes Ctrl-Z as nothing, and the line stays as typed
        feed(&mut ed, &s, "show ver");
        assert_eq!(take_key(&mut ed, ctrl('z'), &s).0, Effect::None);
        assert_eq!(ed.buf, "show ver");
        ed.key(ctrl('u'), &s);
        s.type_line("configure terminal");
        feed(&mut ed, &s, "hostn");
        assert_eq!(take_key(&mut ed, ctrl('z'), &s).0, Effect::End);
        assert_eq!(ed.buf, "");
        // Ctrl-C runs nothing at an EXEC prompt, and `end` leaving config mode
        assert_eq!(cancel_line("privileged"), "");
        assert_eq!(cancel_line("config"), "end");
        assert_eq!(cancel_line("config-if"), "end");
    }

    #[test]
    fn ctrl_v_quotes_the_next_key() {
        let e = engine();
        let s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "show ");
        ed.key(ctrl('v'), &s);
        // the `?` is inserted, not taken as help
        assert_eq!(ed.key(k(KeyCode::Char('?')), &s), Effect::Redraw);
        assert_eq!(ed.buf, "show ?");
        ed.key(ctrl('v'), &s);
        ed.key(k(KeyCode::Tab), &s);
        assert_eq!(ed.buf, "show ?\t");
    }

    #[test]
    fn ctrl_l_and_ctrl_r_redisplay() {
        let e = engine();
        let s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "show ver");
        assert_eq!(ed.key(ctrl('l'), &s), Effect::Redisplay);
        assert_eq!(ed.key(ctrl('r'), &s), Effect::Redisplay);
        assert_eq!(ed.buf, "show ver");
    }

    #[test]
    fn a_paste_splits_into_lines_and_leaves_the_partial() {
        let (lines, pending) =
            paste_split("", 0, "conf t\nhostname X\nno shu").expect("has newlines");
        assert_eq!(lines, vec!["conf t", "hostname X"]);
        assert_eq!(pending, "no shu");
        // a trailing newline leaves nothing being typed
        let (lines, pending) = paste_split("", 0, "vlan 10\nname data\n").unwrap();
        assert_eq!(lines, vec!["vlan 10", "name data"]);
        assert_eq!(pending, "");
        // carriage returns are dropped, and the paste joins what was already typed
        let (lines, pending) = paste_split("vl", 2, "an 10\r\nname data").unwrap();
        assert_eq!(lines, vec!["vlan 10"]);
        assert_eq!(pending, "name data");
        // no newline is not a line to enter
        assert!(paste_split("", 0, "no newline here").is_none());
    }

    #[test]
    fn the_pager_pages_and_lines_through_output() {
        let lines: Vec<String> = (0..25).map(|n| format!("line {n}")).collect();
        let mut p = Pager::new(lines, 10);
        assert_eq!(p.page().len(), 10);
        assert!(!p.done());
        assert_eq!(p.page().len(), 10);
        assert_eq!(p.line(), vec!["line 20".to_string()]);
        assert!(!p.done());
        assert_eq!(p.page(), vec!["line 21", "line 22", "line 23", "line 24"]);
        assert!(p.done());
        assert!(p.page().is_empty());
    }

    /// What `act` prints for an effect, keys at `--More--` taken in turn from `keys`, then q.
    fn acted(
        effect: Effect,
        ed: &mut Editor,
        s: &mut Session<'_>,
        keys: &[KeyEvent],
    ) -> (String, Option<Input>, usize) {
        let mut out = Vec::new();
        let mut asked = 0;
        let mut next = || {
            asked += 1;
            Ok(keys
                .get(asked - 1)
                .copied()
                .unwrap_or(k(KeyCode::Char('q'))))
        };
        let input = act(&mut out, effect, ed, s, &mut next).unwrap();
        (String::from_utf8_lossy(&out).into_owned(), input, asked)
    }

    #[test]
    fn help_pages_at_more_as_output_does() {
        let e = engine();
        let mut s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "show ");
        let help = s.help("show ");
        assert!(help.len() > 100, "{}", help.len());
        let (out, input, asked) = acted(Effect::Help(help.clone()), &mut ed, &mut s, &[]);
        assert!(input.is_none());
        assert_eq!(asked, 1, "one --More--, then q");
        // a page of 24 rows: 23 lines, then --More--
        let shown: Vec<&str> = out.split("\r\n").collect();
        assert_eq!(shown[0], "?");
        assert_eq!(shown[1..24], help[..23]);
        assert!(shown[24].starts_with(MORE), "{:?}", shown[24]);
        assert!(!out.contains(&help[23]), "q stops");
        assert!(
            out.rsplit(MORE)
                .next()
                .is_some_and(|after| after.contains("Switch#show ")),
            "the line is drawn again: {out:?}"
        );
        // paging off prints it whole
        s.type_line("terminal length 0");
        let (out, _, asked) = acted(Effect::Help(help.clone()), &mut ed, &mut s, &[]);
        assert_eq!(asked, 0);
        assert!(out.contains(help.last().unwrap()) && !out.contains("More"));
    }

    #[test]
    fn a_help_description_goes_on_under_its_column() {
        let lines: Vec<String> = [
            "  hosts                       IP domain-name, lookup style, nameservers, and",
            "                              host table",
            "  html                        HTML helper commands",
            "",
        ]
        .map(String::from)
        .to_vec();
        // as the switch printed it at 80 columns
        assert_eq!(fit_help(lines.clone(), 80), lines);
        assert_eq!(
            fit_help(lines.clone(), 60),
            [
                "  hosts                       IP domain-name, lookup style,",
                "                              nameservers, and host table",
                "  html                        HTML helper commands",
                ""
            ]
        );
        assert_eq!(
            fit_help(lines.clone(), 132)[0],
            "  hosts                       IP domain-name, lookup style, nameservers, and host table"
        );
        // keywords across the line and a message are left as they came
        let across = vec!["access-lists  accounting".to_string(), String::new()];
        assert_eq!(fit_help(across.clone(), 20), across);
    }

    #[test]
    fn altgr_types_its_character_and_ctrl_after_esc_is_ctrl() {
        let e = engine();
        let s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "show running-config ");
        // `|` on a German keyboard: AltGr and <, which Windows reports as Ctrl and Alt
        let altgr = KeyEvent::new(
            KeyCode::Char('|'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert_eq!(ed.key(altgr, &s), Effect::Redraw);
        assert_eq!(ed.buf, "show running-config |");
        // Esc, then Ctrl-C: the line is discarded, not capitalized
        assert_eq!(ed.key(k(KeyCode::Esc), &s), Effect::None);
        assert_eq!(ed.key(ctrl('c'), &s), Effect::Cancel);
        assert_eq!(ed.buf, "");
        // Alt with a letter is still its Esc command
        feed(&mut ed, &s, "show ver");
        ed.key(alt('b'), &s);
        assert_eq!(ed.cursor, 5);
    }

    #[test]
    fn pasted_lines_reach_the_history() {
        let e = engine();
        let s = session(&e);
        let mut ed = Editor::default();
        feed(&mut ed, &s, "show clock");
        ed.key(k(KeyCode::Enter), &s);
        let lines = ed.paste("interface Gi1/0/1\nshutdown\nno shu").unwrap();
        assert_eq!(lines, ["interface Gi1/0/1", "shutdown"]);
        assert_eq!(ed.buf, "no shu");
        ed.key(ctrl('u'), &s);
        ed.key(k(KeyCode::Up), &s);
        assert_eq!(ed.buf, "shutdown");
        ed.key(k(KeyCode::Up), &s);
        assert_eq!(ed.buf, "interface Gi1/0/1");
        ed.key(k(KeyCode::Up), &s);
        assert_eq!(ed.buf, "show clock");
        // a paste with no line end is typed where the cursor is
        ed.key(ctrl('u'), &s);
        assert_eq!(ed.paste("vlan 10"), None);
        assert_eq!(ed.buf, "vlan 10");
    }

    #[test]
    fn a_banner_s_text_takes_question_marks_and_control_keys_end_nothing_in_it() {
        let e = engine();
        let mut s = session(&e);
        s.type_line("configure terminal");
        s.type_line("banner motd #");
        assert!(s.in_banner());
        let mut ed = Editor::default();
        feed(&mut ed, &s, "who is this?");
        assert_eq!(ed.buf, "who is this?");
        assert_eq!(ed.key(k(KeyCode::Tab), &s), Effect::Redraw);
        assert_eq!(ed.buf, "who is this?\t");
        // Ctrl-Z ends nothing and leaves the text; Ctrl-C drops the line being typed and types nothing
        assert_eq!(take_key(&mut ed, ctrl('z'), &s).0, Effect::None);
        assert_eq!(ed.buf, "who is this?\t");
        assert_eq!(ed.key(ctrl('c'), &s), Effect::Cancel);
        let (_, input, _) = acted(Effect::Cancel, &mut ed, &mut s, &[]);
        assert!(input.is_none());
        assert!(s.in_banner());
        s.type_line("hello?");
        s.type_line("#");
        s.type_line("end");
        let shown = s.type_line("show running-config").0.join("\n");
        // IOS prints a banner between `^C` delimiters, whatever delimiter was typed
        let banner = shown
            .split("banner motd ^C")
            .nth(1)
            .and_then(|b| b.split("^C").next())
            .unwrap_or_default()
            .to_string();
        assert_eq!(banner.trim(), "hello?", "{shown}");
    }

    #[test]
    fn a_piped_line_ending_in_a_question_mark_is_banner_text_while_a_banner_is_typed() {
        let e = engine();
        let mut s = session(&e);
        let input = "configure terminal\nbanner motd #\nwho is this?\n#\nend\nshow running-config | include who\n";
        let mut out = Vec::new();
        plain_from(&mut s, input.as_bytes(), &mut out).unwrap();
        let out = String::from_utf8_lossy(&out);
        assert!(out.contains("who is this?\n"), "{out}");
        assert!(!out.contains("% Unrecognized"), "{out}");
        // outside a banner it is still help
        let mut out = Vec::new();
        plain_from(&mut s, "show cl?\n".as_bytes(), &mut out).unwrap();
        assert!(String::from_utf8_lossy(&out).contains("class-map"));
    }

    #[test]
    fn enter_on_a_line_known_to_crash_the_switch_warns_and_help_does_not() {
        let e = engine();
        let mut s = session(&e);
        let mut out = Vec::new();
        plain_from(
            &mut s,
            "show ipv6 pim gir?\nshow ipv6 pim gir\n".as_bytes(),
            &mut out,
        )
        .unwrap();
        let out = String::from_utf8_lossy(&out);
        let warning = "% vswitch: this command crashed a Catalyst 9300 at IOS XE 17.6.4 (IOSd segfault and reload)\n";
        assert_eq!(out.matches(warning).count(), 1, "{out}");
        // after the switch's whole answer, before the next prompt
        assert!(
            out.ends_with(&format!(
                "% Invalid input detected at '^' marker.\n\n{warning}Switch#"
            )),
            "{out}"
        );
    }

    #[test]
    fn piped_help_is_fitted_to_the_terminal_width() {
        // the lab C9300 (17.6.4) broke `ip ssh ?` descriptions short of `terminal width 80`
        let e = engine();
        let lines = |width: usize| {
            let mut s = session(&e);
            let mut out = Vec::new();
            let input = format!("terminal width {width}\nshow ?\n");
            plain_from(&mut s, input.as_bytes(), &mut out).unwrap();
            String::from_utf8_lossy(&out).lines().count()
        };
        assert!(lines(40) > lines(200));
    }

    #[test]
    fn a_more_key_is_a_page_a_line_help_or_quit() {
        assert_eq!(more_key(k(KeyCode::Char(' '))), MoreKey::Page);
        assert_eq!(more_key(k(KeyCode::Enter)), MoreKey::Line);
        assert_eq!(more_key(k(KeyCode::Char('?'))), MoreKey::Help);
        assert_eq!(more_key(k(KeyCode::Char('q'))), MoreKey::Quit);
        assert_eq!(more_key(ctrl('c')), MoreKey::Quit);
    }
}
