//! vswitch: virtual Catalyst 9300 and 9200 switches whose commands follow Cisco's command references
//! and configuration guides for the IOS XE release each switch runs. The grammars are compiled into the
//! binary.

mod acl;
mod builder;
mod capture;
mod changes;
mod check;
mod cli;
mod compile;
mod console;
mod context;
mod corpus;
#[cfg(captures)]
mod coverage;
mod device;
mod evidence;
mod filter;
mod generate;
mod grammars;
mod lab;
mod library;
mod license;
mod lint;
mod live;
mod platform;
mod probe;
mod release;
mod shell;
mod show;
mod startup;
mod sweep;
mod syntax;
mod target;
mod topology;
mod tui;
mod verify;

use release::Release;
use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "usage:
  vswitch                                        the switch list; Enter opens a switch's console
  vswitch shell [--stack <members>] [--release <x.y>]
                                                 an interactive session
  vswitch load <design.toml or config> [--stack <members>] [--release <x.y>] [--replace]
                                                 adds a switch to the list (secrets swept from the kept copy);
                                                 one of the same name only with --replace
  vswitch sweep [<file or folder>...] [--out <folder>]
                                                 removes passwords and secrets from configuration files, in
                                                 place or into copies; no path sweeps the sweep folder
  vswitch generate <design.toml> [--release <x.y>] [--out <config.txt>] [--change <change.txt>]
                   [--running <running-config> --target <target-config>] [--show-run]
                                                 a configuration from a design; every line typed into the switch first
  vswitch run <script> [--stack <members>] [--release <x.y>] [--quiet]
                                                 types every line of a script; exit 1 if any is refused
  vswitch examples [--show N] [--canonical] [--platform <book or model>] [--release <x.y>]
                                                 types every Cisco example line at its prompt
  vswitch check <config> --to <x.y> [--from <x.y>] [--stack <members>] [--all]
                                                 the lines a release change breaks; --from defaults to the version line
  vswitch lint <config> [--stack <members>] [--release <x.y>]
                                                 checks a configuration as it stands; exit 1 on a finding
  vswitch capture <user@host> [--key <file>] | --log <session log>... [--config <file>] | --folder <dir>, each [--out <dir>] [--pending <dir>]
                                                 a real switch's show output and running configuration over SSH
                                                 (key only), or from a session log, written as a capture
                                                 under <dir>/<book>-<release>/ (data/live keeps them)
  vswitch probe <user@host> <script> --lab [--key <file>] [--out <dir>] [--skip-unsafe] [--map <from>=<to>]...
                                                 types a script on a private lab switch (its configuration
                                                 saved to flash first and put back after), and compares each
                                                 line's answer and what IOS keeps with vswitch; exit 1 on a
                                                 difference. --skip-unsafe leaves out the lines that could cut
                                                 the session; --map Twe1/0/=Gi1/0/ renames ports
  vswitch lab <lab.toml> [--shell <switch>]     switches cabled together: each cable checked from both ends;
                                                 exit 1 on a finding; --shell opens that switch's console
  vswitch changes <backup file or folder>... --captures <dir>
                                                 what each backup changes on its captured switch: the items
                                                 added, removed and edited, the lines its release refuses,
                                                 and the lint and link findings it adds or clears; exit 1
                                                 on a new refusal or finding
  vswitch fleet <captures dir> --to <x.y>... [--all]
                                                 every capture moved to the first --to release its book documents
                                                 and its hardware runs:
                                                 the lines each breaks and the hardware that cannot run it
  vswitch topology <captures dir> [--out <lab.toml>]
                                                 a lab file of the captured switches cabled where both ends'
                                                 port descriptions name each other; then vswitch lab checks it
  vswitch corpus <captures dir> [--out <file>]  what the captures teach, compiled into the table set the
                                                 binary embeds (data/corpus.json.gz); --scan <dir> lists
                                                 each line under a folder naming the site, --loose with
                                                 words holding a short site word, for review
  vswitch releases                               the IOS XE releases each platform's grammar covers
  vswitch verify <config> [--stack <members>] [--release <x.y>]
                                                 each line's evidence; the lines only the books give, as a script
  vswitch verify --outputs                      the output rules no real switch has shown yet
  vswitch compile <reference.json> <grammar.json[.gz]> [--guides <guides.json>] [--references <refs.json>]
                  [--overrides <overrides.json>]
  vswitch compile-releases <reference dir> <grammar dir> [--references <refs.json>] [--overrides <overrides.json>]
                                                 every <dir>/cisco-<release>/<platform>-cr.json and its guides
<members>: models joined by ',' (member 1 first), each with an optional '+<network module>' and an
  optional '<number>=' prefix (a Catalyst 9400 line card's slot: 2=C9400-LC-24XS),
  default C9300X-24Y+C9300X-NM-4C,C9300X-24Y+C9300X-NM-4C
<x.y>: an IOS XE release (17.9, 16.12, 26.x; a patch such as 17.9.4 runs its train), default 17.15";

fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// `vswitch load`: a design or configuration kept in the switch list. A listed switch of the same name,
/// or one kept in the same file, is replaced only with `--replace`, as the selector replaces one only
/// once that is confirmed. Returns what was kept, and what it replaced.
fn load_into(lib: &mut library::Library, path: &str, args: &[String]) -> Result<String, String> {
    let (name, stack, own, config) = match library::import(std::path::Path::new(path))? {
        library::Import::Ready {
            name,
            stack,
            release,
            config,
        } => (name, stack, release, config),
        library::Import::NeedsStack {
            name,
            inferred,
            release,
            config,
        } => {
            let stack = arg(args, "--stack")
                .map(str::to_string)
                .or(inferred)
                .ok_or(format!(
                    "{path}: give its hardware with --stack; its interfaces fit no model"
                ))?;
            (name, stack, release, config)
        }
    };
    let (_, r) = stack_at(&stack, args, own.as_deref())?;
    let listed: Vec<String> = lib
        .switches
        .iter()
        .filter(|s| library::same_switch(&s.name, &name))
        .map(|s| s.name.clone())
        .collect();
    if !listed.is_empty() && !args.iter().any(|a| a == "--replace") {
        return Err(format!(
            "{path}: {} is in the list already; --replace replaces it and its kept configuration",
            listed.join(" and ")
        ));
    }
    let i = lib.add(&name, &stack, r, path, &config)?;
    let s = &lib.switches[i];
    let mut said = format!(
        "{}: kept as {} on {} ({} secret line(s) removed from the kept copy)",
        s.name,
        s.stack,
        s.release().named(),
        s.swept
    );
    if !listed.is_empty() {
        said.push_str(&format!("; it replaced {}", listed.join(" and ")));
    }
    Ok(said)
}

/// A script file's text without the byte order mark Notepad and Windows PowerShell 5.1 write, which
/// would otherwise stand in front of its first command.
fn script_text(path: &str) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(match text.strip_prefix('\u{feff}') {
        Some(t) => t.to_string(),
        None => text,
    })
}

/// A capture's release and hardware as its session named them: `IOS XE 17.15.2, C9200L-24T-4G`.
fn capture_identity(c: &capture::Capture) -> String {
    let release = c
        .version
        .as_deref()
        .map_or("(no show version)".to_string(), |v| {
            let classic = Release::parse(v).is_some_and(Release::classic);
            format!("{} {v}", if classic { "IOS" } else { "IOS XE" })
        });
    let hardware = match (&c.stack, c.unknown.first()) {
        (Some(s), _) => s.clone(),
        (None, Some(u)) => format!("{u} (not in vswitch's catalog)"),
        (None, None) => "(no show inventory)".into(),
    };
    format!("{release}, {hardware}")
}

/// Refuses an option the command does not take, and one of `values` given no value: a misspelt
/// `--stak` would otherwise leave its default in place unseen.
fn options(args: &[String], values: &[&str], bare: &[&str]) -> Result<(), String> {
    let command = args.first().map_or("", String::as_str);
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        if values.contains(&a) {
            if args.get(i + 1).is_none_or(|v| v.starts_with("--")) {
                return Err(format!("{command}: {a} needs a value"));
            }
            i += 2;
            continue;
        }
        if a.starts_with("--") && !bare.contains(&a) {
            let mut known: Vec<&str> = values.iter().chain(bare).copied().collect();
            known.sort_unstable();
            return Err(format!(
                "{command}: {a} is not an option of {command}, which takes {}",
                known.join(", ")
            ));
        }
        i += 1;
    }
    Ok(())
}

/// The arguments that are neither an option nor an option's value: the files a command reads.
fn operands<'a>(args: &'a [String], values: &[&str]) -> Vec<&'a str> {
    let mut out = vec![];
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        if values.contains(&a) {
            i += 2;
            continue;
        }
        if !a.starts_with("--") {
            out.push(a);
        }
        i += 1;
    }
    out
}

/// A hardware problem as `check` and `fleet` print it: the member it concerns, or the stack's own
/// (member 0: too many members, a 9400 without a supervisor).
fn hardware_line(member: u32, why: &str) -> String {
    if member == 0 {
        format!("hardware: {why}")
    } else {
        format!("hardware: member {member}: {why}")
    }
}

/// `capture --folder`: a capture per switch a folder's files name, each line of the account handed
/// to `say`. A switch captured already (its name in any case) gains what the new files hold that its
/// capture at the same release does not; at another release, an upgrade, they are a capture of their
/// own. Whether every switch named was captured or added to.
fn capture_folder(
    dir: &std::path::Path,
    out: &std::path::Path,
    pending: Option<&std::path::Path>,
    say: &mut dyn FnMut(String),
) -> Result<bool, String> {
    let (mut made, mut merged, mut failed) = (0, 0, 0);
    for (host, c) in capture::from_folder(dir)? {
        let live = capture::captured(out, &host);
        let kept: Vec<std::path::PathBuf> = live
            .iter()
            .cloned()
            .chain(
                pending
                    .map(|p| capture::captured(p, &host))
                    .unwrap_or_default(),
            )
            .collect();
        if !kept.is_empty() {
            let at = c
                .as_ref()
                .map_err(String::clone)
                .and_then(|c| capture::kept_at(c, &kept, out, pending));
            if let Err(e) = &at {
                failed += 1;
                let shown: Vec<String> = kept.iter().map(|k| k.display().to_string()).collect();
                say(format!(
                    "{host}: captured already as {}; {e}",
                    shown.join(", ")
                ));
                continue;
            }
            if let (Ok(Some(had)), Ok(c)) = (at, &c) {
                match capture::merge(&had, c) {
                    Ok((added, differs)) => {
                        merged += 1;
                        say(format!(
                            "{host}: captured already as {}; {}{}",
                            had.display(),
                            if added.is_empty() {
                                "nothing new".to_string()
                            } else {
                                format!("added {}", added.join(", "))
                            },
                            if differs {
                                "; its configuration differs from the kept one, which stays"
                            } else {
                                ""
                            }
                        ));
                        // what the backup changes, read against that capture; a pending one's
                        // hardware is not modelled
                        if differs && live.contains(&had) {
                            match changes::report_at(out, &had, &c.running) {
                                Ok(r) => {
                                    for l in r.lines() {
                                        say(format!("  {l}"));
                                    }
                                }
                                Err(e) => say(format!("  change report: {e}")),
                            }
                        }
                    }
                    Err(e) => {
                        failed += 1;
                        say(format!(
                            "{host}: captured already as {}; {e}",
                            had.display()
                        ));
                    }
                }
                continue;
            }
        }
        let written = c.and_then(|c| capture::write(&c, out, pending).map(|f| (c, f)));
        match written {
            Ok((c, files)) => {
                made += 1;
                let at = files[0].parent().map_or(String::new(), |p| {
                    p.file_name()
                        .map_or(String::new(), |n| n.to_string_lossy().into())
                });
                let refused = match capture_typed(&c) {
                    Some(Ok((typed, refused))) => {
                        format!("; {} of {typed} lines refused", refused.len())
                    }
                    Some(Err(e)) => format!("; {e}"),
                    None => String::new(),
                };
                say(format!("{host}: {} -> {at}{refused}", capture_identity(&c)));
            }
            Err(e) => {
                failed += 1;
                say(format!("{host}: {e}"));
            }
        }
    }
    say(format!(
        "{made} captured, {merged} added to a kept capture, {failed} not"
    ));
    // a folder that names no switch captured nothing either
    Ok(failed == 0 && made + merged > 0)
}

/// A capture's configuration typed into vswitch at its release: the lines typed and those refused;
/// nothing where the session named no release and modelled hardware.
fn capture_typed(c: &capture::Capture) -> Option<Result<(usize, Vec<check::Typed>), String>> {
    let (spec, v) = (c.stack.as_ref()?, c.version.as_ref()?);
    Some((|| {
        let stack = platform::parse_stack(spec).map_err(|e| format!("stack: {e}"))?;
        let r = grammars::resolve(v, c.book)?;
        let engine = cli::Engine::new(grammars::grammar(c.book, r)?);
        let (_, typed) = check::type_config(&engine, stack, &c.running);
        let n = typed.len();
        Ok((n, typed.into_iter().filter(|t| t.error.is_some()).collect()))
    })())
}

/// The grammar a family's switches run at a release: `--grammar <file>`, else the family's book
/// (`grammars::book`) at that release.
fn grammar(
    args: &[String],
    family: Option<platform::Family>,
    release: Release,
) -> Result<compile::Grammar, String> {
    match arg(args, "--grammar") {
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
            serde_json::from_str(&text).map_err(|e| format!("grammar: {e}"))
        }
        None => grammars::grammar(grammars::book(family), release),
    }
}

/// `--platform 9200`, `9400`, `2960x`, `2960`, `ie3300`, `ie3000`, `ie3010` or any model vswitch
/// knows (`IE-3000-8TC`) for commands that take no stack (explain, examples); the 9300 otherwise.
fn platform_arg(args: &[String]) -> Option<platform::Family> {
    let named = arg(args, "--platform")?;
    match named.to_ascii_lowercase().as_str() {
        "9200" | "9200l" => Some(platform::Family::C9200),
        "9400" => Some(platform::Family::C9400),
        "2960x" | "2960-x" => Some(platform::Family::C2960X),
        "2960" | "2960+" | "2960-plus" | "2960c" | "2960-c" => Some(platform::Family::C2960),
        "ie3300" => Some(platform::Family::IE3300),
        "ie3000" => Some(platform::Family::IE3000),
        "ie3010" => Some(platform::Family::IE3010),
        _ => platform::MODELS
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(named))
            .map(|m| m.family),
    }
}

/// The release a command runs in a book: `--release`, else the one its design or configuration names
/// (`own`), else 17.15. A `--release` that disagrees with the file's own is refused.
fn release(args: &[String], own: Option<&str>, book: &str) -> Result<Release, String> {
    let flag = arg(args, "--release")
        .map(|t| grammars::resolve(t, book))
        .transpose()?;
    let own_text = own;
    let own = own.map(|t| grammars::resolve(t, book)).transpose()?;
    match (flag, own) {
        (Some(f), Some(o)) if !f.agrees(o, own_text.unwrap_or("")) => Err(format!(
            "--release {f} disagrees with {}, which the file names",
            o.named()
        )),
        (Some(r), _) | (None, Some(r)) => Ok(r),
        (None, None) => Ok(grammars::default_release(book)),
    }
}

/// `release` for a stack: with no release named, the newest its book documents that the hardware runs
/// (`grammars::fitting_release`).
fn release_at(
    args: &[String],
    own: Option<&str>,
    stack: &platform::Stack,
) -> Result<Release, String> {
    let book = grammars::book(stack.family());
    if arg(args, "--release").is_none() && own.is_none() {
        return Ok(grammars::fitting_release(book, stack));
    }
    release(args, own, book)
}

/// A stack and the release it runs, checked together: hardware newer than the release is refused.
fn stack_at(
    spec: &str,
    args: &[String],
    own: Option<&str>,
) -> Result<(platform::Stack, Release), String> {
    let parsed = platform::parse_stack(spec).map_err(|e| format!("--stack: {e}"))?;
    let r = release_at(args, own, &parsed)?;
    let st = platform::checked_stack(spec, r).map_err(|e| format!("--stack: {e}"))?;
    Ok((st, r))
}

/// The stack `--stack` names (the cluster pair when none) at the release `--release` names.
fn stack(args: &[String]) -> Result<(platform::Stack, Release), String> {
    let spec = arg(args, "--stack").unwrap_or("C9300X-24Y+C9300X-NM-4C,C9300X-24Y+C9300X-NM-4C");
    stack_at(spec, args, None)
}

/// One book compiled: the command reference, then the configuration guides' procedure steps, the general
/// references, and the overrides, which have the last word.
fn build_book(
    src: &str,
    guides: Option<&str>,
    references: Option<&str>,
    overrides: Option<&str>,
) -> Result<compile::Grammar, String> {
    let text = std::fs::read_to_string(src).map_err(|e| format!("{src}: {e}"))?;
    let reference: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{src}: {e}"))?;
    build_reference(&reference, guides, references, overrides)
}

/// `build_book` on a command reference already read.
fn build_reference(
    reference: &serde_json::Value,
    guides: Option<&str>,
    references: Option<&str>,
    overrides: Option<&str>,
) -> Result<compile::Grammar, String> {
    let mut g = compile::compile(reference);
    // configuration lines the release's own books show, the evidence an override is called for
    let mut docs = compile::documented_lines(reference);
    if let Some(p) = guides {
        let t = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
        let v: serde_json::Value = serde_json::from_str(&t).map_err(|e| format!("{p}: {e}"))?;
        let (lines, cmds) = compile::add_guides(&mut g, &v);
        println!("{lines} syntax lines ({cmds} new commands) from {p}");
        docs.extend(compile::documented_lines(&v));
    }
    if let Some(p) = references {
        let t = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
        let v: serde_json::Value = serde_json::from_str(&t).map_err(|e| format!("{p}: {e}"))?;
        let n = compile::add_references(&mut g, &v);
        println!("{n} syntax lines from {p}, for commands the platform's own books name");
    }
    if let Some(p) = overrides {
        let t = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
        let (applied, skipped) =
            compile::apply_overrides(&mut g, &t, &docs).map_err(|e| format!("{p}: {e}"))?;
        println!("{} commands from {p}", applied.len());
        if !skipped.is_empty() {
            println!(
                "{} not called for by this release's sources: {}",
                skipped.len(),
                skipped.join(", ")
            );
        }
    }
    let dropped = compile::prune(&mut g);
    println!("{dropped} syntax lines dropped: they start with an argument");
    // example output is reference material, not needed to run
    for c in &mut g.commands {
        for e in &mut c.examples {
            e.output.clear();
        }
    }
    Ok(g)
}

