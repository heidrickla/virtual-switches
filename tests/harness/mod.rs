//! Runs vswitch on a pseudo-terminal, types at it and reads the screen as a terminal draws it.
//! Windows gets a ConPTY, the console Windows Terminal hosts, with keys sent in the win32 input
//! mode the ConPTY asks for; elsewhere an openpty terminal with xterm's key sequences. A VT parser
//! keeps the screen as a grid of cells.

// each test crate uses part of the harness
#![allow(dead_code)]

pub mod shot;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long a wait lasts before it fails with the screen. Waits end as soon as the screen
/// matches, so this costs time only when a test fails.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// Lines kept above the screen; `line_no` counts them, so it stops rising past this many.
const SCROLLBACK: usize = 20_000;

/// Red, as the terminal draws ANSI colour 1.
pub const RED: vt100::Color = vt100::Color::Idx(1);

/// The binary under test: `VSWITCH_BIN` (a release build, to time it), else the one cargo built.
pub fn binary() -> PathBuf {
    std::env::var_os("VSWITCH_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_vswitch")))
}

/// The repository, where the tests' design and configuration files are.
pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Where screen dumps go: `VSWITCH_SCREENS`, else `target/tmp/screens`.
pub fn screens_dir() -> PathBuf {
    std::env::var_os("VSWITCH_SCREENS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("screens"))
}

static SEQ: AtomicUsize = AtomicUsize::new(0);
static DUMPS: AtomicUsize = AtomicUsize::new(0);

/// The switch list a spawn runs on (`VSWITCH_HOME`): a fresh folder in the temp folder, removed
/// when dropped unless `VSWITCH_HARNESS_KEEP` is set. A spawn refuses any other folder, so a test
/// never reads or writes the user's own list.
pub struct Home {
    dir: PathBuf,
    /// what is removed when dropped: the list's folder, or the harness folder it sits in
    root: PathBuf,
    owned: bool,
}

/// A fresh harness folder in the temp folder.
fn fresh() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "vswitch-tui-{}-{}-{stamp}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    dir
}

impl Home {
    pub fn new() -> Home {
        let dir = fresh();
        Home {
            root: dir.clone(),
            dir,
            owned: true,
        }
    }

    /// A list at `.vswitch` in a fresh harness folder that vswitch takes for the home folder, so the
    /// screens show it as they show a user's list (`~\.vswitch`).
    pub fn profiled() -> Home {
        let root = fresh();
        let dir = root.join(".vswitch");
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        Home {
            dir,
            root,
            owned: true,
        }
    }

    /// An existing folder, never removed: a kept harness folder to look at again. A spawn still
    /// refuses it unless it is a harness folder in the temp folder.
    pub fn at(dir: &Path) -> Home {
        Home {
            dir: dir.to_path_buf(),
            root: dir.to_path_buf(),
            owned: false,
        }
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// `switches.json`, empty when nothing was kept.
    pub fn switches(&self) -> String {
        std::fs::read_to_string(self.dir.join("switches.json")).unwrap_or_default()
    }

    /// The kept configuration of a switch, by its name.
    pub fn config(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join("configs").join(format!("{name}.cfg"))).ok()
    }

    /// Runs `vswitch <args>` on this list without a terminal, as a script would (`vswitch load`).
    pub fn run(&self, args: &[&str]) -> std::process::Output {
        let home = self.checked().unwrap_or_else(|e| panic!("{e}"));
        let mut cmd = std::process::Command::new(binary());
        cmd.args(args).current_dir(repo());
        for (k, v) in child_env(&home) {
            cmd.env(k, v);
        }
        cmd.output().expect("vswitch runs")
    }

    /// The folder to hand vswitch, once it is known to be a harness folder in the temp folder and
    /// not the user's list (`%USERPROFILE%\.vswitch`, or `vswitch` in the XDG data folder).
    pub fn checked(&self) -> Result<PathBuf, String> {
        let refuse = |why: String| {
            Err(format!(
                "refusing to run vswitch with VSWITCH_HOME={}: {why}",
                self.dir.display()
            ))
        };
        let Ok(dir) = self.dir.canonicalize() else {
            return refuse("it does not exist".into());
        };
        let tmp = std::env::temp_dir()
            .canonicalize()
            .map_err(|e| format!("the temp folder: {e}"))?;
        let harness = |p: &Path| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("vswitch-tui-"))
        };
        // a harness folder, or the `.vswitch` list in one (`profiled`)
        let named = harness(&dir)
            || dir.file_name().is_some_and(|n| n == ".vswitch")
                && dir.parent().is_some_and(harness);
        if !dir.starts_with(&tmp) || !named {
            return refuse(format!("not a harness folder in {}", tmp.display()));
        }
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let lists = [
            env("USERPROFILE").map(|p| p.join(".vswitch")),
            env("HOME").map(|p| p.join(".vswitch")),
            env("HOME").map(|p| p.join(".local").join("share").join("vswitch")),
            env("XDG_DATA_HOME").map(|p| p.join("vswitch")),
        ];
        for live in lists.into_iter().flatten() {
            let canonical = live.canonicalize().unwrap_or_else(|_| live.clone());
            if dir.starts_with(&canonical) || self.dir.starts_with(&live) {
                return refuse(format!("the user's list {}", live.display()));
            }
        }
        // the folder as created: a canonical path on Windows carries a `\\?\` prefix
        Ok(self.dir.clone())
    }
}

