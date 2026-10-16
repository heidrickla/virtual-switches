# Hardware and releases

`vswitch` is a virtual Cisco switch command line in one self-contained Rust binary. Each switch follows Cisco's documentation for its platform and the release it runs, for writing and testing switch configurations without hardware.

Platforms: Catalyst 9300 (9300X, 9300, 9300L, 9300LM), 9200 (9200, 9200L), 9400, 2960-X, 2960 (2960, 2960-Plus, 2960-C), IE3300, Cisco IE 3000 and Cisco IE 3010. `vswitch releases` lists the releases each platform has. Hardware is named with `--stack <members>`: models joined by `,`, member 1 first, each with an optional `+<network module>`, for example `C9300X-24Y+C9300X-NM-4C,C9300X-24Y+C9300X-NM-4C` (the default stack). A 9400 card or supervisor is the member numbered by its slot (`3=C9400-SUP-1XL`). A `+` inside a model name belongs to the name (`--stack WS-C2960+24TC-L`).

## Releases

Every release Cisco publishes a command reference for has a compiled grammar.

| Platform | Releases |
|---|---|
| Catalyst 9300 | 16.5, 16.6, 16.8 to 16.12, 17.1 to 17.18, 26.x |
| Catalyst 9200 | 16.9 to 16.12, 17.1 to 17.18, 26.x |
| Catalyst 9400 | 16.6, 16.9 to 16.12, 17.1 to 17.18, 26.x |
| Catalyst 2960-X | 15.2(2)E, 15.2(4)E, 15.2(6)E, 15.2(7)E |
| Catalyst 2960, 2960-Plus, 2960-C | 12.2(44)SE, 12.2(55)SE, 15.0(2)SE, 15.2(2)E, 15.2(4)E |
| Catalyst IE3300 | 17.6 |
| Cisco IE 3000 | 12.2(50)SE, 12.2(52)SE, 12.2(55)SE, 12.2(58)SE, 15.0(2)EY, 15.0(2)SE, 15.2(2)E |
| Cisco IE 3010 | 12.2(53)EZ, 15.0(2)SE, 15.2(2)E |

A release not in a platform's row is refused, naming the releases the platform has.

### Choosing the release

| Case | Release |
|---|---|
| `--release`, a design's `release = "17.9"`, or a configuration's `version 17.9` line | that release |
| None named, IOS XE platform | 17.15 where the book documents it and the hardware runs it |
| None named, no 17.15 | the book's newest release |
| None named, hardware that stops earlier | the release leaving the fewest problems, tried in the order 17.15 then newest to oldest (a WS-C2960-24TC-L stops at 15.0(2)SE) |
| Patch release (`17.9.4`) | its train's book (`17.9`) |
| Any 26 release | the 26.x book |
| Rebuild (`15.2(6)E1`) | its train's book |
| Classic `version 15.2` line, which names no train | the newest 15.2 train the book documents |
| A classic train past the last book of its line | that book (a 2960-Plus at 15.2(6)E1 reads 15.2(4)E, which Cisco titles "and later") |
| `--release` that disagrees with the file's own release | refused |
| Hardware newer than the release, or older than the last release that runs it | refused, naming the release that introduced or last ran it |

Default release by platform when nothing names one:

| Platform | Default |
|---|---|
| Catalyst 9300, 9200, 9400 | 17.15 |
| Catalyst 2960-X | 15.2(7)E |
| Catalyst 2960-Plus, 2960-C | 15.2(4)E |
| WS-C2960-24TC-L, WS-C2960G-24TC-L, WS-C2960G-8TC-L | 15.0(2)SE |
| Catalyst IE3300 | 17.6 |
| Cisco IE 3000, IE 3010 | 15.2(2)E |

Each release's grammar is compiled from that release's own books, so commands, keywords and modes follow what its documentation lists. The switch list keeps each switch's release, and a configuration loaded with a `version` line opens the builder at that release.

Refusals, carets, privilege levels and terminal settings: [Output](output.md).

## Models

Sources are the data sheets, hardware installation guides and release notes, and captures of real switches.