/// The book's captured running configurations (the captures folder, `capture_files`), each with the
/// train its folder names.
fn capture_lines(grammars_dir: &str, book: &str) -> Vec<(Release, String)> {
    capture_files(grammars_dir, book, "-running-config.txt")
}

/// The capture files a release of a book reads: those of the trains it serves (15.2(6)E reads
/// 15.2(4)E's); at a release no capture runs, the older trains' (15.0(2)SE the 12.2 ones), else the
/// nearest newer train's (the IE 3000's 12.2(55)SE its 15.2(6)E ones), or `hostname` and `line` would
/// read as breaks.
fn captured_at(files: &[(Release, String)], documented: &[Release], r: Release) -> Vec<String> {
    let of = |keep: &dyn Fn(Release) -> bool| -> Vec<String> {
        files
            .iter()
            .filter(|(t, _)| keep(*t))
            .map(|(_, c)| c.clone())
            .collect()
    };
    let own = of(&|t| t.book_for(documented) == Some(r));
    if !own.is_empty() {
        return own;
    }
    let older = of(&|t| t < r);
    if !older.is_empty() {
        return older;
    }
    match files.iter().map(|(t, _)| *t).filter(|t| *t > r).min() {
        Some(next) => of(&|t| t == next),
        None => vec![],
    }
}

/// The text of each capture file of a book whose name ends `suffix`, with its train.
fn capture_files(grammars_dir: &str, book: &str, suffix: &str) -> Vec<(Release, String)> {
    // the captures folder VSWITCH_CAPTURES names, else `live` beside the grammars' folder
    let live = std::env::var("VSWITCH_CAPTURES")
        .ok()
        .filter(|d| !d.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(grammars_dir)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("live")
        });
    let mut out = Vec::new();
    for dir in std::fs::read_dir(&live).into_iter().flatten().flatten() {
        let name = dir.file_name().to_string_lossy().to_string();
        let Some(train) = name
            .strip_prefix(book)
            .and_then(|r| r.strip_prefix('-'))
            .and_then(Release::parse)
        else {
            continue;
        };
        for f in std::fs::read_dir(dir.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            if f.file_name().to_string_lossy().ends_with(suffix)
                && let Ok(text) = std::fs::read_to_string(f.path())
            {
                out.push((train, text));
            }
        }
    }
    out
}

/// Submodes a release's own examples show too seldom for `Engine::new` (two prompts), taken from the
/// nearest release of the same book whose examples show them (newer first at equal distance), from the
/// command of the same name in one of the same modes, where the release runs commands in that
/// submode. A release's own evidence is never replaced. Returns one log line per command filled.
fn borrow_enters(books: &mut [(Release, compile::Grammar)]) -> Vec<String> {
    // per release, each command that enters a submode: its name, its modes and the submode
    let inferred: Vec<Vec<(String, Vec<String>, String)>> = books
        .iter()
        .map(|(_, g)| {
            cli::Engine::new(g.clone())
                .grammar
                .commands
                .iter()
                .filter_map(|c| Some((c.name.clone(), c.modes.clone(), c.enters.clone()?)))
                .collect()
        })
        .collect();
    let mut log = Vec::new();
    for i in 0..books.len() {
        let engine = cli::Engine::new(books[i].1.clone());
        let mut order: Vec<usize> = (0..books.len()).filter(|&j| j != i).collect();
        order.sort_by_key(|&j| (j.abs_diff(i), j < i));
        for (ci, c) in engine.grammar.commands.iter().enumerate() {
            if c.enters.is_some() {
                continue;
            }
            let Some((j, to)) = order.iter().find_map(|&j| {
                inferred[j]
                    .iter()
                    .find(|(name, modes, to)| {
                        *name == c.name
                            && modes.iter().any(|m| c.modes.contains(m))
                            && cli::is_submode(to)
                            && engine.has_mode(to)
                    })
                    .map(|(_, _, to)| (j, to.clone()))
            }) else {
                continue;
            };
            let from = books[j].0;
            let cmd = &mut books[i].1.commands[ci];
            let why = format!("enters {to} as the {from} books' examples show");
            cmd.evidence = Some(match cmd.evidence.take() {
                Some(e) => format!("{e}; {why}"),
                None => why,
            });
            cmd.enters = Some(to.clone());
            log.push(format!(
                "{} {}: {} enters {to} ({from})",
                books[i].1.platform, books[i].0, c.name
            ));
        }
    }
    log
}

/// A compiled grammar written as JSON, gzip when `out` ends in `.gz`.
fn write_grammar(g: &compile::Grammar, out: &str) -> Result<(), String> {
    let lines: usize = g.commands.iter().map(|c| c.lines.len()).sum();
    let repaired: usize = g
        .commands
        .iter()
        .flat_map(|c| &c.lines)
        .filter(|l| !l.repairs.is_empty())
        .count();
    let examples: usize = g.commands.iter().map(|c| c.examples.len()).sum();
    write_json(&serde_json::to_string(g).map_err(|e| e.to_string())?, out)?;
    println!(
        "{} commands, {lines} syntax lines ({repaired} repaired), {examples} example lines, {} mode transitions -> {out}",
        g.commands.len(),
        g.transitions.len()
    );
    Ok(())
}

/// JSON written to `out`, gzip when it ends in `.gz`.
fn write_json(json: &str, out: &str) -> Result<(), String> {
    let json = format!("{json}\n");
    let bytes = if out.ends_with(".gz") {
        // no timestamp in the header, so a recompile of the same sources writes the same bytes
        let mut z = flate2::GzBuilder::new()
            .mtime(0)
            .write(Vec::new(), flate2::Compression::best());
        z.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
        z.finish().map_err(|e| e.to_string())?
    } else {
        json.into_bytes()
    };
    std::fs::write(out, bytes).map_err(|e| format!("{out}: {e}"))
}

/// A book's help moved out of its releases' grammars into one table, command name to keyword to
/// text, the newest release's text kept; every release reads it back (`grammars::grammar`). A name
/// that commands of different modes share (the QoS `set` and the boot loader's) keeps each command's
/// own help in its grammar.
fn take_help(
    built: &mut [(Release, compile::Grammar)],
) -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut modes: BTreeMap<&str, BTreeSet<Vec<String>>> = BTreeMap::new();
    for c in built.iter().flat_map(|(_, g)| &g.commands) {
        let mut m = c.modes.clone();
        m.sort();
        modes.entry(&c.name).or_default().insert(m);
    }
    let shared: BTreeSet<String> = modes
        .into_iter()
        .filter(|(_, m)| m.len() > 1)
        .map(|(n, _)| n.to_string())
        .collect();
    let mut table: BTreeMap<String, BTreeMap<_, _>> = Default::default();
    built.sort_by_key(|(r, _)| *r);
    for (_, g) in built.iter_mut() {
        for c in &mut g.commands {
            if !c.help.is_empty() && !shared.contains(&c.name) {
                table
                    .entry(c.name.clone())
                    .or_default()
                    .extend(std::mem::take(&mut c.help));
            }
        }
    }
    table
}