impl Default for Home {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        if self.owned && std::env::var_os("VSWITCH_HARNESS_KEEP").is_none() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// The environment vswitch runs in: the harness folder as its list, and the profile and data
/// folders it would fall back to inside it too, should `VSWITCH_HOME` ever be ignored.
fn child_env(home: &Path) -> Vec<(&'static str, PathBuf)> {
    // a `.vswitch` list has the harness folder it is in for a profile, as the user's has theirs
    let profile = match home.parent() {
        Some(p) if home.file_name().is_some_and(|n| n == ".vswitch") => p.to_path_buf(),
        _ => home.join("profile"),
    };
    let _ = std::fs::create_dir_all(&profile);
    vec![
        ("VSWITCH_HOME", home.to_path_buf()),
        ("USERPROFILE", profile.clone()),
        ("HOME", profile.clone()),
        ("XDG_DATA_HOME", profile),
    ]
}

/// A key as a user presses it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Enter,
    Tab,
    BackTab,
    Esc,
    Backspace,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
    Char(char),
    Ctrl(char),
    Alt(char),
}

const SHIFT: u32 = 0x10;
const CTRL: u32 = 0x08;
const ALT: u32 = 0x02;
const ENHANCED: u32 = 0x100;

impl Key {
    /// xterm's bytes for the key, which Windows Terminal also sends outside win32 input mode.
    pub fn vt(self, app_cursor: bool) -> Vec<u8> {
        let csi_or_ss3 = |c: u8| {
            if app_cursor {
                vec![0x1b, b'O', c]
            } else {
                vec![0x1b, b'[', c]
            }
        };
        match self {
            Key::Enter => b"\r".to_vec(),
            Key::Tab => b"\t".to_vec(),
            Key::BackTab => b"\x1b[Z".to_vec(),
            Key::Esc => b"\x1b".to_vec(),
            Key::Backspace => b"\x7f".to_vec(),
            Key::Delete => b"\x1b[3~".to_vec(),
            Key::Insert => b"\x1b[2~".to_vec(),
            Key::Up => csi_or_ss3(b'A'),
            Key::Down => csi_or_ss3(b'B'),
            Key::Right => csi_or_ss3(b'C'),
            Key::Left => csi_or_ss3(b'D'),
            Key::Home => csi_or_ss3(b'H'),
            Key::End => csi_or_ss3(b'F'),
            Key::PageUp => b"\x1b[5~".to_vec(),
            Key::PageDown => b"\x1b[6~".to_vec(),
            Key::F(n @ 1..=4) => vec![0x1b, b'O', b'P' + n - 1],
            Key::F(n) => {
                let code = [15, 17, 18, 19, 20, 21, 23, 24][usize::from(n.clamp(5, 12)) - 5];
                format!("\x1b[{code}~").into_bytes()
            }
            Key::Char(c) => c.to_string().into_bytes(),
            Key::Ctrl(c) => vec![(c.to_ascii_lowercase() as u8) & 0x1f],
            Key::Alt(c) => {
                let mut v = vec![0x1b];
                v.extend(c.to_string().bytes());
                v
            }
        }
    }