| Family | Models |
|---|---|
| Catalyst 9300X | C9300X-24Y, -12Y, -48TX, -24HX, -48HX, -48HXN |
| Catalyst 9300 | C9300-24T, -48T, -24S, -48S; PoE+ -24P, -48P; UPOE -24U, -48U, -24UX, -48UXM, -48UN; UPOE+ -24H, -48H; B models C9300-24UB, -48UB, -24UXB |
| Catalyst 9300L | C9300L-24T-4G, -48T-4G, -24T-4X, -48T-4X; PoE+ -24P-4G, -48P-4G, -24P-4X, -48P-4X, -48PF-4G, -48PF-4X; UPOE -24UXG-4X, -24UXG-2Q, -48UXG-4X, -48UXG-2Q; built-in uplinks |
| Catalyst 9300LM | C9300LM-48T-4Y; UPOE -24U-4Y, -48U-4Y, -48UX-4Y; built-in uplinks |
| Catalyst 9200 | C9200-24T, -48T; PoE+ -24P, -48P, -48PL; B models -24PB, -48PB; PoE+ C9200-24PXG, -48PXG |
| Catalyst 9200L | C9200L-24T-4G, -48T-4G, -24T-4X, -48T-4X; PoE+ -24P-4G, -48P-4G, -24P-4X, -48P-4X, -48PL-4G, -48PL-4X, -24PXG-4X, -48PXG-4X, -24PXG-2Y, -48PXG-2Y; built-in uplinks |
| Catalyst 9400 supervisors | C9400-SUP-1, -SUP-1XL, -SUP-1XL-Y, C9400X-SUP-2, -SUP-2XL |
| Catalyst 9400 line cards | C9400-LC-24S, -48S, -24XS, -48XS, -24XY, -12QC, -48T, -48TX, -48U, -48P, -48H, -48UX, -48HN, -48HX |
| Catalyst 2960-X | WS-C2960X-24TS-L, -48TS-L, -24TD-L, -48TD-L, -24TS-LL, -48TS-LL; PoE+ -24PS-L, -48LPS-L, -48FPS-L, -24PD-L, -48LPD-L, -48FPD-L, -24PSQ-L (data sheet) |
| Catalyst 2960, 2960-Plus, 2960-C | WS-C2960-24TC-L, WS-C2960G-24TC-L, WS-C2960G-8TC-L, WS-C2960+24TC-L, WS-C2960C-8TC-L (data sheets) |
| Catalyst IE3300 | IE-3300-8T2S (data sheet) |
| Cisco IE 3000 | IE-3000-8TC (data sheet, captures) |
| Cisco IE 3010 | IE-3010-16S-8PC (data sheet, captures) |

### Network modules

| Platform | Models that take a module | Modules |
|---|---|---|
| Catalyst 9300X | C9300X-24Y, -48TX, -48HX | C9300X-NM-2C, -NM-8Y, -NM-8M, -NM-4C |
| Catalyst 9300X | C9300X-12Y, -24HX, -48HXN | C9300X-NM-2C, -NM-8Y, -NM-8M |
| Catalyst 9300, 9300 B | every model | C9300-NM-4G, -NM-4M, -NM-8X, -NM-2Q, -NM-2Y; C3850-NM-4-1G, -NM-2-10G, -NM-4-10G, -NM-8-10G, -NM-2-40G |
| Catalyst 9200, 9200 B | C9200-24T, -48T, -24P, -48P, -48PL, -24PB, -48PB | C9200-NM-4G, -NM-4X |
| Catalyst 9200 | C9200-24PXG, -48PXG | C9200-NM-4X, -NM-2Y, -NM-2Q |
| Catalyst 9300L, 9300LM, 9200L, 2960-X, 2960, IE | none | built-in uplinks |

The 9400 takes no network module.

### Stacks

| Rule | Detail |
|---|---|
| Mixed 9300X and 9300 | allowed; StackWise-480 when mixed |
| Mixed 9300L and 9300LM | allowed; StackWise-320 |
| 9200 and 9200L | each stacks only with itself |
| B models (C9300-24UB, -48UB, -24UXB; C9200-24PB, -48PB) | stack only with B models of their series |
| Size | 8 members at most, numbered 1 to 8 |
| Models that do not stack | a switch that does not stack is member 1: every 2960, 2960-Plus, 2960-C, IE3300, IE 3000 and IE 3010, and the WS-C2960X-24TS-LL, -48TS-LL and -24PSQ-L |
| 2960-X | up to eight in a FlexStack-Plus stack |
| 9400 | a chassis whose members are its slots, 1 to 10 |

### Releases that introduce hardware

A model or module runs from the release Cisco introduced it in (26.1 release notes).

