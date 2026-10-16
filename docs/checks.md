# Checks

`vswitch lint` checks one configuration, `vswitch lab` checks switches cabled together, `vswitch topology` writes a lab file from captures, and `vswitch generate` checks a design before it writes a configuration.

## Lint

`vswitch lint <config> [--stack <members>] [--release <x.y>]` checks a configuration however it was written: a capture, a console session, a generated file. The configuration is typed in at the release it runs, then checked as it stands.

A finding is something IOS acts on (a member suspended, a setting with no effect) and fails the command with exit 1. A note names a choice a design may intend. A `not checked:` line names a check the configuration gave nothing to run on. The last line counts findings and notes.

| Check | Rule |
|---|---|
| VLANs a trunk or access port names | defined in the configuration, by a VLAN block or by a range held as a global line (`vlan 65-66`, the IE 3000s at 15.2(6)E): an access VLAN, and the VLANs a trunk's list names |
| VLAN list size | a trunk list carrying over 1000 VLANs is an `all` or `except` list and names none |
| VLAN definitions absent | skipped, said once, when the configuration defines no VLAN of its own and is not in VTP transparent or off mode, as a VTP server keeps them in vlan.dat |
| Routed ports | a port with `no switchport` is not checked as a switch port |
| Extended-range VLANs | need VTP transparent or off mode, under VTP versions 1 and 2 (command reference); VTP version 3 keeps them on a server |
| Native VLAN | a note: a configured native VLAN left off the trunk's allowed list drops untagged frames, which a design may intend; the default VLAN 1 is often left off; `native vlan tag` sets no VLAN |
| Portfast | `spanning-tree portfast` or `spanning-tree portfast edge` on a trunk acts only while the port is not trunking; a trunk takes `spanning-tree portfast trunk` or `spanning-tree portfast edge trunk` |
| BPDU guard and filter | not both on one port: the filter drops the BPDUs the guard acts on |
| Addresses | no address on two interfaces, no two subnets overlapping in one VRF (`vrf forwarding` or `ip vrf forwarding`); every address counts, a secondary as the primary; one finding for each pair of interfaces |
| Hardware | no configuration on a port the hardware does not run (a supervisor uplink that is off, a network module's unused slot ports); a port that is shut down or holds no lines is passed over |
| Hardware read from the configuration | with no `--stack`, the hardware read from the configuration names no module, so its slot ports are left out and a `not checked:` line says so |
| PoE | `power inline` only on a PoE model's front-panel port |
| PoE maximum | a `max` no higher than the port gives: PoE+ 30000 mW, UPOE 60000, UPOE+ 90000 |
| PoE budget | a note when a member's static ports reserve more than its default power supply gives |

### Port-channel members

| Rule | Detail |
|---|---|
| Protocol | one per bundle: LACP, PAgP or `on` |
| Switchport settings | each member's mode (`dynamic auto` with no mode line), access VLAN, allowed VLANs and native VLAN match the port-channel's; with no Port-channel interface in the configuration the first member stands in |
| Speed and duplex | match the first member's, `auto` where unset |
| Shutdown | no member shut down |

IOS suspends a member that differs. A port-channel's switchport lines reach its members, as IOS applies them.

### VLANs held outside the configuration

A VTP server or client keeps its VLANs and VTP settings in vlan.dat, outside the running configuration (9300 command reference). `show running-config` prints them only in transparent mode. `write memory` keeps them ahead of the configuration under a `! vlan.dat` line, so the switch list reopens with them.

## Lab files

`vswitch lab <lab.toml> [--shell <switch>]` builds each switch from its files, then checks each cable from both ends as the two switches bring the link up. It prints each switch with its hardware, release and typed line count, each cable's state, the findings, then `note:` lines. Exit 1 on a finding. `--shell` opens that switch's console after the report.

A lab file names switches and the cables between them. `tests/example-lab.toml` cables the example stack's design to an example core's running configuration with the core's change typed on top.

```toml
[[switch]]
name = "stack"
files = ["example-design.toml"]

[[switch]]
name = "core"
files = ["example-9400-running-config.txt", "example-core-design.toml"]
stack = "1=C9400-LC-48S,2=C9400-LC-24XS,3=C9400-SUP-1XL,4=C9400-SUP-1XL,5=C9400-LC-48U"
release = "17.5"

[[cable]]
a = "stack:Twe1/0/24"
b = "core:Te2/0/3"
optic = "SFP-10G-SR"
```

| Switch field | Meaning |
|---|---|
| `name` | the switch's name in cables; unique in the file |
| `files` | designs (`.toml`) and configurations, typed in order into one session, paths relative to the lab file; a design's `existing_vlans` go into the switch's VLAN database |
| `stack` | as `--stack`; else the first file that names hardware (a design's stack, or a configuration's provision lines and interface names); a switch whose files name none is refused |
| `release` | as `--release`; else the first file that names one (a design's `release`, or a configuration's `version` line); else the newest release the platform's book documents that the hardware runs |
| `mac` | the base MAC address as `xxxx.xxxx.xxxx`, which the bridge IDs and the VTP Device ID carry; else a locally administered address derived from `name` and the stack |

A `release` that disagrees with one a file names is refused. A malformed or multicast `mac`, and two switches with the same MAC, are refused.

| Cable field | Meaning |
|---|---|
| `a`, `b` | `switch:port`; a name alone is a device the lab does not model, taken to run what the port expects (a trunk, LACP or a static bundle, no DTP or PAgP) |
| `optic` | the transceiver in each transceiver port of the cable; none between two RJ-45 ports |
| `speed` | the speed a copper link negotiates when the far end runs slower (Mb/s), one the copper port runs |
| `half` | the far end negotiates half duplex (a device on an old hub, `a-half a-100`) |

Accepted optics: SFP-10G-SR, SFP-10G-SR-S, SFP-10G-LR, SFP-10G-LR-S, GLC-SX-MMD, GLC-LH-SMD, GLC-FE-100FX, GLC-T and GLC-TE. GLC-T and GLC-TE are copper SFPs. A cable with neither end a switch is refused.

### Link checks

| Check | Rule |
|---|---|
| The link | both ends run; a port shut down is a note and the link stays down; neither end is a supervisor uplink that is off |
| Transceiver ports | a transceiver port has an optic it runs and no other `speed`: SFP28 25G, 10G, 1G; SFP+ 10G, 1G; SFP 1G, and 100M on the 9400 SFP card, a 2960-X's SFP uplinks and an IE 3010's FastEthernet SFP ports |
| RJ-45 ports | an RJ-45 port takes no optic, or a copper SFP at the far end |
| Dual-purpose uplinks | a dual-purpose uplink (2960, 2960-Plus, 2960-C, IE 3000, IE 3010) takes copper or an optic as its `media-type` allows |
| QSFP ports | a QSFP port takes a transceiver vswitch does not model, and says so |
| Cabled twice | a port named by two cables is a finding |
| DTP | trunk mode trunks; dynamic auto (the default) trunks when the far end is a trunk or dynamic desirable that negotiates; dynamic desirable also trunks with dynamic auto |
| No DTP sent | `switchport nonegotiate`, a routed port and a device send no DTP |
| Ports that never trunk | `dot1q-tunnel` and `private-vlan` host and promiscuous ports; a `private-vlan trunk` does trunk; a member runs its port-channel's switchport lines |
| Trunk at one end | a finding; a trunk facing a tunnel port is not (QinQ) |
| Channel | `on` with `on`, LACP with one end active, PAgP with one end desirable; `on` bundles alone |
| Channel findings | a channel that does not form, a channel at one end only, a bundle whose members reach two bundles; a PAgP member facing a device is a note, as the device is taken to run no PAgP |
| Trunks | native VLANs match (`native vlan tag` names none); the VLANs either switch defines that one end allows and the other does not are a note |
| Access ports | the same access VLAN at both ends |
| Routed ports | both routed or neither; a routed port facing a switch port is a finding; the two addresses in one subnet and not the same address; a routed end with no address is a note |
| Spanning tree | BPDU guard on a port facing a switch err-disables it, set on the port or its bundle, or by PortFast with `spanning-tree portfast bpduguard default`, unless `spanning-tree bpduguard disable` is set |
| BPDU filter | a link between switches that filters BPDUs at both ends and closes a loop is a finding, as neither end blocks |
| PortFast | PortFast on a port facing a switch is a note; plain PortFast acts only while the port does not trunk, and `spanning-tree portfast default` sets it on access ports |
| MTU | a note when the MTUs a link runs differ (`system mtu jumbo` on a classic Gigabit link) |
| VTP | a note when both switches are VTP servers or clients over a link that trunks at both ends, as one domain's highest revision then sets both VLAN databases |
| Two ports of one switch | make no pair for the MTU and VTP notes |

### A linked port in the show commands

| Command | Shows |
|---|---|
| `show interfaces status` | `connected`, `trunk` in the Vlan column while the port trunks, the transceiver's speed and type |
| `show vlan brief` | drops a port while it trunks |
| `show etherchannel summary` | flags a bundled member `(P)` and its bundle in use `(SU)` |
| `show interfaces trunk` | lists the trunking port or bundle, and the VLANs spanning tree forwards it in |
| `show spanning-tree` | the port in each VLAN it carries, with its role and state across the lab |
| `show ip interface brief` | brings the port up, with a VLAN interface whose VLAN a running port carries |

Captured cores, cabled as they ran, print every row of `show interfaces status` their configuration decides and both `show vlan brief` tables as the switches printed them.

## Topology

`vswitch topology <captures dir> [--out <lab.toml>]` writes the plant as its captures show it, to `plant-lab.toml` unless `--out` names a file. `vswitch lab plant-lab.toml` then brings every cable up and checks it from both ends.

| Input | Source |
|---|---|
| Captures | `<book>-<train>/<host>-running-config.txt` with `<host>-show.log` beside it |
| Switch to use | the newest capture of a host captured at several releases; a line names each capture left out |
| Hardware | the session's `show inventory`, else `show version`; a `<host>-stack.txt` beside the capture names the members and takes their place when present |
| Ports read | front-panel ports whose `interface` block holds a description, with `media-type` and the state `show interfaces status` gives |
| Transceiver | a known part from either end's `show inventory`, else the Type its status table prints, else an unknown inventory part |

The file holds each captured switch with a cable, at its newest capture's train and hardware.

### Cable pairing

| Rule | Detail |
|---|---|
| A cable | a connected port's description names another captured switch, and that switch has a port whose description names this one |
| Naming | a whole host name in any case, or a word of three or more characters that ends another host's name where this host's name begins with the rest of it |
| Example | on `site1-dist1`, a description `access1 (rack 2)` names `site1-access1` by its site prefix; `site1-access1` names it whole |
| Whole name first | a port naming the far switch by its site suffix alone gives way to one naming it whole where one end names more |
| Port order | ports pair in port order |
| Down ports | a port that is not connected at either end is no cable |
| Unanswered | each description the far switch does not answer is listed, with the down port that answers it where there is one |
| Unequal counts | a switch with more ports naming the other than the other names back is listed with the number matched |
| Transceiver unnamed | each cable on a transceiver port whose transceiver no capture names is listed and marked in the lab file, where the cable takes an optic by hand |
| Differing parts | two ends holding different parts are listed with the part the lab runs |

The command prints the captures read, the switches cabled and the cables written.

## Design files

`tests/example-design.toml` is a top-of-rack stack: two C9300X-24Y with C9300X-NM-4C, LACP port-channels to five Proxmox nodes (bond0, bond1), the backup server, the core trunk and the 100G link to MDF. Host-facing port-channels take `spanning-tree portfast trunk` and `spanning-tree bpduguard enable`.

`ssh_only` sets AAA new-model with local login and local exec authorization, SSH version 2, a 2048-bit RSA key and the HTTP and HTTPS servers off. It needs `domain`, which names the key. The generated file holds no passwords. Its comment lines, after the design's `notes`, say how to create the local account at the console, and the change script carries the same step first.

| Design key | Meaning |
|---|---|
| `hostname`, `stack`, `release`, `provision` | the switch; `stack` lists members from 1 (`C9300X-24Y+C9300X-NM-4C`); `release`, when absent, is chosen as [Hardware](hardware.md) Choosing the release says; `provision` writes `switch N provision` lines, one per stack member of a model that stacks, and is on by default |
| `notes` | comment lines at the top of the configuration |
| `vtp_mode` | `transparent`, `off`, `server` or `client`, written before the VLANs |
| `existing_vlans` | VLANs the switch already has, which trunks and access ports may name without defining |
| `system_mtu`, `port_channel_load_balance`, `spanning_tree` | global lines; `spanning_tree` takes `mode`, `priority` and `vlans` (1-4094 when absent) |
| `domain`, `name_servers`, `ntp_servers`, `syslog_hosts` | naming and services |
| `management` | `address`, `mask`, `gateway` on GigabitEthernet0/0 in `Mgmt-vrf` |
| `native_vlan` | the native VLAN of every trunk |
| `vlan` | VLANs with `id`, optional `name`, optional `verify` |
| `port_channel` | `id`, `allowed` (`459`, `450,456` or `except 451-453,459`), `members`, `description`, `edge`, `lacp_fast`, `existing`, `connects`, `verify` |
| `access` | `port`, `vlan`, `description`, `edge`, `verify`, `shutdown` |
| `shutdown` | ports administratively down |
| `change` | the change script around the configuration |

A `verify` value is written as a `! verify:` comment above its entry, as a sample to confirm before applying. A `port_channel` with `existing` writes only its description and allowed list. An `edge` access port takes `spanning-tree portfast` and `spanning-tree bpduguard enable`. `lacp_fast` writes `lacp rate fast` on the members.

The `change` table takes `title`, `steps` (steps that are not configuration lines), `before` and `after` (`<command>   (<what it shows>)`), `rollback` (configuration lines that undo the change) and `rollback_note`.

### Design checks

`vswitch generate <design.toml>` refuses a design, prints each problem as `design: <problem>` and exits 1 for any of these.

| Check | Rule |
|---|---|
| VLAN ranges | `existing_vlans` and each allowed list name VLANs inside 1-4094 |
| SSH | `ssh_only` has a `domain` |
| Access ports | each names a defined VLAN, is used once, and is no port-channel member |
| Native VLAN | defined |
| VTP | a new switch (`provision`) that defines VLANs sets `vtp_mode`, as it is a VTP server by default |
| Port-channels | written once, with members unless `existing`; an `existing` one has no members, `edge` or `lacp_fast`; a trunk list names defined VLANs (an `except` list names none) |
| Members | a port is in one port-channel; ports compare by the name IOS gives them, so `Twe1/0/1` and `TwentyFiveGigE 1/0/1` are one port |
| Shutdown | a shut-down port is no member and no access port |
| Hardware | each port the design uses runs on the stack; a supervisor QSFP port the design uses is enabled |

### Generated output

| Step | Detail |
|---|---|
| Typing | the configuration is typed into the switch at the design's release; a refused line is printed with the switch's error and nothing is written |
| `Port-channelN carries` | printed for each port-channel, with `except` lists resolved against the defined VLANs |
| Change checks | each `before` and `after` command must be one the switch takes, and the rollback is typed after the change and must be accepted |
| `--out <file>` | the configuration; the standard output when absent |
| `--change <file>` | the change script: title, port table, notes, steps, the paste block, checks after and rollback; needs a `change` table |
| `--running <file> --target <file>` | the target configuration: the running configuration with the change applied; the rollback must lead back to the captured configuration line for line |
| `--target <file>` alone | the new switch's configuration after the change, in IOS section order, without the default lines IOS prints |
| `--show-run` | prints the switch's running configuration after the change |
