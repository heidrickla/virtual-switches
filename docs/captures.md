# Captures and real switches

What a real switch printed overrides what vswitch derives from the books. A capture is a running configuration and a session log read from a real switch and swept of secrets. Captures are kept outside this repository. `vswitch corpus <captures dir>` compiles what they teach into `data/corpus.json.gz`, which the binary embeds. Crawls of lab switches add `?` answers to the same file. `vswitch capture`, `vswitch changes` and `vswitch probe` read real switches and their backups.

## Secret sweep

`vswitch sweep [<file or folder>...] [--out <folder>]` removes passwords and other secrets from configuration files and session logs (Cisco IOS, IOS XE and HPE Onyx). The value goes and the rest of the line stays, so a cleaned file still reads and loads. `<removed>` marks each value. A value that runs to the end of its line (a password with spaces) goes whole. A changed line is rewritten with single spaces.

Keywords count abbreviated as IOS takes them. The `no` and `default` form of each command below is swept too.

| Covered | Kind reported |
|---|---|
| `enable secret`, `enable password`, `enable algorithm-type`, `enable common-criteria-policy` | `enable secret` |
| `username` passwords and secrets | `username` |
| `snmp-server community` | `snmp-server community` |
| the community of `snmp-server host` | `snmp-server host` |
| `snmp-server user` keys, `sha-2` and `aes` lengths included | `snmp-server user` |
| `tacacs-server key`, `radius-server key`, per-host server keys, `key` inside `tacacs server` and `radius server`, PAC keys, `server-private`, `server-key` | `server key` |
| `key-string` | `key-string` |
| `pre-shared-key` (keyrings, IKEv2 `local` and `remote`) | `pre-shared-key` |
| `authentication-key` | `authentication-key` |
| OSPF and NTP `md5` keys, OSPFv3 IPsec keys | `md5 key` |
| IS-IS `area-password` and `domain-password` | `password` |
| HSRP authentication with or without a group, VRRP and GLBP authentication | `standby authentication` |
| MSDP peer passwords | `password` |
| the `key config-key` master key | `config key` |
| `password` anywhere: BGP neighbours, VTY lines, VTP, PPP | `password` |
| `secret` anywhere else | `secret` |
| a URL's password | `password` |
| `wpa-psk` | `wpa-psk` |
| `crypto isakmp key` | `isakmp key` |
| any `$1$`, `$5$`, `$6$`, `$8$`, `$9$` or `$14$` hash left elsewhere | `hash` |

Left alone:

- `description`, `remark` and `alias` text, wherever it starts, and `snmp-server location` and `contact` text.
- Banner text and `?` answers.
- `secret masked-secret`, the prompt for a value, and `<placeholders>`.
- The keywords some commands take after `password` or `pre-shared-key` (`password encryption`, `system disable password recovery`, `pre-shared-key key-chain`).

A session log is swept command by command after each prompt, and the prompt's mode gives the block. A byte order mark hides nothing.

Each file's report names the kinds of secret cleaned and never a value. It says when a removed value of four characters or more still appears elsewhere in the file. Binary files, other encodings and files over 32 MB are left alone and named in the report.

The command exits 1 for a path that does not exist, a file left alone, or a removed value that still appears elsewhere. A folder is read recursively. Without `--out` each changed file is cleaned in place, written beside and renamed over. `--out <folder>` (or `--out=<folder>`) writes cleaned copies of every file there under the same relative names and never cleans in place.

The sweep folder is `sweep` beside the switch list. Put configuration files there, then press `s` in the list or run `vswitch sweep` with no path.

## Corpus

`vswitch corpus <captures dir> [--out <file>]` writes the corpus, by default `data/corpus.json.gz`, from the `<book>-<release>` folders in the captures dir, and refuses a dir with no running configuration or session log in one. The corpus holds:

- the lines the captures held, and where and how IOS printed them
- the `?` answers of session logs and crawls, and the keyword descriptions beside them
- IOS's printing order, and the `!` separator lines a release printed between two top-level commands
- each book's NVRAM size
- the command forms the captures used

Every value that names a site is characterized as `«ipv4»`, `«ipv6»`, `«mac»`, `«number»`, `«word»` or `«text»`. The values covered are a host, a domain, an address, a user, free text, a configured object's name, a MAC address, and a serial or certificate number. A held line of that form takes any value of its kind, and the corpus names nothing of the site. A `?` answer that names one is left out. Captures are listed in the corpus as `<book> <release> capture <n>`.

The command reads `<captures dir>/../designs` for the names it gives and nothing else. It writes nothing when a corpus entry still names the site. Its other options:

| Option | Does |
|---|---|
| `--scan <tree>` | lists every published file under the tree that names the site, by file, line and word, after the counts of captures and site words read; exits 1 on a hit, and refuses captures that name no site word |
| `--loose` | with `--scan`, the wider match |
| `--why` | counts which denied word took out each `?` answer row |

### Site words

A site word is a word the captures name the site by and IOS does not. A word of IOS's vocabulary or a part of one is never one: names, syntax lines, example lines and their output and `?` descriptions, from the grammars and help tables, and the names of the access lists IOS XE makes itself (`acl::SYSTEM_V4`, `SYSTEM_V6`).

A keyword an image's `?` answer offered, a crawl's at any level or a session log's, passes where it stands offered and nowhere else (`Deny::line_hits`, `Deny::row_hits`):

