# Testing

## Tests

| Command | Runs |
|---|---|
| `cargo nextest run` | the unit tests, the corpus's integrity checks, the terminal tests, and with a captures folder the capture tests below |
| `VSWITCH_CAPTURES=<captures folder> cargo test --release` | the full suite with captures, which takes about 10 minutes |
| `VSWITCH_CAPTURES=<empty or missing folder> cargo test --release` | the suite without captures |
| `node --test tools/extract-releases.test.mjs` | the extractor tests |

A build reads captures from the folder `VSWITCH_CAPTURES` names, else from `../virtual-switches-captures/live`. A name that is not a folder gives a build without captures.

### Without captures

- The unit tests.
- The corpus's integrity: only characterized values (`«ipv4»`, `«ipv6»`, `«mac»`, `«number»`, `«word»`, `«text»`), and no address, MAC address or serial.
- The terminal tests.
- The key table of [Terminal](terminal.md) held against the list's keys.

### With a captures folder

- Every capture typed at its platform and release, the captures shared over the machine's threads and each platform release's engine built once.
- Each capture's configuration prints back as captured.
- Each show command and each error or `?` answer prints as the switch gave it.
- The lines the books alone refuse are held against `coverage.txt`, the baseline `vswitch coverage --baseline` writes.
- Every capture holds no secret the sweep would take.
- `data/corpus.json.gz` is what `vswitch corpus` writes from the folder.
- No file this repository publishes names the site: every published file is scanned, `.gz` files decompressed.
- Each lab session (`<host>-session-<name>.log` beside a capture) is typed in order on that capture, with every answer and prompt as the switch gave them.

### nextest profiles

`.config/nextest.toml`:

| Profile | Setting |
|---|---|
| default | a test still running after 60 s is re-reported every 60 s and stopped after 5 periods, 300 s |
| default, `corpus::tests::` | the corpus's own checks, which rebuild it from the captures, stop after 10 periods of 120 s, 20 minutes |
| `ci` | every test runs and every failure is reported; a test is named after 15 s and stopped after 20 periods, 300 s; each test's time is kept in `target/nextest/ci/junit.xml` |

## Extractor tests

`node --test tools/extract-releases.test.mjs` tests `tools/extract-releases.js` in a headless Edge, Chrome or Chromium. Node 22 or later runs it.

- Every page is served through the DevTools protocol, so nothing reaches cisco.com or the Wayback Machine.
- The tests cover page kinds, redirects, retries, failures and refusals, archived captures, platform names and the flush.
- Each test stops after 60 s.
- `VSWITCH_BROWSER` names the browser. Otherwise Edge, Chrome (Windows paths) or `/usr/bin/chromium`, `/usr/bin/chromium-browser`, `/usr/bin/google-chrome` is used. The tests are skipped when none exists.
- `VSWITCH_EXTRACTOR` names another copy of the extractor.

## CI

`.gitea/workflows/ci.yml` runs on the forge's shared runner, which has capacity 1, Linux labels only and 4 CPUs a job.

| Trigger | Condition |
|---|---|
| push to main | the push changes `src/`, `data/`, `tests/`, `tools/extract-releases.js`, `docs/*.md`, `build.rs`, `Cargo.toml`, `Cargo.lock`, `deny.toml`, `.config/` or `.gitea/workflows/` |
| by hand | `workflow_dispatch` |
| weekly | Monday 04:23 UTC |

`tools/extract-releases.js` is a trigger because a test holds the extractor's platform names to the grammars'. `docs/*.md` is a trigger because a test holds the list's keys to the table in `docs/terminal.md`.

The job stops at 60 minutes. Each step carries its own limit, and the checks that need no build run first. The job has no captures folder, so it runs the tests without captures.

| Step | Limit | Runs on |
|---|---|---|
| checkout, full history | 5 min | every run |
| line endings: files listed, none stored with CRLF | 2 min | every run |
| attribution: no co-author or generated-with trailer on any ref, no `refs/original/` or backup ref; commits are counted first and the matcher must find a trailer written to test it | 2 min | push and by hand |
| toolchain, pinned to Rust 1.98.1 | 10 min | every run |
| cargo-deny 0.20.2 and cargo-nextest 0.9.146 release binaries, checked against their published SHA-256 digests | 5 min | every run |
| `cargo deny check` | 10 min | every run |
| `cargo fmt --check` | 5 min | push and by hand |
| `cargo clippy --locked --all-targets -- -D warnings` | 20 min | push and by hand |
| `cargo nextest run --locked --cargo-profile ci-test --profile ci` | 40 min | push and by hand |
| upload of `test.log` and `target/nextest/ci/junit.xml` as `test-results`, kept 14 days | 5 min | push and by hand, whatever the test result |

- The weekly run covers line endings, the toolchain, the pinned tools and `cargo deny check`, since advisories move while `Cargo.lock` does not.
- The test step prints the count of tests run, which must be at least 488, and the 15 slowest tests.
- `ci-test` builds the crate at opt-level 1 with line tables, which runs the suite in about 70% of the time for about 28 s more build, measured on four CPUs.
- Builds use `CARGO_INCREMENTAL=0`.
- Steps run under bash with `-e` and pipefail.

## Testing the terminal interface

`tests/terminal.rs` runs the binary on a pseudo-terminal at 80x24 and 132x43 and reads the screen through a VT parser. Each scenario runs at both sizes as `at_80x24::<name>` and `at_132x43::<name>`. The scenarios cover the list, loading a design and a configuration, the builder, a switch's console and leaving it. Each test checks what a step does (a prompt, a kept file, the list coming back) and not where it is drawn.

- On Windows the terminal is a ConPTY, the console Windows Terminal hosts, and keys go in the win32 input mode Windows Terminal sends.
- Elsewhere it is an openpty terminal with xterm's key sequences.
- Each run uses a fresh `VSWITCH_HOME` in the temp folder, and the harness (`tests/harness`) refuses any other folder, so a test never reads or writes the user's own list.
- A failed wait prints the screen in a frame with its size, the cursor, the screen in use and the bells rung, and writes it to a file.

| Command | Runs |
|---|---|
| `cargo nextest run --test terminal` | the scenarios |
| `cargo test --test terminal_audit audit -- --ignored` | every script in `tests/audit`, each screen written to `target/tmp/screens/audit-<script>/` |
| `cargo test --test terminal_audit script -- --ignored --nocapture` | the script `VSWITCH_SCRIPT` names; the commands a script takes are listed at `run` in `tests/terminal_audit.rs` |
| `cargo test --test terminal_audit timings -- --ignored --nocapture` | time to the list's first screen and from Enter to a switch's prompt, five runs each, for a design and two configurations |
| `cargo test --test terminal_audit screens -- --ignored` | the screenshots in `docs/screens` from `tests/screens.txt`: each screen as Windows Terminal draws it, an HTML page photographed by a headless Edge, Chrome or Chromium (`VSWITCH_BROWSER` names another) |

| Variable | Sets |
|---|---|
| `VSWITCH_BIN` | the binary to run, such as `target/release/vswitch`; the test build's by default |
| `VSWITCH_SCREENS` | where screens are written; `target/tmp/screens` by default |
| `VSWITCH_HARNESS_KEEP` | keeps each run's switch list folder |
| `VSWITCH_SCRIPT` | the script the `script` audit runs |
| `VSWITCH_BROWSER` | the browser that photographs screens |
