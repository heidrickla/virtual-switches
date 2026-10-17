# Commands

`vswitch` with no subcommand opens the switch list (see [terminal.md](terminal.md)). Every other form is `vswitch <subcommand> [arguments] [options]`. An unknown subcommand or a missing required argument prints the usage and exits 1.

```
cargo build --release
target/release/vswitch
target/release/vswitch shell
target/release/vswitch run tests/example-draft.txt
target/release/vswitch explain config-if "spanning-tree portfast trunk"
```

## Overview

| Subcommand | Does | Exit 1 when |
|---|---|---|
| (none) | the switch list and the consoles of its switches | never |
| `load` | adds a switch to the list without opening it | the file does not load, or the name is listed and `--replace` is absent |
| `sweep` | removes passwords and secrets from files | a path does not exist, a file is left alone, or a removed value remains in a file |
| `shell` | a console on a switch kept nowhere | never |
| `run` | types a script into a switch | any line is refused |
| `generate` | a configuration, change script or target configuration from a design file | the design, a line it produces, a check or the rollback is refused |
| `explain` | the command a line matches | never |
| `examples` | types the release's documentation examples | never |
| `releases` | the releases each book has a grammar for | never |
| `lint` | checks a configuration as it stands | a finding |
| `check` | what a release change breaks in a configuration | a break, or hardware the target release does not run |
| `verify` | the evidence behind each line of a configuration | never |
| `changes` | what a backup changes on its captured switch | a new refusal or finding, or a file that names no switch |
| `fleet` | every capture checked at a target release | a break, or hardware the release does not run |
| `capture` | a real switch, session log or folder of logs written as a capture | the capture fails |
| `probe` | a script typed on a private lab switch and compared with vswitch | a difference |
| `lab` | switches cabled together and checked | a finding |
| `topology` | a lab file from captured switches | never |
| `corpus` | what the captures teach, compiled into the binary's tables | `--scan` finds a line |
| `coverage` | lines real switches held that the books alone refuse | never |
| `compile`, `compile-releases` | rebuild the grammars | never |

A failed read or a refused option exits 1 with its message on stderr.

## Notation

| Item | Form |
|---|---|
| `<members>` | the stack's models from member 1, joined by `,`, each with an optional `+<network module>` and an optional `<number>=` prefix |
| default `<members>` | `C9300X-24Y+C9300X-NM-4C,C9300X-24Y+C9300X-NM-4C` |
| 9400 member | `<slot>=<card>`, as in `2=C9400-LC-24XS` |
| `<x.y>` | an IOS XE release (`17.9`, `16.12`, `26.x`) or a classic train (`15.2(7)E`); a patch such as `17.9.4` runs its train |
| default release | 17.15 where the book documents it and the hardware runs it, else the newest release the book documents that the hardware runs |
| `--grammar <file>` | a compiled grammar file in place of the embedded book, accepted by the commands that read a grammar |

| Variable | Used by |
|---|---|
| `VSWITCH_HOME` | the folder of the switch list, its configurations and the sweep folder (see [terminal.md](terminal.md)) |
| `VSWITCH_SSH` | the `ssh` program that `capture` and `probe` run, `ssh` when unset |
| `VSWITCH_CAPTURES` | at build time, the folder of captures a build embeds; `coverage` exists only in such a build |

## Switch list and console

| Subcommand | Arguments and options |
|---|---|
| (none) | the switch list; Enter opens the selected switch's console with its configuration loaded, `write memory` there keeps its changes, `exit` comes back to the list |
| `load <design.toml or config>` | `--stack <members>`, `--release <x.y>`, `--replace` |
| `sweep [<file or folder>...]` | `--out <folder>` or `--out=<folder>` |
| `shell` | `--stack <members>`, `--release <x.y>` |
| `run <script>` | `--stack <members>`, `--release <x.y>`, `--quiet` |

### load

Adds a switch to the list without opening it and prints what was kept, the secret lines removed from the kept copy, and any switch it replaced. A listed switch of the same name, or one kept in the same file name, is replaced only with `--replace`, and the replaced switch is named. A design (`.toml`) names its hardware. For a configuration, `--stack` applies only when its provision lines and interface names name no hardware. A file saved as UTF-16 loads. A configuration with no `hostname` line is named after the file, spaces made dashes, and the line is written into the kept copy. A file with no `hostname`, `version`, `interface` or `vlan` line at a line start is not loaded.

### sweep

