//! vswitch as a user meets it, on a pseudo-terminal at 80x24 and at 132x43: the switch list,
//! loading a design and a configuration, the hardware builder, a switch's console and leaving it.
//! Each test checks what a step does (a prompt, a kept file, the list coming back), not where it
//! is drawn. `tests/harness` runs the binary and reads the screen.

mod harness;

use harness::{Home, Key, RED, Spawn, Term};

/// The probe shell `Spawn::probed` runs; it returns at once in a normal run.
#[test]
fn probe() {
    harness::probe();
}

/// Each scenario at both sizes, as `at_80x24::<name>` and `at_132x43::<name>`.
macro_rules! at_both_sizes {
    ($($name:ident),* $(,)?) => {
        mod at_80x24 {
            $(#[test]
            fn $name() {
                super::$name(80, 24);
            })*
        }
        mod at_132x43 {
            $(#[test]
            fn $name() {
                super::$name(132, 43);
            })*
        }
    };
}

at_both_sizes!(
    the_list_starts_empty_and_q_quits,
    a_design_loads_and_opens_its_console,
    a_configuration_confirms_its_hardware_then_opens,
    the_builder_marks_a_module_that_does_not_fit_and_keeps_it_only_once_it_fits,
    question_mark_lists_what_may_come_next_and_keeps_the_line,
    tab_completes_keywords_and_interfaces,
    show_running_config_prints_the_configuration,
    long_output_pauses_at_more,
    keys_typed_ahead_show_and_ctrl_keys_do_what_ios_does,
    a_path_pasted_on_the_list_reaches_the_browser_whole,
    write_memory_keeps_the_configuration_in_the_list,
    exit_returns_to_the_list_and_the_switch_opens_again,
    vswitch_shell_runs_without_the_list,
);

/// A design small enough to open fast: two C9200-48T, hostname idf-9200.
const DESIGN: &str = "tests/c9200-design.toml";
const NAME: &str = "idf-9200";

/// The list with the design kept (as `vswitch load` keeps it), and its console open.
fn opened(cols: u16, rows: u16) -> (Term, Home) {
    let home = Home::new();
    let out = home.run(&["load", DESIGN]);
    assert!(out.status.success(), "vswitch load: {out:?}");
    let mut t = Term::spawn_in(&home, &[], cols, rows);
    t.wait_for(NAME);
    t.press(Key::Enter);
    t.wait_prompt(&format!("{NAME}>"));
    (t, home)
}

/// Quitting leaves the terminal as it was: the main screen, the cursor shown.
fn assert_restored(t: &mut Term) {
    assert_eq!(t.wait_exit(), 0, "exit code\n{}", t.framed());
    assert!(
        !t.alternate_screen() && t.cursor_visible(),
        "the terminal is not restored\n{}",
        t.framed()
    );
}

fn the_list_starts_empty_and_q_quits(cols: u16, rows: u16) {
    let (mut t, home) = Term::spawn(&[], cols, rows);
    t.wait_fullscreen();
    t.dump("empty");
    t.press(Key::Char('q'));
    assert_restored(&mut t);
    assert!(home.switches().is_empty(), "{}", home.switches());
}

fn a_design_loads_and_opens_its_console(cols: u16, rows: u16) {
    let (mut t, home) = Term::spawn(&[], cols, rows);
    t.wait_fullscreen();
    t.press(Key::Char('a')).type_text(DESIGN).press(Key::Enter);
    t.wait_prompt(&format!("{NAME}>"));
    assert!(!t.alternate_screen(), "the console runs on the main screen");
    t.dump("console");
    let kept = home.switches();
    assert!(
        kept.contains(NAME) && kept.contains("C9200-48T+C9200-NM-4X"),
        "{kept}"
    );
    assert!(
        home.config(NAME)
            .is_some_and(|c| c.contains("hostname idf-9200"))
    );
}