| Family | Introduced |
|---|---|
| C9300-24T, -48T, -24P, -48P, -24U, -48U; C9300-NM modules except -NM-4M and -NM-2Y; C3850-NM modules | 16.5 |
| C9300-24UX, -48UXM | 16.6 |
| C9300-48UN; C9300-NM-4M, -NM-2Y | 16.8 |
| C9300-24S, -48S; C9300L | 16.11 |
| C9300L-48PF and -UXG models; B models C9300-24UB, -48UB, -24UXB | 16.12 |
| C9300-24H, -48H | 17.2 |
| C9300X-12Y, -24Y, -48TX, -48HX; C9300X-NM-2C, -NM-8Y, -NM-8M | 17.5 |
| C9300X-NM-4C | 17.6 |
| C9300X-24HX, -48HXN; C9300LM | 17.9 |
| C9200L | 16.9 |
| C9200-24T, -48T, -24P, -48P; C9200-NM-4G, -NM-4X | 16.10 |
| C9200L PXG models | 16.11 |
| C9200-24PXG, -48PXG; C9200-NM-2Y, -NM-2Q | 17.1 |
| C9200-24PB, -48PB | 17.2 |
| PL models (C9200-48PL, C9200L-48PL) | 17.3 |
| C9400-SUP-1, -SUP-1XL, C9400-LC-24XS, -48T, -48U, -48UX | 16.6 |
| C9400-LC-24S, -48S, -48P | 16.8 |
| C9400-SUP-1XL-Y | 16.9 |
| C9400-LC-48H | 16.12 |
| C9400-LC-48HN | 17.5 |
| C9400X-SUP-2, -SUP-2XL | 17.7 |
| C9400-LC-48XS, -48HX | 17.8 |
| C9400-LC-24XY, -12QC | 17.12 |
| C9400-LC-48TX | 17.13 |

The C9400-SUP-1, -SUP-1XL and -SUP-1XL-Y run up to 17.18 (26.1 release notes). The 2960 models WS-C2960-24TC-L, WS-C2960G-24TC-L and WS-C2960G-8TC-L run up to 15.0(2)SE.

### 9400 chassis

The supervisors' slots fix the chassis (supervisor data sheet). A configuration has one or two supervisors, two of one model, and line cards in the chassis's line card slots.

| Chassis | Supervisor slots | Line card slots |
|---|---|---|
| C9404R | 2 and 3 | 1 and 4 |
| C9407R | 3 and 4 | 1, 2, 5 to 7 |
| C9410R | 5 and 6 | 1 to 4, 7 to 10 |

### 9400 supervisor uplinks

A supervisor's QSFP port runs once `enable` turns it on (`interface FortyGigabitEthernet3/0/9`, `enable`). Turning it on turns off the SFP ports it pairs with (Supervisor Module Installation Note).

| Supervisor | Ports that run together |
|---|---|
| Supervisor 1 | ports 1-4 or port 9; ports 5-8 or port 10 |
| Supervisor 2 | ports 1-4 or port 5 |

With two supervisors, a Supervisor 1's ports 5-8 and 10 and a Supervisor 2's ports 7 and 8 never run. `generate` writes `enable` under a QSFP uplink a design uses and refuses the ports it turns off.

## Ports

Ports are `<type><member>/0/<port>` and module uplinks `<type><member>/1/<port>`. The management port is `GigabitEthernet0/0`.