fn run(args: &[String]) -> Result<bool, String> {
    match args.first().map(String::as_str) {
        None => {
            // no arguments: the switch selector, then the chosen switch's console
            tui::run(
                |st: &platform::Stack, r: Release| {
                    Ok(cli::Engine::new(grammar(args, st.family(), r)?))
                },
                console::interact,
            )?;
            Ok(true)
        }
        Some("compile") => {
            let (Some(src), Some(out)) = (args.get(1), args.get(2)) else {
                return Err(USAGE.into());
            };
            let g = build_book(
                src,
                arg(args, "--guides"),
                arg(args, "--references"),
                arg(args, "--overrides"),
            )?;
            write_grammar(&g, out)?;
            Ok(true)
        }
        Some("compile-releases") => {
            // every <dir>/cisco-<release>/<platform>-cr.json, with its guides, into <out>/<platform>-<release>.json.gz
            let (Some(dir), Some(out)) = (args.get(1), args.get(2)) else {
                return Err(USAGE.into());
            };
            let mut releases: Vec<(Release, std::path::PathBuf)> = std::fs::read_dir(dir)
                .map_err(|e| format!("{dir}: {e}"))?
                .filter_map(|e| {
                    let p = e.ok()?.path();
                    let name = p.file_name()?.to_str()?.strip_prefix("cisco-")?.to_string();
                    Some((Release::parse(&name)?, p))
                })
                .collect();
            releases.sort();
            // each book's grammars as built, before their help moves to the help table
            let mut done: std::collections::BTreeMap<&str, Vec<(Release, compile::Grammar)>> =
                Default::default();
            // Cisco's general command references, and what each release has of them
            let general_refs: Option<serde_json::Value> = match arg(args, "--references") {
                Some(p) => {
                    let t = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
                    Some(serde_json::from_str(&t).map_err(|e| format!("{p}: {e}"))?)
                }
                None => None,
            };
            let mut general: std::collections::BTreeMap<Release, compile::General> =
                Default::default();
            for (book, _, _) in grammars::BOOKS {
                let mut built = Vec::new();
                for (r, p) in &releases {
                    let cr = p.join(format!("{book}-cr.json"));
                    let cg = p.join(format!("{book}-cg.json"));
                    // a book with guides only: its base book's grammar at this release, projected
                    if let (false, true, Some((_, base))) = (
                        cr.exists(),
                        cg.exists(),
                        grammars::PROJECTIONS.iter().find(|x| x.0 == *book),
                    ) {
                        let Some((_, bg)) = done
                            .get(base)
                            .and_then(|b| b.iter().find(|(br, _)| br == r))
                        else {
                            println!("== {book} {r}: no {base} grammar at {r} to project");
                            continue;
                        };
                        let text = std::fs::read_to_string(&cg).map_err(|e| e.to_string())?;
                        let v: serde_json::Value =
                            serde_json::from_str(&text).map_err(|e| e.to_string())?;
                        // the captures at this release, never another's
                        let at = |suffix: &str| -> Vec<String> {
                            capture_files(out, book, suffix)
                                .into_iter()
                                .filter(|(t, _)| t == r)
                                .map(|(_, c)| c)
                                .collect()
                        };
                        let (mut g, kept, dropped) =
                            compile::project(bg, &v, &at("-running-config.txt"), &at(".log"));
                        let (lines, cmds) = compile::add_guides(&mut g, &v);
                        // the overrides scoped to its platform; the rest came with its base book
                        if let Some(p) = arg(args, "--overrides") {
                            let t = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
                            let mut o: serde_json::Value =
                                serde_json::from_str(&t).map_err(|e| format!("{p}: {e}"))?;
                            if let Some(cs) = o["commands"].as_array_mut() {
                                cs.retain(|c| c["platform"].as_str() == Some(g.platform.as_str()));
                            }
                            let (applied, _) = compile::apply_overrides(
                                &mut g,
                                &o.to_string(),
                                &compile::documented_lines(&v),
                            )?;
                            println!("{} {} overrides", applied.len(), g.platform);
                        }
                        compile::prune(&mut g);
                        println!(
                            "== {book} {r}: {kept} {base} commands its guides attest ({dropped} not), {lines} syntax lines and {cmds} commands from its guides"
                        );
                        built.push((*r, g));
                        continue;
                    }
                    // a release whose command reference is lost and its guides kept (the IE 3010's
                    // 12.2(53)EZ, from the Wayback Machine): the book's next command reference as of
                    // this release, by its Command History
                    if !cr.exists() && cg.exists() {
                        let Some(next) = releases
                            .iter()
                            .filter(|(nr, _)| nr > r)
                            .map(|(_, np)| np.join(format!("{book}-cr.json")))
                            .find(|p| p.exists())
                        else {
                            println!("== {book} {r}: guides, and no command reference after them");
                            continue;
                        };
                        let text = std::fs::read_to_string(&next).map_err(|e| e.to_string())?;
                        let mut v: serde_json::Value =
                            serde_json::from_str(&text).map_err(|e| e.to_string())?;
                        let (dropped, trimmed) = compile::as_of(&mut v, *r);
                        v["release"] = r.to_string().into();
                        println!(
                            "== {book} {r}: {} as of {r}, {} commands it introduces later left out ({}), later keywords left out of {} ({})",
                            next.display(),
                            dropped.len(),
                            dropped.join(", "),
                            trimmed.len(),
                            trimmed.join(", ")
                        );
                        let g = build_reference(
                            &v,
                            Some(&cg.to_string_lossy()),
                            arg(args, "--references"),
                            arg(args, "--overrides"),
                        )?;
                        built.push((*r, g));
                        continue;
                    }
                    if !cr.exists() {
                        continue;
                    }
                    println!("== {book} {r}");
                    let g = build_book(
                        &cr.to_string_lossy(),
                        cg.exists()
                            .then(|| cg.to_string_lossy().to_string())
                            .as_deref(),
                        arg(args, "--references"),
                        arg(args, "--overrides"),
                    )?;
                    built.push((*r, g));
                }
                let log = borrow_enters(&mut built);
                println!(
                    "== {book}: {} submodes from a neighbouring release's examples",
                    log.len()
                );
                for l in &log {
                    println!("{l}");
                }
                // every session the book's switches answered: `<host>-show.log` and the labs'
                // `<host>-session-<name>.log`
                let logs = capture_files(out, book, ".log");
                // general IOS commands the book's own switches and guides attest, as the general
                // references document them at each release
                if let Some(refs) = &general_refs {
                    let configs: Vec<String> = capture_lines(out, book)
                        .into_iter()
                        .map(|(_, c)| c)
                        .collect();
                    let sessions: Vec<String> = logs.iter().map(|(_, c)| c.clone()).collect();
                    let overrides = match arg(args, "--overrides") {
                        Some(p) => {
                            Some(std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?)
                        }
                        None => None,
                    };
                    for (r, g) in &mut built {
                        let gen_r = match general.entry(*r) {
                            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                            std::collections::btree_map::Entry::Vacant(v) => {
                                v.insert(compile::General::new(compile::general_for(
                                    refs,
                                    *r,
                                    overrides.as_deref(),
                                )?))
                            }
                        };
                        let guided = releases
                            .iter()
                            .find(|(x, _)| x == r)
                            .map(|(_, p)| p.join(format!("{book}-cg.json")))
                            .and_then(|p| std::fs::read_to_string(p).ok())
                            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                            .map(|v| compile::prompted_lines(&v))
                            .unwrap_or_default();
                        let names = compile::borrow_general(g, gen_r, &configs, &sessions, &guided);
                        println!(
                            "== {book} {r}: {} general commands its switches and guides attest: {}",
                            names.len(),
                            names.join(", ")
                        );
                    }
                }
                // general IOS commands a capture of the book's switches uses, from the donor book
                // at its newest release not after this one, else its oldest
                if let Some(donor) = grammars::DONORS
                    .iter()
                    .find(|d| d.0 == *book)
                    .and_then(|d| done.get(d.1))
                {
                    let captured = capture_lines(out, book);
                    let documented: Vec<Release> = built.iter().map(|(r, _)| *r).collect();
                    for (r, g) in &mut built {
                        // the captures, and the sessions those switches answered, of the trains this
                        // book serves at the release (`captured_at`)
                        let configs = captured_at(&captured, &documented, *r);
                        let sessions = captured_at(&logs, &documented, *r);
                        let Some((_, dg)) = donor.iter().rfind(|(dr, _)| dr <= r).or(donor.first())
                        else {
                            continue;
                        };
                        let names = compile::borrow_commands(g, dg, &configs, &sessions);
                        println!(
                            "== {book} {r}: {} commands from {} {}: {}",
                            names.len(),
                            dg.platform,
                            dg.release,
                            names.join(", ")
                        );
                    }
                }
                done.insert(book, built.clone());
                let help = take_help(&mut built);
                for (r, g) in &built {
                    write_grammar(g, &format!("{out}/{book}-{r}.json.gz"))?;
                }
                // beside the grammars' folder: data/help/<book>.json.gz
                let dir = std::path::Path::new(out)
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("help");
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                let path = dir.join(format!("{book}.json.gz"));
                write_json(
                    &serde_json::to_string(&help).map_err(|e| e.to_string())?,
                    &path.to_string_lossy(),
                )?;
                println!("help for {} commands -> {}", help.len(), path.display());
            }
            Ok(true)
        }
        Some("examples") => {
            let canonical = args.iter().any(|a| a == "--canonical");
            let show: usize = arg(args, "--show")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let family = platform_arg(args);
            let g = grammar(args, family, release(args, None, grammars::book(family))?)?;
            let examples: Vec<(String, String)> = g
                .commands
                .iter()
                .flat_map(|c| c.examples.iter().map(|e| (e.mode.clone(), e.line.clone())))
                .collect();
            let engine = cli::Engine::new(g);
            let mut counts = std::collections::BTreeMap::new();
            let mut shown = std::collections::BTreeMap::<&str, usize>::new();
            for (mode, line) in &examples {
                let mut outcome = engine.parse(mode, line);
                // a configuration submode hands a line it does not know to global configuration
                if cli::is_submode(mode)
                    && !matches!(outcome, cli::Outcome::Ok(_))
                    && matches!(engine.parse("config", line), cli::Outcome::Ok(_))
                {
                    outcome = engine.parse("config", line);
                }
                let kind = match outcome {
                    cli::Outcome::Ok(p) => {
                        if canonical {
                            println!("({mode}) {line} -> {}", p.canonical());
                        }
                        "accepted"
                    }
                    cli::Outcome::Invalid(_) => "invalid",
                    cli::Outcome::Incomplete => "incomplete",
                    cli::Outcome::Ambiguous => "ambiguous",
                };
                *counts.entry(kind).or_insert(0usize) += 1;
                let n = shown.entry(kind).or_default();
                if kind != "accepted" && *n < show {
                    *n += 1;
                    println!("{kind:10} ({mode}) {line}");
                }
            }
            println!("{} example lines: {counts:?}", examples.len());
            Ok(true)
        }
        Some("run") => {
            let path = args.get(1).ok_or(USAGE)?;
            let text = script_text(path)?;
            let (st, r) = stack(args)?;
            let engine = cli::Engine::new(grammar(args, st.family(), r)?);
            let mut s = shell::Session::new(&engine, device::Device::new(st));
            let (transcript, refused) = s.script(&text);
            if !args.iter().any(|a| a == "--quiet") {
                for l in &transcript {
                    println!("{l}");
                }
            }
            eprintln!("{refused} line(s) refused");
            Ok(refused == 0)
        }
        Some("generate") => {
            // a configuration from a design file, typed into the switch before it is written
            let path = args.get(1).ok_or(USAGE)?;
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let spec: generate::Spec = toml::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
            let problems = generate::check(&spec);
            if !problems.is_empty() {
                for p in &problems {
                    eprintln!("design: {p}");
                }
                return Ok(false);
            }
            let lines = generate::render(&spec);
            let (stack, r) = stack_at(&spec.stack.join(","), args, spec.release.as_deref())?;
            let inactive = generate::inactive_ports(&spec, &stack);
            if !inactive.is_empty() {
                for p in &inactive {
                    eprintln!("design: {p}");
                }
                return Ok(false);
            }
            let engine = cli::Engine::new(grammar(args, stack.family(), r)?);
            let mut s = shell::Session::new(&engine, device::Device::new(stack));
            let script = format!("enable\nconfigure terminal\n{}\n", lines.join("\n"));
            let (transcript, refused) = s.script(&script);
            if refused > 0 {
                // each refused line with the switch's error under it
                for (i, l) in transcript.iter().enumerate() {
                    if l.starts_with('%') {
                        let at = if l.starts_with("% Invalid") {
                            i.saturating_sub(2)
                        } else {
                            i.saturating_sub(1)
                        };
                        for t in &transcript[at..=i] {
                            eprintln!("{t}");
                        }
                    }
                }
                eprintln!("{refused} line(s) refused; nothing written");
                return Ok(false);
            }
            // what each port-channel carries, with `except` lists resolved against the defined VLANs
            let mut defined: std::collections::BTreeSet<u32> =
                spec.vlan.iter().map(|v| v.id).collect();
            defined.extend(
                spec.existing_vlans
                    .as_deref()
                    .map(generate::vlan_ids)
                    .unwrap_or_default(),
            );
            for pc in &spec.port_channel {
                let vs: Vec<String> = generate::carried(&pc.allowed, &defined)
                    .iter()
                    .map(u32::to_string)
                    .collect();
                eprintln!("Port-channel{} carries {}", pc.id, vs.join(","));
            }
            let run_after = show::running_config(&s.device);
            if let Some(ch) = &spec.change {
                // every check is a command the switch takes, and the rollback types in after the change
                let mut bad = Vec::new();
                for c in ch.before.iter().chain(&ch.after) {
                    let cmd = generate::check_command(c);
                    if !matches!(s.type_line(cmd).1, shell::Result::Ok) {
                        bad.push(format!("check '{cmd}' refused"));
                    }
                }
                let (t, n) = s.script(&format!(
                    "configure terminal\n{}\nend\n",
                    ch.rollback.join("\n")
                ));
                if n > 0 {
                    for l in t.iter().filter(|l| l.starts_with('%')) {
                        eprintln!("rollback: {l}");
                    }
                    bad.push(format!("{n} rollback line(s) refused"));
                }
                if !bad.is_empty() {
                    for b in &bad {
                        eprintln!("change: {b}");
                    }
                    return Ok(false);
                }
            }
            let target_text = match (arg(args, "--running"), arg(args, "--target")) {
                (Some(rp), _) => {
                    let running = std::fs::read_to_string(rp).map_err(|e| format!("{rp}: {e}"))?;
                    let mut t = target::Target::new(&running);
                    t.apply(&lines).map_err(|e| format!("target: {e}"))?;
                    if let Some(ch) = &spec.change
                        && let Err(e) = target::restores(&running, &t, &ch.rollback)
                    {
                        // the rollback leads back to the captured configuration, line for line
                        eprintln!("rollback: {e}");
                        return Ok(false);
                    }
                    let name = std::path::Path::new(rp)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let mut head = vec![
                        format!(
                            "! Target configuration for {}: {name} with the change applied. Generated by vswitch, not captured from the switch.",
                            spec.hostname
                        ),
                        "! Allowed VLAN lists are printed resolved, as IOS prints them: a range for three or more consecutive VLANs.".to_string(),
                    ];
                    head.extend(t.notes.iter().map(|n| format!("! {n}")));
                    if running.contains("removed>") {
                        head.push("! Credential lines were removed from the source copy.".into());
                    }
                    head.extend(t.render());
                    Some(head)
                }
                (None, Some(_)) => {
                    let mut head = vec![
                        format!(
                            "! Target configuration for {}: the new switch's running configuration after the change. Generated by vswitch, not captured from the switch.",
                            spec.hostname
                        ),
                        "! It lists the configured lines in IOS section order. IOS also prints its default lines (version, service, platform, license, the Mgmt-vrf definition, the control-plane policy) and every interface without configuration; those are not reproduced.".to_string(),
                    ];
                    head.extend(
                        run_after
                            .iter()
                            .skip_while(|l| !l.starts_with('!'))
                            .cloned(),
                    );
                    Some(head)
                }
                (None, None) => None,
            };
            let config = lines.join("\n") + "\n";
            match arg(args, "--out") {
                Some(out) => {
                    std::fs::write(out, &config).map_err(|e| format!("{out}: {e}"))?;
                    eprintln!("{} lines, all accepted by the switch -> {out}", lines.len());
                }
                None => print!("{config}"),
            }
            if let (Some(out), Some(text)) = (arg(args, "--target"), &target_text) {
                std::fs::write(out, text.join("\n") + "\n").map_err(|e| format!("{out}: {e}"))?;
                eprintln!("target configuration -> {out}");
            }
            if let Some(out) = arg(args, "--change") {
                let c = generate::render_change(&spec, &lines);
                if c.is_empty() {
                    return Err("--change needs a [change] table in the design".into());
                }
                std::fs::write(out, c.join("\n") + "\n").map_err(|e| format!("{out}: {e}"))?;
                eprintln!("change script -> {out}");
            }
            if args.iter().any(|a| a == "--show-run") {
                for l in &run_after {
                    println!("{l}");
                }
            }
            Ok(true)
        }
        Some("explain") => {
            // which command a line matches in a mode, from which source, and the mode it enters
            let (Some(mode), Some(line)) = (args.get(1), args.get(2)) else {
                return Err(
                    "usage: vswitch explain <mode> <line> [--platform <book or model>] [--release <x.y>]"
                        .into(),
                );
            };
            let family = platform_arg(args);
            let r = release(args, None, grammars::book(family))?;
            let engine = cli::Engine::new(grammar(args, family, r)?);
            match engine.parse(mode, line) {
                cli::Outcome::Ok(p) => {
                    let c = &engine.grammar.commands[p.command];
                    println!(
                        "command  {}\nsource   {}\nline     {}\nmodes    {:?}\nenters   {:?}\nshown as {}",
                        c.name,
                        c.source,
                        c.lines[p.line].text,
                        c.modes,
                        c.enters,
                        p.canonical()
                    );
                    if let Some(e) = &c.evidence {
                        println!("evidence {e}");
                    }
                }
                other => println!("{other:?}"),
            }
            Ok(true)
        }
        Some("load") => {
            // into the switch list, as the selector's `a` does, without opening the console
            let path = args.get(1).ok_or(USAGE)?;
            let dir =
                library::Library::default_dir().ok_or("no data directory: set VSWITCH_HOME")?;
            let mut lib = library::Library::open(dir)?;
            println!("{}", load_into(&mut lib, path, args)?);
            Ok(true)
        }
        Some("sweep") => {
            // `--out` always names a folder: a sweep asked for copies never cleans the originals
            let mut out: Option<std::path::PathBuf> = None;
            let mut roots: Vec<std::path::PathBuf> = Vec::new();
            let mut i = 1;
            while i < args.len() {
                let a = &args[i];
                let folder = if a == "--out" {
                    i += 1;
                    args.get(i)
                        .filter(|f| !f.starts_with("--"))
                        .map(String::as_str)
                } else if let Some(f) = a.strip_prefix("--out=") {
                    Some(f).filter(|f| !f.is_empty())
                } else if a.starts_with("--") {
                    return Err(format!(
                        "sweep: {a} is not an option of sweep, which takes --out <folder>"
                    ));
                } else {
                    roots.push(a.into());
                    i += 1;
                    continue;
                };
                let folder =
                    folder.ok_or("sweep: --out names the folder for the cleaned copies")?;
                if out.replace(folder.into()).is_some() {
                    return Err("sweep: --out is given twice".into());
                }
                i += 1;
            }
            if roots.is_empty() {
                let dir = tui::sweep_folder()?;
                println!(
                    "Sweep folder: {} (put configuration files there and run vswitch sweep)",
                    dir.display()
                );
                roots.push(dir);
            }
            // a path not found and a file left alone fail the run: neither is a clean file
            let mut clean = true;
            for root in &roots {
                let reports = match sweep::sweep_path(root, out.as_deref()) {
                    Ok(r) => r,
                    Err(e) => {
                        println!("{e}");
                        clean = false;
                        continue;
                    }
                };
                for f in reports {
                    match &f.outcome {
                        Ok(s) => {
                            println!("{}: {}", f.path.display(), s.summary());
                            if s.left > 0 {
                                println!(
                                    "  {} removed value(s) still appear elsewhere in the file",
                                    s.left
                                );
                                clean = false;
                            }
                        }
                        Err(e) => {
                            println!("{}: {e}", f.path.display());
                            clean = false;
                        }
                    }
                }
            }
            Ok(clean)
        }
        Some("lint") => {
            // a configuration typed in at the release it runs, then checked as it stands
            let path = args.get(1).ok_or(USAGE)?;
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let text = text.trim_start_matches('\u{feff}');
            let given = arg(args, "--stack");
            let spec = given
                .map(str::to_string)
                .or_else(|| platform::infer_stack(text))
                .ok_or(format!(
                    "{path}: give its hardware with --stack; its interfaces fit no model"
                ))?;
            let stack = platform::parse_stack(&spec).map_err(|e| format!("--stack: {e}"))?;
            let r = release_at(args, release::version_line(text), &stack)?;
            let engine = cli::Engine::new(grammar(args, stack.family(), r)?);
            let (s, typed) = check::type_config(&engine, stack, text);
            let refused = typed.iter().filter(|t| t.error.is_some()).count();
            // hardware read from the configuration says nothing of the module fitted
            let report = if given.is_some() {
                lint::lint(&s.device)
            } else {
                lint::lint_inferred(&s.device)
            };
            println!(
                "{path}: {spec}, {}, {} lines typed ({refused} not modelled)",
                r.named(),
                typed.len()
            );
            for f in &report.findings {
                println!("{f}");
            }
            for n in &report.notes {
                println!("note: {n}");
            }
            for k in &report.skipped {
                println!("not checked: {k}");
            }
            println!(
                "{} finding(s), {} note(s)",
                report.findings.len(),
                report.notes.len()
            );
            Ok(report.findings.is_empty())
        }
        Some("check") => {
            // a configuration at the release it runs and at the one it would move to
            options(
                args,
                &["--to", "--from", "--stack", "--grammar"],
                &["--all"],
            )?;
            let path = args.get(1).ok_or(USAGE)?;
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let text = text.trim_start_matches('\u{feff}');
            let spec = arg(args, "--stack")
                .map(str::to_string)
                .or_else(|| platform::infer_stack(text))
                .ok_or(format!(
                    "{path}: give its hardware with --stack; its interfaces fit no model"
                ))?;
            let stack = platform::parse_stack(&spec).map_err(|e| format!("--stack: {e}"))?;
            let book = grammars::book(stack.family());
            let from = match arg(args, "--from") {
                Some(t) => grammars::resolve(t, book)?,
                None => {
                    let v = release::version_line(text)
                        .ok_or(format!("{path}: no version line; give --from"))?;
                    // classic IOS's `version 15.2` names no train: not the release the switch runs
                    if Release::parse(v).is_some_and(|r| !r.classic() && r.major() < 16) {
                        return Err(format!(
                            "{path}: its version line ({v}) names no train; give --from"
                        ));
                    }
                    grammars::resolve(v, book)?
                }
            };
            let to = grammars::resolve(arg(args, "--to").ok_or("check needs --to <x.y>")?, book)?;
            let all = args.iter().any(|a| a == "--all");
            println!("{path}: {spec}, {} -> {to}", from.named());
            let hardware = stack.problems(to);
            for (n, why) in &hardware {
                println!("{}", hardware_line(*n, why));
            }
            let at = |r: Release| -> Result<Vec<check::Typed>, String> {
                let engine = cli::Engine::new(grammar(args, stack.family(), r)?);
                Ok(check::type_config(&engine, stack.clone(), text).1)
            };
            let report = check::compare(&at(from)?, &at(to)?);
            let show = |title: String, lines: &[check::Typed], limit: usize| {
                println!("{title} ({}):", lines.len());
                for t in lines.iter().take(limit) {
                    println!(
                        "  line {}: {}   {}",
                        t.number,
                        t.line,
                        t.error.as_deref().unwrap_or("")
                    );
                }
                if lines.len() > limit {
                    println!("  ... {} more (--all lists them)", lines.len() - limit);
                }
            };
            let cap = if all { usize::MAX } else { 12 };
            show(
                format!("Refused at {to}, taken at {from}"),
                &report.breaks,
                usize::MAX,
            );
            show(
                format!("Taken at {to}, refused at {from}"),
                &report.gains,
                cap,
            );
            show(
                "Refused at both: vswitch models neither".to_string(),
                &report.neither,
                cap,
            );
            show(
                format!(
                    "Taken at {from} because a real switch held it, not in the {to} books: a capture at {to} settles them"
                ),
                &report.unproven,
                cap,
            );
            for (r, lines) in [(from, &report.unreached.0), (to, &report.unreached.1)] {
                if !lines.is_empty() {
                    show(
                        format!("Not typed at {r}: its session had left configuration mode"),
                        lines,
                        cap,
                    );
                }
            }
            println!(
                "{} configuration lines typed at both releases",
                report.lines
            );
            Ok(report.breaks.is_empty() && hardware.is_empty())
        }
        Some("verify") if args.iter().any(|a| a == "--outputs") => {
            for l in evidence::unverified() {
                println!("{l}");
            }
            Ok(true)
        }
        Some("verify") => {
            // each line's evidence: a real switch held it, used the command, or only the books give it
            options(args, &["--stack", "--release", "--grammar"], &[])?;
            let path = args.get(1).ok_or(USAGE)?;
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let text = text.trim_start_matches('\u{feff}');
            let spec = arg(args, "--stack")
                .map(str::to_string)
                .or_else(|| platform::infer_stack(text))
                .ok_or(format!(
                    "{path}: give its hardware with --stack; its interfaces fit no model"
                ))?;
            let stack = platform::parse_stack(&spec).map_err(|e| format!("--stack: {e}"))?;
            let book = grammars::book(stack.family());
            let r = release(args, release::version_line(text), book)?;
            let engine = cli::Engine::new(grammar(args, stack.family(), r)?);
            let (lines, captures) = verify::classify(&engine, stack, text);
            let count = |e: verify::Evidence| lines.iter().filter(|l| l.evidence == e).count();
            println!("{path}: {spec}, {}", r.named());
            if captures.is_empty() {
                println!(
                    "no capture of a {} at {r} yet: every line comes from the books",
                    grammars::book_name(book)
                );
            } else {
                println!("weighed against: {}", captures.join(", "));
            }
            println!(
                "held by a real switch: {}; the same command in another form: {}; from the books only: {}",
                count(verify::Evidence::Held),
                count(verify::Evidence::Form),
                count(verify::Evidence::Books)
            );
            let script = verify::script(&lines);
            if !script.is_empty() {
                println!(
                    "! from the books only: type these on a switch of this platform and release"
                );
                for l in script {
                    println!("{l}");
                }
            }
            Ok(true)
        }
        Some("capture") => {
            options(
                args,
                &[
                    "--out",
                    "--pending",
                    "--folder",
                    "--log",
                    "--config",
                    "--key",
                ],
                &[],
            )?;
            let out = std::path::Path::new(arg(args, "--out").unwrap_or("."));
            let pending = arg(args, "--pending").map(std::path::Path::new);
            // a folder of session logs and a backup tool's files: a capture per switch they name,
            // each file read for what it holds whatever its name
            if let Some(dir) = arg(args, "--folder") {
                let mut say = |l: String| println!("{l}");
                return capture_folder(std::path::Path::new(dir), out, pending, &mut say);
            }
            // a real switch's show output and running configuration, over SSH or from session logs
            // and a backup tool's output, one after another; `--config` a configuration file
            let logs: Vec<&String> = args
                .windows(2)
                .filter(|w| w[0] == "--log")
                .map(|w| &w[1])
                .collect();
            let mut transcript = if logs.is_empty() {
                let target = args.get(1).filter(|a| !a.starts_with("--")).ok_or(USAGE)?;
                capture::session(target, arg(args, "--key"), capture::COMMANDS)?
            } else {
                let mut t = String::new();
                for p in logs {
                    let bytes = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
                    t.push_str(&capture::normalize(&capture::text_of(&bytes)));
                }
                t
            };
            // `--config` stands where the session printed no configuration of its own
            if let Some(p) = arg(args, "--config") {
                let bytes = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
                let text = capture::text_of(&bytes);
                if capture::from_transcript(&transcript).is_ok_and(|c| !c.running.is_empty()) {
                    println!("{p}: not used; the session printed its own configuration");
                } else {
                    transcript = capture::with_config(&transcript, &text)
                        .ok_or("--config needs a session log with the switch's prompt")?;
                }
            }
            let c = capture::from_transcript(&transcript)?;
            if c.running.is_empty() {
                return Err("the transcript prints no configuration".into());
            }
            println!("{}: {}", c.host, capture_identity(&c));
            for u in &c.unknown {
                println!("not in vswitch's catalog: {u}");
            }
            println!("swept: {}", c.swept);
            for f in capture::write(&c, out, pending)? {
                println!("-> {}", f.display());
            }
            // the switch's own configuration typed into vswitch at its release
            if let Some(r) = capture_typed(&c) {
                let (typed, refused) = r?;
                println!(
                    "{typed} configuration lines typed; vswitch refuses {} (`vswitch verify` on the file lists each line's evidence)",
                    refused.len()
                );
                for t in refused.iter().take(20) {
                    println!("  line {}: {}", t.number, t.line);
                }
            }
            Ok(true)
        }
        Some("probe") => {
            // a script typed on a real lab switch, compared line by line with vswitch
            if !args.iter().any(|a| a == "--lab") {
                return Err(
                    "probe types configuration on the switch: give --lab, for a private lab reservation only"
                        .into(),
                );
            }
            let target = args.get(1).filter(|a| !a.starts_with("--")).ok_or(USAGE)?;
            let path = args.get(2).filter(|a| !a.starts_with("--")).ok_or(USAGE)?;
            let key = arg(args, "--key");
            let skip = args.iter().any(|a| a == "--skip-unsafe");
            // `--map Twe1/0/=Gi1/0/` renames the script's ports onto the lab switch's
            let maps: Vec<(String, String)> = args
                .windows(2)
                .filter(|w| w[0] == "--map")
                .filter_map(|w| w[1].split_once('=').map(|(a, b)| (a.into(), b.into())))
                .collect();
            let run = match probe::sessions(
                &script_text(path)?,
                &maps,
                &probe::addresses(target),
                skip,
                |commands| capture::session(target, key, commands),
            )? {
                probe::Probed::Refused(bad) => {
                    for (n, why) in &bad {
                        println!("{path} line {n}: {why}");
                    }
                    println!(
                        "nothing sent: {} line(s) refused (--skip-unsafe leaves them out)",
                        bad.len()
                    );
                    return Ok(false);
                }
                probe::Probed::Ran(run) => run,
            };
            if !run.dropped.is_empty() {
                println!("{} line(s) left out:", run.dropped.len());
                for (n, why) in &run.dropped {
                    println!("  {path} line {n}: {why}");
                }
            }
            let stack = platform::parse_stack(&run.spec).map_err(|e| format!("stack: {e}"))?;
            let book = grammars::book(stack.family());
            let r = grammars::resolve(&run.version, book)?;
            let engine = cli::Engine::new(grammars::grammar(book, r)?);
            let report = probe::compare(&engine, stack, &run.before, &run.exchanges);
            println!(
                "{}: {}, {}; {} script line(s) typed",
                run.host,
                run.spec,
                probe::named(&run.version),
                report.lines
            );
            for (l, e) in &report.vswitch_refuses {
                println!("the switch took, vswitch refuses: {l}   {e}");
            }
            for (l, e) in &report.vswitch_takes {
                println!("the switch refused, vswitch takes: {l}   {e}");
            }
            for l in &report.only_ios {
                println!("IOS kept, vswitch does not: {l}");
            }
            for l in &report.only_vswitch {
                println!("vswitch keeps, IOS did not: {l}");
            }
            if report.restored {
                println!("configure replace: the configuration is back as it was");
            } else {
                println!(
                    "configure replace: no `Rollback Done` in its output, so the switch may hold the \
                     script's lines; `configure replace {} force` at its console puts it back",
                    probe::ARCHIVE
                );
            }
            // the probe's sessions and the configuration IOS kept, beside the switch's capture
            let dir = std::path::Path::new(arg(args, "--out").unwrap_or("."))
                .join(probe::folder(book, &run.version));
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let stem = std::path::Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "script".into());
            for (name, text) in [
                (format!("{}-probe-{stem}.log", run.host), &run.log),
                (
                    format!("{}-probe-{stem}-running-config.txt", run.host),
                    &report.after,
                ),
            ] {
                let f = dir.join(name);
                std::fs::write(&f, sweep::sweep(text).text)
                    .map_err(|e| format!("{}: {e}", f.display()))?;
                println!("-> {}", f.display());
            }
            Ok(report.clean())
        }
        Some("lab") => {
            // switches cabled together, each cable brought up from both ends
            let path = args.get(1).ok_or(USAGE)?;
            let (switches, cables) = lab::prepare(std::path::Path::new(path))?;
            let mut built: Vec<lab::Built> = live::par_map(&switches, lab::build);
            for (sw, b) in switches.iter().zip(&built) {
                println!(
                    "{}: {}, {}, {} lines typed ({} not modelled)",
                    sw.name,
                    sw.stack_spec,
                    sw.release.named(),
                    b.typed,
                    b.refused.len()
                );
            }
            let report = lab::connect(&mut built, &cables)?;
            for l in &report.links {
                println!("{l}");
            }
            for f in &report.findings {
                println!("{f}");
            }
            for n in &report.notes {
                println!("note: {n}");
            }
            println!(
                "{} cable(s), {} finding(s), {} note(s)",
                cables.len(),
                report.findings.len(),
                report.notes.len()
            );
            if let Some(name) = arg(args, "--shell") {
                let b = built
                    .iter_mut()
                    .find(|b| b.name == name)
                    .ok_or(format!("--shell: no switch named {name} in {path}"))?;
                console::interact(&mut b.session)?;
            }
            Ok(report.findings.is_empty())
        }
        Some("changes") => {
            options(args, &["--captures"], &[])?;
            let live = std::path::Path::new(arg(args, "--captures").ok_or(USAGE)?);
            let mut files = vec![];
            // every backup named, wherever the option stands among them
            for a in operands(args, &["--captures"]) {
                let p = std::path::PathBuf::from(a);
                if p.is_dir() {
                    files.extend(
                        std::fs::read_dir(&p)
                            .map_err(|e| format!("{a}: {e}"))?
                            .flatten()
                            .map(|e| e.path())
                            .filter(|p| p.is_file()),
                    );
                } else {
                    files.push(p);
                }
            }
            if files.is_empty() {
                return Err(USAGE.into());
            }
            let mut clean = true;
            // a file that names no switch is reported, not passed over as unchanged
            for f in &files {
                let bytes = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
                if capture::inputs(&capture::text_of(&bytes)).is_empty() {
                    clean = false;
                    println!(
                        "{}: no configuration (a `version` and a `hostname` line) or session read in it",
                        f.display()
                    );
                }
            }
            for (host, c) in capture::from_files(files)? {
                let r = c.and_then(|c| {
                    if c.running.trim().is_empty() {
                        Err("its files print no configuration".into())
                    } else {
                        changes::report(live, &host, &c.running)
                    }
                });
                match r {
                    Ok(r) => {
                        clean &= r.clean();
                        for l in r.lines() {
                            println!("{l}");
                        }
                    }
                    Err(e) => {
                        clean = false;
                        println!("{host}: {e}");
                    }
                }
            }
            Ok(clean)
        }
        Some("fleet") => {
            // every captured switch checked at the release it would move to
            options(args, &["--to"], &["--all"])?;
            let dir = args.get(1).ok_or(USAGE)?;
            let targets: Vec<&str> = args
                .windows(2)
                .filter(|w| w[0] == "--to")
                .map(|w| w[1].as_str())
                .collect();
            if targets.is_empty() {
                return Err("fleet needs --to <x.y>, once per platform's release".into());
            }
            let all = args.iter().any(|a| a == "--all");
            let caps = topology::read(std::path::Path::new(dir))?;
            let (mut clear, mut breaking, mut hardware_out, mut no_book, mut there, mut unchecked) =
                (0, 0, 0, 0, 0, 0);
            let mut rows = Vec::new();
            // each capture that moves: its host, text, hardware and the two releases
            let mut moves = Vec::new();
            for c in &caps {
                // a capture that cannot be read or placed is a row of its own; the rest are checked
                let mut skip = |why: String| {
                    unchecked += 1;
                    rows.push((usize::MAX, format!("{}: {why}", c.host)));
                };
                let text = match std::fs::read_to_string(&c.config) {
                    Ok(t) => t,
                    Err(e) => {
                        skip(format!("{}: {e}", c.config.display()));
                        continue;
                    }
                };
                let Some(spec) = c.stack.clone().or_else(|| platform::infer_stack(&text)) else {
                    skip("its hardware is unknown".into());
                    continue;
                };
                let stack = match platform::parse_stack(&spec) {
                    Ok(s) => s,
                    Err(e) => {
                        skip(e);
                        continue;
                    }
                };
                let book = grammars::book(stack.family());
                let from = match grammars::resolve(&c.release, book) {
                    Ok(r) => r,
                    Err(e) => {
                        skip(format!("{}: {e}", c.config.display()));
                        continue;
                    }
                };
                // the first release its book documents and its hardware runs, else the first documented
                let documented: Vec<Release> = targets
                    .iter()
                    .filter_map(|t| grammars::resolve(t, book).ok())
                    .collect();
                let Some(to) = documented
                    .iter()
                    .find(|r| stack.problems(**r).is_empty())
                    .or(documented.first())
                    .copied()
                else {
                    no_book += 1;
                    continue;
                };
                if to == from {
                    there += 1;
                    continue;
                }
                moves.push((c.host.clone(), text, spec, stack, book, from, to));
            }
            // each platform release's engine built once, and the captures typed over every thread
            let wanted: Vec<(&'static str, Release)> = moves
                .iter()
                .flat_map(|m| [(m.4, m.5), (m.4, m.6)])
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            let built = live::par_map(&wanted, |&(book, r)| {
                grammars::grammar(book, r).map(cli::Engine::new)
            });
            let mut engines = std::collections::BTreeMap::new();
            for (k, e) in wanted.into_iter().zip(built) {
                engines.insert(k, e?);
            }
            let compared = live::par_map(&moves, |(_, text, _, stack, book, from, to)| {
                let at =
                    |r: Release| check::type_config(&engines[&(*book, r)], stack.clone(), text).1;
                check::compare(&at(*from), &at(*to))
            });
            for ((host, _, spec, stack, _, from, to), report) in moves.iter().zip(compared) {
                let hardware = stack.problems(*to);
                if !hardware.is_empty() {
                    hardware_out += 1;
                } else if report.breaks.is_empty() {
                    clear += 1;
                } else {
                    breaking += 1;
                }
                let mut row = format!(
                    "{host} ({spec}) {} -> {to}: {} line(s) break, {} unproven at {to}",
                    from.named(),
                    report.breaks.len(),
                    report.unproven.len()
                );
                for (n, why) in &hardware {
                    row.push_str(&format!("\n  {}", hardware_line(*n, why)));
                }
                for b in report.breaks.iter().take(if all { usize::MAX } else { 5 }) {
                    row.push_str(&format!("\n  line {}: {}", b.number, b.line));
                }
                if !all && report.breaks.len() > 5 {
                    row.push_str(&format!("\n  ... {} more (--all)", report.breaks.len() - 5));
                }
                rows.push((report.breaks.len() + 1000 * hardware.len(), row));
            }
            rows.sort_by_key(|r| std::cmp::Reverse(r.0));
            for (_, r) in &rows {
                println!("{r}");
            }
            println!(
                "{} captures: {clear} move with no line breaking, {breaking} with lines that break, {hardware_out} on hardware the release does not run, {there} already there, {no_book} with no book for {}, {unchecked} not checked",
                caps.len(),
                targets.join(" or ")
            );
            Ok(breaking == 0 && hardware_out == 0)
        }
        Some("topology") => {
            options(args, &["--out"], &[])?;
            let dir = args.get(1).ok_or(USAGE)?;
            let out = arg(args, "--out").unwrap_or("plant-lab.toml");
            // a switch captured before and after an upgrade is one switch: its newest capture
            let (caps, left) = topology::current(topology::read(std::path::Path::new(dir))?);
            let linked = topology::link(&caps);
            let lab_dir = std::path::Path::new(out)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            std::fs::write(out, topology::lab_file(&caps, &linked, lab_dir))
                .map_err(|e| format!("{out}: {e}"))?;
            for r in left.iter().chain(&linked.report) {
                println!("{r}");
            }
            let cabled: std::collections::BTreeSet<&str> = linked
                .cables
                .iter()
                .flat_map(|(a, b, _)| [a, b])
                .filter_map(|e| e.split(':').next())
                .collect();
            println!(
                "{} captures, {} cabled, {} cable(s) -> {out}",
                caps.len(),
                cabled.len(),
                linked.cables.len()
            );
            Ok(true)
        }
        Some("corpus") => {
            // what the captures teach, compiled: the table set the binary embeds
            options(args, &["--out", "--scan"], &["--why", "--loose"])?;
            let loose = args.iter().any(|a| a == "--loose");
            let dir = args.get(1).ok_or(USAGE)?;
            let out = arg(args, "--out").unwrap_or("data/corpus.json.gz");
            let src = corpus::Source::read(std::path::Path::new(dir))?;
            let deny = corpus::Deny::of(&src);
            if let Some(tree) = arg(args, "--scan") {
                // every published file under the tree that names the site, by file, line and word
                let hits = corpus::scan_tree(std::path::Path::new(tree), &deny, loose);
                for (file, line, word) in &hits {
                    println!("{file}:{line}: {word}");
                }
                println!("{} lines name the site", hits.len());
                return Ok(hits.is_empty());
            }
            if args.iter().any(|a| a == "--why") {
                // which denied word took out each `?` answer row
                let mut seen = std::collections::BTreeMap::new();
                for (_, _, t) in &corpus::build(&src).held {
                    let answers = t
                        .help
                        .iter()
                        .map(|(_, _, a)| a)
                        .chain(t.crawl.iter().map(|(.., a)| a));
                    for a in answers {
                        for r in a {
                            if let Some(w) = deny.hit(r) {
                                *seen.entry(w).or_insert(0) += 1;
                            }
                        }
                    }
                }
                for (w, n) in seen {
                    println!("{n} {w}");
                }
                return Ok(true);
            }
            for f in &src.crawls {
                println!("{}-{}/{}: {}", f.book, f.release, f.folder, f.level);
            }
            for f in &src.unleveled {
                println!("{f}: no level file, not read");
            }
            let built = corpus::with_forms(corpus::build(&src), &src)?;
            let crawled = |c: &corpus::Corpus| c.held.iter().map(|(_, _, t)| t.crawl.len()).sum();
            let asked: usize = crawled(&built);
            let c = corpus::filtered(built, &deny);
            let kept: usize = crawled(&c);
            let leaks = corpus::leaks(&c, &deny);
            if !leaks.is_empty() {
                return Err(format!(
                    "{} corpus entries still name the site; nothing written",
                    leaks.len()
                ));
            }
            std::fs::write(out, c.to_gzip()).map_err(|e| format!("{out}: {e}"))?;
            println!(
                "{} running configurations, {} session logs, {} crawl logs ({kept} of {asked} answers kept), {} site words left out -> {out}",
                src.running.len(),
                src.logs.len(),
                src.crawls.len(),
                deny.len()
            );
            Ok(true)
        }
        #[cfg(captures)]
        Some("coverage") => {
            // the lines real switches held that their books alone refuse
            let caps = live::captures();
            let roots = coverage::all_roots(&caps)?;
            if args.iter().any(|a| a == "--baseline") {
                print!("{}", coverage::baseline(&caps, &roots));
            } else if args.iter().any(|a| a == "--all") {
                for r in &roots {
                    println!(
                        "{} {} {} line {}: {}   {}",
                        r.book, r.release, r.host, r.number, r.text, r.error
                    );
                }
            } else {
                for l in coverage::report(&caps, &roots) {
                    println!("{l}");
                }
                println!("{} roots in {} captures", roots.len(), caps.len());
            }
            Ok(true)
        }
        Some("releases") => {
            for (book, _, _) in grammars::BOOKS {
                let list: Vec<String> = grammars::documented(book)
                    .iter()
                    .map(Release::to_string)
                    .collect();
                println!("{}: {}", grammars::book_name(book), list.join(" "));
            }
            Ok(true)
        }
        Some("shell") => {
            let (st, r) = stack(args)?;
            let engine = cli::Engine::new(grammar(args, st.family(), r)?);
            let mut s = shell::Session::new(&engine, device::Device::new(st));
            console::interact(&mut s)?;
            Ok(true)
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(engine: &cli::Engine) -> shell::Session<'_> {
        shell::Session::new(engine, device::Device::new(stack(&[]).unwrap().0))
    }

    fn engine() -> cli::Engine {
        cli::Engine::new(
            serde_json::from_str(&grammars::text("c9300", release::DEFAULT).unwrap()).unwrap(),
        )
    }

    #[test]
    fn the_cluster_draft_types_in_with_nothing_refused() {
        let e = engine();
        let mut s = session(&e);
        let (transcript, refused) = s.script(include_str!("../tests/example-draft.txt"));
        assert_eq!(refused, 0, "{}", transcript.join("\n"));
        let run = show::running_config(&s.device).join("\n");
        for want in [
            "hostname dc1-stack-sw01",
            "spanning-tree vlan 1-4094 priority 61440",
            "interface Port-channel11\n description pve-h1-proxhost bond0",
            "interface GigabitEthernet0/0\n vrf forwarding Mgmt-vrf\n ip address 203.0.113.150 255.255.255.0",
            "interface TwentyFiveGigE1/0/14\n shutdown",
            "ntp server vrf Mgmt-vrf 192.0.2.121\nntp server vrf Mgmt-vrf 192.0.2.122",
        ] {
            assert!(run.contains(want), "missing {want:?} in\n{run}");
        }
        assert!(
            !run.contains("interface range"),
            "a range is written to its members"
        );
    }

    #[test]
    fn what_the_9300_sources_do_not_document_is_refused_with_the_caret_at_the_error() {
        let e = engine();
        let mut s = session(&e);
        s.script("enable\nconfigure terminal\ninterface TwentyFiveGigE1/0/1");
        // interface-level portfast is documented as `spanning-tree portfast [trunk]`
        let (out, r) = s.type_line(" spanning-tree portfast edge trunk");
        assert_eq!(r, shell::Result::Error);
        assert_eq!(out[1], "% Invalid input detected at '^' marker.");
        assert_eq!(
            out[0].find('^'),
            Some(s.prompt().len() + " spanning-tree portfast ".len())
        );
        // a port the two-member 24Y stack does not have
        let (out, r) = s.type_line("interface TwentyFiveGigE1/0/25");
        assert_eq!(r, shell::Result::Error, "{out:?}");
        let (_, r) = s.type_line("interface TwentyFiveGigE3/0/1");
        assert_eq!(r, shell::Result::Error);
        let (_, r) = s.type_line("interface HundredGigE2/1/4");
        assert_eq!(r, shell::Result::Ok);
        let (out, r) = s.type_line(" switchport access vlan");
        assert_eq!(
            (out[0].as_str(), r),
            ("% Incomplete command.", shell::Result::Error)
        );
    }
}

#[cfg(test)]
mod generate_tests {
    use super::*;

    fn typed(spec: &generate::Spec) -> (shell::Session<'static>, usize) {
        let e: &'static cli::Engine = Box::leak(Box::new(cli::Engine::new(
            serde_json::from_str(&grammars::text("c9300", release::DEFAULT).unwrap()).unwrap(),
        )));
        let st = stack(&["--stack".into(), spec.stack.join(",")]).unwrap().0;
        let mut s = shell::Session::new(e, device::Device::new(st));
        let lines = generate::render(spec);
        let (_, refused) = s.script(&format!(
            "enable\nconfigure terminal\n{}\n",
            lines.join("\n")
        ));
        (s, refused)
    }

    #[test]
    fn the_cluster_design_generates_a_configuration_the_switch_accepts_whole() {
        let spec: generate::Spec =
            toml::from_str(include_str!("../tests/example-design.toml")).unwrap();
        assert!(
            generate::check(&spec).is_empty(),
            "{:?}",
            generate::check(&spec)
        );
        let (mut s, refused) = typed(&spec);
        assert_eq!(refused, 0);
        let run = show::running_config(&s.device).join("\n");
        assert!(run.contains("interface Port-channel100\n description MDF SN2410M pair Mpo3\n switchport trunk native vlan 499\n switchport trunk allowed vlan 451,452,456,459"));
        assert!(run.contains("interface HundredGigE2/1/1\n description HP-SN2410M-SW-B Eth1/49"));
        assert_eq!(
            run.matches("channel-group").count(),
            26,
            "13 port-channels of two members"
        );
        // the account lines the generated comment names are ones the switch takes
        s.type_line("configure terminal");
        for l in [
            "username admin privilege 15",
            "username admin algorithm-type scrypt secret masked-secret",
        ] {
            assert_eq!(s.type_line(l).1, shell::Result::Ok, "{l}");
        }
    }

    #[test]
    fn a_new_stack_leaves_vtp_before_it_defines_vlans() {
        let mut spec: generate::Spec =
            toml::from_str(include_str!("../tests/example-design.toml")).unwrap();
        let lines = generate::render(&spec);
        let at = |p: &str| lines.iter().position(|l| l.starts_with(p));
        assert!(at("vtp mode transparent").unwrap() < at("vlan ").unwrap());
        let (s, refused) = typed(&spec);
        assert_eq!(refused, 0);
        let run = show::running_config(&s.device).join("\n");
        assert!(run.contains("\nvtp mode transparent\n"), "{run}");
        // as IOS prints it: an except list resolved, and no one-time key generation
        assert!(
            run.contains("interface Port-channel21\n description pve-h1-proxhost bond1\n switchport trunk native vlan 499\n switchport trunk allowed vlan 1-450,454-458,460-4094\n"),
            "{run}"
        );
        assert!(!run.contains("crypto key generate"));
        spec.vtp_mode = None;
        assert!(
            generate::check(&spec)
                .iter()
                .any(|p| p.contains("set vtp_mode"))
        );
        // a change to a switch already in a VTP domain does not set the mode
        let core: generate::Spec =
            toml::from_str(include_str!("../tests/example-core-design.toml")).unwrap();
        assert!(core.vtp_mode.is_none() && generate::check(&core).is_empty());
    }

    #[test]
    fn a_design_with_a_port_the_stack_lacks_is_refused_before_anything_is_written() {
        let mut spec: generate::Spec =
            toml::from_str(include_str!("../tests/example-design.toml")).unwrap();
        spec.port_channel[0].members[0].port = "Twe1/0/25".into();
        let (_, refused) = typed(&spec);
        assert!(refused > 0);
    }
}