    /// The key as Windows Terminal sends it once the ConPTY asks for win32 input mode
    /// (`ESC [ ? 9001 h`): a key-down and a key-up record, `ESC [ Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`,
    /// on a US keyboard layout.
    pub fn win32(self) -> Vec<u8> {
        let (vk, sc, uc, cs) = match self {
            Key::Enter => (0x0d, 0x1c, 13, 0),
            Key::Tab => (0x09, 0x0f, 9, 0),
            Key::BackTab => (0x09, 0x0f, 9, SHIFT),
            Key::Esc => (0x1b, 0x01, 27, 0),
            Key::Backspace => (0x08, 0x0e, 8, 0),
            Key::Delete => (0x2e, 0x53, 0, ENHANCED),
            Key::Insert => (0x2d, 0x52, 0, ENHANCED),
            Key::Up => (0x26, 0x48, 0, ENHANCED),
            Key::Down => (0x28, 0x50, 0, ENHANCED),
            Key::Left => (0x25, 0x4b, 0, ENHANCED),
            Key::Right => (0x27, 0x4d, 0, ENHANCED),
            Key::Home => (0x24, 0x47, 0, ENHANCED),
            Key::End => (0x23, 0x4f, 0, ENHANCED),
            Key::PageUp => (0x21, 0x49, 0, ENHANCED),
            Key::PageDown => (0x22, 0x51, 0, ENHANCED),
            Key::F(n) => {
                let n = u32::from(n.clamp(1, 12));
                let sc = match n {
                    11 => 0x57,
                    12 => 0x58,
                    n => 0x3a + n,
                };
                (0x6f + n, sc, 0, 0)
            }
            Key::Char(c) => {
                let (vk, sc, shift) = us_key(c);
                (vk, sc, c as u32, if shift { SHIFT } else { 0 })
            }
            Key::Ctrl(c) => {
                let (vk, sc, _) = us_key(c.to_ascii_lowercase());
                (vk, sc, (c.to_ascii_lowercase() as u32) & 0x1f, CTRL)
            }
            Key::Alt(c) => {
                let (vk, sc, shift) = us_key(c);
                (vk, sc, c as u32, ALT | if shift { SHIFT } else { 0 })
            }
        };
        // a character outside the basic plane is two UTF-16 units, a record each
        let mut buf = [0u16; 2];
        let units: Vec<u32> = match char::from_u32(uc) {
            Some(ch) if uc > 0xffff => ch
                .encode_utf16(&mut buf)
                .iter()
                .map(|u| *u as u32)
                .collect(),
            _ => vec![uc],
        };
        let mut out = Vec::new();
        for down in [1, 0] {
            for u in &units {
                out.extend(format!("\x1b[{vk};{sc};{u};{down};{cs};1_").bytes());
            }
        }
        out
    }
}

/// The virtual-key code, scan code and shift state of a character on a US keyboard; 0 for a
/// character it does not have, which the console takes by its character alone.
fn us_key(c: char) -> (u32, u32, bool) {
    const ROWS: [(&str, &str, u32); 4] = [
        ("1234567890-=", "!@#$%^&*()_+", 0x02),
        ("qwertyuiop[]", "QWERTYUIOP{}", 0x10),
        ("asdfghjkl;'`", "ASDFGHJKL:\"~", 0x1e),
        ("\\zxcvbnm,./", "|ZXCVBNM<>?", 0x2b),
    ];
    if c == ' ' {
        return (0x20, 0x39, false);
    }
    for (plain, shifted, first) in ROWS {
        for (i, (p, s)) in plain.chars().zip(shifted.chars()).enumerate() {
            if c == p || c == s {
                let vk = match p {
                    'a'..='z' => p.to_ascii_uppercase() as u32,
                    '0'..='9' => p as u32,
                    '-' => 0xbd,
                    '=' => 0xbb,
                    '[' => 0xdb,
                    ']' => 0xdd,
                    ';' => 0xba,
                    '\'' => 0xde,
                    '`' => 0xc0,
                    '\\' => 0xdc,
                    ',' => 0xbc,
                    '.' => 0xbe,
                    '/' => 0xbf,
                    _ => 0,
                };
                return (vk, first + i as u32, c == s);
            }
        }
    }
    (0, 0, false)
}

/// What the screen asked of the terminal besides drawing: replies the terminal owes, bells, and
/// whether win32 input mode is on.
#[derive(Default)]
struct Seen {
    replies: Vec<u8>,
    bells: usize,
    win32_input: bool,
}

impl vt100::Callbacks for Seen {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bells += 1;
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied();
        match (i1, c, first) {
            // cursor position report, which a ConPTY asks for before it draws anything
            (None, 'n', Some(6)) => {
                let (row, col) = screen.cursor_position();
                self.replies
                    .extend(format!("\x1b[{};{}R", row + 1, col + 1).bytes());
            }
            (None, 'n', Some(5)) => self.replies.extend(b"\x1b[0n"),
            // primary device attributes: a VT220 with ANSI colour
            (None, 'c', None | Some(0)) => self.replies.extend(b"\x1b[?62;22c"),
            (Some(b'?'), 'h', Some(9001)) => self.win32_input = true,
            (Some(b'?'), 'l', Some(9001)) => self.win32_input = false,
            _ => {}
        }
    }
}