| Platform | Naming |
|---|---|
| Catalyst 9300 and 9300 B member | lists the whole uplink slot whatever module is fitted, as IOS does: `GigabitEthernet<n>/1/1-4`, `TenGigabitEthernet<n>/1/1-8`, `FortyGigabitEthernet<n>/1/1-2`, `TwentyFiveGigE<n>/1/1-2`; and `AppGigabitEthernet<n>/0/1` |
| Catalyst 9300L | its built-in uplinks on slot 1; and `AppGigabitEthernet<n>/0/1` |
| Catalyst 9300X | the fitted module's ports on slot 1 |
| Catalyst 9300LM | its built-in uplinks on slot 1 |
| Modular Catalyst 9200 | the ports of every module its model takes (a C9200-24T lists the NM-4G's and the NM-4X's) |
| Catalyst 9200L | its built-in uplinks on slot 1 |
| Catalyst 9400 supervisor | `<type><slot>/0/<port>`; `AppGigabitEthernet<slot>/0/1` |
| Catalyst 9400 card | `<type><slot>/0/<port>` |
| Catalyst 2960-X | uplinks follow the front ports on slot 0 (`Gi1/0/25-28`, `Gi1/0/49-52`, `Te1/0/1-2`; the LL models `Gi1/0/25-26`, `Gi1/0/49-50`); the management port is `FastEthernet0`, as a 24PS-L's running configuration names it |
| Catalyst 2960, 2960-Plus, 2960-C | `<type>0/<port>` (`FastEthernet0/1`, `GigabitEthernet0/2`), the dual-purpose uplinks after the front ports, no management port |
| Catalyst IE3300 | one switch, no management port: SFP ports `GigabitEthernet1/1` and `1/2`, copper `1/3` to `1/10` |
| Cisco IE 3000 | one switch, no management port: copper `FastEthernet1/1` to `1/8`, dual-purpose uplinks `GigabitEthernet1/1` and `1/2` |
| Cisco IE 3010 | one switch, no management port: `FastEthernet0/1` to `0/16` SFP, `0/17` to `0/24` copper, dual-purpose uplinks `GigabitEthernet0/1` and `0/2` |

The WS-C2960X-24PSQ-L's uplinks are two SFP and two 10/100/1000 ports (data sheet); the model takes all four as SFP.

### Interfaces that carry no traffic

IOS accepts configuration on these interfaces. `generate` and `lab` refuse a design that uses one.

| Interface | Why |
|---|---|
| Slot interfaces the fitted module does not use | the hardware installation guide's "Disregard" rows; the 10G ones of a 40G module carry a 4x10G breakout cable |
| Ports 7 and 8 of a C9300X-NM-8Y or -NM-8M in a C9300X-48HXN | disabled (release notes) |
| The ports a redundant 9400 supervisor pair does not run | see 9400 supervisor uplinks |
| A single supervisor's ports beyond its uplink option | see 9400 supervisor uplinks |

### Taken as

Port names Cisco's documents do not give are taken as follows.

| Hardware | Taken as |
|---|---|
| C9200-24PXG, -48PXG, C9200L-24PXG-4X, -24PXG-2Y, -48PXG-2Y | the mGig ports are the last eight, `TenGigabitEthernet` |
| C9200L-48PXG-4X, C9300L-48UXG | the mGig ports are the last twelve, `TenGigabitEthernet` |
| C9300L-24UXG, C9300LM-48UX-4Y | the mGig ports are the last eight, `TenGigabitEthernet` |
| C9300X-48HXN | 1-40 `FiveGigabitEthernet`, 41-48 `TenGigabitEthernet` |
| C9400-SUP-1XL-Y | ports 1-8 `TwentyFiveGigE`, 9-10 `FortyGigabitEthernet` |
| C9400X-SUP-2, -SUP-2XL | ports 1-4 `TwentyFiveGigE`, 5-8 `HundredGigE` |
| C9400-LC-24XY | 1-20 `TwentyFiveGigE`, 21-24 `TenGigabitEthernet` |
| C9400-LC-12QC | 12 `FortyGigabitEthernet` |
| C9400-LC-48HN | `FiveGigabitEthernet` |

### Front-panel ports of mixed-speed models

| Model | Ports |
|---|---|
| C9300-48UXM | 1-36 `TwoGigabitEthernet`, 37-48 `TenGigabitEthernet` |
| C9300-24UX, C9300X-24HX, C9300X-48HX, C9300X-48TX | `TenGigabitEthernet` |
| C9300-48UN | `FiveGigabitEthernet` |
| C9300X-24Y, -12Y | `TwentyFiveGigE` |
| C9400-LC-48UX | 1-24 `GigabitEthernet`, 25-48 `TenGigabitEthernet` |
| C9400-LC-24XS, -48XS, -48TX, -48HX | `TenGigabitEthernet` |
| C9400-SUP-1, -SUP-1XL | ports 1-8 `TenGigabitEthernet`, 9-10 `FortyGigabitEthernet` |

## PoE

Each PoE model has a kind (PoE+ 30 W a port, UPOE 60 W, UPOE+ 90 W) and the budget its default power supply gives (hardware installation guide, data sheet). A 9400 PoE card has a kind only, because its chassis power supplies set the budget.