| Field | A site word passes |
|---|---|
| a `?` answer's row | as the row's keyword, the answer offering it; the row's description with it |
| the line before a `?` answer, a sweep line, a session log's line | as a keyword the image's answer for the words before it offered, whole, whichever of its parts the site uses; a form's value token stands for any word typed there |
| a family's keyword description | read as the session log's `?` row it came from, against every image of the family |
| configuration text, any other help text, a message | never |

- A row's keyword is a lowercase word (`-` and `_` within it) or a `<range>`. A row of another shape offers nothing, and a word of it that names the site leaves its answer out.
- A row naming an object a configuration made offers nothing: a `User defined` row, a VRF's `VRF Name` row other than `Mgmt-vrf`, or a row whose keyword a capture names a host or an object by.
- A help text is IOS's text about its row's keyword: its words pass in that description alone.
- An object's name is the word after its kind (`ip access-list extended <name>`, `logging discriminator <name> ...`). A description, a location, a contact and a banner name no object.

### Use

| Use | Effect |
|---|---|
| Lines | a switch of that platform and release takes a line a capture held, as captured or with each characterized value filled in, where the books' grammar refuses it: at the top level, in a block of the kind it sat in (any interface for an interface line), and a block the capture opened under a line the books give no submode. It prints where that switch printed it, after the line before it in its capture. A banner's text is text and never a held line |
| Separators | `show running-config` prints between two top-level commands as many `!` lines as the release printed most often (the smaller number on a tie), else as the family printed most often, else as the books build it |
| `verify` | a line a capture held as typed, a command a capture used with other arguments (a characterized line among them), or a line from the books only; the last are the list to check on hardware |
| `check` | a line taken at the first release only because a capture held it is not counted as broken at a release no capture covers |
| `?` | a line a switch of that platform and release was asked in its session log prints that switch's answer whole, in the same mode. `vswitch capture` asks the common show prefixes. An error answer (`% Unrecognized command`) belongs to that switch's configuration and is not kept. The first answer kept for a line is the one held |
| `?` from a crawl | see Crawls |
| `do-exec` | see below |

### do-exec

- In configuration modes `do-exec <command>` runs the command at the privileged prompt as `do` does.
- `?` after `do-exec ` answers as the crawl held `do-exec ?` in that mode, which leaves out the commands that change mode (`configure`, `enable`), else as privileged EXEC does.
- `do-exec` may be abbreviated to three characters or more (`do-`, `do-ex`).
- `?` after a first word lists `do-exec` among the words it begins, and Tab completes it.
- A crawl's `do-exec <line> ?` is privileged EXEC's answer for `<line>` where the crawl did not ask the privileged prompt `<line> ?`.

## Captures folder

The layout is `<book>-<release>/<host>-running-config.txt`, `<host>-show.log` and `<host>-stack.txt`, the layout `vswitch capture` writes. The release is the train: 17.15.1 is `17.15`, 15.2(7)E0a is `15.2(7)E`. Any file ending `.log` in a release folder is read as a session log, so lab sessions (`<host>-session-<name>.log`) and probe logs (`<host>-probe-<script>.log`) sit beside a capture.

A build reads the folder when `VSWITCH_CAPTURES` names it, or when it sits beside this repository as `../virtual-switches-captures/live`. That build also runs the tests that replay each capture (see testing.md).

The sessions hold `show version`, `show inventory` with serials removed, `show interfaces status`, `show vlan brief`, `show etherchannel summary`, `show interfaces trunk`, `show power inline` and `show vtp status`, some `show ip interface brief` and `?` answers. `<host>-stack.txt` holds the hardware of the 9400 cores, whose sessions name none.

Switches by platform and release:

| Platform | Release | Switches |
|---|---|---|
| Catalyst 2960, 2960-Plus, 2960-C | 12.2(44)SE | 2 |
| Catalyst 2960, 2960-Plus, 2960-C | 12.2(55)SE | 2 |
| Catalyst 2960, 2960-Plus, 2960-C | 15.0(2)SE | 1 |
| Catalyst 2960, 2960-Plus, 2960-C | 15.2(2)E | 1 |
| Catalyst 2960, 2960-Plus, 2960-C | 15.2(6)E | 6 |
| Catalyst 2960-X | 15.2(2)E | 12 |
| Catalyst 2960-X | 15.2(6)E | 1 |
| Catalyst 2960-X | 15.2(7)E | 19 |
| Catalyst 9200, 9200L | 17.6 | 1 |
| Catalyst 9200, 9200L | 17.9 | 5 |
| Catalyst 9200, 9200L | 17.12 | 1 |
| Catalyst 9200, 9200L | 17.15 | 30 |
| Catalyst 9300, 9300L, 9300-24S | 17.6 | 5 |
| Catalyst 9300, 9300L, 9300-24S | 17.15 | 6 |
| Catalyst 9400 cores | 17.5 | 2 |
| Cisco IE 3000 | 15.2(6)E | 4 |
| Cisco IE 3010 | 12.2(53)EZ | 2 |
| Cisco IE3300 | 17.6 | 6 |

That is 106 switches, each a running configuration and a session.

## Crawls

A crawl is a transcript of `?` probes typed in each mode of a lab switch. `vswitch corpus` reads crawls and the binary does not embed them.