struct State {
    parser: vt100::Parser<Seen>,
    /// everything the program wrote, as the terminal received it
    log: Vec<u8>,
    exit: Option<u32>,
    /// the terminal's output has ended
    closed: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// How keys are encoded: as the ConPTY asks (win32 input mode once it has asked for it), or
/// always as xterm sequences.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Keys {
    AsAsked,
    Vt,
}

/// Options for a spawn beyond its arguments and size.
pub struct Spawn<'a> {
    home: &'a Home,
    cwd: PathBuf,
    name: String,
    probe: bool,
}

impl<'a> Spawn<'a> {
    pub fn new(home: &'a Home) -> Self {
        Spawn {
            home,
            cwd: repo(),
            name: test_name(),
            probe: false,
        }
    }

    /// The name screen dumps are filed under; the test's own by default.
    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// The folder relative paths typed into vswitch start from; the repository by default.
    pub fn cwd(mut self, dir: &Path) -> Self {
        self.cwd = dir.to_path_buf();
        self
    }

    /// Runs vswitch under this test binary acting as the shell (`probe`), which prints the
    /// terminal mode before and after vswitch.
    pub fn probed(mut self) -> Self {
        self.probe = true;
        self
    }

    pub fn run(self, args: &[&str], cols: u16, rows: u16) -> Term {
        self.try_run(args, cols, rows)
            .unwrap_or_else(|e| panic!("{e}"))
    }

    /// The spawn, or why it was refused before any terminal or process existed.
    pub fn try_run(self, args: &[&str], cols: u16, rows: u16) -> Result<Term, String> {
        let home = self.home.checked()?;
        Ok(Term::start(self, &home, args, cols, rows))
    }
}

/// The test's name, from the thread libtest runs it on: `at_80x24-the_list_starts_empty`.
fn test_name() -> String {
    std::thread::current()
        .name()
        .unwrap_or("vswitch")
        .replace("::", "-")
}

/// vswitch on a pseudo-terminal of a given size.
pub struct Term {
    name: String,
    args: Vec<String>,
    master: Option<Box<dyn MasterPty + Send>>,
    input: Option<Sender<Vec<u8>>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    pub keys: Keys,
}

impl Term {
    /// vswitch with these arguments on a `cols` x `rows` terminal and a fresh switch list. The list
    /// lives as long as the returned `Home`.
    pub fn spawn(args: &[&str], cols: u16, rows: u16) -> (Term, Home) {
        let home = Home::new();
        let t = Spawn::new(&home).run(args, cols, rows);
        (t, home)
    }

    /// vswitch on the switch list in `home`, which outlives it.
    pub fn spawn_in(home: &Home, args: &[&str], cols: u16, rows: u16) -> Term {
        Spawn::new(home).run(args, cols, rows)
    }