#[cfg(test)]
mod core_tests {
    use super::*;

    fn refused(spec: &generate::Spec, lines: &[String]) -> (usize, String) {
        let e: &'static cli::Engine = Box::leak(Box::new(cli::Engine::new(
            serde_json::from_str(&grammars::text("c9300", release::DEFAULT).unwrap()).unwrap(),
        )));
        let st = stack(&["--stack".into(), spec.stack.join(",")]).unwrap().0;
        let mut s = shell::Session::new(e, device::Device::new(st));
        let (transcript, refused) = s.script(&format!(
            "enable\nconfigure terminal\n{}\n",
            lines.join("\n")
        ));
        (refused, transcript.join("\n"))
    }

    #[test]
    fn the_cluster_core_change_uses_its_free_ports_and_types_in_whole() {
        let spec: generate::Spec =
            toml::from_str(include_str!("../tests/example-core-design.toml")).unwrap();
        assert!(
            generate::check(&spec).is_empty(),
            "{:?}",
            generate::check(&spec)
        );
        let st = stack(&["--stack".into(), spec.stack.join(",")]).unwrap().0;
        assert!(generate::inactive_ports(&spec, &st).is_empty());
        let lines = generate::render(&spec);
        assert!(
            !lines.iter().any(|l| l.starts_with("switch ")),
            "no provisioning for a core of another model"
        );
        assert_eq!(lines.iter().filter(|l| l.contains("sample")).count(), 0);
        let text = lines.join("\n");
        assert!(
            text.contains(
                "interface TenGigabitEthernet2/0/3\n description dc1-stack-sw01 Twe1/0/24"
            )
        );
        assert!(
            text.contains(
                "interface TenGigabitEthernet2/0/4\n description dc1-stack-sw01 Twe2/0/24"
            )
        );
        for p in 19..=26 {
            let at = text
                .find(&format!("interface GigabitEthernet5/0/{p}\n"))
                .unwrap_or_else(|| panic!("Gi5/0/{p}"));
            let block: Vec<&str> = text[at..].lines().skip(1).take(6).collect();
            assert_eq!(
                block[2..],
                [
                    " switchport mode access",
                    " spanning-tree portfast",
                    " spanning-tree bpduguard enable",
                    " no shutdown"
                ],
                "Gi5/0/{p} is an edge port as the core's VLAN 450 ports are"
            );
        }
        let (n, transcript) = refused(&spec, &lines);
        assert_eq!(n, 0, "{transcript}");
    }

    #[test]
    fn a_member_on_an_inactive_supervisor_port_is_a_design_problem() {
        let mut spec: generate::Spec =
            toml::from_str(include_str!("../tests/example-core-design.toml")).unwrap();
        let st = stack(&["--stack".into(), spec.stack.join(",")]).unwrap().0;
        spec.port_channel[0].members[0].port = "Te3/0/5".into();
        let p = generate::inactive_ports(&spec, &st);
        assert!(
            p.len() == 1 && p[0].starts_with("Te3/0/5 is inactive"),
            "{p:?}"
        );
        spec.port_channel[0].members[0].port = "Te3/0/4".into();
        assert!(generate::inactive_ports(&spec, &st).is_empty());
    }

    #[test]
    fn an_existing_port_channel_takes_only_its_allowed_list() {
        let mut spec: generate::Spec =
            toml::from_str(include_str!("../tests/example-mdf-design.toml")).unwrap();
        assert!(
            generate::check(&spec).is_empty(),
            "{:?}",
            generate::check(&spec)
        );
        let lines = generate::render(&spec);
        let config: Vec<&String> = lines.iter().filter(|l| !l.starts_with('!')).collect();
        assert_eq!(
            config,
            [
                "hostname core-b",
                "interface Port-channel11",
                " switchport trunk allowed vlan except 451-452,456,459",
                "end"
            ]
        );
        let (n, transcript) = refused(&spec, &lines);
        assert_eq!(n, 0, "{transcript}");
        spec.port_channel[0].members.push(generate::Member {
            port: "Te2/0/1".into(),
            description: "x".into(),
            verify: None,
        });
        assert!(
            generate::check(&spec)
                .iter()
                .any(|p| p.contains("Port-channel11 exists"))
        );
        spec.port_channel[0].existing = false;
        spec.port_channel[0].members.clear();
        assert!(
            generate::check(&spec)
                .iter()
                .any(|p| p.contains("Port-channel11 has no members"))
        );
    }

    #[test]
    fn an_access_port_on_an_unknown_vlan_is_a_design_problem() {
        let mut spec: generate::Spec =
            toml::from_str(include_str!("../tests/example-core-design.toml")).unwrap();
        spec.access[0].vlan = 451;
        assert!(
            generate::check(&spec)
                .iter()
                .any(|p| p.contains("VLAN 451, which is not defined"))
        );
    }
}

