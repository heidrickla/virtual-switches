# Terminal screens

`vswitch` with no subcommand opens the switch list. `vswitch shell` and `vswitch lab --shell` open a console directly. The hardware builder opens from the list.

## Switch list

Each row is a switch. The columns, as many as the terminal fits and dropped from the last:

| Column | Holds |
|---|---|
| Name | the hostname |
| Hardware | the first member's model, a 9400's supervisor, and `+N` for the members besides |
| Release | the release the switch runs |
| Saved | the date `write memory` last saved it |
| Opened | the date its console was last opened |

The date the list is ordered by goes first. The title line shows the folder the list lives in. An empty list says how to load or build a switch. Configurations are swept of passwords and secrets before they are kept.

### Details of the selected switch

| Layout | Shows |
|---|---|
| terminal 110 columns or wider, beside the list | each member's number, model and module; the release with its system (IOS on the 2960, 2960-X, IE 3000 and IE 3010, IOS XE on the rest); the file it came from; where its configuration is kept; when it was loaded, opened and saved; the count of secret lines removed |
| narrower, below the list while the terminal is tall enough | the hardware, the release with its system, the file it came from, one date (saved, else opened, else loaded), the count of secret lines removed |

`?` or F1 lists the keys of the screen in use. The footer names the main ones.

### List keys

| Key | Action |
|---|---|
| Up, Down, k, j | select a switch |
| PgUp, PgDn, Home, End | a page up or down, the first, the last |
| Enter | open its console; `exit` there comes back |
| / | filter by name, model or release as it is typed |
| Esc | clear the filter; with none, quit |
| a, Insert | load a design or a configuration from a file |
| n | a new switch: its hostname, then its hardware |
| h | its hardware and release in the builder |
| r | rename it; the configuration's hostname follows |
| c | copy it under another name |
| v | read its kept configuration |
| e | export its kept configuration to a file |
| d, Delete | remove it from the list, asked first |
| u | bring back the switch removed last |
| o | order by name or by last opened |
| s | sweep the sweep folder of secrets |
| ?, F1 | the keys of the screen in use |
| q, Ctrl-C | quit |

A list command is a plain key: Alt or Ctrl with a letter does nothing. Loading a file or building a switch with `n` ends by opening its console. Ordering by last opened lists switches never opened last, ties by name.

### Screens

| Screen | Behaviour |
|---|---|
| Console | the kept configuration typed in configuration mode, then `hostname>` as after boot. `Opening <name> (<hardware>, <release>)...` prints as the grammar is read, and `<name>: <n> configuration lines loaded from <file>.` once the configuration is typed (`<name>, built in vswitch: ...` for a switch built in the list). Lines the virtual switch does not model are listed, the first 12 and a count of the rest, and skipped. A platform release's grammar is read once a run |
| Filter | each word typed must be in a switch's name, its members' models and modules, or its release: `9300 17.9` |
| Name | one word, no other switch's name; a rename or a copy rewrites the hostname line of the kept configuration, and a copy takes the hardware and release |
| Configuration | the kept configuration with line numbers |
| Export | the browser with a file name, `<hostname>.cfg` offered; Enter writes it, and a file already there is written over only after `y` |
| File browser | the folder's entries; see below |

A switch loaded or built under a listed switch's name is kept only once that is confirmed: `y` replaces the listed switch, `k` keeps both under a name typed for the new one. Each removed switch comes back with `u`, the last removed first, until vswitch closes. Problems read red on the status line, completed saves, exports and hardware changes green, and a hardware choice that changes nothing plain.

### Keys of the other screens

| Screen | Keys |
|---|---|
| Filter | type to narrow; Up, Down, PgUp, PgDn select a switch; Left, Right, Home, End move in the filter; Enter keeps the filter and returns to the list; Esc clears it |
| Name | type; Left, Right, Home, End move; Enter keeps; Esc returns |
| Remove | `y` removes; any other key keeps |
| Replace | `y` replaces the listed switch; `k` keeps both; any other key leaves the list as it is |
| Configuration | Up, Down, k, j a line; PgUp, PgDn, b, Space a page; Home, End, g, G the first and last line; Left, Right sideways; `/` search, Enter finds, Esc cancels; n, N the next and previous match; Esc or q returns |
| Help | Up, k, Down, j, PgUp, PgDn, Home, End scroll; Esc, Enter, F1, `?` or q close |

The configuration search ignores case, scrolls sideways to a match past the edge, and says when it went round the end.

### File browser

