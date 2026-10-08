//! A screen as Windows Terminal draws it, saved as a PNG: an HTML page of the cells in the Campbell
//! scheme, dark, photographed by a headless Edge, Chrome or Chromium (`VSWITCH_BROWSER` names
//! another). Box-drawing and block characters are drawn as shapes, so lines join across cells.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A cell in pixels: Cascadia Mono at 17 px advances 9.96 px and stands 20 px tall.
const W: u32 = 10;
const H: u32 = 20;
/// The background around the cells, as a terminal pads its window.
const PAD: u32 = 12;

/// Windows Terminal's Campbell scheme: ANSI colours 0 to 15, the default foreground and background,
/// and the cursor.
const ANSI: [u32; 16] = [
    0x0C0C0C, 0xC50F1F, 0x13A10E, 0xC19C00, 0x0037DA, 0x881798, 0x3A96DD, 0xCCCCCC, 0x767676,
    0xE74856, 0x16C60C, 0xF9F1A5, 0x3B78FF, 0xB4009E, 0x61D6D6, 0xF2F2F2,
];
const FOREGROUND: u32 = 0xCCCCCC;
const BACKGROUND: u32 = 0x0C0C0C;
const CURSOR: u32 = 0xFFFFFF;

/// The arms of U+2500 to U+257F, up, right, down and left: 1 light, 2 heavy, 3 double. Dashes,
/// arcs (U+256D to U+2570) and diagonals (U+2571 to U+2573) are drawn apart.
const ARMS: [&str; 8] = [
    "0101 0202 1010 2020 0101 0202 1010 2020 0101 0202 1010 2020 0110 0210 0120 0220",
    "0011 0012 0021 0022 1100 1200 2100 2200 1001 1002 2001 2002 1110 1210 2110 1120",
    "2120 2210 1220 2220 1011 1012 2011 1021 2021 2012 1022 2022 0111 0112 0211 0212",
    "0121 0122 0221 0222 1101 1102 1201 1202 2101 2102 2201 2202 1111 1112 1211 1212",
    "2111 1121 2121 2112 2211 1122 1221 2212 1222 2122 2221 2222 0101 0202 1010 2020",
    "0303 3030 0310 0130 0330 0013 0031 0033 1300 3100 3300 1003 3001 3003 1310 3130",
    "3330 1013 3031 3033 0313 0131 0333 1303 3101 3303 1313 3131 3333 0110 0011 1001",
    "1100 0000 0000 0000 0001 1000 0100 0010 0002 2000 0200 0020 0201 1020 0102 2010",
];

/// The image's size in pixels for a screen of `cols` by `rows`.
pub fn size(cols: u16, rows: u16) -> (u32, u32) {
    (u32::from(cols) * W + 2 * PAD, u32::from(rows) * H + 2 * PAD)
}

/// A colour as xterm maps it: the scheme's sixteen, the 6x6x6 cube, the greys, or RGB.
fn rgb(c: vt100::Color, default: u32) -> u32 {
    use vt100::Color::{Default, Idx, Rgb};
    let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * u32::from(v) };
    match c {
        Default => default,
        Idx(i @ 0..=15) => ANSI[usize::from(i)],
        Idx(i @ 16..=231) => {
            let i = i - 16;
            (level(i / 36) << 16) | (level(i / 6 % 6) << 8) | level(i % 6)
        }
        Idx(i) => 0x080808 + 0x0A0A0A * u32::from(i - 232),
        Rgb(r, g, b) => (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
    }
}

/// A cell's foreground and background: inverse swaps them, faint mixes the foreground half into
/// the background.
fn colours(cell: &vt100::Cell) -> (u32, u32) {
    let (mut fg, mut bg) = (
        rgb(cell.fgcolor(), FOREGROUND),
        rgb(cell.bgcolor(), BACKGROUND),
    );
    if cell.inverse() {
        std::mem::swap(&mut fg, &mut bg);
    }
    if cell.dim() {
        fg = [0, 8, 16]
            .iter()
            .map(|s| ((((fg >> s) & 0xFF) + ((bg >> s) & 0xFF)) / 2) << s)
            .sum();
    }
    (fg, bg)
}