#[cfg(test)]
mod ie3300_tests {
    use super::*;

    #[test]
    fn an_ie_3300_runs_its_own_book_and_refuses_the_9300_s_stack() {
        let st = platform::checked_stack("IE-3300-8T2S", Release::parse("17.6").unwrap()).unwrap();
        assert_eq!(grammars::book(st.family()), "ie3300");
        let r = grammars::resolve("17.6.4", "ie3300").unwrap();
        let engine = cli::Engine::new(grammars::grammar("ie3300", r).unwrap());
        let typed = |script: &str| {
            let mut s = shell::Session::new(&engine, device::Device::new(st.clone()));
            s.script(&format!("enable\nconfigure terminal\n{script}")).1
        };
        // what the IE guides and its 9300 base attest
        assert_eq!(
            typed(
                "spanning-tree mode rapid-pvst\ninterface GigabitEthernet1/3\nswitchport mode access\nrep segment 1\n"
            ),
            0
        );
        // StackWise, stack power and network modules are the 9300's (Codex's negative controls)
        for line in [
            "switch 1 provision c9300-24t",
            "switch 1 renumber 2",
            "stack-power stack one",
            "hardware module slot 1",
        ] {
            assert_eq!(typed(line), 1, "{line}");
        }
        // no IE book at a release its sources are not assigned to yet
        assert!(grammars::resolve("17.15", "ie3300").is_err());
    }
}

#[cfg(test)]
mod c9200_tests {
    use super::*;