| Image | Lab switch | Levels |
|---|---|---|
| C9300 17.6 | C9300-24T, 17.6.4 | `network-advantage`, `network-essentials` |
| C9200 17.6 | C9200L-48T-4G, 17.6.4 | `network-advantage`, `network-essentials` |
| C9200 17.15 | C9200L-48T-4G, 17.15.6 | `network-advantage`, `network-essentials` |
| C2960 15.0(2)SE | C2960G, 15.0(2)SE11 | `lanbase` |

The C9300 17.6 crawl at each Catalyst 9000 level and the C2960G 15.0(2)SE crawl at `lanbase` serve as donors. The C9200 17.6 and 17.15 crawls answer for their own images at both levels, the licence gate included.

### Crawl folders

- The log is `<book>-<release>/crawl/crawl.log`, or `crawl-<name>/crawl.log` beside it for the same image at another level or the same one.
- Folders at one level are read as one, in name order: an answer the first holds stands (`crawl-advantage` before `crawl-advantage-probe`, the C9200 17.15 probes of 2026-10-23).
- The folder's `level` file names the license level the switch ran at on its first line: lowercase letters, digits and `-` (`network-advantage`, `network-essentials`, `lanbase`).
- A crawl folder without a level is named by `vswitch corpus` as not read.
- The log is read for its answers, and for the keywords it offers (Site words).
- No other file of a crawl folder is read.
- `vswitch corpus` prints each crawl folder it read with its level.
- Answers are held by level, mode, context and the line before `?`. The first answer for a key is the one held.
- An answer replays as printed, duplicate rows and spacing included.

### Exchanges held

- Only a complete exchange is held: a probe followed by the next prompt.
- A probe asked where the context is unknown is left out.
- An answer holding `--More--`, a control character (a tab excepted) or undecodable bytes is left out.
- The last exchange of a log is left out, since a log still being written may end in a cut-short probe.
- A line typed after `do-exec` in a configuration mode is held in privileged EXEC, after the privileged prompt's own answers, so the prompt's own is held where both were asked. `do-exec ?` is held in its own mode as well.

### Refusals

- `% Unrecognized command` is held and answered as the switch printed it.
- `% Ambiguous command:  "<line>"` is held and answered quoting the line as the user typed it, `do-exec` included.
- A refusal of a line naming an interface (`buffers Loopback0`, `buffers Lspvif 0`) is not held, since which interfaces exist is the switch's hardware and configuration.
- An answer with any other `%` line (`%Illegal Hostname or IP/IPv6 address`, `% LISP is not running.`) is not held.
- A held refusal does not move the caret of a refused line, which stands where the books put it.

### Context

The context is the chain of lines that entered the submodes. Each keyword is expanded from its abbreviation, and object names, numbers and addresses are left out.

| Entering line | Context |
|---|---|
| `router ospf 1` | `router ospf` |
| `router bgp 65000` | `router bgp` |
| `router eigrp 100` | `router eigrp as` |
| `router eigrp NAME` | `router eigrp named` |
| `interface GigabitEthernet1/0/1` | `interface physical` |
| `interface Vlan10` | `interface vlan` |
| `interface Loopback0` | `interface loopback` |
| `interface Port-channel1` | `interface port-channel` |
| `interface GigabitEthernet0/0` | `interface mgmt` |
| `interface GigabitEthernet1/0/1.10` | `interface physical sub` |
| `interface range GigabitEthernet1/0/1 - 4` | `interface range physical` |
| `router bgp 65000` then `address-family ipv4` | `router bgp > address-family ipv4` |
| `policy-map NAME` then `class NAME` | `policy-map «word» > class «word»` |