    fn start(opts: Spawn<'_>, home: &Path, args: &[&str], cols: u16, rows: u16) -> Term {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("a pseudo-terminal");
        let mut cmd = if opts.probe {
            let mut c = CommandBuilder::new(std::env::current_exe().expect("the test binary"));
            for a in ["--exact", "probe", "--nocapture", "--test-threads=1", "-q"] {
                c.arg(a);
            }
            c.env("VSWITCH_HARNESS_PROBE", args.join("\u{1f}"));
            c
        } else {
            let mut c = CommandBuilder::new(binary());
            c.args(args);
            c
        };
        cmd.cwd(&opts.cwd);
        for (k, v) in child_env(home) {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env_remove("NO_COLOR");
        let mut child = pty.slave.spawn_command(cmd).expect("vswitch starts");
        drop(pty.slave);
        let killer = child.clone_killer();
        let mut reader = pty.master.try_clone_reader().expect("pty reader");
        let mut writer = pty.master.take_writer().expect("pty writer");
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                parser: vt100::Parser::new_with_callbacks(rows, cols, SCROLLBACK, Seen::default()),
                log: Vec::new(),
                exit: None,
                closed: false,
            }),
            changed: Condvar::new(),
        });
        let (tx, rx) = channel::<Vec<u8>>();
        let mut threads = Vec::new();
        // one writer, so keys and the replies the screen asks for never interleave mid-sequence
        threads.push(std::thread::spawn(move || {
            for bytes in rx {
                if writer
                    .write_all(&bytes)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        }));
        let (s, replies) = (shared.clone(), tx.clone());
        threads.push(std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let owed = {
                    let mut st = s.state.lock().unwrap_or_else(|e| e.into_inner());
                    st.parser.process(&buf[..n]);
                    st.log.extend_from_slice(&buf[..n]);
                    std::mem::take(&mut st.parser.callbacks_mut().replies)
                };
                if !owed.is_empty() {
                    let _ = replies.send(owed);
                }
                s.changed.notify_all();
            }
            s.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
            s.changed.notify_all();
        }));
        let s = shared.clone();
        threads.push(std::thread::spawn(move || {
            let code = child.wait().map_or(u32::MAX, |st| st.exit_code());
            s.state.lock().unwrap_or_else(|e| e.into_inner()).exit = Some(code);
            s.changed.notify_all();
        }));
        Term {
            name: opts.name,
            args: args.iter().map(|a| a.to_string()).collect(),
            master: Some(pty.master),
            input: Some(tx),
            killer,
            shared,
            threads,
            keys: Keys::AsAsked,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Waits until `until` holds of the state or `timeout` passes; whether it held.
    fn wait_state(&self, timeout: Duration, mut until: impl FnMut(&mut State) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        let mut st = self.lock();
        loop {
            if until(&mut st) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            st = self
                .shared
                .changed
                .wait_timeout(st, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// Why a wait stopped short: the program ended, or time ran out.
    fn short(&self, timeout: Duration) -> String {
        let st = self.lock();
        if st.closed {
            format!("the terminal closed first, exit code {:?}", st.exit)
        } else {
            format!("not within {} s", timeout.as_secs_f32())
        }
    }

    // typing

    /// Bytes as typed: text, or escape sequences built by hand.
    pub fn send_bytes(&mut self, bytes: &[u8]) {
        if let Some(tx) = &self.input {
            let _ = tx.send(bytes.to_vec());
        }
    }

    /// Presses one key.
    pub fn press(&mut self, key: Key) -> &mut Self {
        let (win32, app_cursor) = {
            let st = self.lock();
            (
                st.parser.callbacks().win32_input,
                st.parser.screen().application_cursor(),
            )
        };
        let bytes = if win32 && self.keys == Keys::AsAsked {
            key.win32()
        } else {
            key.vt(app_cursor)
        };
        self.send_bytes(&bytes);
        self
    }

    /// Presses a key `n` times.
    pub fn press_n(&mut self, key: Key, n: usize) -> &mut Self {
        for _ in 0..n {
            self.press(key);
        }
        self
    }

    /// Types text a key at a time; `\n` is Enter.
    pub fn type_text(&mut self, text: &str) -> &mut Self {
        for c in text.chars() {
            self.press(if c == '\n' { Key::Enter } else { Key::Char(c) });
        }
        self
    }

    /// Types a line and presses Enter.
    pub fn line(&mut self, text: &str) -> &mut Self {
        self.type_text(text).press(Key::Enter)
    }

    /// Pastes text as Windows Terminal does: line ends become CR, and the text is wrapped in
    /// bracketed-paste marks when the program turned that mode on.
    pub fn paste(&mut self, text: &str) -> &mut Self {
        let bracketed = self.lock().parser.screen().bracketed_paste();
        self.paste_as(text, bracketed)
    }

    /// Pastes text with or without bracketed-paste marks, whatever the program asked for.
    pub fn paste_as(&mut self, text: &str, bracketed: bool) -> &mut Self {
        let body = text.replace("\r\n", "\r").replace('\n', "\r");
        let mut bytes = Vec::with_capacity(body.len() + 12);
        if bracketed {
            bytes.extend(b"\x1b[200~");
        }
        bytes.extend(body.bytes());
        if bracketed {
            bytes.extend(b"\x1b[201~");
        }
        self.send_bytes(&bytes);
        self
    }

    /// Resizes the terminal, as dragging the window's edge does.
    pub fn resize(&mut self, cols: u16, rows: u16) -> &mut Self {
        self.lock().parser.screen_mut().set_size(rows, cols);
        if let Some(m) = &self.master {
            m.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize");
        }
        self
    }

    // reading

    /// The screen's text, a line per row, trailing blanks trimmed.
    pub fn screen(&self) -> String {
        text(self.lock().parser.screen())
    }

    /// The screen's text with a row the terminal wrapped joined to the next, for reading lines
    /// longer than the screen is wide.
    pub fn contents(&self) -> String {
        self.lock().parser.screen().contents()
    }

    /// The text drawn in a foreground colour, one line per run of cells: `colored(RED)`.
    pub fn colored(&self, fg: vt100::Color) -> String {
        let st = self.lock();
        let s = st.parser.screen();
        let (rows, cols) = s.size();
        let mut runs = Vec::new();
        for r in 0..rows {
            let mut run = String::new();
            for c in 0..cols {
                match s.cell(r, c) {
                    Some(cell) if cell.fgcolor() == fg && cell.has_contents() => {
                        run.push_str(cell.contents())
                    }
                    Some(cell) if cell.fgcolor() == fg => run.push(' '),
                    _ if !run.trim().is_empty() => runs.push(std::mem::take(&mut run)),
                    _ => run.clear(),
                }
            }
            if !run.trim().is_empty() {
                runs.push(run);
            }
        }
        runs.iter()
            .map(|r| r.trim().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Reads anything else off the screen: attributes, modes, single cells.
    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        f(self.lock().parser.screen())
    }

    /// The cursor's row and column, from 0.
    pub fn cursor(&self) -> (u16, u16) {
        self.lock().parser.screen().cursor_position()
    }

    pub fn cursor_visible(&self) -> bool {
        !self.lock().parser.screen().hide_cursor()
    }

    /// The cursor's row up to the cursor: at a console prompt, the prompt and the line typed.
    pub fn cursor_line(&self) -> String {
        before_cursor(self.lock().parser.screen())
    }

    /// The cursor's line counted from the first line of the main screen, scrolled-off lines
    /// included: a mark that later output only moves down.
    pub fn line_no(&self) -> usize {
        line_no(self.lock().parser.screen_mut())
    }

    pub fn alternate_screen(&self) -> bool {
        self.lock().parser.screen().alternate_screen()
    }

    /// Bells rung so far.
    pub fn bells(&self) -> usize {
        self.lock().parser.callbacks().bells
    }

    /// Everything the program wrote, escape sequences and all.
    pub fn output(&self) -> Vec<u8> {
        self.lock().log.clone()
    }

    pub fn output_len(&self) -> usize {
        self.lock().log.len()
    }

    /// The exit code once the program has ended.
    pub fn exit_code(&self) -> Option<u32> {
        self.lock().exit
    }

    // waiting

    /// Waits until the condition holds of the screen; fails with the screen when it does not in
    /// time or the program ends first.
    #[track_caller]
    pub fn wait_until(&mut self, what: &str, cond: impl Fn(&vt100::Screen) -> bool) -> &mut Self {
        if let Err(e) = self.try_wait_until(TIMEOUT, cond) {
            self.fail(&format!("waited for {what}: {e}"));
        }
        self
    }

    /// Waits up to `timeout`; says why it stopped short.
    pub fn try_wait_until(
        &self,
        timeout: Duration,
        cond: impl Fn(&vt100::Screen) -> bool,
    ) -> Result<(), String> {
        let mut held = false;
        self.wait_state(timeout, |st| {
            held = cond(st.parser.screen());
            held || st.closed
        });
        if held {
            Ok(())
        } else {
            Err(self.short(timeout))
        }
    }

    /// Waits until the screen shows `text`.
    #[track_caller]
    pub fn wait_for(&mut self, text: &str) -> &mut Self {
        let want = text.to_string();
        self.wait_until(&format!("{text:?} on the screen"), move |s| {
            s.contents().contains(&want)
        })
    }

    /// Waits until `text` is no longer on the screen.
    #[track_caller]
    pub fn wait_gone(&mut self, text: &str) -> &mut Self {
        let want = text.to_string();
        self.wait_until(&format!("{text:?} gone from the screen"), move |s| {
            !s.contents().contains(&want)
        })
    }

    /// Waits until a full-screen program is drawn: the alternate screen, with text on it.
    #[track_caller]
    pub fn wait_fullscreen(&mut self) -> &mut Self {
        self.wait_until("a full-screen program on the alternate screen", |s| {
            s.alternate_screen() && !s.contents().trim().is_empty()
        })
    }

    /// Waits for a prompt at the cursor with nothing typed after it.
    #[track_caller]
    pub fn wait_prompt(&mut self, prompt: &str) -> &mut Self {
        self.wait_prompt_after(None, prompt)
    }

    /// Waits for `prompt` (with what is typed after it) at the cursor on a line below `after`:
    /// the prompt a key brings, never one already on the screen.
    #[track_caller]
    pub fn wait_prompt_after(&mut self, after: Option<usize>, prompt: &str) -> &mut Self {
        let mut held = false;
        self.wait_state(TIMEOUT, |st| {
            let s = st.parser.screen_mut();
            held = after.is_none_or(|a| line_no(s) > a) && before_cursor(s) == prompt;
            held || st.closed
        });
        if !held {
            let why = self.short(TIMEOUT);
            self.fail(&format!(
                "waited for the prompt {prompt:?} at the cursor: {why}"
            ));
        }
        self
    }

    /// Presses a key and waits for `prompt` on a later line: Enter on a typed line, or `?`.
    #[track_caller]
    pub fn press_for(&mut self, key: Key, prompt: &str) -> &mut Self {
        let mark = self.line_no();
        self.press(key);
        self.wait_prompt_after(Some(mark), prompt)
    }

    /// Types a command, presses Enter and waits for the prompt after it.
    #[track_caller]
    pub fn command(&mut self, text: &str, prompt: &str) -> &mut Self {
        self.type_text(text);
        self.press_for(Key::Enter, prompt)
    }

    /// Waits until the program writes `text` after output position `from` (`output_len` taken
    /// before the keys that bring it): for text the screen may still show from before.
    #[track_caller]
    pub fn wait_written(&mut self, from: usize, text: &str) -> &mut Self {
        let needle = text.as_bytes();
        let mut scanned = from;
        let mut found = false;
        self.wait_state(TIMEOUT, |st| {
            // what arrived since the last look, and the overlap a needle split across reads needs
            let start = scanned
                .saturating_sub(needle.len())
                .max(from)
                .min(st.log.len());
            found = st.log[start..].windows(needle.len()).any(|w| w == needle);
            scanned = st.log.len();
            found || st.closed
        });
        if !found {
            let why = self.short(TIMEOUT);
            self.fail(&format!("waited for the program to write {text:?}: {why}"));
        }
        self
    }

    /// Waits until the program has written nothing for `quiet`: for looking at a screen whose
    /// content is not known in advance. Tests wait on content instead.
    pub fn settle(&mut self, quiet: Duration) -> &mut Self {
        let deadline = Instant::now() + TIMEOUT;
        let mut st = self.lock();
        let (mut len, mut since) = (st.log.len(), Instant::now());
        // woken by output, or when the quiet spell would be over
        while !st.closed && since.elapsed() < quiet && Instant::now() < deadline {
            let left = quiet.saturating_sub(since.elapsed());
            st = self
                .shared
                .changed
                .wait_timeout(st, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
            if st.log.len() != len {
                (len, since) = (st.log.len(), Instant::now());
            }
        }
        drop(st);
        self
    }

    /// Waits for the program to end and for the terminal to pass on all it wrote; its exit code.
    #[track_caller]
    pub fn wait_exit(&mut self) -> u32 {
        self.wait_state(TIMEOUT, |st| st.exit.is_some());
        let Some(code) = self.exit_code() else {
            self.fail("waited for the program to exit");
        };
        self.close();
        code
    }

    /// Closes the terminal and waits for its last output.
    fn close(&mut self) {
        self.input.take();
        self.master.take();
        self.wait_state(Duration::from_secs(10), |st| st.closed);
    }

    // reporting

    /// The screen in a frame, with its size, cursor and modes, as dumps and failures show it.
    pub fn framed(&self) -> String {
        let st = self.lock();
        let s = st.parser.screen();
        let (rows, cols) = s.size();
        let (r, c) = s.cursor_position();
        let mut out = format!(
            "{} {}  {cols}x{rows}  cursor {r},{c} {}  {}  bells {}  exit {}\n",
            self.name,
            self.args.join(" "),
            if s.hide_cursor() { "hidden" } else { "shown" },
            if s.alternate_screen() {
                "alternate screen"
            } else {
                "main screen"
            },
            st.parser.callbacks().bells,
            st.exit.map_or("-".to_string(), |c| c.to_string()),
        );
        let edge = format!("+{}+\n", "-".repeat(usize::from(cols)));
        out.push_str(&edge);
        for row in grid(s) {
            out.push('|');
            out.push_str(&row);
            out.push_str("|\n");
        }
        out.push_str(&edge);
        out
    }

    /// Writes the screen to `<screens>/<test>/<n>-<label>.txt`, numbered in order across the
    /// process's spawns, and returns the path.
    pub fn dump(&mut self, label: &str) -> PathBuf {
        let n = DUMPS.fetch_add(1, Ordering::Relaxed) + 1;
        let dir = screens_dir().join(&self.name);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{n:02}-{label}.txt"));
        let _ = std::fs::write(&path, self.framed());
        path
    }

    /// Writes the screen as Windows Terminal draws it to `png`, through the HTML page `html`.
    pub fn shot(&self, html: &Path, png: &Path) {
        let (page, size) = {
            let st = self.lock();
            let s = st.parser.screen();
            let (rows, cols) = s.size();
            (shot::page(s), shot::size(cols, rows))
        };
        std::fs::write(html, page).unwrap_or_else(|e| panic!("{}: {e}", html.display()));
        shot::photograph(html, png, size);
    }

    /// Fails the test with the screen as it stands, and writes it to a file.
    #[track_caller]
    pub fn fail(&mut self, why: &str) -> ! {
        let path = self.dump("failed");
        panic!(
            "{why}\n{}screen written to {}",
            self.framed(),
            path.display()
        );
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = self.killer.kill();
        self.wait_state(Duration::from_secs(5), |st| st.exit.is_some());
        self.close();
        // a thread still blocked here is left to end with the process
        for t in std::mem::take(&mut self.threads) {
            if t.is_finished() {
                let _ = t.join();
            }
        }
    }
}

/// The screen's text, a line per row, trailing blanks trimmed.
pub fn text(s: &vt100::Screen) -> String {
    grid(s)
        .iter()
        .map(|r| r.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Each row as its cells show it, blanks as spaces.
fn grid(s: &vt100::Screen) -> Vec<String> {
    let (rows, cols) = s.size();
    (0..rows)
        .map(|r| {
            let mut line = String::new();
            for c in 0..cols {
                match s.cell(r, c) {
                    Some(cell) if cell.is_wide_continuation() => {}
                    Some(cell) if cell.has_contents() => line.push_str(cell.contents()),
                    _ => line.push(' '),
                }
            }
            line
        })
        .collect()
}

/// Lines scrolled off the top, which the parser keeps, and the cursor's row below them.
fn line_no(s: &mut vt100::Screen) -> usize {
    s.set_scrollback(usize::MAX);
    let above = s.scrollback();
    s.set_scrollback(0);
    above + usize::from(s.cursor_position().0)
}

fn before_cursor(s: &vt100::Screen) -> String {
    let (row, col) = s.cursor_position();
    grid(s)
        .get(usize::from(row))
        .map(|r| r.chars().take(usize::from(col)).collect())
        .unwrap_or_default()
}

/// When this test binary runs as the probe (`Spawn::probed`), it stands in for the shell: it
/// reads the terminal's mode, runs vswitch, reads the mode again, prints both and exits. In any
/// other run it returns at once. Each test crate that probes calls it from a test named `probe`.
pub fn probe() {
    let Some(args) = std::env::var_os("VSWITCH_HARNESS_PROBE") else {
        return;
    };
    let args = args.to_string_lossy().to_string();
    let args: Vec<&str> = args.split('\u{1f}').filter(|a| !a.is_empty()).collect();
    ignore_ctrl_c();
    let before = mode();
    let status = std::process::Command::new(binary()).args(&args).status();
    let after = mode();
    let code = status.map_or("none".to_string(), |s| format!("{:?}", s.code()));
    println!("probe: exit {code} mode before {before} after {after}");
    let _ = std::io::stdout().flush();
    std::process::exit(0);
}

/// A shell outlives a Ctrl-C its foreground program gets; so does the probe.
#[cfg(windows)]
fn ignore_ctrl_c() {
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }
    // a handler, not the null one: a child inherits "ignore Ctrl-C" but never a handler
    unsafe extern "system" fn swallow(_event: u32) -> i32 {
        1
    }
    // SAFETY: registers a handler function that lives as long as the process
    unsafe {
        SetConsoleCtrlHandler(Some(swallow), 1);
    }
}

#[cfg(unix)]
fn ignore_ctrl_c() {}

/// The console's input and output modes.
#[cfg(windows)]
fn mode() -> String {
    use crossterm_winapi::{ConsoleMode, Handle};
    let read = |h: std::io::Result<Handle>| {
        h.and_then(|h| ConsoleMode::from(h).mode())
            .map_or("?".to_string(), |m| format!("{m:#x}"))
    };
    format!(
        "in={},out={}",
        read(Handle::current_in_handle()),
        read(Handle::current_out_handle())
    )
}

/// The terminal's settings, as `stty -g` prints them; `?` and why when they cannot be read.
#[cfg(unix)]
fn mode() -> String {
    match std::process::Command::new("stty")
        .arg("-g")
        .stdin(std::process::Stdio::inherit())
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Ok(o) => format!("? stty {}", o.status),
        Err(e) => format!("? {e}"),
    }
}
