//! Scripted walks through vswitch's screens that write each screen to a file for review, the
//! README's screenshots, and the timings of startup and of opening a switch. Ignored in a normal
//! run; `tests/audit/` holds the scripts, `tests/screens.txt` the screenshots' script, and `run`
//! below their commands.

mod harness;

use harness::{Home, Key, Keys, Spawn, Term};
use std::path::Path;
use std::time::{Duration, Instant};

/// The probe shell a script's `probe` command runs; it returns at once in a normal run.
#[test]
fn probe() {
    harness::probe();
}

/// Every script in `tests/audit`, its screens under `<screens>/audit-<script>/`.
#[test]
#[ignore = "an audit: writes screens for review"]
fn audit() {
    let dir = harness::repo().join("tests").join("audit");
    let mut scripts: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    scripts.sort();
    for path in scripts {
        run_file(&path);
    }
}

/// The script `VSWITCH_SCRIPT` names.
#[test]
#[ignore = "runs the script VSWITCH_SCRIPT names"]
fn script() {
    let path = std::env::var_os("VSWITCH_SCRIPT").expect("VSWITCH_SCRIPT names a script");
    run_file(Path::new(&path));
}

/// The README's screenshots, `docs/screens/<name>.png`, from `tests/screens.txt`.
#[test]
#[ignore = "writes the README's screenshots; needs Edge, Chrome or Chromium"]
fn screens() {
    let path = harness::repo().join("tests").join("screens.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    run_in(Home::profiled(), &text, "audit-screens");
}

fn run_file(path: &Path) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let stem = path
        .file_stem()
        .map_or("script".into(), |s| s.to_string_lossy());
    println!("== {}", path.display());
    run(&text, &format!("audit-{stem}"));
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

/// Time to the list's first screen, and from Enter on a switch to its prompt, five runs each, for
/// a design and two captures. `VSWITCH_BIN` times another build, such as a release one.
#[test]
#[ignore = "timings for the audit report"]
fn timings() {
    let files = [
        ("tests/example-design.toml", "dc1-stack-sw01"),
        ("tests/example-2960-running-config.txt", "access-2960"),
        ("tests/example-9400-running-config.txt", "core-9400"),
    ];
    println!("binary {}", harness::binary().display());
    for (file, name) in files {
        let home = Home::new();
        let out = home.run(&["load", file]);
        assert!(out.status.success(), "{out:?}");
        let lines =
            std::fs::read_to_string(harness::repo().join(file)).map_or(0, |t| t.lines().count());
        let (mut starts, mut opens) = (vec![], vec![]);
        for _ in 0..5 {
            let begun = Instant::now();
            let mut t = Term::spawn_in(&home, &[], 80, 24);
            t.wait_for(name);
            starts.push(begun.elapsed());
            let pressed = Instant::now();
            t.press(Key::Enter);
            t.wait_prompt(&format!("{name}>"));
            opens.push(pressed.elapsed());
            t.line("exit").wait_fullscreen();
            t.press(Key::Char('q'));
            t.wait_exit();
        }
        let ms = |v: &[Duration]| {
            v.iter()
                .map(|d| d.as_millis().to_string())
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!(
            "{file} ({lines} lines): first screen median {} ms [{}]; open median {} ms [{}]",
            median(starts.clone()).as_millis(),
            ms(&starts),
            median(opens.clone()).as_millis(),
            ms(&opens)
        );
    }
}

/// A key by its name in a script: `Enter`, `Tab`, `BackTab`, `Esc`, `Backspace`, `Delete`,
/// `Insert`, `Up`, `Down`, `Left`, `Right`, `Home`, `End`, `PageUp`, `PageDown`, `F1`-`F12`,
/// `Ctrl-x`, `Alt-x`, `Space`, or one character.
fn key(name: &str) -> Key {
    match name {
        "Enter" => Key::Enter,
        "Tab" => Key::Tab,
        "BackTab" => Key::BackTab,
        "Esc" => Key::Esc,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "Insert" => Key::Insert,
        "Up" => Key::Up,
        "Down" => Key::Down,
        "Left" => Key::Left,
        "Right" => Key::Right,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Space" => Key::Char(' '),
        n if n.starts_with("Ctrl-") => Key::Ctrl(n[5..].chars().next().expect("Ctrl-x")),
        n if n.starts_with("Alt-") => Key::Alt(n[4..].chars().next().expect("Alt-x")),
        n if n.starts_with('F') && n.len() > 1 => Key::F(n[1..].parse().expect("F1-F12")),
        n if n.chars().count() == 1 => Key::Char(n.chars().next().unwrap_or(' ')),
        n => panic!("no key {n:?}"),
    }
}

fn term(t: &mut Option<Term>, line: usize) -> &mut Term {
    t.as_mut()
        .unwrap_or_else(|| panic!("line {line}: nothing spawned"))
}

/// How long nothing is written before a dump: a frame arrives in pieces through a ConPTY.
const DUMP_QUIET: Duration = Duration::from_millis(150);

/// The text drawn in the colours that carry meaning, a line per run with its row and column:
/// problems red, completed work green, notes yellow, files the list does not load dark gray, the
/// pick black on cyan, a pick with a problem black on red, and the search hit black on yellow. A
/// dump's frame holds the text alone.
fn colours(t: &Term) -> String {
    use vt100::Color::Idx;
    type Kind = (&'static str, fn(&vt100::Cell) -> bool);
    let kinds: [Kind; 7] = [
        ("red", |c| matches!(c.fgcolor(), Idx(1) | Idx(9))),
        ("green", |c| matches!(c.fgcolor(), Idx(2) | Idx(10))),
        ("yellow", |c| matches!(c.fgcolor(), Idx(3) | Idx(11))),
        ("dim", |c| c.fgcolor() == Idx(8)),
        ("pick", |c| matches!(c.bgcolor(), Idx(6) | Idx(14))),
        ("pick-red", |c| matches!(c.bgcolor(), Idx(1) | Idx(9))),
        ("hit", |c| matches!(c.bgcolor(), Idx(3) | Idx(11))),
    ];
    t.with_screen(|s| {
        let (rows, cols) = s.size();
        let mut out = String::new();
        for (name, is) in kinds {
            for r in 0..rows {
                let (mut run, mut start) = (String::new(), None);
                for c in 0..=cols {
                    match (c < cols).then(|| s.cell(r, c)).flatten() {
                        Some(cell) if cell.is_wide_continuation() => {}
                        Some(cell) if is(cell) => {
                            start.get_or_insert(c);
                            run.push_str(if cell.has_contents() {
                                cell.contents()
                            } else {
                                " "
                            });
                        }
                        _ => {
                            if let Some(from) = start.take() {
                                // a blank pick is a highlighted empty field; other blank runs are none
                                let text = match run.trim_end() {
                                    "" if name == "pick" => "(blank)",
                                    t => t,
                                };
                                if !text.is_empty() {
                                    out.push_str(&format!("{name:<6} {r:>2},{from:<3} {text}\n"));
                                }
                                run.clear();
                            }
                        }
                    }
                }
            }
        }
        out
    })
}

fn size(s: &str) -> (u16, u16) {
    let (c, r) = s.split_once('x').expect("<cols>x<rows>");
    (c.parse().expect("cols"), r.parse().expect("rows"))
}

/// Runs a script on one switch list, one command a line (`#` starts a comment), its screens
/// filed under `name`:
///
/// | Command | |
/// |---|---|
/// | `spawn 80x24 [args]`, `probe 80x24 [args]` | start vswitch; `probe` under the probe shell, which prints the terminal mode before and after |
/// | `type <text>`, `line <text>` | type text; `line` presses Enter after it |
/// | `key <name> [n]` | press a key (names at `key`) n times |
/// | `paste <file>`, `paste-plain <file>`, `paste-text <text>` | paste a repository file as Windows Terminal does, the same without bracketed-paste marks, or text with `\n` line ends |
/// | `wait <text>`, `gone <text>` | wait until the screen shows text, or no longer does |
/// | `fullscreen` | wait for the list (or any full-screen view) on the alternate screen |
/// | `prompt <prompt>` | wait for the prompt at the cursor with nothing typed after it |
/// | `command <prompt> <text>` | type a console line, press Enter and wait for the prompt after it |
/// | `typed <prompt> <text>` | as `command`, Enter pressed once the line is drawn, as at a person's pace; a line shorter than the terminal |
/// | `since`, `written <text>` | mark the output so far; wait until the program writes text after the mark |
/// | `settle [ms]` | wait until nothing is written for ms (500); for screens with nothing known to wait for |
/// | `resize 60x20` | resize the terminal |
/// | `dump <label>` | write the screen to a file and print it, once nothing has been written for 150 ms, with the runs of red, green, yellow, dim, picked and search-hit text |
/// | `shot <name>` | as `dump`, and the screen as Windows Terminal draws it, dark, to `docs/screens/<name>.png`: an HTML page beside the dump, photographed by a headless Edge, Chrome or Chromium (`VSWITCH_BROWSER` names another) |
/// | `exit` | wait for the program to end and print its exit code |
/// | `mark <label>` | print the time since the last mark or spawn |
/// | `list` | print the switch list's files |
/// | `file <name> <text>` | write a file into the switch list's folder, `\n` a line end; `{home}` in any command is that folder, `{repo}` the repository |
/// | `run <args>` | run `vswitch <args>` on the switch list without a terminal, as `vswitch load` is run, and print what it printed |
/// | `keys vt`, `keys win32` | send keys as xterm sequences, or as the ConPTY asks |
/// | `raw` | print what the program wrote since the last `raw`, escaped |
pub fn run(script: &str, name: &str) {
    run_in(Home::new(), script, name);
}

/// `run` on the switch list `home`.
pub fn run_in(home: Home, script: &str, name: &str) {
    let mut t: Option<Term> = None;
    let mut mark = Instant::now();
    let (mut seen, mut since) = (0, 0);
    for (n, raw) in script.lines().enumerate() {
        let line = raw.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
        let arg = &arg
            .replace("{home}", &home.path().to_string_lossy())
            .replace("{repo}", &harness::repo().to_string_lossy());
        println!("> {line}");
        let at = n + 1;
        match cmd {
            "spawn" | "probe" => {
                let mut words = arg.split_whitespace();
                let (cols, rows) = size(words.next().expect("a size"));
                let args: Vec<&str> = words.collect();
                t.take();
                let s = Spawn::new(&home).name(name);
                let s = if cmd == "probe" { s.probed() } else { s };
                t = Some(s.run(&args, cols, rows));
                mark = Instant::now();
                seen = 0;
            }
            "type" => {
                term(&mut t, at).type_text(arg);
            }
            "line" => {
                term(&mut t, at).line(arg);
            }
            "key" => {
                let mut w = arg.split_whitespace();
                let k = key(w.next().expect("a key name"));
                let times = w.next().map_or(1, |x| x.parse().expect("a count"));
                term(&mut t, at).press_n(k, times);
            }
            "paste" | "paste-plain" => {
                let text = std::fs::read_to_string(harness::repo().join(arg))
                    .unwrap_or_else(|e| panic!("{arg}: {e}"));
                if cmd == "paste" {
                    term(&mut t, at).paste(&text);
                } else {
                    term(&mut t, at).paste_as(&text, false);
                }
            }
            "paste-text" => {
                term(&mut t, at).paste(&arg.replace("\\n", "\n"));
            }
            "wait" => {
                term(&mut t, at).wait_for(arg);
            }
            "gone" => {
                term(&mut t, at).wait_gone(arg);
            }
            "fullscreen" => {
                term(&mut t, at).wait_fullscreen();
            }
            "prompt" => {
                term(&mut t, at).wait_prompt(arg);
            }
            "command" => {
                let (prompt, text) = arg.split_once(' ').unwrap_or((arg, ""));
                term(&mut t, at).command(text, prompt);
            }
            "typed" => {
                let (prompt, text) = arg.split_once(' ').unwrap_or((arg, ""));
                let t = term(&mut t, at);
                let drawn = format!("{}{text}", t.cursor_line());
                t.type_text(text).wait_prompt(&drawn);
                t.press_for(Key::Enter, prompt);
            }
            "since" => since = term(&mut t, at).output_len(),
            "written" => {
                term(&mut t, at).wait_written(since, arg);
            }
            "settle" => {
                let ms = if arg.is_empty() {
                    500
                } else {
                    arg.parse().expect("ms")
                };
                term(&mut t, at).settle(Duration::from_millis(ms));
            }
            "resize" => {
                let (cols, rows) = size(arg);
                term(&mut t, at).resize(cols, rows);
            }
            "dump" | "shot" => {
                let t = term(&mut t, at);
                // a wait ends at the first row that matches; the rest of the frame is still arriving
                t.settle(DUMP_QUIET);
                let path = t.dump(arg);
                let colours = colours(t);
                let mut kept = std::fs::read_to_string(&path).unwrap_or_default();
                kept.push_str(&colours);
                let _ = std::fs::write(&path, kept);
                println!("{}{colours}({})", t.framed(), path.display());
                if cmd == "shot" {
                    let png = harness::repo()
                        .join("docs")
                        .join("screens")
                        .join(format!("{arg}.png"));
                    t.shot(&path.with_extension("html"), &png);
                    println!("({})", png.display());
                }
            }
            "exit" => {
                let code = term(&mut t, at).wait_exit();
                println!("exit code {code}");
            }
            "mark" => {
                println!("{arg}: {} ms", mark.elapsed().as_millis());
                mark = Instant::now();
            }
            "list" => {
                println!("switches.json: {}", home.switches());
                if let Ok(d) = std::fs::read_dir(home.path().join("configs")) {
                    for e in d.flatten() {
                        let len = e.metadata().map_or(0, |m| m.len());
                        println!("  configs/{} {len} bytes", e.file_name().to_string_lossy());
                    }
                }
            }
            "run" => {
                let args: Vec<&str> = arg.split_whitespace().collect();
                let out = home.run(&args);
                print!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                println!("exit code {:?}", out.status.code());
            }
            "file" => {
                let (file, body) = arg.split_once(' ').unwrap_or((arg, ""));
                let path = home.path().join(file);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("folder made");
                }
                std::fs::write(&path, body.replace("\\n", "\n")).expect("file written");
            }
            "keys" => {
                term(&mut t, at).keys = if arg == "vt" { Keys::Vt } else { Keys::AsAsked };
            }
            "raw" => {
                let out = term(&mut t, at).output();
                let new = String::from_utf8_lossy(&out[seen.min(out.len())..]).to_string();
                seen = out.len();
                println!("{}", new.escape_debug());
            }
            other => panic!("line {at}: no command {other:?}"),
        }
    }
}