/// Path data for a rectangle.
fn rect(d: &mut String, x: f32, y: f32, w: f32, h: f32) {
    let _ = write!(d, "M{x} {y}h{w}v{h}h{}z", -w);
}

/// The shapes of a box-drawing or block character in the cell at `(x, y)`: filled path data, the
/// fill's opacity, and stroked path data. None for any other character.
fn shapes(ch: char, x: f32, y: f32) -> Option<(String, f32, String)> {
    let (mut fill, mut stroke) = (String::new(), String::new());
    let (w, h) = (W as f32, H as f32);
    let mut at =
        |x0: f32, y0: f32, x1: f32, y1: f32| rect(&mut fill, x + x0, y + y0, x1 - x0, y1 - y0);
    let code = ch as u32;
    match code {
        0x2500..=0x257F => {
            let arms: Vec<u32> = ARMS[(code as usize - 0x2500) / 16]
                .split(' ')
                .nth((code as usize - 0x2500) % 16)?
                .chars()
                .filter_map(|c| c.to_digit(10))
                .collect();
            let [u, r, d, l] = arms[..] else { return None };
            let dashes = match code {
                0x2504..=0x2507 => 3,
                0x2508..=0x250B => 4,
                0x254C..=0x254F => 2,
                _ => 1,
            };
            match code {
                // arcs and diagonals, stroked along the light lines' centres so their ends meet them
                0x256D..=0x2570 => {
                    let d = [
                        "M10 9.5H8.5A4 4 0 0 0 4.5 13.5V20",
                        "M0 9.5H0.5A4 4 0 0 1 4.5 13.5V20",
                        "M0 9.5H0.5A4 4 0 0 0 4.5 5.5V0",
                        "M10 9.5H8.5A4 4 0 0 1 4.5 5.5V0",
                    ][code as usize - 0x256D];
                    let _ = write!(stroke, r#"<path transform="translate({x} {y})" d="{d}"/>"#);
                }
                0x2571..=0x2573 => {
                    if code != 0x2572 {
                        let _ = write!(stroke, r#"<path d="M{} {y}L{x} {}"/>"#, x + w, y + h);
                    }
                    if code != 0x2571 {
                        let _ = write!(stroke, r#"<path d="M{x} {y}L{} {}"/>"#, x + w, y + h);
                    }
                }
                _ if dashes > 1 => {
                    let (heavy, across) = (u == 2 || r == 2, r > 0);
                    let (t0, t1) = if heavy { (3.0, 6.0) } else { (4.0, 5.0) };
                    let (u0, u1) = if heavy { (8.0, 11.0) } else { (9.0, 10.0) };
                    let n = dashes as f32;
                    for k in 0..dashes {
                        let k = k as f32;
                        if across {
                            let p = w / n;
                            at(k * p + p * 0.2, u0, k * p + p * 0.8, u1);
                        } else {
                            let p = h / n;
                            at(t0, k * p + p * 0.2, t1, k * p + p * 0.8);
                        }
                    }
                }
                _ if [u, r, d, l].contains(&3) => doubles([u, r, d, l], &mut at),
                _ => {
                    // light and heavy: each arm runs through the widest band across it
                    let v = |k: u32| if k == 2 { (3.0, 6.0) } else { (4.0, 5.0) };
                    let hz = |k: u32| if k == 2 { (8.0, 11.0) } else { (9.0, 10.0) };
                    let across =
                        |a: u32, b: u32, own: u32| if a.max(b) > 0 { a.max(b) } else { own };
                    if u > 0 {
                        at(v(u).0, 0.0, v(u).1, hz(across(l, r, u)).1);
                    }
                    if d > 0 {
                        at(v(d).0, hz(across(l, r, d)).0, v(d).1, h);
                    }
                    if l > 0 {
                        at(0.0, hz(l).0, v(across(u, d, l)).1, hz(l).1);
                    }
                    if r > 0 {
                        at(v(across(u, d, r)).0, hz(r).0, w, hz(r).1);
                    }
                }
            }
            Some((fill, 1.0, stroke))
        }
        0x2580..=0x259F => {
            let eighth = |k: u32| k as f32 / 8.0;
            let mut opacity = 1.0;
            match code {
                0x2580 => at(0.0, 0.0, w, h / 2.0),
                0x2581..=0x2588 => at(0.0, h * (1.0 - eighth(code - 0x2580)), w, h),
                0x2589..=0x258F => at(0.0, 0.0, w * eighth(0x2590 - code), h),
                0x2590 => at(w / 2.0, 0.0, w, h),
                0x2591..=0x2593 => {
                    opacity = (code - 0x2590) as f32 / 4.0;
                    at(0.0, 0.0, w, h);
                }
                0x2594 => at(0.0, 0.0, w, h / 8.0),
                0x2595 => at(w * 7.0 / 8.0, 0.0, w, h),
                _ => {
                    // quadrants: upper left 1, upper right 2, lower left 4, lower right 8
                    let q = [4, 8, 1, 13, 9, 7, 11, 2, 6, 14][code as usize - 0x2596];
                    for (bit, qx, qy) in
                        [(1, 0.0, 0.0), (2, 1.0, 0.0), (4, 0.0, 1.0), (8, 1.0, 1.0)]
                    {
                        if q & bit != 0 {
                            let (x0, y0) = (qx * w / 2.0, qy * h / 2.0);
                            at(x0, y0, x0 + w / 2.0, y0 + h / 2.0);
                        }
                    }
                }
            }
            Some((fill, opacity, stroke))
        }
        _ => None,
    }
}

/// Double lines, mixed with single ones. Each line of a double arm stops at the arm beside it on
/// its side (at that arm's inner line when it is double), else at the outer line of the corner
/// the arm on the other side makes, else runs on into the opposite arm.
fn doubles([u, r, d, l]: [u32; 4], at: &mut impl FnMut(f32, f32, f32, f32)) {
    fn reach(
        same: u32,
        other: u32,
        opposite: u32,
        [inner, single, outer, through]: [f32; 4],
    ) -> f32 {
        match (same, other, opposite) {
            (3, _, _) => inner,
            (0, 3, _) => outer,
            (0, 0, 1..) => through,
            _ => single,
        }
    }
    // the single bands, then the double lines: left, right, top, bottom
    let (vs, hs) = ((4.0, 5.0), (9.0, 10.0));
    let (vl, vr, ht, hb) = ((2.0, 3.0), (6.0, 7.0), (7.0, 8.0), (11.0, 12.0));
    let (w, h) = (W as f32, H as f32);
    if u == 3 {
        let ends = [ht.1, hs.1, hb.1, h];
        at(vl.0, 0.0, vl.1, reach(l, r, d, ends));
        at(vr.0, 0.0, vr.1, reach(r, l, d, ends));
    }
    if d == 3 {
        let starts = [hb.0, hs.0, ht.0, 0.0];
        at(vl.0, reach(l, r, u, starts), vl.1, h);
        at(vr.0, reach(r, l, u, starts), vr.1, h);
    }
    if l == 3 {
        let ends = [vl.1, vs.1, vr.1, w];
        at(0.0, ht.0, reach(u, d, r, ends), ht.1);
        at(0.0, hb.0, reach(d, u, r, ends), hb.1);
    }
    if r == 3 {
        let starts = [vr.0, vs.0, vl.0, 0.0];
        at(reach(u, d, l, starts), ht.0, w, ht.1);
        at(reach(d, u, l, starts), hb.0, w, hb.1);
    }
    // a single arm crossing double lines runs through; a tee stops at the near line, a corner at
    // the far one
    let stop = |crossed: bool, both: bool, [through, near, far]: [f32; 3]| match (crossed, both) {
        (true, _) => through,
        (false, true) => near,
        (false, false) => far,
    };
    let (across, along) = ((l == 3, r == 3), (u == 3, d == 3));
    if u == 1 {
        let end = match across {
            (false, false) => hs.1,
            (a, b) => stop(d > 0, a && b, [hs.1, ht.1, hb.1]),
        };
        at(vs.0, 0.0, vs.1, end);
    }
    if d == 1 {
        let start = match across {
            (false, false) => hs.0,
            (a, b) => stop(u > 0, a && b, [hs.0, hb.0, ht.0]),
        };
        at(vs.0, start, vs.1, h);
    }
    if l == 1 {
        let end = match along {
            (false, false) => vs.1,
            (a, b) => stop(r > 0, a && b, [vs.1, vl.1, vr.1]),
        };
        at(0.0, hs.0, end, hs.1);
    }
    if r == 1 {
        let start = match along {
            (false, false) => vs.0,
            (a, b) => stop(l > 0, a && b, [vs.0, vr.0, vl.0]),
        };
        at(start, hs.0, w, hs.1);
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The screen as an HTML page of `size` pixels: the backgrounds, each glyph centred in its cell,
/// then lines, blocks, underlines and the cursor as shapes over them.
pub fn page(s: &vt100::Screen) -> String {
    let (rows, cols) = s.size();
    let (width, height) = (u32::from(cols) * W, u32::from(rows) * H);
    let mut back: BTreeMap<u32, String> = BTreeMap::new();
    // by colour and opacity in hundredths
    let mut fill: BTreeMap<(u32, u32), String> = BTreeMap::new();
    let mut stroke: BTreeMap<u32, String> = BTreeMap::new();
    let mut text = String::new();
    for r in 0..rows {
        let y = u32::from(r) * H;
        let mut c = 0;
        while c < cols {
            let Some(cell) = s.cell(r, c) else {
                c += 1;
                continue;
            };
            let x = u32::from(c) * W;
            let span = if cell.is_wide() { 2 } else { 1 };
            let (fg, bg) = colours(cell);
            if bg != BACKGROUND {
                rect(
                    back.entry(bg).or_default(),
                    x as f32,
                    y as f32,
                    (span * W) as f32,
                    H as f32,
                );
            }
            let glyph = cell.contents();
            let mut chars = glyph.chars();
            let drawn = match (chars.next(), chars.next()) {
                (Some(ch), None) => shapes(ch, x as f32, y as f32),
                _ => None,
            };
            match drawn {
                Some((d, opacity, lines)) => {
                    fill.entry((fg, (opacity * 100.0) as u32))
                        .or_default()
                        .push_str(&d);
                    stroke.entry(fg).or_default().push_str(&lines);
                }
                None if !glyph.trim().is_empty() => {
                    let class = [(cell.bold(), "b"), (cell.italic(), "i"), (span == 2, "w")]
                        .iter()
                        .filter(|(on, _)| *on)
                        .map(|(_, c)| *c)
                        .collect::<Vec<_>>()
                        .join(" ");
                    let _ = write!(
                        text,
                        r#"<span class="{class}" style="left:{x}px;top:{y}px;color:#{fg:06X}">{}</span>"#,
                        escape(glyph)
                    );
                }
                None => {}
            }
            if cell.underline() {
                let d = fill.entry((fg, 100)).or_default();
                rect(d, x as f32, (y + H - 3) as f32, (span * W) as f32, 1.0);
            }
            c += span as u16;
        }
    }
    if !s.hide_cursor() {
        let (r, c) = s.cursor_position();
        if r < rows {
            let x = u32::from(c.min(cols.saturating_sub(1))) * W;
            let d = fill.entry((CURSOR, 100)).or_default();
            rect(d, x as f32, (u32::from(r) * H) as f32, 2.0, H as f32);
        }
    }
    let svg = |paths: String| {
        format!(
            r#"<svg width="{width}" height="{height}" viewBox="0 0 {width} {height}">{paths}</svg>"#
        )
    };
    let backs: String = back
        .iter()
        .map(|(c, d)| format!(r##"<path fill="#{c:06X}" d="{d}"/>"##))
        .collect();
    let mut fronts: String = fill
        .iter()
        .filter(|(_, d)| !d.is_empty())
        .map(|((c, o), d)| {
            format!(
                r##"<path fill="#{c:06X}" fill-opacity="{}" d="{d}"/>"##,
                *o as f32 / 100.0
            )
        })
        .collect();
    for (c, paths) in stroke.iter().filter(|(_, p)| !p.is_empty()) {
        let _ = write!(
            fronts,
            r##"<g fill="none" stroke="#{c:06X}" stroke-width="1">{paths}</g>"##
        );
    }
    format!(
        r##"<!doctype html>
<html><head><meta charset="utf-8"><title>vswitch</title><style>
html,body{{margin:0;background:#{BACKGROUND:06X}}}
#t{{position:absolute;left:{PAD}px;top:{PAD}px;width:{width}px;height:{height}px;font:17px/20px "Cascadia Mono",Consolas,monospace;font-variant-ligatures:none}}
#t>*{{position:absolute;left:0;top:0}}
#t>span{{width:{W}px;height:{H}px;text-align:center;white-space:pre}}
#t>span.w{{width:{}px}}
.b{{font-weight:700}}
.i{{font-style:italic}}
</style></head><body><div id="t">{}{text}{}</div></body></html>
"##,
        2 * W,
        svg(backs),
        svg(fronts)
    )
}

/// The browser that photographs a page: `VSWITCH_BROWSER`, else Edge, Chrome or Chromium where
/// installed.
fn browser() -> PathBuf {
    if let Some(named) = std::env::var_os("VSWITCH_BROWSER") {
        let named = PathBuf::from(named);
        assert!(named.exists(), "VSWITCH_BROWSER: no {}", named.display());
        return named;
    }
    [
        "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
        "C:/Program Files/Google/Chrome/Application/chrome.exe",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
    .expect("no Chromium-family browser; VSWITCH_BROWSER names one")
}

/// A local file as a `file:` URL, each byte outside the unreserved set escaped.
fn file_url(path: &Path) -> String {
    let p = path.display().to_string().replace('\\', "/");
    let mut url = String::from(if p.starts_with('/') {
        "file://"
    } else {
        "file:///"
    });
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                url.push(char::from(b))
            }
            _ => {
                let _ = write!(url, "%{b:02X}");
            }
        }
    }
    url
}

/// Photographs the page `html` into `png` at `size` pixels, and checks the browser wrote a PNG of
/// that size. The browser runs on a profile of its own in the target folder.
pub fn photograph(html: &Path, png: &Path, (width, height): (u32, u32)) {
    let browser = browser();
    let profile = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("shot-browser");
    if let Some(dir) = png.parent() {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    }
    // a PNG left from an earlier run must not pass for this one
    let _ = std::fs::remove_file(png);
    let mut child = Command::new(&browser)
        .args([
            "--headless=new",
            "--disable-gpu",
            "--hide-scrollbars",
            "--force-device-scale-factor=1",
            // grey antialiasing: LCD text's coloured fringes show once a page scales the image
            "--disable-lcd-text",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-sync",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-breakpad",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--window-size={width},{height}"))
        .arg(format!("--screenshot={}", png.display()))
        .arg(file_url(html))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("{}: {e}", browser.display()));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "{} did not photograph {} within 60 s",
                    browser.display(),
                    html.display()
                );
            }
        }
    }
    let head = std::fs::read(png).unwrap_or_else(|e| panic!("{}: {e}", png.display()));
    let dim = |at: usize| {
        head.get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    assert!(
        head.starts_with(b"\x89PNG\r\n\x1a\n") && dim(16) == Some(width) && dim(20) == Some(height),
        "{} is not a {width}x{height} PNG ({} bytes)",
        png.display(),
        head.len()
    );
}