fn a_configuration_confirms_its_hardware_then_opens(cols: u16, rows: u16) {
    let (mut t, home) = Term::spawn(&[], cols, rows);
    t.wait_fullscreen();
    // a 9300 configuration naming its hardware in a provision line, kept only once confirmed
    t.press(Key::Char('a'))
        .type_text("tests/check-config.txt")
        .press(Key::Enter);
    t.wait_for("C9300-48T");
    t.dump("hardware");
    assert!(home.switches().is_empty(), "kept before it was confirmed");
    t.press(Key::Enter);
    t.wait_prompt("idf-check>");
    let kept = home.switches();
    assert!(kept.contains("\"stack\": \"C9300-48T\""), "{kept}");
    assert!(kept.contains("\"release\": \"16.12\""), "{kept}");
}

fn the_builder_marks_a_module_that_does_not_fit_and_keeps_it_only_once_it_fits(
    cols: u16,
    rows: u16,
) {
    let (mut t, home) = Term::spawn(&[], cols, rows);
    t.wait_fullscreen();
    t.press(Key::Char('n')).type_text("lab1").press(Key::Enter);
    t.wait_for("C9300X-24Y");
    // the module field's list, filtered by typing to the one 9200 module: offered, and chosen once
    // the list closes, it is red
    t.press(Key::Tab).type_text("9200 2q");
    t.wait_until("the list narrowed to C9200-NM-2Q", |s| {
        s.contents().contains(" 1 of ") && s.contents().contains("C9200-NM-2Q")
    });
    t.press(Key::Enter);
    t.wait_gone("Network module for");
    t.wait_until("C9200-NM-2Q chosen and marked", |s| {
        s.contents()
            .contains("C9200-NM-2Q is a Catalyst 9200 module")
    });
    let red = t.colored(RED);
    assert!(
        red.contains("C9200-NM-2Q"),
        "red text:\n{red}\n{}",
        t.framed()
    );
    t.dump("does-not-fit");
    // Enter keeps nothing while a problem stands; stepping the field brings a module the model takes
    t.press(Key::Enter).press(Key::Right);
    t.wait_until("the problem cleared", |s| {
        !s.contents().contains("C9200-NM-2Q")
    });
    assert!(
        home.switches().is_empty(),
        "kept with a problem: {}",
        home.switches()
    );
    t.press(Key::Enter);
    t.wait_prompt("lab1>");
    assert!(home.switches().contains("\"stack\": \"C9300X-24Y"));
}

fn question_mark_lists_what_may_come_next_and_keeps_the_line(cols: u16, rows: u16) {
    let (mut t, _home) = opened(cols, rows);
    // a long answer pages at --More-- as output does; the line comes back as typed
    t.type_text("show ").press(Key::Char('?'));
    t.wait_for("--More--");
    t.dump("show-question");
    assert!(t.screen().contains("  aaa "), "{}", t.framed());
    t.press(Key::Char('q'));
    t.wait_prompt(&format!("{NAME}>show "));
    t.press(Key::Ctrl('u'));
    t.command("terminal length 0", &format!("{NAME}>"));
    t.type_text("show ")
        .press_for(Key::Char('?'), &format!("{NAME}>show "));
    assert!(t.screen().contains("vlan"), "{}", t.framed());
    // right after a word: the keywords it begins
    t.press(Key::Ctrl('u'))
        .type_text("en")
        .press_for(Key::Char('?'), &format!("{NAME}>en"));
    assert!(t.screen().contains("enable"), "{}", t.framed());
}

fn tab_completes_keywords_and_interfaces(cols: u16, rows: u16) {
    let (mut t, _home) = opened(cols, rows);
    t.type_text("en").press(Key::Tab);
    t.wait_prompt(&format!("{NAME}>enable "));
    t.press_for(Key::Enter, &format!("{NAME}#"));
    t.type_text("configure t").press(Key::Tab);
    t.wait_prompt(&format!("{NAME}#configure terminal "));
    t.press_for(Key::Enter, &format!("{NAME}(config)#"));
    // the rest of the one keyword begun, appended to the word as typed (lab C9300 17.6.4)
    t.type_text("int").press(Key::Tab);
    t.wait_prompt(&format!("{NAME}(config)#interface "));
    t.type_text("gi").press(Key::Tab);
    t.wait_prompt(&format!("{NAME}(config)#interface gigabitEthernet "));
    // a type with its number attached rings the bell and stays as typed
    let bells = t.bells();
    t.press(Key::Ctrl('u'))
        .type_text("interface gi1/0/1")
        .press(Key::Tab);
    t.press_for(Key::Enter, &format!("{NAME}(config-if)#"));
    assert_eq!(t.bells(), bells + 1, "{}", t.framed());
}