Removes passwords and secrets from configuration files and session logs, in place or, with `--out`, into copies in that folder; `--out` never cleans in place. With no path it sweeps the sweep folder beside the switch list and prints its path. An option other than `--out` is refused, as is `--out` given twice or without a folder. Each file's report names the kinds of secret cleaned, never a value, and says when a removed value still appears elsewhere in the file. What is covered and what is left alone: [Captures](captures.md), Secret sweep.

### shell

A console from `Switch>` on a switch kept nowhere. A C9300 at IOS XE 17.6 boots its factory default at network-advantage with dna-advantage ([output.md](output.md#factory-default)), its boot the computer's clock, which `show version` counts its uptime from; other platforms and releases start blank. The keys are in [terminal.md](terminal.md). A line known to crash the switch's model prints a warning before its answer, here, in the switch list's console and in `lab --shell` ([output.md](output.md#commands-known-to-crash-a-switch)).

### run

Types each line of a script at its prompt and prints the session, each line after its prompt. The switch starts as `shell` starts it, a C9300 at 17.6 from its factory default. Lines starting with `!` are skipped in configuration modes, and a banner's text is typed as written. A line ending in `?` prints the help for what precedes it and is not entered. `--quiet` prints only the count. The count `<n> line(s) refused` goes to stderr, and exit is 1 when it is above 0. A line known to crash the switch's model prints its warning in the session ([output.md](output.md#commands-known-to-crash-a-switch)); with `--quiet`, `<n> line(s) known to crash this model` goes to stderr before the refused count when n is above 0. The warnings do not change the exit code. A byte order mark at the start of the file is dropped.

## Designs

### generate

`generate <design.toml> [--release <x.y>] [--out <config>] [--change <change.txt>] [--running <capture> --target <target>] [--show-run]`

The configuration is written to `--out`, else to stdout. Messages go to stderr.

| Check | Result |
|---|---|
| a trunk naming undefined VLANs or VLANs outside 1-4094 | design problem |
| a port in two port-channels, or written two ways | design problem |
| a port-channel written twice | design problem |
| `edge` or `lacp_fast` on an existing port-channel | design problem |
| `ssh_only` with no domain for the RSA key | design problem |
| a port the hardware leaves without traffic | design problem |
| any generated line the switch refuses | the line with the switch's error under it, then `nothing written` |

Design problems print as `design: <problem>` and nothing is written.

Every line is typed into the switch before anything is written. The command prints what each port-channel carries, with `except` lists resolved against the defined VLANs.

| Option | Effect |
|---|---|
| `--release <x.y>` | the release; a design's own `release` must agree |
| `--out <config>` | writes the configuration there |
| `--change <change.txt>` | writes the change script from the design's `[change]` table: port table, notes, steps and checks before, the configuration to paste, checks after, rollback |
| `--running <capture> --target <target>` | writes the capture with the change applied as IOS prints it: lines in IOS order, allowed-VLAN lists resolved, a port-channel's switchport lines on its members |
| `--target <target>` alone | writes the new switch's running configuration after the change, in IOS section order, without the lines IOS adds by default |
| `--show-run` | prints the running configuration after the change |

With `--change`, every check command and rollback line is typed into the switch, and a `[change]` table is required. With `--running`, the target is refused unless the rollback leads back to the capture line for line. `--running` without `--target` writes nothing.

## Reference

| Subcommand | Arguments and options |
|---|---|
| `explain <mode> <line>` | `--platform <book or model>`, `--release <x.y>` |
| `examples` | `--show N`, `--canonical`, `--platform <book or model>`, `--release <x.y>` |
| `releases` | none |

### explain

Prints the command a line matches in a mode: its name, source, syntax line, modes, the mode it enters, how it is shown, and the evidence where a real switch held it. A line that does not match prints its outcome (invalid, incomplete or ambiguous). A line known to crash the platform's model adds a `hazard` line: the command, what it does at the release, and the day it was measured ([output.md](output.md#commands-known-to-crash-a-switch)). `--platform` names a book or a model; the 9300 is the default.

| `--platform` value | Book |
|---|---|
| `9200`, `9200l` | Catalyst 9200 |
| `9400` | Catalyst 9400 |
| `2960x`, `2960-x` | Catalyst 2960-X |
| `2960`, `2960+`, `2960-plus`, `2960c`, `2960-c` | Catalyst 2960 |
| `ie3300` | IE 3300 |
| `ie3000` | IE 3000 |
| `ie3010` | IE 3010 |
| a model name such as `IE-3000-8TC` | that model's book |

### examples

Types every example line of the release's documentation at its prompt and counts the outcomes: accepted, invalid, incomplete, ambiguous. `--show N` prints the first N lines of each outcome other than accepted. `--canonical` prints each accepted line as the switch shows it.

### releases

Prints, per book, the releases it has a grammar for.

## Analysis of configurations

| Subcommand | Arguments and options |
|---|---|
| `lint <config>` | `--stack <members>`, `--release <x.y>` |
| `check <config> --to <x.y>` | `--from <x.y>`, `--stack <members>`, `--all` |
| `verify <config>` | `--stack <members>`, `--release <x.y>` |
| `verify --outputs` | none |

The hardware of `lint`, `check` and `verify` comes from `--stack`, else from the configuration's provision lines and interface names. A configuration whose interfaces fit no model needs `--stack`. A byte order mark is dropped. `check`, `verify`, `capture`, `changes`, `fleet`, `topology`, `corpus` and `sweep` refuse an option they do not take and an option given without its value.

### lint

Types a configuration at the release it runs and checks it as it stands. The output is the file, hardware, release and lines typed with the count not modelled, then each finding, `note:` line and `not checked:` line, then the counts. Exit is 1 on a finding. Hardware read from the configuration says nothing of the network module fitted, so module checks run only with `--stack`.

### check

Types a configuration at the release it runs (its `version` line, or `--from`) and at `--to`.

| Section | Lines |
|---|---|
| Refused at `--to`, taken at the first release | the lines the change breaks |
| Taken at `--to`, refused at the first | the lines it newly takes |
| Refused at both | the lines vswitch models at neither |
| Taken at the first release because a real switch held it | lines the `--to` books do not carry; no capture at `--to` says whether they break |
| Not typed at a release | lines one release does not reach, the session having left configuration mode |
| hardware | stack members the `--to` release does not run, and problems of the whole stack, which name no member |

The sections other than the breaks list 12 lines each; `--all` lists every line. A classic `version 15.2` line names only a train, so such a configuration needs `--from`. Exit is 1 on a break or on hardware.

### verify

Prints each line's evidence: held by a real switch of that platform and release, the same command in another form, or from the books only. It prints the captures weighed against, the three counts, and the books-only lines as a script to type on such a switch, section lines with the indentation they were typed with and banners whole.

`verify --outputs` prints the output rules no real switch has shown.

## Captured switches

| Subcommand | Arguments and options |
|---|---|
| `changes <backup file or folder>...` | `--captures <dir>` (required) |
| `fleet <captures dir>` | `--to <x.y>` (repeatable, required), `--all` |
| `capture <user@host>` | `--key <file>`, `--out <dir>`, `--pending <dir>` |
| `capture --log <session log>...` | `--config <file>`, `--out <dir>`, `--pending <dir>` |
| `capture --folder <dir>` | `--out <dir>`, `--pending <dir>` |
| `probe <user@host> <script> --lab` | `--key <file>`, `--out <dir>`, `--skip-unsafe`, `--map <from>=<to>` (repeatable) |

### changes

Reports what each backup changes on its captured switch: the items added, removed and edited, the lines its release refuses, and the lint and link findings it adds or clears. A folder is read file by file. A file that names no switch (no `version` and `hostname` line, no session) is reported. Exit is 1 on a new refusal or finding, a file that names no switch, or a file with no configuration.

### fleet

Checks every capture at the first `--to` release its book documents and its hardware runs, else the first documented. `--to 17.15 --to 15.2(7)E --to 15.0(2)SE` sends each family to its own release. Each row names the switch, its hardware, the two releases, the count of breaking lines and the count unproven at the target; the breaking lines follow, five per switch, all with `--all`. Rows are worst first. The closing line counts the captures that move clean, those with breaking lines, those on hardware the release does not run, those already there, those with no book for the releases named, and those not checked (unreadable or unplaceable, each named). Exit is 1 when any switch breaks or sits on hardware the release does not run.

### capture

Writes a capture under `<out>/<book>-<release>/` as `<host>-running-config.txt` and `<host>-show.log`, swept of secrets. `--out` defaults to the current folder. A switch whose release and hardware vswitch does not model goes under `<pending>/<part>-<release>/` when `--pending` is given. After the write it types the configuration into vswitch at its release and prints the lines refused.

| Form | Reads |
|---|---|
| `<user@host>` | a real switch over SSH, key authentication only (`BatchMode=yes`), host keys accepted on first sight; `--key` names the key file |
| `--log <session log>...` | terminal logs of sessions with a switch, each `--log` read in turn |
| `--config <file>` | a configuration file used where the session printed none, with `--log` |
| `--folder <dir>` | a folder of session logs and backup files from many switches, each file read for what it holds whatever its name |

For `--folder`, a switch captured already gains what the new files hold that its capture at the same release lacks, and the change the backup makes is reported; at another release the files are a capture of their own. Exit is 1 when any switch fails, or when the folder names no switch.

### probe

Types a script on a private lab switch and compares each line's answer, and what IOS keeps, with vswitch. `--lab` is required and states that the switch is a private lab reservation. The switch's configuration is saved to flash first and put back after with `configure replace`; the output says whether `Rollback Done` was seen. Lines that could cut the session are refused before anything is sent, and `--skip-unsafe` leaves them out. So is a line known to crash the switch's model ([output.md](output.md#commands-known-to-crash-a-switch)), bare or after `do`: a crash ends the session and skips the `configure replace` rollback. `--map Twe1/0/=Gi1/0/` renames the script's ports onto the lab switch's. The session log and the configuration IOS kept are written under `--out` beside the capture, swept. Exit is 1 on a difference, when the replace did not answer `Rollback Done`, and when the script is refused.

## Labs

| Subcommand | Arguments and options |
|---|---|
| `lab <lab.toml>` | `--shell <switch>` |
| `topology <captures dir>` | `--out <lab.toml>`, default `plant-lab.toml` |

### lab

Builds each switch from its files, brings each cable up from both ends and checks it. The output is each switch with its lines typed, each link, each finding and `note:` line, and the counts. `--shell` opens that switch's console with its links up, after the report; a name not in the file is refused. Exit is 1 on a finding.

In the console, `show spanning-tree` and its forms read the whole lab: the other switches as they came up and the switch's own configuration as it stands, so a priority, cost, mode, MST region, guard or `shutdown` typed there shows at once. Trunking, bundling and a link down at startup stay as the lab brought them up. A `[[device]]` entry gives the BPDUs a device the lab does not model sends ([checks.md](checks.md#lab-files)); the forms are in [output.md](output.md#show-spanning-tree).

### topology

Writes a lab file of the captured switches, cabled where both ends' port descriptions name each other, each at its newest capture. A switch captured before and after an upgrade is one switch. Lines of the report name what was left out and why. The closing line counts the captures, the switches cabled and the cables.

## Captures folder

### corpus

`corpus <captures dir> [--out <file>] [--scan <dir>] [--why] [--loose]`

Compiles what the captures teach into `data/corpus.json.gz` (or `--out`) with every site value characterized. It also reads each crawl folder, `<book>-<release>/crawl` or `crawl-<name>` beside it, whose `level` file names a license level, for the `?` answers its `crawl.log` holds. It prints each crawl read with its level, each crawl folder without a `level` file as not read, and how many answers it kept. Nothing is written while an entry still names the site.

| Option | Effect |
|---|---|
| `--out <file>` | the output file |
| `--scan <dir>` | lists each line under the folder that names the site, as `file:line: word`, and exits 1 on one |
| `--loose` | with `--scan`, also each word holding a site word of four or five characters, for a person to review |
| `--why` | counts the `?` answer rows taken out, by the denied word that took each |

### coverage

`coverage [--all] [--baseline]`

Available in a build with a captures folder. Types every capture at its platform and release with the lines real switches held set aside. Per book and release it prints the lines the books alone refuse under a line they take, and the commands refused most, then the count of roots and captures. `--all` lists each. `--baseline` writes the list the captures folder's `coverage.txt` holds, which a test compares.

## Grammar build

| Subcommand | Arguments and options |
|---|---|
| `compile <reference.json> <grammar.json[.gz]>` | `--guides <guides.json>`, `--references <refs.json>`, `--overrides <overrides.json>` |
| `compile-releases <reference dir> <grammar dir>` | `--references <refs.json>`, `--overrides <overrides.json>` |

`compile` builds one book's grammar from a command reference, then the configuration guides' procedure steps, the general references and the overrides, which have the last word. The file is gzip when its name ends in `.gz`, written without a timestamp so a recompile of the same sources writes the same bytes. `compile-releases` reads every `<dir>/cisco-<release>/<platform>-cr.json` with its guides, writes `<platform>-<release>.json.gz` into the grammar folder and the help tables into a `help` folder beside it.