    fn session_for(spec: &str) -> (platform::Stack, &'static cli::Engine) {
        let st = platform::checked_stack(spec, crate::release::DEFAULT).unwrap();
        let engine: &'static cli::Engine = Box::leak(Box::new(cli::Engine::new(
            serde_json::from_str(
                &grammars::text(grammars::book(st.family()), release::DEFAULT).unwrap(),
            )
            .unwrap(),
        )));
        (st, engine)
    }

    #[test]
    fn a_9200_stack_runs_on_the_9200_book_s_grammar() {
        for (spec, takes_mpls) in [
            ("C9200-48T+C9200-NM-4X", false),
            ("C9200L-24T-4G", false),
            ("C9300X-24Y+C9300X-NM-4C", true),
        ] {
            let (st, engine) = session_for(spec);
            let mut s = shell::Session::new(engine, device::Device::new(st));
            let (_, refused) =
                s.script("enable\nconfigure terminal\ninterface GigabitEthernet0/0\n");
            assert_eq!(refused, 0, "{spec}");
            let ok = matches!(s.type_line("mpls ip").1, shell::Result::Ok);
            assert_eq!(ok, takes_mpls, "{spec}: the 9200 book has no MPLS chapter");
        }
    }

    #[test]
    fn a_9200_design_generates_a_configuration_the_switch_accepts_whole() {
        let spec: generate::Spec =
            toml::from_str(include_str!("../tests/c9200-design.toml")).unwrap();
        assert!(
            generate::check(&spec).is_empty(),
            "{:?}",
            generate::check(&spec)
        );
        let (st, engine) = session_for(&spec.stack.join(","));
        let lines = generate::render(&spec);
        assert!(lines.contains(&"switch 1 provision c9200-48t".to_string()));
        let mut s = shell::Session::new(engine, device::Device::new(st));
        let (transcript, refused) = s.script(&format!(
            "enable\nconfigure terminal\n{}\n",
            lines.join("\n")
        ));
        assert_eq!(refused, 0, "{}", transcript.join("\n"));
        let run = show::running_config(&s.device).join("\n");
        assert!(
            run.contains("interface TenGigabitEthernet2/1/1\n description core Te2/0/1"),
            "{run}"
        );
        assert!(
            run.contains("interface GigabitEthernet2/0/48\n description phone 48"),
            "{run}"
        );
    }

    #[test]
    fn hardware_cisco_does_not_build_is_refused_with_the_reason() {
        let e = stack(&["--stack".into(), "C9200-24T+C9200-NM-2Y".into()]).unwrap_err();
        assert!(e.contains("C9200-NM-2Y does not fit the C9200-24T"), "{e}");
        let e = stack(&["--stack".into(), "C9200-48T,C9200L-48T-4X".into()]).unwrap_err();
        assert!(
            e.contains("member 2") && e.contains("does not stack with"),
            "{e}"
        );
        assert!(
            stack(&[
                "--stack".into(),
                "C9200-48PXG+C9200-NM-2Q,C9200-24PXG+C9200-NM-2Y".into()
            ])
            .is_ok()
        );
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;

    fn r(t: &str) -> Release {
        Release::parse(t).unwrap()
    }

    #[test]
    fn the_release_comes_from_the_flag_or_the_file_and_they_must_agree() {
        let flag = |v: &str| vec!["--release".to_string(), v.to_string()];
        assert_eq!(release(&[], None, "c9300").unwrap(), release::DEFAULT);
        assert_eq!(release(&[], Some("17.9.4"), "c9300").unwrap(), r("17.9"));
        assert_eq!(release(&flag("16.12"), None, "c9300").unwrap(), r("16.12"));
        assert_eq!(release(&flag("26.1"), None, "c9200").unwrap(), r("26.x"));
        assert_eq!(
            release(&flag("17.9"), Some("17.9.2"), "c9300").unwrap(),
            r("17.9")
        );
        let e = release(&flag("17.9"), Some("17.12"), "c9300").unwrap_err();
        assert!(
            e.contains("--release 17.9 disagrees with IOS XE 17.12"),
            "{e}"
        );
        let e = release(&flag("16.7"), None, "c9300").unwrap_err();
        assert!(
            e.contains("IOS XE 16.7 has no Catalyst 9300 command reference"),
            "{e}"
        );
        assert!(
            release(&flag("16.5"), None, "c9200").is_err(),
            "the 9200 book starts at 16.9"
        );
        // with nothing named, a plain 2960 runs the newest train its hardware runs, as the builder
        // chooses: 15.0(2)SE, before the 2960 book's 15.2(4)E
        for model in ["WS-C2960-24TC-L", "WS-C2960G-8TC-L"] {
            let (st, rel) = stack_at(model, &[], None).unwrap();
            assert_eq!(rel, r("15.0(2)SE"), "{model}");
            assert!(st.problems(rel).is_empty(), "{model}");
        }
        assert_eq!(
            stack_at("C9300-48P", &[], None).unwrap().1,
            release::DEFAULT
        );
    }

    #[test]
    fn every_documented_release_has_a_grammar_of_its_own() {
        let c9300 = grammars::documented("c9300");
        let c9200 = grammars::documented("c9200");
        assert_eq!(c9300.len(), 26, "16.5 16.6 16.8-16.12 17.1-17.18 26.x");
        assert_eq!(c9200.len(), 23, "16.9-16.12 17.1-17.18 26.x");
        assert_eq!((c9300[0], *c9300.last().unwrap()), (r("16.5"), r("26.x")));
        assert_eq!((c9200[0], *c9200.last().unwrap()), (r("16.9"), r("26.x")));
        // each release's grammar is its own, by the release its first bytes name; a book's newest is
        // read whole, as a recompile writes a book's grammars together
        for (book, _, _) in grammars::BOOKS {
            let documented = grammars::documented(book);
            for &rel in &documented {
                let head = grammars::head(book, rel, 64).unwrap();
                let named = head
                    .strip_prefix("{\"release\":\"")
                    .and_then(|r| r.split('"').next())
                    .and_then(Release::parse);
                assert_eq!(named, Some(rel), "{book} {rel}: {head}");
            }
            let newest = *documented.last().unwrap();
            let g: compile::Grammar =
                serde_json::from_str(&grammars::text(book, newest).unwrap()).unwrap();
            assert_eq!(Release::parse(&g.release), Some(newest), "{book} {newest}");
        }
        // a 2960-X runs its own book, by default its newest train; an IOS XE book 17.15
        assert_eq!(grammars::book(Some(platform::Family::C2960X)), "c2960x");
        assert_eq!(grammars::default_release("c2960x"), r("15.2(7)E"));
        assert_eq!(grammars::default_release("c9300"), release::DEFAULT);
        assert_eq!(
            cli::interface_name("Fa0", None),
            Some(("FastEthernet".to_string(), "0".to_string(), 1))
        );
    }

    /// A grammar of `(name, modes, line)` commands and `(from, line, to)` example prompt changes.
    fn tiny(
        release: &str,
        cmds: &[(&str, &[&str], &str)],
        moves: &[(&str, &str, &str)],
    ) -> compile::Grammar {
        compile::Grammar {
            release: release.into(),
            platform: "Catalyst 9300".into(),
            source: String::new(),
            commands: cmds
                .iter()
                .map(|(name, modes, line)| {
                    let (node, repairs) = syntax::parse(line);
                    compile::Command {
                        name: name.to_string(),
                        source: String::new(),
                        modes: modes.iter().map(|m| m.to_string()).collect(),
                        lines: vec![compile::Line {
                            text: line.to_string(),
                            node,
                            negated: false,
                            repairs,
                        }],
                        args: Default::default(),
                        examples: vec![],
                        enters: None,
                        evidence: None,
                        help: Default::default(),
                    }
                })
                .collect(),
            transitions: moves
                .iter()
                .map(|(from, line, to)| compile::Transition {
                    from: from.to_string(),
                    line: line.to_string(),
                    to: to.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn borrow_enters_fills_only_what_a_release_s_own_examples_leave_open() {
        let cmds: &[(&str, &[&str], &str)] = &[
            ("router ospf", &["config"], "router ospf <id>"),
            ("maximum-paths", &["config-router"], "maximum-paths <n>"),
            ("police", &["config-pmap-c"], "police <rate>"),
            ("class", &["config-pmap"], "class <name>"),
            ("policy-map", &["config"], "policy-map <name>"),
        ];
        // two observations each, as Engine::new needs
        let own = [
            ("config", "router ospf 1", "config-router"),
            ("config", "router ospf 1", "config-router"),
            ("config-pmap-c", "police 8000", "config"),
            ("config-pmap-c", "police 8000", "config"),
            ("config", "policy-map p", "config-pmap"),
            ("config", "policy-map p", "config-pmap"),
        ];
        // 17.1's examples never show router ospf or police change the prompt, and show policy-map
        // entering a mode of their own
        let later = [
            ("config", "policy-map p", "config-pmap-c"),
            ("config", "policy-map p", "config-pmap-c"),
        ];
        let mut books = vec![
            (Release::parse("16.12").unwrap(), tiny("16.12", cmds, &own)),
            (Release::parse("17.1").unwrap(), tiny("17.1", cmds, &later)),
        ];
        let log = borrow_enters(&mut books);
        let enters = |i: usize, name: &str| {
            books[i]
                .1
                .commands
                .iter()
                .find(|c| c.name == name)
                .unwrap()
                .enters
                .clone()
        };
        assert_eq!(
            enters(1, "router ospf").as_deref(),
            Some("config-router"),
            "{log:?}"
        );
        assert!(
            books[1].1.commands[0]
                .evidence
                .as_deref()
                .unwrap()
                .contains("16.12")
        );
        // a move into plain configuration is an artifact of an example that skipped `exit`
        assert_eq!(enters(1, "police"), None);
        // a release's own evidence is never replaced: 17.1 keeps its own policy-map submode
        assert_eq!(enters(1, "policy-map"), None);
        let own17 = cli::Engine::new(books[1].1.clone());
        let pm = own17
            .grammar
            .commands
            .iter()
            .find(|c| c.name == "policy-map");
        assert_eq!(pm.and_then(|c| c.enters.as_deref()), Some("config-pmap-c"));
        assert_eq!(
            enters(0, "router ospf"),
            None,
            "16.12 infers its own at run time"
        );
        assert_eq!(log.len(), 1, "{log:?}");
    }

    #[test]
    fn borrow_enters_takes_a_submode_from_a_command_in_the_same_mode() {
        // two `class` commands: the policy map's, and the control policy map's negated-only one
        let cmds: &[(&str, &[&str], &str)] = &[
            ("policy-map", &["config"], "policy-map <name>"),
            ("class", &["config-pmap"], "class <name>"),
            ("set", &["config-pmap-c"], "set dscp <value>"),
            (
                "class",
                &["config-class-control-policymap"],
                "no <priority-number>",
            ),
            ("event", &["config-class-control-policymap"], "event <name>"),
        ];
        let shown = [
            ("config-pmap", "class c1", "config-pmap-c"),
            ("config-pmap", "class c1", "config-pmap-c"),
        ];
        let mut books = vec![
            (r("17.15"), tiny("17.15", cmds, &[])),
            (r("17.16"), tiny("17.16", cmds, &shown)),
        ];
        let log = borrow_enters(&mut books);
        let classes: Vec<(Vec<String>, Option<String>)> = books[0]
            .1
            .commands
            .iter()
            .filter(|c| c.name == "class")
            .map(|c| (c.modes.clone(), c.enters.clone()))
            .collect();
        assert_eq!(
            classes,
            [
                (
                    vec!["config-pmap".to_string()],
                    Some("config-pmap-c".to_string())
                ),
                (vec!["config-class-control-policymap".to_string()], None)
            ],
            "{log:?}"
        );
    }

    #[test]
    fn a_release_no_capture_runs_reads_the_nearest_trains_captures() {
        // the IE 3000's only captures run 15.2(6)E, which its 15.2(2)E book reads
        let files = vec![(r("15.2(6)E"), "new".to_string())];
        let documented = [r("12.2(55)SE"), r("15.0(2)SE"), r("15.2(2)E")];
        assert_eq!(captured_at(&files, &documented, r("15.2(2)E")), ["new"]);
        assert_eq!(captured_at(&files, &documented, r("12.2(55)SE")), ["new"]);
        // an older train's come before a newer one's
        let files = vec![
            (r("12.2(44)SE"), "old".to_string()),
            (r("15.2(6)E"), "new".to_string()),
        ];
        let documented = [
            r("12.2(44)SE"),
            r("12.2(55)SE"),
            r("15.0(2)SE"),
            r("15.2(2)E"),
            r("15.2(4)E"),
        ];
        assert_eq!(captured_at(&files, &documented, r("15.0(2)SE")), ["old"]);
        assert_eq!(captured_at(&files, &documented, r("15.2(4)E")), ["new"]);
    }

    #[test]
    fn commands_that_share_a_name_keep_their_own_help() {
        let mut books = vec![(
            r("15.2(7)E"),
            tiny(
                "15.2(7)E",
                &[
                    ("set", &["config-pmap-c"], "set dscp <value>"),
                    ("set", &[], "set <variable> <value>"),
                    ("hostname", &["config"], "hostname <name>"),
                ],
                &[],
            ),
        )];
        let helps = [
            "Set QoS values",
            "Set or display environment variables",
            "Set the system's network name",
        ];
        for (c, h) in books[0].1.commands.iter_mut().zip(helps) {
            c.help.insert(String::new(), h.to_string());
        }
        let table = take_help(&mut books);
        assert_eq!(table.keys().collect::<Vec<_>>(), ["hostname"]);
        let g = &books[0].1;
        assert_eq!(g.commands[0].help[""], "Set QoS values");
        assert_eq!(
            g.commands[1].help[""],
            "Set or display environment variables"
        );
        assert!(g.commands[2].help.is_empty());
    }

    #[test]
    fn a_submode_its_own_examples_show_too_seldom_comes_from_a_neighbouring_release() {
        for (spec, rel, lines, mode) in [
            (
                "C9300-48T",
                "16.5",
                "router ospf 1\nmaximum-paths 2\n",
                "(config-router)#",
            ),
            (
                "C9300-48T",
                "17.5",
                "ip access-list extended web\n",
                "(config-ext-nacl)#",
            ),
        ] {
            let args: Vec<String> = ["--stack", spec, "--release", rel]
                .iter()
                .map(|s| s.to_string())
                .collect();
            let (st, r) = stack(&args).unwrap();
            let engine = cli::Engine::new(grammar(&[], st.family(), r).unwrap());
            let mut s = shell::Session::new(&engine, device::Device::new(st));
            let (t, refused) = s.script(&format!("enable\nconfigure terminal\n{lines}"));
            assert_eq!(refused, 0, "{rel}: {}", t.join("\n"));
            assert!(s.prompt().ends_with(mode), "{rel}: {}", s.prompt());
        }
    }

    #[test]
    fn a_9400_runs_the_9400_book() {
        // `power budget mode` is documented for the 9400 only
        let r = release::DEFAULT;
        for (family, takes) in [
            (platform::Family::C9400, true),
            (platform::Family::C9300X, false),
        ] {
            let engine = cli::Engine::new(grammar(&[], Some(family), r).unwrap());
            let ok = matches!(
                engine.parse("config", "power budget mode single-sup"),
                cli::Outcome::Ok(_)
            );
            assert_eq!(ok, takes, "{family:?}");
        }
        assert_eq!(grammars::book(Some(platform::Family::C9400)), "c9400");
        assert_eq!(grammars::book(Some(platform::Family::C9200B)), "c9200");
        assert_eq!(
            grammars::documented("c9400").len(),
            24,
            "16.6, 16.9-16.12, 17.1-17.18, 26.x"
        );
    }

    #[test]
    fn the_cluster_core_change_is_accepted_at_the_release_the_cores_run() {
        let out = std::env::temp_dir().join(format!("vswitch-core-{}.cfg", std::process::id()));
        let args: Vec<String> = [
            "generate",
            "tests/example-core-design.toml",
            "--release",
            "17.5",
            "--out",
            &out.to_string_lossy(),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(run(&args), Ok(true));
        let _ = std::fs::remove_file(out);
    }
}

#[cfg(test)]
mod c9300_tests {
    use super::*;

    /// Generates the design, types it into its stack, and returns the running configuration.
    fn accepted_whole(design: &str) -> String {
        let spec: generate::Spec = toml::from_str(design).unwrap();
        assert!(
            generate::check(&spec).is_empty(),
            "{:?}",
            generate::check(&spec)
        );
        let st = stack(&["--stack".into(), spec.stack.join(",")]).unwrap().0;
        assert!(generate::inactive_ports(&spec, &st).is_empty());
        let engine = cli::Engine::new(
            serde_json::from_str(
                &grammars::text(grammars::book(st.family()), release::DEFAULT).unwrap(),
            )
            .unwrap(),
        );
        let lines = generate::render(&spec);
        let mut s = shell::Session::new(&engine, device::Device::new(st));
        let (transcript, refused) = s.script(&format!(
            "enable\nconfigure terminal\n{}\n",
            lines.join("\n")
        ));
        assert_eq!(refused, 0, "{}", transcript.join("\n"));
        show::running_config(&s.device).join("\n")
    }

    #[test]
    fn a_9300_and_9300x_stack_generates_a_configuration_the_switch_accepts_whole() {
        let run = accepted_whole(include_str!("../tests/c9300-design.toml"));
        assert!(run.contains("switch 2 provision c9300-48t"), "{run}");
        assert!(
            run.contains("interface TenGigabitEthernet2/1/2\n description core Te2/0/1"),
            "{run}"
        );
    }

    #[test]
    fn a_9300l_and_9300lm_stack_generates_a_configuration_the_switch_accepts_whole() {
        let run = accepted_whole(include_str!("../tests/c9300l-design.toml"));
        assert!(run.contains("switch 2 provision c9300lm-48t-4y"), "{run}");
        assert!(
            run.contains("interface TwentyFiveGigE2/1/2\n description core Twe2/0/1"),
            "{run}"
        );
    }

    #[test]
    fn a_design_on_a_port_its_module_does_not_use_is_named() {
        let mut spec: generate::Spec =
            toml::from_str(include_str!("../tests/c9300-design.toml")).unwrap();
        spec.stack[1] = "C9300-48T+C9300-NM-4G".into();
        let st = stack(&["--stack".into(), spec.stack.join(",")]).unwrap().0;
        assert_eq!(
            generate::inactive_ports(&spec, &st),
            [
                "Te2/1/1 is listed by IOS but not used by the C9300-NM-4G (hardware installation guide)",
                "Te2/1/2 is listed by IOS but not used by the C9300-NM-4G (hardware installation guide)",
            ]
        );
    }

    #[test]
    fn hardware_cisco_does_not_build_is_refused_with_the_reason() {
        for (spec, why) in [
            (
                "C9300X-24Y+C9300-NM-8X",
                "C9300-NM-8X is a Catalyst 9300 module; the C9300X-24Y takes",
            ),
            (
                "C9300X-12Y+C9300X-NM-4C",
                "C9300X-NM-4C does not fit the C9300X-12Y",
            ),
            (
                "C9300-48T+C9300X-NM-2C",
                "C9300X-NM-2C is a Catalyst 9300X module",
            ),
            (
                "C9300L-48T-4X+C9300-NM-8X",
                "the C9300L-48T-4X has built-in uplinks",
            ),
            (
                "C9300-48T,C9300L-48T-4G",
                "member 2: a Catalyst 9300L does not stack with a Catalyst 9300",
            ),
            (
                "C9300LM-48T-4Y,C9300X-24Y",
                "member 2: a Catalyst 9300X does not stack with a Catalyst 9300LM",
            ),
        ] {
            let e = stack(&["--stack".into(), spec.into()]).unwrap_err();
            assert!(e.contains(why), "{spec}: {e}");
        }
        for spec in [
            "C9300X-48TX+C9300X-NM-4C,C9300-24S+C3850-NM-2-40G",
            "C9300-48S+C9300-NM-2Y,C9300X-12Y+C9300X-NM-8Y",
            "C9300L-24T-4G,C9300LM-48T-4Y,C9300L-48T-4X",
        ] {
            assert!(stack(&["--stack".into(), spec.into()]).is_ok(), "{spec}");
        }
    }
}

#[cfg(test)]
mod check_tests {
    use super::*;

    const CAPTURE: &str = include_str!("../tests/check-config.txt");

    /// The fixture typed on a C9300-48T at a release, and the session after it.
    fn typed_at(rel: &str) -> (Vec<check::Typed>, String) {
        let r = Release::parse(rel).unwrap();
        let st = platform::parse_stack("C9300-48T").unwrap();
        let engine = cli::Engine::new(grammar(&[], st.family(), r).unwrap());
        let (s, typed) = check::type_config(&engine, st, CAPTURE);
        (typed, show::running_config(&s.device).join("\n"))
    }

    #[test]
    fn an_upgrade_lists_the_lines_each_release_takes() {
        let (from, _) = typed_at("16.12");
        let (to, _) = typed_at("17.15");
        let r = check::compare(&from, &to);
        let lines = |v: &[check::Typed]| v.iter().map(|t| t.line.clone()).collect::<Vec<_>>();
        assert_eq!(lines(&r.gains), ["spanning-tree bpdu sender-conflict"]);
        assert!(r.breaks.is_empty() && r.neither.is_empty(), "{r:?}");
        assert_eq!(r.lines, 12);
    }

    #[test]
    fn a_line_its_submode_does_not_know_goes_to_the_nearest_enclosing_mode() {
        // 17.15's examples show `vrf forwarding` entering (config-if-vrf); `no ip address` belongs to
        // the interface (global configuration refuses it as incomplete), which keeps it as the cores'
        // running configurations print it, and the next block opens
        let (typed, run) = typed_at("17.15");
        assert!(typed.iter().all(|t| t.error.is_none()), "{typed:?}");
        assert!(
            run.contains(
                "interface GigabitEthernet0/0\n vrf forwarding Mgmt-vrf\n no ip address\n!\ninterface "
            ),
            "{run}"
        );
    }

    #[test]
    fn an_ordinary_line_never_climbs_out_of_its_mode() {
        // 17.5's examples show `ip address` followed by a (config) prompt three times
        let (typed, run) = typed_at("17.5");
        assert!(
            typed
                .iter()
                .all(|t| t.error.is_none() || t.line.contains("sender-conflict")),
            "{typed:?}"
        );
        assert!(
            run.contains(
                "interface Vlan10\n ip address 10.0.10.2 255.255.255.0\n standby 0 ip 10.0.10.1"
            ),
            "{run}"
        );
    }

    #[test]
    fn a_check_fails_on_hardware_the_target_release_does_not_run() {
        // without the line 17.4 refuses, the hardware is the only difference
        let path = std::env::temp_dir().join(format!("vswitch-check-{}.txt", std::process::id()));
        let text: String = CAPTURE
            .lines()
            .filter(|l| !l.contains("sender-conflict"))
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(&path, text).unwrap();
        let file = path.to_string_lossy().to_string();
        let args = |to: &str| -> Vec<String> {
            [
                "check",
                &file,
                "--stack",
                "C9300X-24Y",
                "--from",
                "17.15",
                "--to",
                to,
            ]
            .iter()
            .map(|s| s.to_string())
            .collect()
        };
        let at_17_4 = run(&args("17.4"));
        let at_17_5 = run(&args("17.5"));
        let _ = std::fs::remove_file(&path);
        assert_eq!(at_17_4, Ok(false));
        assert_eq!(at_17_5, Ok(true));
    }

    #[test]
    fn a_check_fails_on_a_line_the_target_release_refuses() {
        // C9300-48T hardware runs at both; 16.12 does not document spanning-tree bpdu sender-conflict
        let args = |to: &str| -> Vec<String> {
            [
                "check",
                "tests/check-config.txt",
                "--from",
                "17.15",
                "--to",
                to,
            ]
            .iter()
            .map(|s| s.to_string())
            .collect()
        };
        assert_eq!(run(&args("16.12")), Ok(false));
        assert_eq!(run(&args("17.18")), Ok(true));
    }

    #[test]
    fn a_classic_version_line_names_no_release_to_check_from() {
        let t = library::tests::TempDir::new("check-classic");
        let f = t.0.join("c.txt");
        std::fs::write(
            &f,
            "!\nversion 15.2\nhostname sw\ninterface GigabitEthernet1/0/1\n description x\nend\n",
        )
        .unwrap();
        let file = f.to_string_lossy().to_string();
        let at = |from: &[&str]| {
            let mut a: Vec<String> = [
                "check",
                &file,
                "--stack",
                "WS-C2960X-24PS-L",
                "--to",
                "15.2(7)E",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            a.extend(from.iter().map(|s| s.to_string()));
            run(&a)
        };
        let e = at(&[]).unwrap_err();
        assert!(e.contains("names no train; give --from"), "{e}");
        assert_eq!(at(&["--from", "15.2(2)E"]), Ok(true));
    }

    #[test]
    fn a_problem_of_the_whole_stack_names_no_member() {
        let stack = platform::parse_stack(&["C9300-24T"; 9].join(",")).unwrap();
        let lines: Vec<String> = stack
            .problems(Release::parse("17.15").unwrap())
            .iter()
            .map(|(n, why)| hardware_line(*n, why))
            .collect();
        assert!(
            lines.contains(&"hardware: a Catalyst 9300 stack takes at most 8 members".to_string()),
            "{lines:?}"
        );
        assert!(!lines.iter().any(|l| l.contains("member 0")), "{lines:?}");
    }
}

#[cfg(test)]
mod capture_command_tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    const VERSION: &str = "Cisco IOS Software, C2960X Software (C2960X-UNIVERSALK9-M), Version 15.2(7)E8, RELEASE SOFTWARE (fc1)\n\nSwitch Ports Model                     SW Version            SW Image\n------ ----- -----                     ----------            ----------\n*    1 28    WS-C2960X-24PS-L          15.2(7)E8             C2960X-UNIVERSALK9-M\n\n";

    /// SW1's configuration describing its Gi1/0/1.
    fn config(description: &str) -> String {
        format!(
            "!\nversion 15.2\nhostname SW1\ninterface GigabitEthernet1/0/1\n description {description}\nend\n"
        )
    }

    /// `capture --folder`: what it said and whether every switch was taken.
    fn folder(
        dir: &std::path::Path,
        out: &std::path::Path,
        pending: Option<&std::path::Path>,
    ) -> (Vec<String>, bool) {
        let mut said = vec![];
        let ok = capture_folder(dir, out, pending, &mut |l| said.push(l)).unwrap();
        (said, ok)
    }

    #[test]
    fn a_switch_captured_again_is_added_to_its_capture_at_that_release_only() {
        let t = library::tests::TempDir::new("capture-again");
        let (input, out) = (t.0.join("in"), t.0.join("out"));
        std::fs::create_dir_all(&input).unwrap();
        // SW1 captured before an upgrade, at 15.2(2)E, and after it, at 15.2(7)E
        for (release, description) in [("15.2(2)E", "OLD"), ("15.2(7)E", "KEPT")] {
            let dir = out.join(format!("c2960x-{release}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("SW1-running-config.txt"), config(description)).unwrap();
            std::fs::write(
                dir.join("SW1-show.log"),
                "SW1#show cdp neighbors detail\nKEPT NEIGHBOURS\n",
            )
            .unwrap();
        }
        let log = |release: &str| {
            std::fs::read_to_string(out.join(format!("c2960x-{release}/SW1-show.log"))).unwrap()
        };
        let p = |p: &std::path::Path| p.to_string_lossy().to_string();
        let capture = || {
            run(&args(&[
                "capture",
                "--folder",
                &p(&input),
                "--out",
                &p(&out),
            ]))
        };
        // a 15.2(7)E8 session: added to the capture at 15.2(7)E, whose configuration stays
        std::fs::write(
            input.join("sw1.log"),
            format!(
                "SW1#show version\n{VERSION}SW1#show vlan brief\nVLAN1\nSW1#show running-config\n{}",
                config("NEW")
            ),
        )
        .unwrap();
        assert_eq!(capture(), Ok(true));
        let kept =
            std::fs::read_to_string(out.join("c2960x-15.2(7)E/SW1-running-config.txt")).unwrap();
        assert!(kept.contains("description KEPT"), "{kept}");
        assert!(
            log("15.2(7)E").starts_with("SW1#show cdp neighbors detail\nKEPT NEIGHBOURS\n")
                && log("15.2(7)E").contains("SW1#show vlan brief\n"),
            "{}",
            log("15.2(7)E")
        );
        assert!(!log("15.2(2)E").contains("show vlan brief"));
        // a session at a release with no capture, naming no hardware: no capture's, and a failure
        let other = VERSION
            .replace("15.2(7)E8", "15.2(4)E10")
            .replace("*    1 28    WS-C2960X-24PS-L", "");
        std::fs::write(
            input.join("sw1.log"),
            format!("SW1#show version\n{other}SW1#show interfaces trunk\ntrunks\n"),
        )
        .unwrap();
        assert_eq!(capture(), Ok(false));
        // a session naming no release at all: no capture's either
        std::fs::write(
            input.join("sw1.log"),
            "SW1#show vtp status\nVTP Version : 2\n",
        )
        .unwrap();
        assert_eq!(capture(), Ok(false));
        for release in ["15.2(2)E", "15.2(7)E"] {
            assert!(
                !log(release).contains("show interfaces trunk")
                    && !log(release).contains("show vtp status"),
                "{release}"
            );
        }
    }

    #[test]
    fn a_pending_switch_captured_again_reads_no_change_report_from_live_captures() {
        let t = library::tests::TempDir::new("capture-pending");
        let (input, out, pending) = (t.0.join("in"), t.0.join("out"), t.0.join("pending"));
        std::fs::create_dir_all(&input).unwrap();
        let session = |description: &str| {
            format!(
                "SW1#show version\nCisco IOS XE Software, Version 17.15.01\nSW1#show inventory\nNAME: \"Switch 1\", DESCR: \"x\"\nPID: WS-C3850-48P , VID: V01\nSW1#show running-config\n!\nversion 17.15\nhostname SW1\ninterface GigabitEthernet1/0/1\n description {description}\nend\n"
            )
        };
        std::fs::write(input.join("sw1.log"), session("FIRST")).unwrap();
        assert!(folder(&input, &out, Some(&pending)).1);
        std::fs::write(input.join("sw1.log"), session("SECOND")).unwrap();
        let (said, ok) = folder(&input, &out, Some(&pending));
        assert!(ok, "{said:?}");
        assert!(said[0].contains("its configuration differs"), "{said:?}");
        assert!(
            !said.iter().any(|l| l.contains("change report")),
            "{said:?}"
        );
    }

    #[cfg(all(test, captures))]
    include!(concat!(
        env!("VSWITCH_SITE_TESTS"),
        "/main-capture_command_tests.rs"
    ));

    #[test]
    fn an_option_a_command_does_not_take_is_refused_and_every_operand_is_read() {
        let e = run(&args(&[
            "check",
            "c.txt",
            "--stak",
            "C9300-48P",
            "--to",
            "17.12",
        ]))
        .unwrap_err();
        assert!(
            e.starts_with("check: --stak is not an option of check"),
            "{e}"
        );
        let e = run(&args(&["fleet", "captures", "--to"])).unwrap_err();
        assert_eq!(e, "fleet: --to needs a value");
    }

    #[test]
    fn fleet_reports_every_capture_it_can_and_names_the_rest() {
        let t = library::tests::TempDir::new("fleet-rows");
        for (folder, host) in [("c9300-16.3", "old1"), ("c9300-17.9", "new1")] {
            let dir = t.0.join(folder);
            std::fs::create_dir_all(&dir).unwrap();
            let release = folder.split_once('-').unwrap().1;
            std::fs::write(
                dir.join(format!("{host}-running-config.txt")),
                format!("!\nversion {release}\nhostname {host}\nend\n"),
            )
            .unwrap();
            std::fs::write(dir.join(format!("{host}-stack.txt")), "C9300-48P\n").unwrap();
        }
        // 16.3 has no 9300 book: its row says so, and new1 is checked
        let a = args(&["fleet", &t.0.to_string_lossy(), "--to", "17.15"]);
        assert_eq!(run(&a), Ok(true));
    }
}

#[cfg(test)]
mod console_tests {
    use super::*;
    use console::{Editor, Effect};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn engine() -> cli::Engine {
        cli::Engine::new(grammars::grammar("c9300", release::DEFAULT).unwrap())
    }

    fn at<'a>(e: &'a cli::Engine, lines: &[&str]) -> shell::Session<'a> {
        let mut s = shell::Session::new(e, device::Device::new(stack(&[]).unwrap().0));
        for l in lines {
            s.type_line(l);
        }
        s
    }

    #[test]
    fn question_mark_lists_what_may_follow_with_the_reference_s_help() {
        let e = engine();
        let s = at(&e, &["enable", "configure terminal", "interface Twe1/0/1"]);
        let h = s.help("switchport mode ");
        assert!(
            h.iter()
                .any(|l| l.starts_with("  access") && l.ends_with("Sets the port to access mode")),
            "{h:?}"
        );
        assert_eq!(s.help("shutdown "), vec!["  <cr>", ""]);
        assert_eq!(s.help("frobnicate "), vec!["% Unrecognized command"]);
        // a word and `?`: the keywords it begins, across the line
        assert_eq!(s.help("sp"), vec!["spanning-tree  speed", ""]);
    }

    #[test]
    fn an_interface_is_offered_as_the_stack_s_types() {
        let e = engine();
        let s = at(&e, &["enable", "configure terminal"]);
        let h = s.help("interface ");
        let has = |w: &str| h.iter().any(|l| l.split_whitespace().next() == Some(w));
        assert!(has("TwentyFiveGigE") && has("HundredGigE") && has("Port-channel"));
        assert!(
            !has("FortyGigabitEthernet"),
            "no 40G port on two C9300X-24Y"
        );
        assert!(!has("port-channel"), "listed once, as IOS spells it: {h:?}");
    }

    #[test]
    fn tab_completes_a_keyword_or_an_interface_type() {
        let e = engine();
        let s = at(&e, &["enable"]);
        assert_eq!(s.complete("conf").as_deref(), Some("configure "));
        assert_eq!(s.complete("configure "), None);
        let s = at(&e, &["enable", "configure terminal"]);
        assert_eq!(s.complete("s"), None, "s begins many keywords");
        assert_eq!(
            s.complete("interface twe").as_deref(),
            Some("interface TwentyFiveGigE")
        );
        assert_eq!(
            s.complete("interface twe1/0/1").as_deref(),
            Some("interface TwentyFiveGigE1/0/1 ")
        );
        // a whole keyword is itself, though ipv6 begins with it
        assert_eq!(s.complete("ip").as_deref(), Some("ip "));
    }

    #[test]
    fn a_script_line_ending_in_a_question_mark_prints_help_and_is_not_entered() {
        let e = engine();
        let mut s = at(&e, &[]);
        let (out, refused) = s.script("enable\nconfigure terminal\nhostname ?\nend\n");
        assert_eq!(refused, 0);
        assert!(
            out.contains(&"Switch(config)#hostname ?".to_string()),
            "{out:?}"
        );
        assert!(
            out.iter().any(|l| l.trim_start().starts_with("WORD")),
            "{out:?}"
        );
        assert_eq!(s.device.hostname, "Switch");
    }

    #[test]
    fn write_memory_keeps_the_configuration_in_the_switch_list() {
        let t = library::tests::TempDir::new("wrmem");
        let mut lib = library::Library::open(t.0.clone()).unwrap();
        lib.add(
            "lab9",
            "C9300X-24Y",
            release::DEFAULT,
            "x.cfg",
            "hostname lab9\n",
        )
        .unwrap();
        let e = engine();
        {
            let mut s = at(&e, &[]);
            let mut name = "lab9".to_string();
            let list = &mut lib;
            s.on_save(move |text: &str| {
                name = list.save_running(&name, text)?;
                Ok(())
            });
            for l in [
                "enable",
                "configure terminal",
                "hostname lab9",
                "vlan 10",
                "end",
            ] {
                s.type_line(l);
            }
            let (out, r) = s.type_line("write memory");
            assert_eq!(out, vec!["Building configuration...", "[OK]"]);
            assert_eq!(r, shell::Result::Ok);
        }
        let kept = lib.config("lab9").unwrap();
        // a VTP server (the default) keeps vlan 10 in vlan.dat, kept ahead of the configuration
        let (vlan_dat, config) = kept.split_once("!\nversion ").unwrap();
        assert!(vlan_dat.starts_with("! vlan.dat") && vlan_dat.contains("\nvlan 10\n"));
        assert!(
            !config.contains("\nvlan 10\n") && kept.ends_with("end\n"),
            "{kept}"
        );
        assert!(lib.switches[0].saved_at.is_some());
        // what was kept boots to the same running configuration and VLAN database
        let (mut booted, _, refused) = tui::boot(&e, stack(&[]).unwrap().0, &kept);
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(
            show::configuration(&booted.device).join("\n") + "\n",
            format!("!\nversion {config}")
        );
        let brief = booted.type_line("show vlan brief").0;
        assert!(
            brief.iter().any(|l| l.starts_with("10   VLAN0010 ")),
            "{brief:?}"
        );
    }

    #[test]
    fn a_vtp_server_keeps_vlans_out_of_the_running_configuration() {
        // 9300 command reference: in VTP versions 1 and 2 the mode must be transparent for VLAN and
        // VTP information to be saved in the running configuration
        let e = engine();
        let mut s = at(
            &e,
            &[
                "enable",
                "configure terminal",
                "vlan 30",
                "name cams",
                "vtp domain lab",
                "end",
            ],
        );
        let run = show::configuration(&s.device).join("\n");
        assert!(
            !run.contains("vlan 30") && !run.contains("vtp domain"),
            "{run}"
        );
        assert!(
            s.type_line("show vlan brief")
                .0
                .iter()
                .any(|l| l.starts_with("30   cams "))
        );
        for l in ["configure terminal", "vtp mode transparent", "end"] {
            s.type_line(l);
        }
        let run = show::configuration(&s.device).join("\n");
        assert!(
            run.contains("\nvlan 30\n name cams\n") && run.contains("vtp domain lab"),
            "{run}"
        );
        assert!(run.contains("vtp mode transparent"));
    }

    #[test]
    fn copy_running_config_asks_for_the_destination_first() {
        let e = engine();
        let mut s = at(&e, &["enable", "configure terminal", "vlan 20", "end"]);
        let (out, _) = s.type_line("copy running-config startup-config");
        assert!(out.is_empty());
        assert_eq!(s.prompt(), "Destination filename [startup-config]? ");
        assert!(s.startup.is_none());
        let (out, r) = s.type_line("");
        assert_eq!(out, vec!["Building configuration...", "[OK]"]);
        assert_eq!(r, shell::Result::Ok);
        assert!(s.startup.as_deref().is_some_and(|t| t.contains("vlan 20")));
        assert_eq!(s.prompt(), "Switch#");
        // another destination is not saved
        s.type_line("copy running-config startup-config");
        let (_, r) = s.type_line("backup-config");
        assert_eq!(r, shell::Result::Error);
    }

    #[test]
    fn a_saved_hostname_renames_the_switch_unless_another_has_it() {
        let t = library::tests::TempDir::new("rename");
        let mut lib = library::Library::open(t.0.clone()).unwrap();
        for n in ["a1", "b1"] {
            let config = format!("hostname {n}\n");
            lib.add(n, "C9300X-24Y", release::DEFAULT, "x.cfg", &config)
                .unwrap();
        }
        assert_eq!(
            lib.save_running("a1", "hostname b1\n!\nend\n").unwrap_err(),
            "a switch named b1 is already in the list"
        );
        assert_eq!(
            lib.save_running("a1", "hostname c1\n!\nend\n").unwrap(),
            "c1"
        );
        let names: Vec<&str> = lib.switches.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["b1", "c1"]);
        assert!(lib.config("a1").is_err() && lib.config("c1").is_ok());
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn typed(ed: &mut Editor, s: &shell::Session<'_>, text: &str) {
        for c in text.chars() {
            ed.key(key(KeyCode::Char(c)), s);
        }
    }

    #[test]
    fn the_line_editor_completes_helps_and_recalls() {
        let e = engine();
        let s = at(&e, &["enable"]);
        let mut ed = Editor::default();
        typed(&mut ed, &s, "conf");
        assert_eq!(ed.key(key(KeyCode::Tab), &s), Effect::Redraw);
        assert_eq!(ed.buf, "configure ");
        // `?` prints help and leaves the line as typed
        let help = ed.key(key(KeyCode::Char('?')), &s);
        assert!(
            matches!(&help, Effect::Help(h) if h.iter().any(|l| l.contains("terminal"))),
            "{help:?}"
        );
        assert_eq!(ed.buf, "configure ");
        typed(&mut ed, &s, "tx");
        ed.key(key(KeyCode::Backspace), &s);
        assert_eq!(ed.key(key(KeyCode::Tab), &s), Effect::Redraw);
        assert_eq!(ed.buf, "configure terminal ");
        assert_eq!(
            ed.key(key(KeyCode::Enter), &s),
            Effect::Enter("configure terminal ".into())
        );
        assert_eq!(ed.buf, "");
        // a word typed at the start of a line, and taken back
        typed(&mut ed, &s, "show vlan");
        ed.key(ctrl('a'), &s);
        typed(&mut ed, &s, "do ");
        assert_eq!(ed.buf, "do show vlan");
        ed.key(ctrl('w'), &s);
        assert_eq!(ed.buf, "show vlan");
        ed.key(key(KeyCode::Enter), &s);
        // Up recalls the lines entered, newest first
        ed.key(key(KeyCode::Up), &s);
        assert_eq!(ed.buf, "show vlan");
        ed.key(key(KeyCode::Up), &s);
        assert_eq!(ed.buf, "configure terminal ");
        assert_eq!(ed.key(key(KeyCode::Up), &s), Effect::Bell);
        ed.key(key(KeyCode::Down), &s);
        assert_eq!(ed.buf, "show vlan");
        assert_eq!(ed.key(ctrl('z'), &s), Effect::End);
        assert_eq!(ed.buf, "");
        typed(&mut ed, &s, "zzz");
        assert_eq!(ed.key(key(KeyCode::Tab), &s), Effect::Bell);
    }
}

#[cfg(all(test, captures))]
include!(concat!(env!("VSWITCH_SITE_TESTS"), "/main-live_tests.rs"));

#[cfg(all(test, captures))]
mod lab_tests {
    use super::*;

    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/main-lab_tests.rs"));
}

#[cfg(test)]
mod show_tests {
    use super::*;

    fn session(e: &cli::Engine) -> shell::Session<'_> {
        let mut s = shell::Session::new(e, device::Device::new(stack(&[]).unwrap().0));
        for l in [
            "enable",
            "configure terminal",
            "hostname lab1",
            "vlan 10",
            "name users",
            "interface range Twe1/0/1, Twe2/0/1",
            "channel-group 10 mode active",
            "interface Port-channel10",
            "switchport mode trunk",
            "interface Twe1/0/5",
            "switchport access vlan 10",
            "description printer",
            "interface Vlan10",
            "ip address 10.0.10.1 255.255.255.0",
            "interface Loopback0",
            "ip address 10.255.0.1 255.255.255.255",
            "end",
        ] {
            let (o, r) = s.type_line(l);
            assert_eq!(r, shell::Result::Ok, "{l}: {o:?}");
        }
        s
    }

    fn engine() -> cli::Engine {
        cli::Engine::new(grammars::grammar("c9300", release::DEFAULT).unwrap())
    }

    #[test]
    fn show_interfaces_switchport_gives_a_trunk_as_the_command_reference_does() {
        let e = engine();
        let mut s = session(&e);
        for l in [
            "configure terminal",
            "interface Twe1/0/2",
            "switchport mode trunk",
            "switchport trunk native vlan 10",
            "switchport trunk allowed vlan 11-20",
            "interface Twe1/0/3",
            "switchport voice vlan 10",
            "switchport access vlan 30",
            "interface Twe1/0/4",
            "no switchport",
            // the console made VLAN 30 for the access port; deleted, the port's VLAN is inactive
            "exit",
            "no vlan 30",
            "end",
        ] {
            let (o, r) = s.type_line(l);
            assert_eq!(r, shell::Result::Ok, "{l}: {o:?}");
        }
        let out = s.type_line("show interfaces switchport").0;
        let block = |name: &str| -> Vec<String> {
            let at = out
                .iter()
                .position(|l| l == &format!("Name: {name}"))
                .unwrap();
            out[at..]
                .iter()
                .take_while(|l| !l.starts_with("Appliance"))
                .cloned()
                .collect()
        };
        // the 9300 command reference's `show interfaces gigabitethernet1/0/1 switchport`, a trunk
        // with no link (its VLAN 10 unnamed; this one's is `users`)
        let trunk = block("Twe1/0/2");
        assert_eq!(
            trunk[..10],
            [
                "Name: Twe1/0/2",
                "Switchport: Enabled",
                "Administrative Mode: trunk",
                "Operational Mode: down",
                "Administrative Trunking Encapsulation: dot1q",
                "Negotiation of Trunking: On",
                "Access Mode VLAN: 1 (default)",
                "Trunking Native Mode VLAN: 10 (users)",
                "Administrative Native VLAN tagging: enabled",
                "Voice VLAN: none",
            ]
        );
        assert!(
            trunk.contains(&"Trunking VLANs Enabled: 11-20".to_string()),
            "{trunk:?}"
        );
        // a port with no mode set negotiates; an access VLAN no VLAN block defines is inactive
        let access = block("Twe1/0/3");
        assert!(access.contains(&"Administrative Mode: dynamic auto".to_string()));
        assert!(access.contains(&"Negotiation of Trunking: On".to_string()));
        assert!(access.contains(&"Access Mode VLAN: 30 (Inactive)".to_string()));
        assert!(access.contains(&"Voice VLAN: 10 (users)".to_string()));
        // a routed port is not listed; the first block opens the output
        assert!(!out.iter().any(|l| l == "Name: Twe1/0/4"), "{out:?}");
        assert_eq!(out[0], "Name: Twe1/0/1");
        assert!(out.iter().any(|l| l == "Vepa Enabled: false"));
    }

    #[test]
    fn show_power_inline_lists_each_poe_member_and_port() {
        let e = engine();
        let shown = |spec: &str, lines: &[&str]| {
            let mut s = shell::Session::new(
                &e,
                device::Device::new(platform::parse_stack(spec).unwrap()),
            );
            for l in ["enable", "configure terminal"]
                .iter()
                .chain(lines)
                .chain(&["end"])
            {
                assert_eq!(s.type_line(l).1, shell::Result::Ok, "{l}");
            }
            s.type_line("show power inline").0
        };
        // the 9300 command reference's example is a C9300-48UN: its header, its 610 W module row
        // with nothing drawn, its Fi1/0/2 row
        let un = shown("C9300-48UN", &[]);
        assert_eq!(
            un[..9],
            [
                "",
                "Module   Available     Used     Remaining",
                "          (Watts)     (Watts)    (Watts) ",
                "------   ---------   --------   ---------",
                "1           610.0        0.0       610.0",
                "Interface Admin  Oper       Power   Device              Class Max",
                "                            (Watts)                            ",
                "--------- ------ ---------- ------- ------------------- ----- ----",
                "Fi1/0/1   auto   off        0.0     n/a                 n/a   60.0 ",
            ]
        );
        assert_eq!(
            un[9],
            "Fi1/0/2   auto   off        0.0     n/a                 n/a   60.0 "
        );
        assert_eq!(un.len(), 8 + 48);
        // static reserves its maximum from the budget; never is off
        let set = shown(
            "C9300-48UN",
            &[
                "interface FiveGigabitEthernet1/0/3",
                "power inline static max 30000",
                "interface FiveGigabitEthernet1/0/4",
                "power inline never",
            ],
        );
        assert_eq!(set[4], "1           610.0       30.0       580.0");
        assert_eq!(
            set[10],
            "Fi1/0/3   static off        30.0    n/a                 n/a   30.0 "
        );
        assert_eq!(
            set[11],
            "Fi1/0/4   off    off        0.0     n/a                 n/a   60.0 "
        );
        // a stack lists its PoE members only; PoE+ ports give 30 W
        let mixed = shown("C9300-48P,C9300-48T", &[]);
        assert_eq!(mixed[4], "1           437.0        0.0       437.0");
        assert!(mixed[5].starts_with("Interface"));
        assert!(
            mixed
                .iter()
                .any(|l| l.starts_with("Gi1/0/48 ") && l.ends_with("30.0 "))
        );
        assert!(!mixed.iter().any(|l| l.starts_with("Gi2/")));
        assert!(shown("C9300-48T", &[]).is_empty());
        // 2.5G ports print `Tw`; an auto port's max lowers its Max
        let uxm = shown(
            "C9300-48UXM",
            &[
                "interface TwoGigabitEthernet1/0/36",
                "power inline auto max 15400",
            ],
        );
        assert!(
            uxm.contains(
                &"Tw1/0/36  auto   off        0.0     n/a                 n/a   15.4 ".to_string()
            ),
            "{uxm:?}"
        );
        assert!(uxm.iter().any(|l| l.starts_with("Te1/0/37 ")));
        // a 9400 card's budget is the chassis's: its ports, no module row
        let e4 =
            cli::Engine::new(grammars::grammar("c9400", Release::parse("17.5").unwrap()).unwrap());
        let mut s = shell::Session::new(
            &e4,
            device::Device::new(platform::parse_stack("1=C9400-LC-48U,3=C9400-SUP-1XL").unwrap()),
        );
        s.type_line("enable");
        let card = s.type_line("show power inline").0;
        assert!(card[1].starts_with("Interface"), "{card:?}");
        assert_eq!(card.len(), 4 + 48);
    }

    #[test]
    fn a_pipe_filters_an_exec_command_s_output() {
        let e = engine();
        let mut s = session(&e);
        assert_eq!(
            s.type_line("show running-config | include hostname").0,
            ["hostname lab1"]
        );
        assert_eq!(
            s.type_line("sh run | sec interface Vlan10").0,
            [
                "interface Vlan10",
                " ip address 10.0.10.1 255.255.255.255"
                    .replace("255.255.255.255", "255.255.255.0")
                    .as_str()
            ]
        );
        assert_eq!(
            s.type_line("show running-config | count ^interface Port").0,
            ["Number of lines which match regexp = 1"]
        );
        // the modifier is refused as IOS refuses it, the caret under the line typed after the prompt
        let (o, r) = s.type_line("show running-config | bogus x");
        assert_eq!(r, shell::Result::Error);
        assert_eq!(
            o[0],
            format!("{}^", " ".repeat("lab1#show running-config | ".len()))
        );
        assert_eq!(
            s.type_line("show running-config | include").0,
            ["% Incomplete command.", ""]
        );
        // in configuration mode `|` is text, and `do` hands EXEC the whole line
        s.type_line("configure terminal");
        assert_eq!(
            s.type_line("do show running-config | include ^hostname").0,
            ["hostname lab1"]
        );
        s.type_line("interface Twe1/0/6");
        assert_eq!(s.type_line("description a|b").1, shell::Result::Ok);
        assert!(show::configuration(&s.device).contains(&" description a|b".to_string()));
    }

    #[test]
    fn show_interfaces_status_vlan_brief_and_ip_interface_brief_follow_the_configuration() {
        let e = engine();
        let mut s = session(&e);
        let status = s.type_line("show interfaces status").0;
        assert_eq!(
            status[1],
            "Port         Name               Status       Vlan       Duplex  Speed Type"
        );
        let row = |p: &str| status.iter().find(|r| r.starts_with(p)).unwrap().clone();
        assert_eq!(
            row("Twe1/0/5 "),
            "Twe1/0/5     printer            notconnect   10           auto   auto unknown"
        );
        assert!(row("Po10 ").contains(" notconnect "));
        assert!(
            !status.iter().any(|r| r.starts_with("Gi0/0 ")),
            "the management port is not listed"
        );
        let brief = s.type_line("show vlan brief").0;
        assert!(brief.contains(&format!(
            "{:<4} {:<32} {:<9} Twe1/0/5",
            10, "users", "active"
        )));
        let ip = s.type_line("show ip interface brief").0;
        let line = |i: &str| ip.iter().find(|r| r.starts_with(i)).unwrap().clone();
        assert_eq!(
            line("Vlan1 "),
            "Vlan1                  unassigned      YES unset  administratively down down    "
        );
        // VLAN 10 exists and no running port carries it: up, its protocol down (the C2960G's Vlan1)
        assert_eq!(
            line("Vlan10 "),
            "Vlan10                 10.0.10.1       YES NVRAM  up                    down    "
        );
        assert_eq!(
            line("Loopback0 "),
            "Loopback0              10.255.0.1      YES NVRAM  up                    up      "
        );
        assert!(s.type_line("show interfaces trunk").0.is_empty());
        // every port is listed, those never configured as an empty block
        let run = show::configuration(&s.device).join("\n");
        assert!(
            run.contains("\ninterface TwentyFiveGigE2/0/24\n!\n"),
            "{run}"
        );
        assert!(run.starts_with("!\nversion 17.15\n"));
    }

    #[test]
    fn show_etherchannel_summary_lists_each_bundle_down() {
        let e = engine();
        let mut s = session(&e);
        let out = s.type_line("show etherchannel summary").0;
        assert_eq!(
            out[0],
            "Flags:  D - down        P - bundled in port-channel"
        );
        assert!(out.contains(&"Number of channel-groups in use: 1".to_string()));
        assert_eq!(
            out[out.len() - 2],
            "10     Po10(SD)        LACP      Twe1/0/1(D) Twe2/0/1(D)"
        );
        // `switchport mode trunk` typed on the port-channel reached its members, as IOS applies it
        let member = s.type_line("show running-config interface Twe2/0/1").0;
        assert!(
            member.contains(&" switchport mode trunk".to_string()),
            "{member:?}"
        );
        assert!(lint::lint(&s.device).findings.is_empty());
    }

    #[test]
    fn enable_on_a_supervisor_qsfp_port_runs_it_and_turns_off_ports_1_to_4() {
        // Supervisor Module Installation Note: `enable` under FortyGigabitEthernet3/0/9
        let r = Release::parse("17.15").unwrap();
        let e = cli::Engine::new(grammars::grammar("c9400", r).unwrap());
        let st = platform::parse_stack("1=C9400-LC-48T,3=C9400-SUP-1XL,4=C9400-SUP-1XL").unwrap();
        let mut s = shell::Session::new(&e, device::Device::new(st));
        let status = |s: &mut shell::Session<'_>, port: &str| {
            let out = s.type_line("show interfaces status").0;
            out.iter()
                .find(|r| r.starts_with(&format!("{port} ")))
                .map(|r| r[32..44].trim().to_string())
                .unwrap()
        };
        for l in [
            "enable",
            "configure terminal",
            "interface Fo3/0/9",
            "enable",
            "end",
        ] {
            assert_eq!(s.type_line(l).1, shell::Result::Ok, "{l}");
        }
        assert_eq!(status(&mut s, "Fo3/0/9"), "notconnect");
        assert_eq!(status(&mut s, "Te3/0/1"), "inactive");
        assert_eq!(status(&mut s, "Fo4/0/9"), "inactive");
        assert_eq!(status(&mut s, "Te4/0/1"), "notconnect");
        for l in [
            "configure terminal",
            "interface Fo3/0/9",
            "no enable",
            "end",
        ] {
            s.type_line(l);
        }
        assert_eq!(status(&mut s, "Fo3/0/9"), "inactive");
        assert_eq!(status(&mut s, "Te3/0/1"), "notconnect");
    }

    #[test]
    fn a_design_on_a_qsfp_uplink_writes_enable_and_refuses_the_ports_it_turns_off() {
        let mut spec: generate::Spec =
            toml::from_str(include_str!("../tests/example-core-design.toml")).unwrap();
        let st = platform::parse_stack(&spec.stack.join(",")).unwrap();
        spec.port_channel[0].members[0].port = "Fo3/0/9".into();
        let config = generate::render(&spec);
        let at = config
            .iter()
            .position(|l| l == "interface FortyGigabitEthernet3/0/9")
            .unwrap();
        assert_eq!(config[at + 1], " enable");
        assert!(generate::inactive_ports(&spec, &st).is_empty());
        spec.port_channel[0].members[1].port = "Te3/0/1".into();
        let why = generate::inactive_ports(&spec, &st);
        assert!(
            why.len() == 1 && why[0].contains("is off while port 9 is enabled"),
            "{why:?}"
        );
    }
}

#[cfg(all(test, captures))]
include!(concat!(
    env!("VSWITCH_SITE_TESTS"),
    "/main-live_capture_tests.rs"
));

#[cfg(test)]
mod command_tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    // Every secret here is invented for the test.
    const CONFIG: &str = "hostname t1\nenable secret 9 $9$INVENTEDhashFOR.test\n";

    #[test]
    fn sweep_never_cleans_in_place_when_out_was_named() {
        let t = library::tests::TempDir::new("sweep-out");
        let f = t.0.join("c.txt");
        std::fs::write(&f, CONFIG).unwrap();
        let file = f.to_string_lossy().to_string();
        // `--out` with its folder left off, or before another option, names no folder
        for bad in [
            args(&["sweep", &file, "--out"]),
            args(&["sweep", &file, "--out", "--quiet"]),
            args(&["sweep", &file, "--copies"]),
        ] {
            assert!(run(&bad).is_err(), "{bad:?}");
            assert_eq!(std::fs::read_to_string(&f).unwrap(), CONFIG, "{bad:?}");
        }
        let copies = t.0.join("copies");
        let out = format!("--out={}", copies.display());
        assert_eq!(run(&args(&["sweep", &file, &out])), Ok(true));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), CONFIG);
        assert_eq!(
            std::fs::read_to_string(copies.join("c.txt")).unwrap(),
            "hostname t1\nenable secret 9 <removed>\n"
        );
    }

    #[test]
    fn sweep_fails_on_a_path_it_cannot_find_or_a_file_it_left_alone() {
        let t = library::tests::TempDir::new("sweep-exit");
        let missing = t.0.join("conifg.txt").to_string_lossy().to_string();
        assert_eq!(run(&args(&["sweep", &missing])), Ok(false));
        // UTF-16, as Windows PowerShell 5.1 redirection writes it, is left alone: not a clean file
        let wide = t.0.join("wide.txt");
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(CONFIG.encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(&wide, &bytes).unwrap();
        assert_eq!(run(&args(&["sweep", &wide.to_string_lossy()])), Ok(false));
        let plain = t.0.join("plain.txt");
        std::fs::write(&plain, CONFIG).unwrap();
        assert_eq!(run(&args(&["sweep", &plain.to_string_lossy()])), Ok(true));
    }

    #[test]
    fn load_replaces_a_listed_switch_only_when_told_to() {
        let t = library::tests::TempDir::new("load-replace");
        let mut lib = library::Library::open(t.0.join("list")).unwrap();
        let (a, b) = (t.0.join("a.cfg"), t.0.join("b.cfg"));
        std::fs::write(
            &a,
            "hostname core1\nvlan 10\n name KEEPME\ninterface GigabitEthernet1/0/1\n",
        )
        .unwrap();
        std::fs::write(&b, "hostname CORE1\ninterface GigabitEthernet1/0/2\n").unwrap();
        let load = |lib: &mut library::Library, f: &std::path::Path, more: &[&str]| {
            let f = f.to_string_lossy().to_string();
            let mut a = args(&["load", &f, "--stack", "C9300-48P"]);
            a.extend(args(more));
            load_into(lib, &f, &a)
        };
        load(&mut lib, &a, &[]).unwrap();
        let e = load(&mut lib, &b, &[]).unwrap_err();
        assert!(
            e.contains("core1 is in the list") && e.contains("--replace"),
            "{e}"
        );
        assert!(lib.config("core1").unwrap().contains("KEEPME"));
        let said = load(&mut lib, &b, &["--replace"]).unwrap();
        assert!(said.ends_with("it replaced core1"), "{said}");
        let names: Vec<&str> = lib.switches.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["CORE1"]);
    }

    #[test]
    fn run_reads_a_script_saved_with_a_byte_order_mark() {
        let t = library::tests::TempDir::new("run-bom");
        let f = t.0.join("bom.txt");
        std::fs::write(
            &f,
            "\u{feff}enable\nconfigure terminal\nhostname bomtest\nend\n",
        )
        .unwrap();
        assert_eq!(
            run(&args(&[
                "run",
                &f.to_string_lossy(),
                "--stack",
                "C9300-48P",
                "--quiet"
            ])),
            Ok(true)
        );
    }

    #[cfg(all(test, captures))]
    include!(concat!(
        env!("VSWITCH_SITE_TESTS"),
        "/main-command_tests.rs"
    ));
}