fn show_running_config_prints_the_configuration(cols: u16, rows: u16) {
    let (mut t, _home) = opened(cols, rows);
    t.command("enable", &format!("{NAME}#"));
    t.command("terminal length 0", &format!("{NAME}#"));
    t.command("show running-config", &format!("{NAME}#"));
    t.dump("show-run");
    // the screen holds its last lines: the configuration ends with `end`, then the prompt
    let screen = t.screen();
    let lines: Vec<&str> = screen.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines[lines.len() - 2..],
        ["end", &format!("{NAME}#")],
        "{}",
        t.framed()
    );
}

fn long_output_pauses_at_more(cols: u16, rows: u16) {
    let (mut t, _home) = opened(cols, rows);
    t.command("enable", &format!("{NAME}#"));
    t.line("show running-config");
    t.wait_for("--More--");
    t.dump("more");
    // Space shows the next page and pauses again, each wait on what the key brings
    let page = t.screen();
    let from = t.output_len();
    t.press(Key::Char(' '));
    t.wait_written(from, "--More--");
    let next = t.screen();
    // a page went by: the hostname near the first page's top is gone, as a line would leave it
    assert!(
        page.contains("hostname idf-9200") && !next.contains("hostname idf-9200"),
        "{}",
        t.framed()
    );
    // Return shows one more line: the page moves up a row
    let from = t.output_len();
    t.press(Key::Enter);
    t.wait_written(from, "--More--");
    let rows = |s: &str| s.lines().map(str::to_string).collect::<Vec<_>>();
    let (before, after) = (rows(&next), rows(&t.screen()));
    let n = before.len();
    assert_eq!(after[..n - 2], before[1..n - 1], "{}", t.framed());
    t.press(Key::Char('q'));
    t.wait_prompt(&format!("{NAME}#"));
    assert!(!t.screen().contains("--More--"), "{}", t.framed());
}

fn keys_typed_ahead_show_and_ctrl_keys_do_what_ios_does(cols: u16, rows: u16) {
    let (mut t, home) = opened(cols, rows);
    // the harness types a whole line before the console draws it, as a fast typist or a paste
    // does: the line stays on the screen above the next prompt
    t.command("enable", &format!("{NAME}#"));
    assert!(
        t.contents().contains(&format!("{NAME}>enable")),
        "{}",
        t.framed()
    );
    // Ctrl-Z at an EXEC prompt does nothing
    t.press(Key::Ctrl('z'));
    t.command("terminal length 0", &format!("{NAME}#"));
    assert!(!t.contents().contains("Invalid input"), "{}", t.framed());
    // a line typed ahead and then discarded or ended stays on the screen as it was typed
    t.type_text("show clock")
        .press_for(Key::Ctrl('c'), &format!("{NAME}#"));
    assert!(
        t.contents().contains(&format!("{NAME}#show clock")),
        "{}",
        t.framed()
    );
    t.command("configure terminal", &format!("{NAME}(config)#"));
    t.type_text("vlan 999")
        .press_for(Key::Ctrl('z'), &format!("{NAME}#"));
    assert!(
        t.contents().contains(&format!("{NAME}(config)#vlan 999^Z")),
        "{}",
        t.framed()
    );
    // Ctrl-C at copy's question copies nothing
    let kept = home.config(NAME);
    t.line("copy running-config startup-config");
    t.wait_for("Destination filename");
    t.press_for(Key::Ctrl('c'), &format!("{NAME}#"));
    assert!(!t.contents().contains("[OK]"), "{}", t.framed());
    assert_eq!(home.config(NAME), kept);
}