One function derives the context for the corpus (from a log's prompts) and for the shell (from its entering lines).

One table says which lines leave another such block for one of their own: `interface`, `vlan`, `line`, `router`, `vrf definition`, `ip vrf`, `route-map`, `class-map`, `policy-map` and `ip access-list`.

An answer holds in the context it was asked in. A mode and context the crawl never entered take the books' answer and never another context's.

### Addresses and state

- An address typed in the line, whole or inside a word, is matched as `«ipv4»`, `«ipv6»` or `«mac»`, and a prefix length after it as `«number»`.
- A crawl's key or context still holding an address is not compiled.
- An answer that depends on the configuration replays only while the session lacks the line that changes it. `aaa ?`, `no aaa ?` and `default aaa ?` offer `new-model` alone until `aaa new-model` is configured.

## Reading crawl answers

The shell looks a line up in the image's own answers, then its derived answers, then the donor's, then the books.

### Abbreviations

Before a held answer is looked up, each word typed before the last space is read against the held answer for the line before it, in the same mode and context.

- A word that begins exactly one keyword of that answer is read as that keyword, as IOS takes a unique start of a keyword (`ip acc ?` answers as `ip access-list ?`, `snmp-server enable traps stackwise port-c member-up ?` as its derived answer).
- A word typed whole is itself, though it begins longer keywords (`ip` beside `ipv6`). Where the switch refused such a line (`alias filter ?`), its held refusal answers.
- A word that begins no keyword (a value), stands where no answer is held, or that the switch answered after although its `?` left it out (`ipv6 nd ra`) is kept as typed, and the words after it are read on.
- A word that begins two keywords or more and is neither is refused as the switch refuses it: `% Ambiguous command:  "<line>"` quoting the whole line as typed.
- Where no answer is held for the line before a word, the word is kept. The crawls hold an answer for every line before each word of every key they hold.
- In the C9300 17.6 and C2960G 15.0(2)SE crawls the books offer no keyword for 97% of the switch's keyword positions and read 4% of the unique starts they do offer as another keyword or as ambiguous.

### Derived answers

A line with no held answer, made of a held line `Q` and two picks or more of the keywords its answer offers (`snmp-server enable traps stackwise new-master port-change`), is answered from the held answers after each pick alone. The longest `Q` that qualifies is taken.

- Each pick's answer is a subset answer of `Q`'s by the crawler's rule: only words `Q` offered, at least two, and at least half as many. The pick is not `no` or `default`.
- Each pick is offered after every other pick.
- The answer is `Q`'s rows in its order with their descriptions, each kept where every pick's answer offers it. A pick stays where its own answer offers it again.
- `<cr>` comes last where every pick's answer offers it.
- The word column narrows to the longest word printed, a bare `<cr>` not counted.
- A held answer for the line itself comes first. A value or a pick whose answer is not held leaves the line to the books.

### License level

A session's level is its running configuration's `license boot level <level>`, else the level it booted at, else the book's.

- A configuration typed as a console boots it (`check`, a lab switch's files) boots at the level its licence line states, and every line of it, those before the line too, is read at that level.
- Typed at the console, the line prints `% use 'write' command to make license boot config take effect on next boot` and the level stays.

Without a level stated or booted the book gives it:

| Platform | Level |
|---|---|
| IE3300 | `network-essentials` |
| Catalyst 2960, 2960-X, IE 3000, IE 3010 | `lanbase` |
| Catalyst 9200, 9300, 9400 | `network-advantage`, as a C9300 with no startup-config runs |

An image's own crawl answers only at the session's level.

### Donor crawls

Where the image's own answers, the abbreviations read and the answers derived hold none, a donor crawl answers the same way, in the same mode and the same context.

| Platform | Donor |
|---|---|
| IOS XE (Catalyst 9200, 9300, 9400, IE3300) | the C9300 17.6 crawl at the session's level |
| classic IOS (Catalyst 2960, 2960-X, IE 3000, IE 3010) | the C2960G 15.0(2)SE crawl at `lanbase` |

A donor answers for an image at a level only once that book, release and level is listed in `license::ENABLED` (`src/license.rs`). The listed images are those whose captured `?` answers the donor matched better than the books with those answers masked:

| Book | Release | Level |
|---|---|---|
| C2960-X | 15.2(7)E | `lanbase` |
| C9200 | 17.6 | `network-essentials` and `network-advantage` |
| C9200 | 17.15 | `network-essentials` and `network-advantage` |
| C9300 | 17.15 | `network-essentials` and `network-advantage` |
| C9400 | 17.5 | `network-advantage` |

- A line is read once across the image and the donor. Each abbreviated word is read against the answer for the line before it from the image where it holds one, else from the donor, and the line read takes the image's answer before the donor's.
- A derived answer is built from one of them alone.
- Where the donor holds no answer either, the books answer. A donor's keywords and the books' are never merged.
- A donor's answer does not move the caret of a refused line.
- A donor's answers refuse no line on Enter.

### Lines refused on Enter

A configuration or privileged EXEC line is read against the image's own answers after each whole word, in the same mode and context, and refused where the switch refused it, where that comes before the books' own refusal or the books take the line.

- A word no row of the answer before it offers is invalid there. A line whose answer after its last word holds no `<cr>` is incomplete, unless that answer offers again what the one before it offered (a list, which ends anywhere).
- A word with no answer held before it is not read. Reading stops at a held refusal for the line so far and at a `LINE` row.
- A held `% Unrecognized command` after the line's last word, typed as the line types it, refuses the line at that word, the books' acceptance included.
- A privileged line is read past its first word: where a crawl asked only `do-exec ?`, that answer stands for the prompt's and leaves out the commands that change mode.
- A number range row takes a rate with its unit (`1m`) and a list of numbers and ranges (`31-35,41`).
- A file system's row (`flash:`) fits a path after it and no other word.
- A value the crawl typed otherwise reads the keyword after it as after the crawl's own value: `spanning-tree vlan 2-4 cost` as `spanning-tree vlan 10 cost`. The value fills the same `<lo-hi>` row as the crawl's, or the `WORD` the switch describes as a numbered range (`vlan range, example: 1,3-5,7,9-11`), and the crawl's values there were answered alike. A number after the value can be bounded by it (the switch took `mpls label range 100 1000 static 16 99`, where its crawl's `mpls label range 16 16 static ?` offered 17 and up), and answers further on can depend on it (MST 0 takes `root primary diameter`, instance 10 does not); neither is read this way.
- An answer that lists the objects configured offers this session's: `No monitors configured` and its kin, and the crawl's own names, its `User defined` rows after `record`, `exporter`, `monitor` or `sampler`. On the C9300 17.6 image a flow record counts once it holds a field, and a flow monitor named on an interface once it holds a record, as the lab C9300-24T (17.6.4) refused an empty record in a monitor and a monitor without one on a port; the lab C9200L-48T-4G (17.6.4) read both names. On the C9200 17.6 and 17.15 images a port's monitor without a record answers the lab C9200L-48T-4G's message (output.md, Messages the configuration gives), and a sampler the session lacks is refused at its name. A keyword the answer offers beside the names is read as any other, and so is a word the switch answered `?` after where the answer said none was configured (`name` after `show flow exporter`).
- A refusal held right after a name such an answer listed, or after the keyword before a name (`name` after `clear flow monitor`), describes the objects the crawl's switch held. The line is read on past a name. After the keyword, where this session holds an object of the kind, the next word is read as one of this session's names; where it holds none, the refusal stands.
- A privileged line naming an object this session configured, of a kind `?` names (`live::NAMED_KINDS`: flow monitor, sampler, flow record, flow exporter), takes the name where the crawl, asked with none configured, listed none.
- On a routed port only the configured-object answers are read: every physical port shares one context, which holds the crawled switchport's answers. A PoE port's `power` lines are not read: the crawled ports had no PoE.
- The types after `switch <n> provision` are those the crawled switch stacks with (the lab C9200L-48T-4G listed C9200L models alone): where they name none of the session's stack family, the line is left to the books.
- Lines in states the crawl lacked are left to the books (`context::UNJUDGED`): the AAA lines before `aaa new-model`, VRRP before `fhrp version vrrp v3`, `xconnect` on a sub-interface, port-security MAC addresses, `media-type`, and `ptp sync` on a port, which `?` hides.

Lines the switch refused where its crawl offers the words (`context::REFUSED`), refused on the image and at the levels they were measured on, where that image has its own crawl at the session's level:

| Image | Levels | Mode | Line | State | Caret |
|---|---|---|---|---|---|
| c9300 17.6 | network-advantage, network-essentials | config | `class <name>` | any | after `class`; `class-map` is taken |
| c9200 17.6 | network-advantage, network-essentials | config | `class <name>` | any | after `class`; `class-map` is taken |
| c9200 17.15 | network-advantage, network-essentials | config | `class <name>` | any | after `class`; `class-map` is taken |
| c9300 17.6 | network-advantage | config | `ptp mode boundary delay-req`, `ptp mode boundary pdelay-req`, `ptp mode e2etransparent`, `ptp mode p2ptransparent` | `ptp profile dot1as` | at `mode` |
| c9300 17.6 | network-advantage | config | `ptp transport ipv4 udp` | `ptp profile dot1as` | at `transport` |

- A unique abbreviation of a line stands for it (`ptp m e2etransparent`). Each word is read against the keywords offered before it, the held answer's else the books', and the line's own word, which the switch read there.
- A word beginning the line's word and another, up to the caret's word, answers `% Ambiguous command:  "<line>"` (`clas C1`: `class` and `class-map`). Past the caret's word only a unique start stands.

### Licence gate

A level that runs less of an image than another (`license::GATED`: `network-essentials` against `network-advantage`) refuses, in any mode and EXEC too, a word that the other level's `?` offers and its own does not. The answers are the image's own crawls where it holds both levels, else the donor's at both where `license::ENABLED` lists the image at that level. Where neither holds both, nothing is gated.

- Each word is read against both levels' answers for the line before it, in the same mode and context. A word the level's answer does not fit and the other level's does is refused at its start, and the caret stands where the level's own answer puts it.
- A word the level's answer fits is read on, and one with no answer before it at both levels is passed over. Reading stops at a word neither answer fits, at a `LINE` row of the level's answer and at a refusal held at either level.
- An interface named by its type and number fits a row of that type, a file system's row (`flash:`) fits a path after it, and any other word fits as on Enter.
- A line whose numbers neither crawl typed is read as a line both typed with other numbers in the same `<lo-hi>` ranges: `monitor session 2 type` as `monitor session 1 type`.
- The refusal that reads the line least far stands, among the books', the gate's and the one read against the image's answers; where the books and the gate refuse the same word, the gate's place in it is kept.
- Every physical port shares one context, which holds the crawled switchport's answers, and a routed port's lines are read against them.
- A keyword the gate refuses stays refused under the carry-down (Sweeps), which takes a line only where the level's own crawl offers every keyword of it.

Measured exceptions, `src/license.rs`:

| Table | Level | Mode | Lines | Answer |
|---|---|---|---|---|
| `REFUSED_TYPES` | `network-essentials` | any | `pseudowire`, `Auto-Template` with a number | refused past the characters another listed type begins with (`interface pseudowire 100` under the `s`) |
| `UNLISTED` | `network-essentials` | config | `net` | `net` taken, the NET after it refused under its first character |
| `TAKEN` | `network-essentials` | config | `l2 traceroute` | taken though `?` lists no `l2`; `l2 vfi` is refused |
| `TAKEN_ON` | `network-advantage` on c9200 17.6 and 17.15 | config | `l2 traceroute` | taken though that image's `?` lists no `l2` |
| `LEFT_OUT` | `network-essentials` | config-subif | `xconnect` | refused at the word |

A table's word stands for its unique abbreviations among the keywords offered where it is typed, the held answer's else the books' (`context::stands_for`). A start of it and of another keyword answers `% Ambiguous command` for `UNLISTED` and `LEFT_OUT`, and is not taken for `TAKEN` and `TAKEN_ON`.

What the gated level prints for a line its image leaves out (`network-essentials`, IOS XE):

| Line | Answer |
|---|---|
| `router bgp`, `router isis` | `Protocol not in this image`, the mode kept |
| `vrf definition <name>`, `ip vrf <name>` | `% Feature is not supported`; the management port's `Mgmt-vrf` is taken |
| `redistribute isis` under a router | `% Failed to create routing protocol isis` |
| `show running-config` of a named `router eigrp` | each IPv4 address family ends with `eigrp stub connected summary` and each IPv4 service family starts with `eigrp stub connected`, unless it states its own `eigrp stub` |

## Sweeps

A sweep is a transcript of lines typed on Enter at the privileged prompt of a lab switch, each followed by the switch's parser answer alone. `vswitch corpus` reads sweeps and the binary does not embed them. A sweep's forms and refusals are a grammar overlay for its own book, release and level (Matching in `grammars.md`).

| Image | Lab switch | Levels |
|---|---|---|
| C9300 17.6 | C9300-24T, 17.6.4 | `network-advantage`, `network-essentials` |
| C9200 17.6 | C9200L-48T-4G, 17.6.4 | `network-advantage`, `network-essentials` |
| C9200 17.15 | C9200L-48T-4G, 17.15.6 | `network-advantage`, `network-essentials` |

### Sweep folders

- The log is `<book>-<release>/sweep/enter.log`, or `sweep-<name>/enter.log` beside it for the same image at another level or the same one, with a `level` file as a crawl folder has. Folders at one level add their lines.
- Each line is written after its prompt. After it stands nothing where the switch answered with output or nothing, the `%` lines it printed before any output where it answered with a message, else the caret line and `% Invalid input detected at '^' marker.`, `% Incomplete command.`, or `% Ambiguous command:  "<line>"`.
- The caret's column counts the prompt and the line as typed. A caret before the first word stands at its start.
- A line the switch answered `% Unrecognized command` gives no row, as no data says how vswitch prints that answer.
- `vswitch corpus` prints each sweep folder with its level, then per level the forms, those holding messages, the refusals and the lines that gave neither, by reason.

### Forms

An answered line becomes a form: its words, each value a typed argument. Each word is read against the `?` answer for the line before it in privileged EXEC: the image's crawl at the sweep's level, else its crawl at another level, which types values only and grants no keyword.

| Token | Takes | The answer's row |
|---|---|---|
| a keyword | the keyword, by any unique start | the keyword's row |
| `<number:lo-hi>` | a decimal number in the range | `<lo-hi>` |
| `<hex:lo-hi>` | a hexadecimal number in the range | `<0-FFFFFFFF>` and its kin |
| `<interface:Type>` | that type and a number, by IOS's abbreviations where vswitch models the type; the session's stack decides which ports exist | an interface type's row (`GigabitEthernet`, `Vlan`, `nve`) |
| `<ipv4>`, `<ipv4/len>` | an IPv4 address, a prefix | `A.B.C.D`, `Hostname`; `A.B.C.D/nn` |
| `<ipv6>`, `<ipv6/len>` | an IPv6 address, a prefix | `X:X:X:X::X`, `Hostname or X:X:X:X::X`; `X:X:X:X::X/<0-128>` |
| `<mac>` | a MAC address | `H.H.H` |
| `<word>`, `<line>` | any word, the rest of the line | `WORD`, `LINE` |

- The rows are tried in that order, a keyword first.
- A line gives no form where a word fits no row, or names an object the configuration made: a `User defined` row, or a VRF's `VRF Name` row other than `Mgmt-vrf`.
- A form's value is never another keyword: one observed name stays that name.
- The words after a value the crawl did not type are read against the answers after one it did that fits the same row; after an interface, one of the same type first, then one of another type (the lab C9200L-48T-4G, 17.6.4, crawled `show interfaces GigabitEthernet1/0/1 ?` and no other interface). Where the words after it read through none of the first value's answers, the next value's are tried.

### Messages

A form holds the `%` lines its answered lines printed before any output, as the fourth field of its row, one per line (`src/swept.rs`, `said`; `src/messages.rs`).

- A line is held only where it fits a message of `messages::MESSAGES`: a notice, a fixed message or a state message (Measured show messages in `output.md`). A fixed or state message is held only where it ends the lines. Notices before a line no message fits, or before a fixed or state message another line follows, are held alone: `show bgp ipv6 2001:DB8::/32` printed the obsolete notice, `% Network not in table` and the deprecation notice (the lab C9200L-48T-4G, 17.15.6).
- A value the message names is held as its place in the form (`% No VRF named {3}`), the one value token whose word of the line it is; an interface by its full name. A message naming no value of the form, or two, is not held. A value the form holds as a keyword stays as printed.
- Every line of a form at its level printed the same held lines. A form whose lines printed output beside a fixed message or a notice holds none. A state message beside output is held: the configuration decides between them.
- A line answered at two levels printed the same `%` lines at both, or neither level's form holds a fixed message or notice for it. A state message at one level beside output at the other is held.
- A level's messages print from its own forms. The carry-down carries none.

### Refusals

A refused line becomes a refusal, written `invalid <word> <offset>` (the caret's word and its place in it), `incomplete` or `ambiguous`.

- Its words before the caret are read as a form's are, numbers and interfaces kept as typed.
- The caret's word is kept as typed where it is digits, an interface or an offered lowercase word (Privacy). It is `<word>` where the answer before it holds no value row and the word starts no keyword there.
- Words after the caret are kept as typed where Privacy allows, an address as its token, else `<word>`.
- A refusal is matched whole: the same number of words, each word before the caret its keyword or a unique start of it among the keywords offered there, numbers equal by value and interfaces by name, the caret's word and those after it as written.

### Privacy

- Values are tokens and configured objects give no form, so a form names nothing of the site. A held message is a fixed text of `messages::MESSAGES` with each value written as its place in the form.
- A word a row holds as typed is digits (a minus first), an interface name, a word some privileged EXEC `?` answer of the image's own crawls offers at any level, or such a word with a number after it. A row that would hold any other word is left out (`Unoffered`).
- A form or refusal holding a denied word is left out whole, as crawl answers are; a keyword the image's answer offered where the word stands passes (Site words). The denied words include each sweep's prompt name and the addresses, MAC addresses, serials and numbered names its lines and answers hold, and no word of IOS's vocabulary. The leak check before the corpus is written, and `--scan` on the written corpus, read a sweep row's line as the build does and its messages as any other string.

### Enter, `?` and Tab

At the privileged prompt of a session whose image and level a sweep covers:

| Order | Rule |
|---|---|
| 1 | a refusal of that level stands over the books and the forms, with its caret where the switch drew it |
| 2 | a line a form of that level takes is taken; where the books take it too, the books' command runs, else the form's (Measured forms in `grammars.md`) |
| 3 | otherwise the books, held refusals and the licence gate read it |
| 4 | at `network-essentials`, where the image's own crawls hold both levels, a line no refusal of that level holds and rows 2 and 3 leave refused is taken by a `network-advantage` form that both crawls carry (the carry-down); the books' command runs where they take the line, else the form's |

- A form is carried where each of its keywords is offered at its place by both levels' crawls, each alone, and each value fits a value row of the same kind at both (a number in any range). A keyword the level's crawl leaves out keeps the line refused, a value typed otherwise or a word with no answer before it at either level makes it ineligible, and other rows the two answers offer do not count.
- Outside row 4 a level reads its own sweep alone. A donor's crawls and sweeps, and another release's, carry, grant and refuse nothing.
- `?` and Tab answer from the held crawl first. Where none is held, the books' keywords and the level's forms' together answer; after `no`, the books' alone.
- A switch-observed `?` row that Enter refused stays offered: `show ip route ?` lists `Hostname`, and `show ip route Hostname` is refused.
- A line a form of the session's level takes prints the messages the form holds where they stand, whichever command runs (Measured show messages in `output.md`). A word the books or the level's forms read as a keyword where it stands makes the line that keyword's, and the messages of a form reading it as a value do not print (`show ip vrf detail` beside `show ip vrf <word>`).

## capture

`vswitch capture <user@host> [--key <file>] [--out <dir>] [--pending <dir>]` reads a switch over SSH with show commands only. `--out` defaults to the current folder.

| Step | Commands |
|---|---|
| paging and width | `terminal length 0`, `terminal width 80` |
| `?` answers, the common show prefixes | `show ?`, `show vtp ?`, `show vlan ?`, `show interfaces ?`, `show etherchannel ?`, `show spanning-tree ?`, `show power inline ?`, `show ip ?` |
| identity | `show version` (the release), `show inventory` (the stack as `--stack` notation, and any part vswitch has no model for) |
| configuration | `show running-config` |
| state | `show interfaces status`, `show vlan brief`, `show etherchannel summary`, `show interfaces trunk`, `show ip interface brief`, `show power inline`, `show vtp status` |
| width 132 | `terminal width 132`, `show vlan brief` again, `show running-config | include hostname ` |

The `?` follows show commands only, so IOS runs the line it redisplays and a show command changes nothing. `show vlan brief` runs at widths 80 and 132 to show how the switch wraps.

- SSH runs with `BatchMode=yes`: key authentication only, never a password prompt. `--key` adds `-i <file>` with `IdentitiesOnly=yes`.
- The session also sets `ConnectTimeout=20` and `StrictHostKeyChecking=accept-new`, and allocates a terminal. `VSWITCH_SSH_HOST_KEYS=yes` connects only to a host whose key is already in `known_hosts`.
- The target follows `--`; a target starting with `-` is refused.
- `VSWITCH_SSH` names another ssh program.
- The capture writes `<out>/<book>-<release>/<host>-running-config.txt` and `<host>-show.log`, the layout of a captures folder.
- Both files are swept of secrets. Serial numbers are removed (`show version` serial lines and processor board ID, `show inventory` `SN:`, a configuration's `license udi ... sn`). A value either sweep removes is removed wherever else it stands in the session.
- The switch's configuration is then typed into vswitch at the switch's release, and the first 20 lines vswitch refuses are listed.
- A configuration comes from `show running-config`, else from any command that printed one, and from `show startup-config`, `show configuration` or `show archive` only where nothing else printed one.
- The command fails when the transcript prints no configuration.

A capture naming hardware or a release vswitch does not model, or a member or card it does not model, goes under `--pending <dir>` as `<part>-<release>`, with `unidentified` and the configuration's `version` where they say nothing. Without `--pending` such a capture is refused. A pending capture is never live evidence.

The stack notation numbers a member where its number is not its place (`2=C9300-48P`). A 9400 card always carries its slot number.

### From logs

`--log <session log>` reads a terminal log of such a session instead of connecting.

- A log's lines that a terminal cut every 80 characters and padded to its window's width are joined.
- A byte order mark and UTF-16 are read.
- `--log` repeated reads the logs as one session.
- `--config <file>` adds a configuration file where the session printed none. A session that printed its own configuration reports the file as not used.

### --folder

`vswitch capture --folder <dir> [--out <dir>] [--pending <dir>]` reads every file at the top of a folder for what it holds, whatever its name:

- a session
- a configuration printed before the first prompt
- a configuration file alone
- a backup tool's output (the prompt alone on a line, the command on the next)
- `show tech`, of which the sections that vswitch models or reads and the neighbours are kept

Files group by the switch they name, in any case, and the first session's spelling names it. A session's own configuration is kept over a backup file's. A saved configuration (`show startup-config`) is used only where nothing else printed one.

Commands are read through the abbreviations IOS takes (`sh int status`). A `show tech` typed at its prompt splits into its sections.

For a switch captured already, at the session's own release:

- the capture gains the commands its log lacks, with the kept log's bytes kept
- its configuration is kept, and a differing one is reported as `vswitch changes` reports it

At another release the switch is a capture of its own. A session naming no release is refused. The command prints one line per switch and a summary of switches captured, added to a kept capture and not captured. It exits 1 when any switch fails or when the folder names no switch.

## changes

`vswitch changes <backup file or folder>... --captures <dir>` reads a backup tool's configuration of a captured switch against its capture at the backup's release, else the newest. A folder names the files in it.

Output:

- the items it adds, removes and edits, as a unified diff, ordered and nested
- a line moved between sections, or reordered, as a change
- first-match lists (numbered access lists, AS-path and community lists) paired in order
- a banner as one item
- the lines its release refuses that the kept configuration's did not
- the lint findings and notes it adds or clears
- the cables its port descriptions make or no longer make
- the findings on the switch's cables, each neighbour built from its own capture

Save-time lines are left out: `Building configuration`, `Current configuration :`, `ntp clock-period`, `!` lines and `end`.

A backup whose version line names another release is typed at that release. A file holding no configuration (a `version` and a `hostname` line) or session is named and fails the run. The command exits 1 on a new refusal or a new finding. Notes and cable changes do not fail it.

## probe

`vswitch probe <user@host> <script> --lab [--key <file>] [--out <dir>] [--skip-unsafe] [--map <from>=<to>]...` is for a lab switch whose configuration may change. `--lab` is required.

### Steps

1. It reads the switch: release, hardware, `show ip interface brief`, `show interfaces status` and the running configuration. The target's name resolves to addresses, and one interface must hold one of them, else nothing is sent.
2. `--map Twe1/0/=Gi1/0/` renames the script's ports onto ports the lab switch has, before the script is checked.
3. It refuses the script when a line could cut the session or is known to crash the switch (below) and sends nothing.
4. It copies the running configuration to `flash:vswitch-probe-before.cfg` in a session of its own.
5. It types the script after IOS answers `bytes copied`, then reads the running configuration.
6. It puts the configuration back with `configure replace flash:vswitch-probe-before.cfg force`.

The script stops at `end`. Blank lines and `!` comments are skipped.

### Refused lines

A line is read as IOS reads it: abbreviated, spaced, in its `no` and `default` forms.

| Refused | Reason |
|---|---|
| `aaa`, `username`, `enable secret`, `enable password`, `tacacs`, `radius` | the session's login |
| `line`, `ip ssh`, `crypto`, `ip http`, `netconf`, `restconf`, `gnxi`, `management` | the session's path |
| `ip route`, `ip default-gateway`, `ip domain`, `ntp`, `snmp-server`, `logging host`, `vrf definition` | routes and addresses |
| `hostname`, `boot`, `reload`, `license`, `archive`, `event manager` | the switch's identity |
| `no ip routing`, `default ip routing` | routing turned off |
| `write`, `copy`, `erase`, `delete`, `format` | files |
| a command known to crash the switch's model ([output.md](output.md#commands-known-to-crash-a-switch)), bare or after `do`; any model's when the model is not known | a crash ends the session and skips the `configure replace` rollback |
| `do`, `do-exec` (typed whole or as a start of three characters or more) | EXEC from configuration mode |
| `exit`, `logout`, `quit`, `disable`, `clear` | leaves configuration mode, so later lines run at the EXEC prompt |
| `interface range macro` | ports the probe does not read |
| a control character | the terminal acts on it |
| the management interface or its VRF (`mgmt-vrf`); an `interface range` or VLAN list spanning it too | the session's interface |
| a port carrying the management VLAN, trunking, or bundled into a port-channel | the session's path |

### --skip-unsafe

`--skip-unsafe` leaves the refused lines out, with any lines under a block it leaves out, and lists them. A whole configuration (a generated `.cfg`) probes as a script this way.

### Result

The probe reports:

- each line the switch took and vswitch refuses, or the reverse
- each configuration line IOS kept that vswitch does not keep, or the reverse
- whether `configure replace` answered `Rollback Done`

It writes `<host>-probe-<script>.log` and `<host>-probe-<script>-running-config.txt` under `<out>/<book>-<release>/`, swept of secrets. `--out` defaults to the current folder, and a classic switch's folder is named as its capture is. Added to the captures folder and compiled into the corpus, that configuration's lines are held at the switch's release.

The command exits 1 on a difference, when the replace did not answer `Rollback Done`, and when it refuses the script.
