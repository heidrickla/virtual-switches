//! Embeds every compiled grammar in data/grammars (`<platform>-<release>.json.gz`) as a table the
//! binary picks one from by platform and release, and each book's help table in data/help
//! (`<platform>.json.gz`).

use std::fs;
use std::path::Path;

/// `(stem, absolute path)` of every `.json.gz` in `dir`, sorted; none when the folder is missing.
fn gz(dir: &str) -> Vec<(String, String)> {
    println!("cargo:rerun-if-changed={dir}");
    let mut out: Vec<(String, String)> = fs::read_dir(Path::new(dir))
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let path = e.ok()?.path();
            let stem = path
                .file_name()?
                .to_str()?
                .strip_suffix(".json.gz")?
                .to_string();
            let abs = fs::canonicalize(&path).ok()?;
            Some((stem, abs.display().to_string()))
        })
        .collect();
    out.sort();
    out
}

fn main() {
    // statics, not consts: a const's bytes can be copied into every place that names it
    let mut out = String::from(
        "/// (platform, release, gzip-compressed grammar JSON) for every compiled release\n\
         pub static GRAMMARS: &[(&str, &str, &[u8])] = &[\n",
    );
    for (stem, path) in gz("data/grammars") {
        if let Some((platform, release)) = stem.split_once('-') {
            out.push_str(&format!(
                "    ({platform:?}, {release:?}, include_bytes!({path:?})),\n"
            ));
        }
    }
    out.push_str(
        "];\n\n/// (platform, gzip-compressed help JSON): command name to keyword to text\n\
         pub static HELP: &[(&str, &[u8])] = &[\n",
    );
    for (platform, path) in gz("data/help") {
        out.push_str(&format!("    ({platform:?}, include_bytes!({path:?})),\n"));
    }
    out.push_str("];\n");
    // what the captures teach, compiled (src/corpus.rs); empty until `vswitch corpus` writes it
    println!("cargo:rerun-if-changed=data/corpus.json.gz");
    match fs::canonicalize("data/corpus.json.gz") {
        Ok(path) => out.push_str(&format!(
            "\n/// the corpus, gzip-compressed JSON\npub static CORPUS: &[u8] = include_bytes!({:?});\n",
            path.display().to_string()
        )),
        Err(_) => out.push_str("\n/// the corpus, gzip-compressed JSON\npub static CORPUS: &[u8] = &[];\n"),
    }
    // the captures themselves, only in a build that names a folder of them: VSWITCH_CAPTURES, else
    // the private repository's live/ beside this one
    println!("cargo::rustc-check-cfg=cfg(captures)");
    println!("cargo:rerun-if-env-changed=VSWITCH_CAPTURES");
    let captures = std::env::var("VSWITCH_CAPTURES")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "../virtual-switches-captures/live".to_string());
    let captures = if Path::new(&captures).is_dir() {
        println!("cargo:rustc-cfg=captures");
        // the tests that read particular captures, kept with them (`tests` beside `live`)
        // absolute, but not the `\\?\` form canonicalize gives on Windows, which takes no `/`
        let absolute = |p: &Path| {
            std::path::absolute(p)
                .unwrap_or_else(|_| p.to_path_buf())
                .display()
                .to_string()
        };
        let tests = absolute(&Path::new(&captures).join("../tests"));
        println!("cargo:rerun-if-changed={tests}");
        println!("cargo:rustc-env=VSWITCH_SITE_TESTS={tests}");
        println!(
            "cargo:rustc-env=VSWITCH_CAPTURES_DIR={}",
            absolute(Path::new(&captures))
        );
        captures
    } else {
        String::new()
    };
    if captures.is_empty() {
        let target = Path::new(&std::env::var("OUT_DIR").unwrap()).join("grammars.rs");
        fs::write(target, out).unwrap();
        return;
    }
    // real switches' running configurations: <captures>/<platform>-<release>/<name>running-config.txt
    out.push_str(
        "\n/// (platform, release, file, text) of every running configuration a real switch printed\n\
         pub static LIVE: &[(&str, &str, &str, &str)] = &[\n",
    );
    println!("cargo:rerun-if-changed={captures}");
    let mut live: Vec<(String, String, String, String)> = vec![];
    let mut logs: Vec<(String, String, String, String)> = vec![];
    let mut stacks: Vec<(String, String, String, String)> = vec![];
    for dir in fs::read_dir(&captures).into_iter().flatten().flatten() {
        let name = dir.file_name().to_string_lossy().to_string();
        let Some((platform, release)) = name.split_once('-') else {
            continue;
        };
        for f in fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            let file = f.file_name().to_string_lossy().to_string();
            let abs = fs::canonicalize(f.path()).unwrap().display().to_string();
            if file.ends_with("running-config.txt") {
                println!("cargo:rerun-if-changed={abs}");
                live.push((platform.into(), release.into(), file, abs));
            } else if file.ends_with(".log") {
                println!("cargo:rerun-if-changed={abs}");
                logs.push((platform.into(), release.into(), file, abs));
            } else if file.ends_with("-stack.txt") {
                println!("cargo:rerun-if-changed={abs}");
                stacks.push((platform.into(), release.into(), file, abs));
            }
        }
    }
    live.sort();
    for (platform, release, file, path) in live {
        out.push_str(&format!(
            "    ({platform:?}, {release:?}, {file:?}, include_str!({path:?})),\n"
        ));
    }
    out.push_str("];\n");
    // their session logs: show output and `?` answers
    out.push_str(
        "\n/// (platform, release, file, text) of every session log taken from a real switch\n\
         pub static LIVE_LOGS: &[(&str, &str, &str, &str)] = &[\n",
    );
    logs.sort();
    for (platform, release, file, path) in logs {
        out.push_str(&format!(
            "    ({platform:?}, {release:?}, {file:?}, include_str!({path:?})),\n"
        ));
    }
    out.push_str("];\n");
    // the hardware of a switch whose session names none (the cores'), in `--stack` notation
    out.push_str(
        "\n/// (platform, release, file, text) of every `<host>-stack.txt` beside a capture\n\
         pub static LIVE_STACKS: &[(&str, &str, &str, &str)] = &[\n",
    );
    stacks.sort();
    for (platform, release, file, path) in stacks {
        out.push_str(&format!(
            "    ({platform:?}, {release:?}, {file:?}, include_str!({path:?})),\n"
        ));
    }
    out.push_str("];\n");
    let target = Path::new(&std::env::var("OUT_DIR").unwrap()).join("grammars.rs");
    fs::write(target, out).unwrap();
}