fn a_path_pasted_on_the_list_reaches_the_browser_whole(cols: u16, rows: u16) {
    let home = Home::new();
    let out = home.run(&["load", DESIGN]);
    assert!(out.status.success(), "vswitch load: {out:?}");
    let mut t = Term::spawn_in(&home, &[], cols, rows);
    t.wait_for(NAME);
    // a Windows console delivers a paste as keys, with Shift records crossterm drops between the
    // capitals and the colon: the path reaches the browser whole and runs no list command
    let path = "C:\\ProgramData\\VSwitch Test\\Core-SW.cfg";
    t.paste(path);
    t.wait_for(path);
    t.dump("pasted-path");
    let kept = home.switches();
    assert!(
        kept.contains(NAME) && !kept.contains("ProgramData"),
        "{kept}"
    );
}

fn write_memory_keeps_the_configuration_in_the_list(cols: u16, rows: u16) {
    let (mut t, home) = opened(cols, rows);
    for (line, prompt) in [
        ("enable", "#"),
        ("configure terminal", "(config)#"),
        ("vlan 777", "(config-vlan)#"),
        ("name kept-by-the-harness", "(config-vlan)#"),
        ("end", "#"),
    ] {
        t.command(line, &format!("{NAME}{prompt}"));
    }
    assert!(!home.config(NAME).unwrap_or_default().contains("vlan 777"));
    t.command("write memory", &format!("{NAME}#"));
    assert!(t.screen().contains("[OK]"), "{}", t.framed());
    let kept = home.config(NAME).unwrap_or_default();
    assert!(
        kept.contains("vlan 777") && kept.contains("name kept-by-the-harness"),
        "{kept}"
    );
    assert!(home.switches().contains("saved_at"), "{}", home.switches());
}

fn exit_returns_to_the_list_and_the_switch_opens_again(cols: u16, rows: u16) {
    let (mut t, _home) = opened(cols, rows);
    let console = t.line_no();
    t.line("exit");
    t.wait_fullscreen().wait_for(NAME);
    t.dump("list-again");
    t.press(Key::Enter);
    t.wait_prompt_after(Some(console), &format!("{NAME}>"));
    t.line("exit");
    t.wait_fullscreen();
    t.press(Key::Char('q'));
    assert_restored(&mut t);
}

fn vswitch_shell_runs_without_the_list(cols: u16, rows: u16) {
    let (mut t, home) = Term::spawn(&["shell"], cols, rows);
    t.wait_prompt("Switch>");
    assert!(!t.alternate_screen());
    t.command("enable", "Switch#");
    t.line("exit");
    assert_restored(&mut t);
    assert!(home.switches().is_empty(), "the shell keeps nothing");
}

/// The shell sees the console or tty in the mode it left it in (Windows: the console's input and
/// output modes; elsewhere `stty -g`).
#[test]
fn quitting_leaves_the_terminal_mode_as_it_found_it() {
    let home = Home::new();
    let mut t = Spawn::new(&home).probed().run(&[], 80, 24);
    t.wait_fullscreen();
    t.press(Key::Char('q'));
    t.wait_for("probe: exit");
    assert_eq!(t.wait_exit(), 0, "the probe ends once it has printed");
    let line = t
        .contents()
        .lines()
        .find(|l| l.starts_with("probe: exit"))
        .unwrap_or_default()
        .to_string();
    let (before, after) = line
        .split_once(" mode before ")
        .and_then(|(_, rest)| rest.split_once(" after "))
        .unwrap_or_else(|| panic!("{line}"));
    assert!(line.starts_with("probe: exit Some(0)"), "{line}");
    // a mode the probe could not read is no mode: two of those are equal and prove nothing
    for mode in [before, after] {
        assert!(
            !mode.trim().is_empty() && !mode.contains('?'),
            "the probe could not read the terminal's mode: {line}"
        );
    }
    assert_eq!(before.trim(), after.trim(), "{}", t.framed());
}

#[test]
fn the_harness_refuses_a_switch_list_that_is_not_its_own() {
    for dir in [std::env::temp_dir(), harness::repo()] {
        let not_ours = Home::at(&dir);
        let refused = Spawn::new(&not_ours).try_run(&[], 80, 24);
        assert!(
            refused.as_ref().is_err_and(|e| e.contains("refusing")),
            "{dir:?} was not refused"
        );
    }
}