| Item | Behaviour |
|---|---|
| Entries | folders first, then designs and configurations (`.toml`, `.cfg`, `.txt`, `.log`) bright and other files dim; names starting with a dot, and what Windows marks hidden or system, show only when the typed text starts with a dot |
| Typing | narrows the entries |
| Tab | completes the name typed |
| Enter | opens a folder or loads a file |
| Backspace, Left | with nothing typed, up a folder; above a drive, the drives |
| Right | with nothing typed, into the folder picked |
| Esc | back to the list |
| Path | a typed or pasted path loads as it is; `~` is the home folder, and paths in it are shown from `~`; a path pasted or dropped on the list opens the browser with it |
| Start folder | the folder last loaded from or exported to |
| Export keys | Up, Down pick a folder to open or a file whose name is taken; Enter writes; Tab completes; Backspace with nothing typed goes up |

A design names its hardware. A configuration's is read from its `switch <n> provision` lines and interface names and confirmed in the builder before it is kept. Hardware vswitch does not know, a stack Cisco does not build and a release the hardware does not run open the builder with the problem named in red until a model is chosen. A file with no `hostname`, `version`, `interface` or `vlan` line is not loaded. A file saved as UTF-16 loads. A configuration with no `hostname` line takes the file's name, spaces made dashes.

### Paste and mouse

The list takes no mouse input, so the terminal's own text selection and right-click paste keep working. Elsewhere than Windows, the terminal's bracketed paste is used. A Windows console delivers a paste as keys: on the list, three or more typed keys read together are a paste and act as no command, unless the first opens a field to type into (`a`, `n`, `/`, and `r`, `c`, `e` on a selected switch), which takes the rest as typed. A busy console types a paste in pieces, which the list gathers while each key comes pressed and released at once. A key held down acts as that key, repeated. Ctrl-C over a popup or the builder cancels it as Esc does.

### Terminal size

A terminal smaller than 60 by 15 is told so and no key acts until it is larger; Ctrl-C quits.

### Storage

| Item | Where |
|---|---|
| list folder, Windows | `%USERPROFILE%\.vswitch` (AppData is redirected per MSIX package, so a list written there from a packaged app is seen by that app alone) |
| list folder, elsewhere | `$XDG_DATA_HOME/vswitch`, else `~/.local/share/vswitch` |
| list folder, any system | `VSWITCH_HOME` when set |
| `switches.json` | the list |
| `configs/` | a copy of each configuration as `<hostname>.cfg`, characters other than letters, digits, `.`, `_` and `-` written as `_`, swept of secrets before it is kept |
| `prefs.json` | the list's order and the browser's folder |
| `sweep/` | the sweep folder that `s` and `vswitch sweep` read |

A console's changes are kept by `write memory` or `copy running-config startup-config`, which save the running configuration into the list, swept as a load is. A changed hostname renames the switch unless another has that name. Without a save the changes last until `exit`, as on a switch reloaded without one. A second vswitch, or `vswitch load`, changing the list meanwhile is merged when this one writes.

## Console

The console reads keys as an IOS console does when its input is a terminal, and lines when it is a pipe or a file. The IOS editing keys are matched below. crossterm reports each Esc sequence as Alt+key or as Esc then the key, and both are taken. AltGr types its character.

| Key | Action |
|---|---|
| `?` | after a space, what may come next, each with its help, and `<cr>` when the line is complete; right after a word, the keywords it begins. The line stays as typed. In a banner's text, `?` and Tab are typed as text |
| Tab | completes the last word as IOS does: the word as typed, the rest of the one keyword or interface type it begins, and a space (`interface TWE` is `interface TWEntyFiveGigE `) |
| Up, Down (Ctrl-P, Ctrl-N) | the lines entered before, pasted lines included |
| Left, Right (Ctrl-B, Ctrl-F) | move one character; a line wider than the terminal scrolls, `$` marking the side that runs past the edge |
| Home, End (Ctrl-A, Ctrl-E) | to the start or end of the line |
| Esc B, Esc F | back or forward one word, a word being a run of letters and digits |
| Backspace, Delete (Ctrl-D) | delete the character before or at the cursor |
| Ctrl-K, Ctrl-U (Ctrl-X), Ctrl-W, Esc D | delete to the end, to the start, the word before, the word after |
| Ctrl-Y, Esc Y | recall the last deleted text, then step back through earlier deletions |
| Esc C, Esc L, Esc U | capitalize, lowercase or uppercase from the cursor to the word's end |
| Ctrl-T | transpose the two characters at the cursor |
| Ctrl-L, Ctrl-R | reprint the prompt and line on a fresh row |
| Ctrl-V, Esc Q | take the next key as itself, so `?` or Tab enters a line |
| Ctrl-Z | `end`: leaves configuration mode; at an EXEC prompt and in a banner's text it does nothing and the line stays |
| Ctrl-C | discard the line; in configuration mode, return to the EXEC prompt; in a banner's text, discard only the line being typed |

An impossible key, such as Left at the start of the line or Tab with several candidates, is a beep.

