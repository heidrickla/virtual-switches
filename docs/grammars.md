# Grammars

Each grammar is compiled from Cisco's documentation for a platform and release, with the commands real switches' captures add. This page covers the sources, the extractor, the compile commands and rules, the overrides, and the rules that match a typed line to a command.

## Sources

| Source | Use |
|---|---|
| Command reference of the platform's book at the release | every command's syntax lines, modes, argument ranges and example lines |
| Command reference introductions and Syntax Description rows | the first sentence of each, as `?` help, kept once per book in `data/help/<book>.json.gz` (the newest release's text); a command name shared by commands of different modes keeps each one's help in its grammar |
| The release's configuration guides for the platform | each procedure step's syntax and its example at its prompt; commands the reference does not list |
| Programmability Configuration Guide | read as a configuration guide; to 17.8 one book per release, whose chapters link the per-area guides |
| Cisco IOS and IOS XE general command references, `reference/ios-xe-cr.json` | commands and syntax lines the platform's own evidence attests (General references, below) |
| `data/overrides.json` | corrections and additions written against the 9300 17.15 books, each with its evidence (Overrides, below) |
| `data/corpus.json.gz` | what real switches' captures teach, applied over the compiled grammar at their platform and release |

The general references are IOS, IOS XE, interfaces, IP addressing, IP routing, basic system management, LAN switching, security, configuration fundamentals, SNMP, quality of service, high availability, IP application services and programmability 17.15. Grammars that borrow from captures are compiled where the captures are: the folder `VSWITCH_CAPTURES` names, else `live` beside the grammars' folder.

### Books

| Key | Platform name | Name in messages |
|---|---|---|
| `c9300` | Catalyst 9300 | Catalyst 9300 |
| `c9200` | Catalyst 9200 | Catalyst 9200 |
| `c9400` | Catalyst 9400 | Catalyst 9400 |
| `c2960x` | Catalyst 2960x | Catalyst 2960-X |
| `c2960` | Catalyst 2960 | Catalyst 2960 |
| `ie3300` | Catalyst IE3300 | Catalyst IE3300 |
| `ie3000` | Cisco IE 3000 | Cisco IE 3000 |
| `ie3010` | Cisco IE 3010 | Cisco IE 3010 |

| Relation | Rule |
|---|---|
| Projection | the IE3300 has guides and no command reference: its grammar is the 9300's at the same release, kept for the commands its guides and captures attest, plus its guides' lines and the overrides scoped to its platform |
| Donors | the 2960, IE 3000 and IE 3010 books cover the switch's own commands; a command their captures use that their books refuse is taken whole from the 2960-X grammar at its newest release not after this one, else its oldest, with the transitions its examples showed |
| Lost command reference | a release whose command reference is lost and whose guides are kept takes the book's next command reference as of that release, by its Command History |
| Default release | 17.15 where the book documents it, else the book's newest release (the IE3300's 17.6, a classic book's newest train); a stack with no release named runs the newest release the book documents that the hardware runs |

## Extraction

`tools/extract-releases.js` reads the books in a browser tab on cisco.com, which refuses scripted HTTP clients. Set `window.VSWITCH_JOBS`, paste the file, and it starts the jobs and returns at once. Results are kept as `reference/cisco-<release>/<book>-cr.json` and `-cg.json`, which are not committed. `reference/release-books.json` lists the books read for each 9300 and 9200 release.

| Job field | Meaning |
|---|---|
| `platform` | the book key; the platform name comes from the extractor's `PLATFORMS`, which a test holds to `grammars::BOOKS` |
| `name` | the platform name, for a platform `PLATFORMS` does not hold |
| `release` | the release the books document |
| `cr` | the command reference URL |
| `cg` | a list of configuration guide URLs |
| `archive` | a timestamp prefix (`'2017'`): read the books from the Wayback Machine |

Either of `cr` and `cg` may be left out; the other file is left as it is.

| Behaviour | Detail |
|---|---|
| Parallelism | four jobs at a time with 300 ms between requests; from the Wayback Machine one job at a time with 1500 ms; `window.VSWITCH_PAUSE_MS` sets the pause |
| Progress | `window.__vs` holds the finished jobs, the failed ones, the jobs running and the stop reason |
| Command reference | per command: name, anchor id, first description sentence, syntax lines (`<var>` marks an argument), and each section (Syntax Description, Command Default, Command Modes, Command History, Usage Guidelines, Examples) as text, table rows and preformatted blocks; the chapters come from the book's table of contents |
| Command headings | each chapter records its command headings beside the commands read, so a heading the parser skips shows |
| Configuration guides | per chapter: each procedure step's syntax with its example lines, and every preformatted block (configuration examples, show output) |
| Book formats | newer books, 2960-X 15.2(2)E and 15.2(4)E books, and FrameMaker books (2960, 12.2(44)SE to 15.2(4)E, IE 3000 and IE 3010 15.x) each have a reader; IE3x00 guides write a step as prose and the extractor takes its syntax from the step's keywords |
| Retries | a page answering 5xx is asked again up to three times; a table of contents that lists no chapters is fetched again |
| Summary | each job's summary lists the pages not read as asked: a status other than 200, an answer from another address, a guide book with no chapters |
| Failures | a job that fails leaves the run going; three refused requests in a row (403 or 429) stop it |
| Storage | each finished file is kept gzip-compressed in the origin's IndexedDB `vswitch-extract` |
| Download | `await window.vswitchFlush()` downloads the files the run kept as one tar, each dated when it was kept; `vswitchFlush({ every: true })` takes every file the browser has kept |

A book cisco.com has retired is read from the Wayback Machine in a tab on web.archive.org, with the job's `archive` set. Each page is fetched as archived (`/web/<timestamp>id_/<url>`) from the capture nearest that time, and each chapter records its capture. The Wayback index (`/cdx/search/cdx?url=<prefix>&matchType=prefix`) lists what it holds under a book. A platform is named as `grammars::BOOKS` names it, and a job for any other gives the name.

## Compile commands

```
target/release/vswitch compile-releases reference data/grammars --references reference/ios-xe-cr.json --overrides data/overrides.json
target/release/vswitch compile reference/cisco-17.15/c9300-cr.json data/grammars/c9300-17.15.json.gz --guides reference/cisco-17.15/c9300-cg.json --references reference/ios-xe-cr.json --overrides data/overrides.json
```

| Command | Reads | Writes |
|---|---|---|
| `compile-releases <reference dir> <grammar dir>` | each `<dir>/cisco-<release>/<book>-cr.json` and `-cg.json` for every book | `<grammar dir>/<book>-<release>.json.gz` per release, and `data/help/<book>.json.gz` beside the grammars' folder |
| `compile <reference.json> <grammar.json[.gz]>` | one command reference | one grammar, gzip when the name ends `.gz` |

| Flag | Meaning |
|---|---|
| `--guides <file>` | the configuration guides' steps |
| `--references <file>` | the general references |
| `--overrides <file>` | `data/overrides.json` |

A grammar is written without a timestamp, so a recompile of the same sources writes the same bytes. `compile` lists the overrides a release does not take.

### Order of compilation

| Step | Detail |
|---|---|
| 1 | the command reference, then the guides' procedure steps |
| 2 | the general references' lines for commands the platform's own books name |
| 3 | the overrides, which have the last word |
| 4 | syntax lines that start with an argument are dropped and counted |
| 5 | submodes a release's examples show too seldom are taken from the nearest release of the same book |
| 6 | general commands the platform's captures, sessions and guides attest are taken from the general references |
| 7 | donor commands, for the books that have a donor |
| 8 | help moves to the book's help table |

## Compile rules

### Syntax lines

| Rule | Detail |
|---|---|
| Notation | Cisco notation: keywords, `<argument>`, `[optional]`, `{a \| b}`; an ellipsis of three or more dots repeats the item up to seven more times |
| Repairs | a mistyped line is repaired the same way every time and the repair is listed |
| Branches | a top-level `a <x> \| b <y>` whose later branches do not repeat the command's keywords is the command followed by one of the branches |
| First word | a line that starts with an argument is dropped |
| Several arguments in one run | one italic run naming several arguments (`<ip-address subnet-mask>`, `<weight1 weight2>`) is read as each where the command's Syntax Description gives each its own row, every word is a hyphenated or numbered name, or it is an address and its mask |
| Access list addresses | an access list's `<source source-wildcard>` also takes `host <source>` and `any`, as its Syntax Description says |
| One value | an interface's type and number stay one argument, and a file system with its path one URL; two parts written as one value (`<network>/<length>`, `<hh>:<mm>`, `<user>@<host>`) are one argument |
| Qualified arguments | rows that type an argument apart after different keywords type it after each (`hello-time <seconds>` 1 to 10, `max-age <seconds>` 6 to 40) |
| Foreign first word | a line that begins with another word than its command's stays where the book's examples type it, becomes a command of its own where they type it in another mode, and is dropped otherwise (device-tracking's `vlan {...}` in interface configuration) |

### Argument kinds

| Name or description | Kind |
|---|---|
| What its last words name, underscores taken as hyphens | `interface-priority` is a number |
| `<x> mask` or `<x> wildcard` | the kind of `<x>`; a MAC address's mask is a MAC address, an EtherType's a number |
| Interface type with number, `interface-id`, `source-interface` | an interface |
| Names of things (`ipv4-access-list`, `mask-length`) | a word |
| `ip-address`, `ipv4`, `ipv6`, `vlan list` | an IPv4 address, an IPv6 address, a VLAN list |
| A range in the Syntax Description (`1 to 4094`) | a number with that range |
| A range of characters (`from 1 to 32 characters`) | a word: the range bounds its length |
| `description`, `text`, `character-string` | text to the end of the line |

### Modes

| Rule | Detail |
|---|---|
| From the books | from each command's Command Modes |
| Mode ids | whitespace and a trailing `#` are dropped, underscores become hyphens and `configure` becomes `config` (`config_router-af`, `config#`, `configure-cmap-p`) |
| Mode in words | a mode named in words alone (the classic books' `Policy-map configuration`) is the prompt the command's own examples show it typed at |
| From a guide | a guide adds a mode to a reference command once two example lines show the command there; a command the reference gave no mode takes the guide's |
| Naming | configuration is `config` and its submodes, which IOS names `config-*` or, for some, `conf-*` (`conf-ssh-pubkey-user`), and the PKI, crypto and DHCP families (`ca-trustpoint`, `cs-server`, `cfg-crypto-trans`, `dhcp-config`) |

### Submodes

| Rule | Detail |
|---|---|
| Entering | the mode a command enters comes from example prompts that change after it, seen at least twice, into a mode some command runs in |
| Climbing | a prompt that climbs to an enclosing mode after an ordinary line is an example that left out `exit`, and counts only for `exit...` commands; so does a submode's line landing in a mode global configuration enters that is not deeper |
| Forms | a form of a command, the keywords a line of it takes, enters the mode most of its own examples show, two at least, where that is another than its command's (`ip access-list standard` beside `ip access-list extended` in the books' one `ip access-list`) |
| Few examples | `compile-releases` gives a command whose release's examples show this fewer than twice the configuration submode the nearest release of the same book shows, where the release has that submode; its evidence names the release |
| Header with no mode | a header the books give no mode enters the one the general references' examples show their command of that name entering, where the captures hold lines under it that read there (`crypto pki trustpoint` and its `enrollment selfsigned`) |

### Guide steps

| Rule | Detail |
|---|---|
| Counted | a step counts when one of its example lines starts with the step's first keyword; its prompt gives the mode |
| Joined | a step whose keywords name a reference command adds its line to that command; otherwise it becomes a command of its own |
| Run together | a step whose syntax lost the spaces its example has (`configureterminal`) is the example's words, where the example's first word begins a command the books have |
| Instances | a guide step naming one instance (`interface gigabitethernet0/0`) counts as an example of the command it instantiates |
| Instance words | a step word is an instance only when it starts with a digit, holds `/` or `:`, or is an interface name; `address-family ipv4` is the command |
| Prompt lines | the prompt lines of guide examples and of configuration example blocks join the examples and the mode changes |

### General references

| Rule | Detail |
|---|---|
| Extra lines | further syntax lines for commands the platform's own books name; a line naming a keyword its Command History says a later IOS XE release added is left out of the earlier release |
| Whole commands | a line the platform's evidence attests that its books refuse takes the general command that reads the line in that mode |
| Evidence | a line one of its captures holds; a line its sessions typed and the switch took (an EXEC command, or a configuration line in the mode its prompt names, its `no` and an access list entry's number left off, `do` read as the EXEC command); a line its guides' examples type |
| Modes | the command runs only in its modes where the books have no command of that name |
| IOS XE gating | by Command History: a command IOS XE introduced after the release is left out, and so is each alternative naming a keyword a later IOS XE row adds |
| Classic gating | a command only IOS XE introduced is left out, and a keyword IOS XE or a later LAN switching train (E, SE, EX to EZ) added; rows of other trains (T, S, SR, SX) date nothing |
| Other platforms | a platform's switches never vouch for another platform's grammar |

## Overrides

`data/overrides.json` holds the corrections and additions written against the 9300 17.15 books. Each entry carries the evidence for it. A release takes an override only where its own sources call for it; each taken override keeps the examples and help of the command it replaces.

| Key | Meaning |
|---|---|
| `name` | the command's name |
| `evidence` | why the override exists |
| `lines` | the command's syntax lines; an entry with no lines patches each command of its name |
| `modes` | the modes the command runs in |
| `drop_modes` | takes a submode from a command a guide placed where another command owns it: the global `username` in `ip ssh pubkey-chain` |
| `args` | argument types |
| `enters` | the mode the command enters |
| `adds` | lines beside the command's own: a short `no` form the books state only in full (`no access-list <number>`, `no ip route <prefix> <mask>`) |
| `classic_lines` | the lines a classic release takes instead, with the IOS XE forms its books never show left out (username's `algorithm-type`, ntp server's `vrf`) |
| `renames` | renames an argument outside the `no` form so `args` can bound it there alone (`privilege`'s level, 0 to 15 as the lab C9300 took it, its command every word after the level) |
| `corrects` | the lines the book must document for the override to apply |
| `platform` | the override applies at every release of that platform |
| `remove` | drops the entry as an unusable syntax |

### When a release takes an override

| Override | Taken where |
|---|---|
| Removal | the release's book documents the entry with the lines the override `corrects` |
| Patch | the command exists |
| Replacement or addition | the book documents the lines the override `corrects`, the release's example lines type the command at one of its modes, or its books show the command as a configuration line |
| Submode text | a command enters the submode: an override for a data mode applies wherever a command enters that mode (the SSH public key's `key data`) |
| With `platform` | the platform matches, and the entry has lines or removes, or the release has the command |
| In the general references | the references' own lines and examples call for it (`certificate` under a chain with `self-signed` and `ca`; `crypto pki trustpoint` with `redundancy` optional, entering `ca-trustpoint`) |

A patch reaches every command of its name. `compile` lists the overrides a release does not take.

### Scope of `platform`

| Use | Example |
|---|---|
| A hardware document the release books do not cover | the supervisor installation note's `enable` on the 9400 |
| A mode the lab switch ran a command in that its books do not give | `show license summary` at `#` on the Catalyst 9300, where the release has the command |
| A correction to every release of one book | the 2960-X `channel-group` |
| A line a platform's lab switch took that its books omit | `length (line)`, `width (line)`, `speed (line)`, `ip unreachables`, `key-hash` and `service unsupported-transceiver` on the Catalyst 9300; `username (ssh pubkey-chain)`, `key-hash` and `show running-config interface` on the Catalyst 2960 |

## Matching

A line typed at a prompt is read against the commands the mode runs.

| Rule | Detail |
|---|---|
| Prefix | a keyword matches by any prefix that names one keyword at that point |
| Whole keyword | a whole keyword is itself and no longer one it begins: `service x` is refused at `x` though `service-policy` and `service-template` begin with `service`, and `ip` is never `ipv6` |
| Short spellings | only a book's short spelling of a keyword IOS has is taken as the longer one: a plural's singular, `show interface` for `show interfaces`, and `show env` |
| Keyword and argument | a keyword wins over an argument, and a typed argument (interface, number, address, VLAN list) over a free word |
| Two commands | of two commands that read a line alike, the one named for more of its keywords is taken (`ip access-list standard` over `ip access-list`); a `no` form a book documents as a command of its own never wins over the command it undoes |
| Ambiguity | an abbreviation two keywords at its point both begin is ambiguous whatever follows; a short spelling and its long form count as one keyword |
| `no` forms | a `no` form matches the stated `no` line, or names every keyword of its command up to its first argument (`no ip` is incomplete), or is the command's own line after `no` |
| Numbers | a number or VLAN list is digits, written plain (`010` is `10`); a VLAN is 1 to 4094. A VLAN list may be spaced around its `-` and `,` (`1 - 5 , 9`); `101-110, 200` is a list and another word |
| Interface ranges | a range segment spans at most 4094 numbers; an interface range takes the interface commands and writes them to each member |
| Top-level blocks | a line that opens a top-level block (an interface, a VLAN), typed in another block's submode, goes to global configuration and opens its own block |
| Held blocks | a block real switches hold that the books parse opens at the console in the mode the books' examples name |
| Enclosing modes | a configuration submode hands a line it does not know to the nearest enclosing mode that takes it, global configuration last, and leaves the submode only when that line is accepted |
| Abbreviated first word | a line a submode takes only as an abbreviation goes to an enclosing mode that takes it whole: `router ospf 1` typed in an interface goes to global configuration. Where the nearest mode that has the word whole refuses the line, its refusal stands in the submode unless a mode further out takes the line: `key rad123` in a key chain's key is refused under `rad123` as the key chain's `key` refuses it, and `key chain K2` goes to global configuration |
| Removed values | a configuration typed from a file (a switch booting from the list, `check`, `lint`, `verify`, `changes`) takes the secret sweep's `<removed>` for any argument but an interface, where a value was; at a console it is the word it is |

## Access lists

An IPv4 access list's entries are numbered as IOS numbers them.

| Rule | Detail |
|---|---|
| Numbering | an entry typed without a number takes the highest plus 10; a number is 1 to 2147483647 |
| Placing | `15 permit ...` places an entry at 15 |
| Duplicate number | a number in use answers `% Duplicate sequence number.` |
| Identical entry | an entry identical to one held adds nothing |
| Removing | `no 15` removes the entry of that number with its remarks, and `no <entry>` removes the entry |
| Remarks | a remark takes the number of the entry after it; a remark's number may repeat |
| Resequence | `ip access-list resequence <name> <start> <step>` renumbers, each remark with the entry after it |
| Standard `host` | a standard list keeps `permit host 10.0.0.7` as `permit 10.0.0.7` |
| IOS XE print | each entry's number prints in the running configuration; at 17.6 `deny` pads to the width of `permit` |
| Classic print | classic IOS prints none (a remark at its place), a standard list's host entries first and its actions padded to the width of `permit` |
| `show ip access-lists` | the title, then each entry four spaces in with its number, remarks left out, a standard wildcard written out (`10.1.0.0, wildcard bits 0.0.255.255`), a standard entry's action padded to the width of `permit` |

### Classic entries

| List | `permit` and `deny` take |
|---|---|
| Standard | an address with an optional wildcard, `host <address>` or `any` |
| Extended | a protocol by name or number, each end as an address and wildcard, `host <address>` or `any`, ports for TCP and UDP, and ICMP and IGMP types |

These are overrides as the security reference's Syntax Descriptions give them and the lab 2960G took them, since the classic references' own lines leave out `host` and `any`.

## Certificates

A certificate's hex after `certificate` in a `crypto pki certificate chain` is read as IOS reads it, line by line until `quit`.

| Rule | Detail |
|---|---|
| Lines read | hex digits and spaces, and `quit`, which ends the certificate; any other line ends it and is typed as itself |
| Print | as typed; with no indentation typed, two spaces in and `quit` after a tab, as switches print it |
| Submode names | the PKI, crypto and DHCP families (`ca-trustpoint`, `cs-server`, `cfg-crypto-trans`, `dhcp-config`) |

## SSH public keys

Under `ip ssh pubkey-chain`, `key-string` enters the key's data mode.

| Rule | Detail |
|---|---|
| Key text | every line typed after `key-string` up to `exit` is key text, `end` and `configure terminal` included; lines join with nothing between |
| Blank lines | passed over |
| On `exit` | the key becomes `key-hash <type> <MD5 of the decoded blob, upper hex>` plus the words after the blob as its comment |
| Leading type | the key may be typed with or without its leading `ssh-rsa` |
| Type on its own line | `ssh-rsa` on a line of its own joins the blob and fails with `%SSH: Failed to decode the Key Value` |
| Empty key | says nothing and keeps nothing |
| Typed `key-hash` | prints upper case; on IOS XE it prints with a trailing space when no comment follows; classic IOS prints none |
| Key made from lines | prints with no trailing space |