| Kind | Model and budget in watts |
|---|---|
| PoE+ | C9300-24P 445, -48P 437; C9300L-24P-4G, -48P-4G, -24P-4X, -48P-4X 505; C9300L-48PF-4G, -48PF-4X 890 |
| PoE+ | C9200-24P, -24PXG, -48PL, -24PB; C9200L-24P-4G, -24P-4X, -48PL-4G, -48PL-4X, -24PXG-4X, -24PXG-2Y 370; C9200-48P, -48PXG, -48PB; C9200L-48P-4G, -48P-4X, -48PXG-4X, -48PXG-2Y 740 |
| PoE+ | WS-C2960X-24PS-L, -48LPS-L, -24PD-L, -48LPD-L 370; -48FPS-L, -48FPD-L 740; -24PSQ-L 110 |
| UPOE | C9300-24U 830, -48U 822, -24UX 560, -48UXM 490, -48UN 610; B models C9300-24UB 830, -48UB 822, -24UXB 560 |
| UPOE | C9300L-24UXG-4X 880, -24UXG-2Q 722, -48UXG-4X 675, -48UXG-2Q 675; C9300LM-24U-4Y 420, -48U-4Y 790, -48UX-4Y 790 |
| UPOE+ | C9300-24H 830, -48H 822; C9300X-24HX 735, -48HX 590, -48HXN 690 |
| 9400 line cards | C9400-LC-48P PoE+; C9400-LC-48U, -48UX UPOE; C9400-LC-48H, -48HN, -48HX UPOE+ |

## Port media

A port's media sets how `show interfaces status` shows it with nothing plugged in.

| Media | Ports |
|---|---|
| Copper (RJ-45) | front-panel copper ports and the management port |
| SFP | the 24S and 48S models' front ports; the 2960-X's GigabitEthernet uplinks; the 2960, 2960-Plus, 2960-C, IE 3000 and IE 3010 dual-purpose uplinks; FastEthernet0/1 to 0/16 of the IE-3010-16S-8PC |
| SFP+ | the C9400-LC-24XS, -48XS and the four SFP+ ports of the C9400-LC-24XY; the 2960-X's TenGigabitEthernet uplinks |
| Uplink | a 9400 supervisor's ports |

Transceiver parts known to the switch:

| Part | Speed | Shown as |
|---|---|---|
| SFP-10G-SR, SFP-10G-SR-S | 10G | `SFP-10GBase-SR` |
| SFP-10G-LR, SFP-10G-LR-S | 10G | `SFP-10GBase-LR` |
| GLC-SX-MMD | 1000 | `1000BaseSX SFP` |
| GLC-LH-SMD | 1000 | `1000BaseLX SFP` |
| GLC-FE-100FX | 100 | `100BaseFX SFP` |
| GLC-T, GLC-TE | 1000, negotiated | `10/100/1000BaseTX SFP` |

## Grammars

Grammars are `data/grammars/<book>-<release>.json.gz`, one per platform book and release (93 files), compiled into the binary.

| Platform | Book |
|---|---|
| Catalyst 9300 models | the 9300 book |
| Catalyst 9200, 9200L | the 9200 book |
| Catalyst 9400 | the 9400 book |
| Catalyst 2960-X | the 2960-X book |
| Catalyst 2960, 2960-Plus, 2960-C | the 2960 book |
| Catalyst IE3300 | the IE3300 book |
| Cisco IE 3000 | the IE 3000 book |
| Cisco IE 3010 | the IE 3010 book |

### 2960 and industrial books

The 2960 books document the switch's own commands. A command a 2960 capture uses that its book refuses (`hostname`, `line`, `vlan`, `snmp-server host`) is taken from the 2960-X book of the same IOS, at its newest train not after the 2960's. The IE 3000 and IE 3010 books cover the switches' own commands and take general IOS commands from the 2960-X book as the 2960's do.

### IE3300 book

Cisco publishes no IE3x00 command reference. The IE3300 book projects the 9300 grammar at the same release, keeping the commands its guides (the 17.1 configuration guide and the area guides) or its captures at that release name, then adds the guides' own steps and the overrides scoped to it. StackWise, stack power and network module commands are refused. It has the 17.6 release, the train its captures run.

### IE 3010 book

Cisco publishes no 12.2(53)EZ books for the IE 3010. Its configuration guide is read from the Wayback Machine (the May 2017 copies). Its commands are the IE 3010 15.0(2)SE command reference's as of 12.2(53)EZ: a command its Command History introduces in 15.0(2)SE is left out (34 commands), and so is an alternative naming a keyword a later row adds (`ip access-group ... { in }`, without `out`).