### Tab words

The words Tab completes are those `?` lists for the line before it: a held answer's where one is held, a refusal's none, else the books' keywords and the stack's interface types. A word beginning none or several keywords, a whole one among them (`ip` with `ipv6` beside it; `?` and Enter take it as itself), or an interface name with its number (`twe1/0/1`), is left as typed.

### Paging and help

| Item | Behaviour |
|---|---|
| `--More--` | output longer than the page pauses; Space shows the next page, Return the next line, `?` names the keys, q or any other key stops |
| `terminal length <n>` | sets the page to n rows; `terminal length 0` turns paging off |
| `terminal width <n>` | sets the width a long line wraps to; 0 for none |
| range and spelling | each takes 0 to 512, and `terminal` is taken by any unique start; refused as IOS refuses a number out of range, a missing number or an extra word |
| window size | the console takes the window's size at start, and a resize sets both, unless `terminal length` or `terminal width` set them |
| long help | help longer than the page pauses at `--More--`; a description wider than the terminal goes on under its column |
| pasted configuration | typed line by line, each entered and answered, the last partial line left to finish |

Paging is for the interactive console. Pipes, files and `run` scripts print whole.

### Pipes, files and scripts

In a pipe, a file and a `run` script, a line ending in `?` prints the help for what precedes it and is not entered. In a pipe or a file the help is fitted to the terminal width setting (80 by default, `terminal width 0` for none), as on a console. A `?` in a banner's text is typed as text.

| `?` lists | Form |
|---|---|
| keywords | the word |
| arguments | `WORD`, `LINE`, `<min-max>`, `A.B.C.D`, `X:X:X:X::X`, `H.H.H` |
| interfaces | the stack's interface types and the logical ones (Loopback, Port-channel, Tunnel, Vlan) |

A held answer listing interface types (one with a row for a front-panel type: FastEthernet to HundredGigE) offers the stack's front-panel types in place of the crawled switch's. Rows for types the stack lacks are left out. The stack's types the answer lacks are added in ASCII order, with the description held answers print for the type or bare where none does. The logical types stay as held, and the word column is laid out again.

## Hardware builder

A stack member by member: its number (a 9400 card's is its slot), model and network module, then a 9400's chassis and the release it runs. The builder opens for `n`, `h`, a configuration whose file does not name its hardware (filled in from its provision lines and interface names), and a design whose hardware Cisco does not build. Below the members it shows the selected one's ports as IOS names them, its uplinks, PoE kind and budget and the releases it runs, then the stack as `--stack` takes it. The release is labelled IOS for the 2960, 2960-X, IE 3000 and IE 3010 and IOS XE for the rest.

| Key | Action |
|---|---|
| Up, Down (k, j), Home, End | select a member, the first, the last |
| Tab, Shift-Tab | the next or previous field: number or slot, model, network module, chassis (9400), release |
| Space, or typing | the field's list, filtered by what is typed |
| Left, Right | step the field: the models of the member's family (a 9400's supervisors and line cards apart), the modules the model takes, the free numbers, the chassis, the releases |
| `+`, Insert | add a member like the selected one; in a 9400 the lowest free slot the chassis takes it in |
| `-`, Delete | remove the selected member; the last member stays |
| `[`, `]` | the previous or next release the stack's book documents, going round past the oldest or newest, which the status line says |
| Enter | keep the hardware; while a problem is marked, select the member it names and say so |
| Esc | cancel |
| `?` | the keys; Up and Down scroll them, any other key closes them |

| In a field's list | Action |
|---|---|
| Up, Down, Left, Right, PgUp, PgDn, Home, End | move |
| typing, Backspace, Ctrl-U | filter, erase, clear: every space-separated term matches, dashes ignored (`2960x48` finds WS-C2960X-48TS-L) |
| Enter | choose |
| Esc | close the list; the next Esc cancels the builder |
| Tab | go on to the next field choosing nothing |
| `?` | the keys |

Models are listed by family, a 9400's supervisors and line cards apart, network modules that fit the model first, releases by train. A `*` marks the value chosen now. Every choice is offered. One Cisco does not build, stack or run is marked red with the reason, as is hardware newer than the release, and Enter keeps nothing while anything is marked. Yellow marks what a choice changed or a fact the documents leave open.

| Choosing | Changes as well |
|---|---|
| a model whose family's book does not document the release | the release, to the book's default or else its newest that the hardware runs (a plain 2960 moves to 15.0(2)SE) |
| a model that does not take the fitted network module | the module, taken out |
| a 9400 supervisor or line card for a slot the chassis does not give it | the slot, to the lowest free one of the chassis chosen, else of any chassis that holds the rest |
| a chassis | the slots: supervisors to its supervisor slots, line cards to its line card slots, in slot order |
