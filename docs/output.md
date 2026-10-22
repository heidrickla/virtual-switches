# Output

The shell prints what IOS prints for refusals, help, `show` commands and the save dialogue. Each output rule carries its source in `src/evidence.rs`: a real switch's output (a capture), a Cisco example, or vswitch's inference. What a switch printed overrides the rest. `vswitch verify --outputs` lists the rules no switch has shown, the list to check on hardware.

Show output is `src/show.rs`, startup-config `src/startup.rs`, output filters `src/filter.rs`, help and carets `src/shell.rs` and `src/console.rs`.

## Error messages and carets

Messages come from the command reference's CLI chapter, with ASCII quotes (the chapter's typography prints `‘^’`).

| Message | When |
|---|---|
| `% Invalid input detected at '^' marker.` | a word or value IOS refuses |
| `% Incomplete command.` | the line stops where more is required; on IOS XE a port named last by the start of its number (`interface gigabitethernet1/1`, the network module holding Gi1/1/1), or by that start on a member the stack lacks, up to the family's member count (`interface TenGigabitEthernet 4/1` on one C9300-24T; `GigabitEthernet 6/2` and `gigabitethernet 0/1` are refused) |
| `% Ambiguous command:  "<text>"` | a start that two or more keywords begin |
| `% Unrecognized command` | `?` with nothing that can follow |

The caret stands under the refused word of the line as typed, spaces and `do` included.

| Refusal | Caret |
|---|---|
| a word no keyword there begins | past the longest start of the word that a keyword there begins with: `show config` at `>` on a 2960-X is refused under the `f`, as `controllers` begins `con`; on a 9300 under the second `o` and on a 9200 under the `n`, whose keywords at `>` differ |
| an argument could stand there | at the start of the word |
| a number past the range offered | under the digit that takes it past the top (`terminal length 600` under the last `0`) |
| a number below the range, or a word where only a number stands | at the start of the word |
| an interface the stack does not have | classic IOS: under the start of its number (`GigabitEthernet9/0/1` under the `9`). IOS XE: past the start the word shares with another row of `?`, the type it names not counted (`interface gigabitethernet4/0/1` on a one-member C9300-24T under the `g`, and under the `i` where `GMPLS` is listed); a number going on past a port the stack has, under the `/` after that port (`GigabitEthernet1/0/13/0`), and one departing from the management port's number, past their shared start (`gigabitethernet 0/1` under the `1`), as the lab C9200L-48T-4G (17.6.4) put them; a module no member's ports of the type carry, under it (`gigabitethernet 6/2` and `2/3/0` under the `2` and the `3`, the lab C9300-24T and C9200L-48T-4G); on a C9200, a member past the stack's most, under it (`gigabitethernet 9/0/6` under the `9`). A port channel, VLAN, loopback or tunnel typed with its number, under the digit that takes it past the top (`Po10` on a 2960G under the `0`) |

The place of the caret in a word comes from a 2960-X session, checked against the words its own `?` offered where a capture has them.

- The keywords are the rows of the `?` answer a real switch gave there, else the books'. A heading (`Configure commands:`, `VLAN configuration commands:`) offers none: `configure terminal` in global configuration is refused under the `e` that `configuration` leaves.
- Among two or more interface types, a word in capitals is a type (`PROTECTION_GROUP`, `BDI`), and no argument stands there.
- A word running past a listed type's whole name into no type's is refused past the name, though a host name could stand there: `show ip ospf neighbor Tunnel-tp` at a level whose `?` lists `Tunnel` alone, under the `-`.

`vlan <n>` past 4094 answers two lines with no caret: `Command rejected: Bad VLAN list - character #<k> (EOL) delimits a VLAN` (k the position after the number) and `number which is out of the range 1..4094.`.

### Messages the configuration gives

On IOS XE a line the configuration so far does not let the switch take prints the lab C9300-24T's (17.6.4) message and changes nothing. `no` lines are taken, a service policy's removal and `no switchport` under 802.1X port control excepted. Classic IOS prints none of these.

| Line | State | Message |
|---|---|---|
| `router bgp <asn>` | another AS running | `BGP is already running; AS is <asn>` |
| `address-family ... autonomous-system <n>` in a named EIGRP instance | the instance's family (address family, unicast or multicast, VRF) holds another AS | `Changing from AS(<old>) to AS(<n>) is not allowed` |
| the same | another named instance's family runs AS `<n>` | `%ERROR: AS(<n>) in use by VR(<name>)` |
| `ip\|ipv6 flow monitor <m> ...` on a port, on the C9200 at 17.6 and 17.15 (the lab C9200L-48T-4G at 17.6.4 and 17.15.6) | the monitor holds no record | `% Flow Monitor: Failed to add monitor to interface: The monitor does not have a valid record` |
| `record <r>` in a flow monitor, on the C9200 at 17.6 and 17.15 | the monitor holds another record | `% Flow Monitor: Failed to set record: Already there is an existing record configured for this monitor` |
| `no switchport`, `switchport mode dot1q-tunnel\|private-vlan promiscuous` on an interface | `authentication port-control` on the port (the lab C9200L-48T-4G, 17.6.4 and 17.15.6) | `Command rejected: Conflict with Authentication. ` and `Authentication must be disabled before changing port mode ` |
| `ip admission <name>` on an interface, on the C9200 at 17.6 and 17.15 (the lab C9200L-48T-4G at 17.6.4 and 17.15.6) | no `ip admission name <name>` | `%Auth-proxy name <name> is not defined` and a blank line |
| `class <name>` in a policy map | no class map of the name | `class map <name> not configured` |
| `service-policy input\|output <name>` on an interface | no policy map of the name | `% policy map <name> not configured` |
| `service-export mdns-sd controller <name>` | another controller | `Only one Controller is allowed` and a blank line |
| a line naming a VRF (`vrf forwarding`, `ip multicast-routing vrf`, `ip mroute vrf`, `ntp server vrf`, `snmp-server host ... vrf`, a BGP or OSPFv3 address family; in EXEC `show ip route\|arp vrf`, `show ip vrf detail\|interfaces`, `show ip bgp vpnv4 vrf`, `show mpls forwarding-table vrf`, `show mpls static binding ipv4 vrf`) | no `vrf definition` or `ip vrf` of the name; `Mgmt-vrf` is in the factory default | the command family's own (`% VRF V1 not configured`, `% VPN Routing instance V1 does not exist.`) |
| `router ospf <n> vrf <name>` | no `vrf definition` or `ip vrf` of the name | `Unknown VRF specified` |
| `cts sxp filter-group listener\|speaker global <name>` | no `cts sxp filter-list <name>` | `Filter-list <name> not configured` |
| `route-target` in a VRF | no `rd` | `% VRF <name> does not have an RD configured, please configure the RD before configuring route-target` |
| `<feature> attach-policy <name>`: IPv6 source guard and destination guard on an interface; device tracking, DHCPv6 guard, RA guard in a VLAN configuration | no `<feature> policy <name>` | `% Policy <name> not defined for the feature` and a blank line |
| `ipv6 mld ssm-map static <acl> <source>`, in global configuration or an interface, which hands it there | no `ipv6 mld ssm-map enable` | `%ssm-map is not enabled!`, the mode global configuration |
| `ip nat inside\|outside source list ... pool <name>` | no `ip nat pool <name>` | `%Pool <name> not configured ` and a blank line |
| `ntp access-group ... <acl>` | no access list of the name | `%Access-list <n> is not defined`; a named list `%Access-list <name> not defined` |
| `cts role-based permissions ... <acl> ...` | no role-based list of the name | `% ACL <acl> is not defined.` for each |
| `crypto pki certificate chain <tp>` | no trustpoint | `% CA trustpoint for cert chain not known.` and a blank line |
| `crypto pki enroll <tp>`, `crypto pki import <tp> certificate` | no trustpoint | `% CA server trustpoint '<tp>' is not known.` and a blank line |
| `local-domain <name>` in the Umbrella profile | no `parameter-map type regex <name>` | `% Parameter-map regex <name> does not exist` |
| `member vfi <name>` in a VLAN configuration | no `vpn id` in the VFI | `% VFI <name> must be configured with a valid vpn id first.` |
| `service ipv4\|ipv6` in a LISP instance | `service ethernet` there | `% Instance ID <id> has 'service ethernet' enabled.` and ` L2 and L3 services are exclusive, remove L2 service first.` |
| `ipv6 router ospf <n>` | `router ospfv3 <n>` | `% OSPFv3: Use "router ospfv3 <n>" to configure this process` and `% Failed to create routing protocol ospf` |
| `neighbor <peer> inherit peer-session <t>` | no `template peer-session <t>` | `%Template not found. Please create a template first.` |
| a neighbor's policy in `router bgp` | the process has an address family | `% Policy commands not allowed without an address family` |
| a neighbor's policy or `activate` in an address family, any neighbor line but `remote-as`, `peer-group` and `inherit` in `router bgp` | no `remote-as`, `peer-group` or `inherit peer-session` for the peer | `% Specify remote-as or peer-group commands first` |
| `neighbor <peer> remote-as <asn>` in `router bgp` | an IPv6 link-local peer | `% Link local peer outside address-family scope` |
| `neighbor <peer> remote-as <asn>` in `router bgp` | an interface's own address | `% Cannot configure the local system as neighbor` |
| a private VLAN association or mapping | a VLAN the switch lacks | `%Command rejected: invalid private vlan association between vlan<p> and vlan<s>. VLAN <n> data is not available.` |
| `private-vlan isolated\|community` | the VLAN's SVI up | `%Command rejected: invalid private vlan type assignment. VLAN <n>'s interface is not administratively down and cannot be set to secondary VLAN.` |
| `interface Tunnel<n>` | a tunnel PIM registers through: one per RP and one more where the switch is its own RP, numbered from the lowest free | `%Tunnel<n> used by PIM for Registering, configuration not allowed` |
| `device classifier condition\|device-type` | `device classifier` | `% Device classifier is enabled.Please disable it and execute the command again.` (`device-type`: a space after the period, then a blank line) |
| `ip address <a> <m>` on an interface | another interface in the VRF holds an overlapping subnet | `% <network> overlaps with <interface>`, the lowest such subnet's |
| `monitor session <n> source interface <port>` | the port is a session's destination | `% Interface(s) <port> already configured as monitor destinations` |
| `monitor session <n> destination interface <port>` | the session has a remote destination VLAN | `% Cannot add ports as destination for session <n> - a RSPAN Source session` |
| `monitor session <n> destination interface <port>` | the port is a session's source | `% Interface(s) <port> already configured as monitor sources` |
| `monitor session <n> destination interface <port>` | the port is another session's destination | `% Interface(s) <port> already configured as monitor destinations in other monitor sessions`, after `% Warning: specified default VLAN (<v>) for ingress on dest port does not exist.` where an ingress VLAN is missing |
| `monitor session <n> destination remote vlan <v>` | the session has a destination port | `% Session - <n> is a not a Remote Source session. Can't add RSPAN VLAN as destination` |
| `monitor session <n> destination remote vlan <v>` | `<v>` is the session's filter VLAN | `% Configuring Destination RSPAN VLANs <v>, but the same vlan is ` and `% already configured as filter vlan` |
| `monitor session <n> source remote vlan <v>` | the session has a remote destination VLAN | `% RSPAN VLAN already exists as destination. The same session cannnot have RSPAN VLAN both as source and destination` |
| `monitor session <n> source remote vlan <v>` | the session has a source port or VLAN | `% Cannot add RSPAN VLAN as source for SPAN session <n> as it is not a RSPAN Destination session` |
| `monitor session <n> filter ip\|mac access-group <acl>` | the session filters VLANs | `% vlan filter can not be applied with FSPAN` |
| `standby <group> ip` | the group has an IPv4 address | `% Failed. Unconfigure existing primary vIP address configured for the group.` |
| `standby <group> ipv6 ...` | the group has an IPv4 address | `% Group configured for different address family` |
| `spanning-tree backbonefast` | BackboneFast set | `% Backbonefast already enabled` and a blank line |
| `spanning-tree port-priority <n>` in a template | `<n>` not a multiple of 16 | `% Port Priority in increments of 16 is required` and a blank line |
| `match ip address\|route-source <acl> ...` in a route map | the sequence matches a prefix list of the kind | `% prefix-list and access-list can not co-exist in one route-map sequence.` for each list |
| `ipv6 access-list fqdn <name>` | `ip access-list fqdn <name>` | `% A(n) IP FQDN access list with this name already exists` |
| `nsf cisco` in `router isis` | no `net` | `%ISIS not running (configure NET)` |
| `nsf ietf` in `router ospf` | `nsf cisco` | `% OSPF: Cannot enable both Cisco NSF and IETF Graceful Restart` |
| `mdt default <group>` in a VRF address family | another default group | `% <group> has already been configured for vrf <vrf>`, naming the group set |
| `mdt log-reuse`, `mdt data threshold` in a VRF address family | no `mdt data <group>` | `% First configure the data MDT group!`; `% MDT data must be configured first` |
| `snmp-server engineID local <id>` | `<id>` under 10 characters | `%Engine ID must be 10 hexadecimal characters or more` |
| `duplex half` on an interface | no `speed` line limited to 10 and 100 | a blank line, `% Duplex cannot be set to half when speed autonegotiation subset contains 1Gbps,2.5Gbps,5Gbps or 10Gbps`, a blank line |
| `switchport nonegotiate` | no `switchport mode access\|trunk` | `Command rejected: Conflict between 'nonegotiate' and 'dynamic' status on this interface: <port>` |
| `network-policy <n>` on an interface | `switchport mode trunk` | `Command rejected: Network Policy is not supported on a trunk port` |
| `cts manual` | `dot1x pae authenticator` | `Command rejected (<port>): conflict with Dot1x Auth` and a blank line |
| `umbrella in <tag>` | `umbrella out` | `% 'umbrella out' already configured on interface` |
| `ipv6 address dhcp ...` on an interface | `ipv6 dhcp server` there | `% Interface is in DHCP server mode` |
| `ntp trusted-key <n>` | the key trusted | `%NTP trusted-key is invalid : key already configured ` |
| `track <n> ...` | track `<n>` of another kind (interface, `ip route`, `ip sla`, list) | `% Cannot change tracked object <n> - delete old config first` |
| `offset-list <n> in\|out ...` in a router | `<n>` not a standard list number (1-99, 1300-1999) | `% Invalid access list name.` |
| `ip msdp redistribute list <n>` | `<n>` not an extended list number (100-199, 2000-2699) | `% Invalid access list name.` |
| `rep stcn ...` on an interface | no `switchport mode trunk` | `Command rejected: conflict with the current Switchport Mode` and a blank line |
| `show ip ospf interface ... topology <name>` | no OSPF process with `topology <name>` | `%OSPF: Specified topology not found for any OSPF process` |
| `show ip bgp neighbors <peer> <keyword> ...` | no `neighbor <peer>` in `router bgp` | `% No such neighbor or address family` |
| `show ip wccp ...` | no `ip wccp` line | `% WCCP version 2 is not enabled` |
| `show spanning-tree mst <instance> ...` | spanning-tree mode not MST | `% Switch is not in mst mode` and a blank line |

These lines are taken with a warning first:

| Line | State | Warning |
|---|---|---|
| `mka policy <name>` | the policy exists | a blank line, `% MKA Policy "<name>" may have associated active MKA Sessions.`, a blank line, `  Changes to MKA Policy "<name>" values`, `  will cause all associated active MKA Sessions to be cleared.`, a blank line |
| `policy-map type control subscriber <name>` | the policy map exists | `%Warning, modifying policy-map "<name>" may impact sessions hosted on ports for which "<name>" has already been applied.` |
| `show spanning-tree mst configuration [digest]` | spanning-tree mode not MST | `% Switch is not in mst mode` |
| `standby [<group>] ip <address>` | the interface's subnets do not hold the address | `% Warning: address is not within a subnet on this interface` |
| `auto-cost reference-bandwidth <n>` in `router ospf` | the value changes | `% OSPF: Reference bandwidth is changed. ` and `        Please ensure reference bandwidth is consistent across all routers.` |
| `macsec-cipher-suite ...` in an MKA policy | the suite changes | a blank line, `% If cipher currently being used is not configured, new SAK will be generated`, a blank line |
| `ipv6 mld static-group ...` on an interface | no `ipv6 multicast-routing` | `%Warning: Enable MLD router-side for this command to be effective` |
| `ipv6 dhcp server automatic` on an interface | | `% Warning: Pool  not configured globally - configuring anyway.` |
| `ipv6 dhcp server <name>` on an interface | no `ipv6 dhcp pool <name>` | `% Warning: Pool <name> not configured globally - configuring anyway.` |
| `auto qos global compact` | | `%Warning: This command will reset all autoqos generated configurations` |
| `location civic-location identifier <id>` | the location holds no attributes | `% 250 bytes available to configure civic address attributes`, then `(config-civic)#` |
| `cts role-based sgt-map 0.0.0.0/0 ...` | no `ip route 0.0.0.0 0.0.0.0` | `%Please ensure default route is created using ip route 0.0.0.0 command` |
| `spanning-tree portfast [trunk]` | PortFast changes | `%Warning: portfast should only be enabled on ports connected to a single`, ` host. Connecting hubs, concentrators, switches, bridges, etc... to this`, ` interface  when portfast is enabled, can cause temporary bridging loops.`, ` Use with CAUTION`, a blank line; without `trunk` then `%Portfast has been configured on <port> but will only` and ` have effect when the interface is in a non-trunking mode.` |
| `authentication host-mode\|port-control\|control-direction\|open` on an interface, `authentication mac-move` | the configuration is new-style: it holds a line of `shell::NEW_STYLE` (`policy-map type control subscriber`, `access-session ...`, `autoconf enable`); the lab C9200L-48T-4G (17.6.4), legacy, took them silently | `%Command deprecated (<line>) - use access-session instead` and a blank line |

## Help and Tab

`?` prints what a switch of the platform and release printed for the same line in the same mode where a capture holds it, whole. An error answer (`% Unrecognized command`) belongs to that switch's configuration and is not kept as the release's help. Otherwise the layout and descriptions below apply.

- Layout: two-space indent, a padded column, `<cr>` last, from the guides' and the general references' help examples.
- Description: the first sentence of the command reference's Syntax Description row for that keyword or argument, its asides in parentheses dropped. For a command's first word it is the reference's introduction ("To ..., use the").
- A keyword the books leave bare takes the description a switch of the same family printed beside it after the same line in the same mode (EXEC and privileged EXEC together) in a session log, else one of the other family's (the 9300's `show ?` at 17.15 takes the 9200s').
- `% Unrecognized command` when nothing follows.
- Show lines list `|  Output modifiers` where the line may end.
- IOS XE EXEC prints `<cr>  <cr>`, configuration modes a bare `<cr>`, as the captures print them.
- Classic IOS heads the EXEC list with `Exec commands:`.
- A description wraps under its column at the console's width. With piped input the help is fitted to `terminal width` when it was set to a number.
- Right after a word, `?` prints the keywords the word begins, two spaces apart, wrapped before the console's last column.
- After `do` or `do-exec` (each by any unique start) `?` and Tab answer as the privileged prompt does. `?` right after them prints the crawl's `do-exec ?` for the mode where one is held, which leaves out the commands that change mode.
- Tab completes the one keyword or interface type a word begins, whole keywords included (`ip` beside `ipv6` completes nothing), and nothing after a held refusal.

## Privilege levels

`privilege exec level <n> <command>` takes a level from 0 to 15. The command may be several words (`privilege exec level 14 show version`).

- Each shorter start of the command is raised to the same level (`privilege exec level 14 show`), a start raised before included.
- A command raised above 1 is refused at `>` with the caret under the first word.
- At `>`, `?` after the words of a raised command, or after a word begun past them (`show v?` once `show` is raised), answers `% Unrecognized command`, and Tab completes nothing there.
- At `>`, `?` and Tab leave a raised command out of every list, at any word and held answers included: with `show` at level 1 by a line of its own and `show version` raised, `show ?` lists `vlan` and leaves out `version`.
- The lines print in IOS's parse-tree order: a command's longer forms before it, siblings in the order switches printed them (`rcommand`, `connect`, `telnet`, ... `terminal`, `show`, `clear`; `show ip` before `show version`), others as typed.
- In the running configuration a start a line of its own sets keeps that line's level. `privilege` lines print after `control-plane`.

## Terminal settings

`terminal length <0-512>` and `terminal width <0-512>` are taken at `>` and `#`. The general command reference has them, no platform book.

| Setting | Effect |
|---|---|
| `terminal length` | the rows a page holds at the console's `--More--` prompt, 0 for no paging |
| `terminal width` | the width `show vlan brief` wraps to and `?` help fits to, 0 for none; 80 when output is not a terminal |

- `terminal` is read by any start no other command at the prompt has (`ter len 30`), and the keyword after it by any start of two letters or more.
- A value out of range is refused under its last digit, `terminal length` alone answers `% Incomplete command.`.
- `terminal ?` adds `length` and `width`, and `terminal length ?` prints `<0-512>  Number of lines on screen (0 for no pausing)`.

## Configure banner and saving

- `configure terminal` prints `Enter configuration commands, one per line.  End with CNTL/Z.`, from the guides' example output.
- `write`, `write memory` and `copy running-config startup-config` save at `#` on every platform, each word by any unique start (`copy ru st`).
- `write memory` prints `Building configuration...` and `[OK]`.
- `copy running-config startup-config` asks `Destination filename [startup-config]?` first (IOS XE command reference example).
- A VTP server's VLANs go to the saved text ahead of the configuration, as vlan.dat keeps them outside it.

## show running-config

The output begins `Building configuration...`, a blank line and `Current configuration : <n> bytes`, and ends `end` and a blank line, the interface form's too. n counts each line from the first `!` through `end` with its newline, plus one; an empty line counts nothing.

IOS XE 17.6 prints `! Last configuration change at 12:34:56 UTC Thu Mar 5 2026` and a `!` line after the first `!`, counted in n: the time the switch booted, a replayed configuration's own line, else the last line typed, on the switch's clock. A replayed line's ` by <user>` prints until a line is typed. Other releases print no such line.

`show running-config interface <name>` prints that block alone: a `!` line, the block, `end`. A port never configured prints as an empty block, and the byte count is that of the lines printed, plus one. An interface the switch has not made (Bluetooth0/4 before it is typed) prints `end` alone.

### Contents

- The `version` line, then every interface, one never configured as an empty block.
- The VRF definitions after the hostname.
- Routes, SSH, logging and SNMP after the interfaces, then `control-plane`, the lines and NTP.
- `line con 0`, and `exit-address-family` closing its section however the section is left, a block's own lines before its sections.
- A banner prints with `^C` as its delimiter, nothing kept past the closing one.
- An interface is one block however its name is spaced (`interface vlan 20` is `interface Vlan20`).
- A typed `version` line prints once, as the release the switch runs.
- A new switch's default lines (`vrf definition Mgmt-vrf`, GigabitEthernet0/0's, `service timestamps`) print once typed.
- A member provisioned with no switch there (`switch 2 provision c9300-48p`) prints its ports and takes their lines.
- `monitor session` lines name their ports short (`Gi1/0/11`).
- At IOS XE 17.6 an access list's `deny` pads to `permit`'s width (` 20 deny   any log`, ` 30 deny   ip any any log`); 17.15 pads none.
- An IPv6 access list entry typed with no number prints `sequence <n>` before it, n 10 past the highest.
- On IOS XE a class map's DSCP and precedence values print each followed by a space, a space apart (`match dscp cs4  cs5  ef `), and a CoS value with a space either side (`match cos  5 `).
- A section closes at its `exit-` line: inside an interface at its lines' depth (`  exit-vrrp`), elsewhere at its header's. Sections nest (`topology base` under an EIGRP named-mode family).
- A router's own lines print before its address and service families, whenever typed; its other sections (`bmp server`, templates) keep their place.

### Layout of a replayed configuration

A configuration a switch printed (one with a `version` line) keeps its layout when replayed, as the lab C9300 at 17.6.4 printed it after a configuration replace and after factory boots.

- Two of its top-level lines that meet again keep the `!` and comment lines printed between them, a `! ` with its space included.
- An indented `!` before a line in a block stays before that line.
- Each line keeps its own spacing and case (certificate hex, `match cos  5 `, `mdns-sd service-list <name> IN`); two lines differing only in case each print as printed.
- Its top-level lines print in its order; a line added since follows the line computed before it and every one of its lines whose command IOS prints before its own (the lab C2960G at 15.0(2)SE11: a new `spanning-tree mst configuration` after `spanning-tree extend system-id`).
- Its blank lines and the comment lines inside its blocks stay where it printed them, and a `no` line it printed is kept (` no protocol udp`).
- Lines added between two of its own take the printed `!` lines before the first added line, all of them less those computed after it where those are fewer; the rest take their computed ones.
- At 17.6 a static route follows the `ip http` lines with no `!`, an access list follows the route after one, and `control-plane` follows the access list after three, the block's own closing `!` counted.

### Settings and instances

- A later value of a setting replaces the earlier (`duplex`, `vtp mode`).
- A command that takes several instances keeps one line per instance, named by the arguments that name it (`username`, `ntp server`, `ip route`, `neighbor`, `network`, `monitor session` by its session), and `no` naming an instance removes it whatever it sets.
- A default turned back on prints nothing, while an interface keeps `no cdp enable`, `no lldp transmit` and `receive`, `no keepalive`, `no ip redirects`, `no ip proxy-arp` and `no ip unreachables`.
- `no switchport` drops a port's switchport lines and `switchport` its addresses.
- A member's `channel-group` makes its port-channel with the member's switch port lines.

### The no lines IOS keeps

- `no aaa new-model`, `no ip http server`, `no ip http secure-server`, `no logging console`, `no ip forward-protocol nd`.
- `no ip address` on a routed interface, `no vtp` on a port, and `no switchport` on a routed port.
- `no neighbor <x> activate` under `router bgp`'s IPv4 family, unless `no bgp default ipv4-unicast`, and `no protocol <p>` in a device-tracking policy, which gleans from every protocol by default (the lab C9300-24T at 17.6.4).
- IOS XE keeps `no port-channel standalone-disable` under a port-channel.

These come from captured Catalyst 9400 running configurations at 17.5, and the lab C9300-24T at 17.6.4 for `no switchport`.

### Lines real switches printed

- A line a real switch printed prints back as printed with its `no` form kept.
- A first word the books read as an abbreviation of another command is taken as held (`service call-home` where a book has `service-template` alone).
- A line taken from a capture is placed after the line typed before it, else after the line before it in the first capture that held it.

### Spanning-tree lines

- `spanning-tree vlan` lines print in VLAN order.
- A VLAN's priority, forward-time, hello-time and max-age are kept per VLAN. VLANs holding one value print on one line (`spanning-tree vlan 1,10 priority 4096`); `no spanning-tree vlan 10 priority` then leaves `spanning-tree vlan 1 priority 4096`.
- `spanning-tree vlan <list> root primary` is kept as `priority 24576` where that makes the switch the root, else as the highest multiple of 4096 that beats the current root; `root secondary` as `priority 28672`. VLANs taking one priority share a line.
- `spanning-tree mst configuration` enters its submode, and `instance <n> vlan <list>` prints with `vlan`.
- Classic IOS's first `no spanning-tree mst configuration` returns the region to its defaults and keeps the header line; the second removes it. IOS XE removes it at the first.
- A member's `channel-group <n>` gives a new or empty port-channel interface the member's switch port lines.

### VLAN and VTP lines

- VLANs and VTP lines print only in VTP transparent or off mode. A VTP server or client keeps them in vlan.dat.
- The VLANs with nothing under them print as runs split at each VLAN with lines, each list compressed and ending with a space (`vlan 100-119 `, `vlan 120` with its name, `vlan 121-139 `). A VLAN with lines prints its own block, one given lines after a range named it included, the blocks in order of their first VLAN.
- `state suspend` prints under its VLAN; `state active`, the default, prints nothing.
- The VTP lines print together: `vtp domain`, `vtp mode`, then `vtp version` and `vtp password`.
- `vtp version` prints whatever the VTP mode, but for mode off, which prints none (17.6.4, versions 1 and 3).
- `vlan access-map` blocks print whatever the VTP mode. A `vlan configuration` with nothing under it is not kept.

### Section order

Lines and sections print in the order the captures print them: top-level lines by book and release and by family, block lines from every capture. A pair printed both ways is no rule, and a command no capture printed stays after the line before it.

- `logging buffered` and the other logging lines but host, origin-id, trap, source-interface and facility come at the top.
- The VLAN blocks, the `cdp` lines, then class maps before policy maps.
- Loopbacks come first among the interfaces, and `Null0` ahead of them when it has lines (it prints nothing when empty). Then the port-channels, the tunnels, the management port, the ports with each subinterface after its port, Bluetooth0/4 and the VLAN interfaces (the lab C9300-24T at 17.6.4).
- `no ip ssh` and the access lists follow the interfaces, the lists before `snmp-server`.
- Routing processes follow the interfaces, before `ip forward-protocol` and the routes.
- Line blocks print `con`, `aux`, then each `vty` range in order.
- A banner typed in a submode is taken globally.
- A line typed less indented than its section closes it. Each class of a policy map keeps its own lines (`priority level`).
- A modular C9200's slot 1 lists every module's ports whatever is fitted.
- `router` blocks print after the last interface (measured at 17.6, inferred elsewhere).
- At IOS XE 17.6, `vlan filter` prints after the `vlan access-map` blocks and before the VLANs; elsewhere among the global lines before them (inferred).
- At IOS XE 17.6, `ip route` follows the `ip http` lines, and `ip access-list` blocks follow `ip route`, before `control-plane`.
- IOS XE writes the standard access lists, then the extended, each by name, whatever order they were typed in.

### Blank and comment lines

- Between two top-level lines the output prints as many `!` lines as that release's captures printed most often between the same two commands, then the family's, else as many as the typed text built.
- A line a switch printed keeps its own spacing (`snmp-server host ... <removed>  snmp`).
- On IOS XE the two `! If contact email address ...` comment lines print above `contact-email-addr sch-smart-licensing@cisco.com` in `call-home`. Typing that address prints `The email address configured in Cisco Smart License Portal will be used as contact email address to send SCH notifications.`.
- Held blocks (`call-home`, `crypto pki`) print through the same block printer as the rest.
- Under `ip ssh pubkey-chain` users print two spaces in and their keys three.
- A key typed after `key-string` prints as the `key-hash` line IOS keeps: the key type, the MD5 of the blob in upper-case hex and the words after the blob as its comment. Text with no decodable key answers `%SSH: Failed to decode the Key Value`.

### Classic IOS

Classic IOS is a 2960-X at 15.2(7)E0a for the 15.2 rules, the 2960 family for the 12.2 and 15.0 rules.

- `version 15.2`; a banner kept whole to its delimiter, after the global lines that follow the interfaces.
- `no ip domain-lookup`, `no ip igmp snooping`, `no cdp run`, `no spanning-tree vlan <n>` and a routed interface's `no ip route-cache`.
- Every `snmp-server enable traps` line and port-security MAC.
- `ip default-gateway` after the interfaces.
- A 2960, 2960-Plus or 2960-C has two-part port names and no management port.
- IOS 12.2 writes `ip ssh` after the VLAN blocks, before the interfaces.
- Numbered access lists print after the logging lines, before `snmp-server`, and `logging <address>` after the interfaces.

## show startup-config

- `Using <n> out of <m> bytes`: n is the saved text from its first `!` through `end` with each line's newline, plus one. m is the NVRAM the platform's `show version` names.
- Then the configuration `write memory` saved, else the one the switch was loaded with, `end` and a blank line.
- A switch nothing was saved on prints `startup-config is not present`.

## Factory default

A C9300 at IOS XE 17.6 started by `vswitch shell` or `vswitch run` boots its factory default, printed whole as the lab C9300-24T at 17.6.4 printed it after `write erase`, vlan.dat deleted and a reload, at network-essentials and at network-advantage. Other platforms and releases start blank.

- Lines: `service timestamps`, the call-home comment and `service call-home`, `platform punt-keepalive disable-kernel-core`, `hostname Switch`, `vrf definition Mgmt-vrf`, `no aaa new-model`, `switch 1 provision <model>`, `login on-success log`, both trustpoints and their certificate chains, the licence line, `diagnostic bootup level minimal`, rapid-pvst with extend system-id, `memory free low-watermark processor 131696`, `redundancy` with `mode sso`, `transceiver type all` with `monitoring`, 22 control-plane class maps and the `system-cpp-policy` policy map, GigabitEthernet0/0 in Mgmt-vrf, every port, Vlan1 with `no ip address` and `shutdown`, `ip forward-protocol nd`, the three `ip http` lines, `control-plane` with its service policy, the line blocks and the call-home block.
- The `!` runs follow the level booted: after `switch 1 provision` 8 at network-essentials and 12 at network-advantage, before the trustpoints 6 and 9, before the licence line 1 and 2, before the first port 11 and 12 (the second a `! `), after `ip http secure-server` 4 and 6.
- `license boot level` survives `write erase`: the default prints the level booted, network-advantage with dna-advantage on a new switch.
- The self-signed trustpoint is `TP-self-signed-<n>`, n ten digits derived from the base MAC address. Its certificate (820 bytes, SHA-1, serial 01, a CA's basic constraints and key identifiers) is valid for 3652 days from two seconds before boot.
- The licensing root's certificate (805 bytes, SHA-256) keeps its names, dates and extensions. In both, the key, key identifiers and signature are bytes derived from the base MAC address, so each switch prints the same structure, line lengths and line counts as the lab's, its own values in them.
- The certificate hex prints 32 bytes a line in groups of four, each group followed by a space but the last, then a tab and `quit`.
- The last-change line names the boot.
- `show startup-config` prints `startup-config is not present`.
- VTP version 1, mode Server, no domain, revision 0, 5 VLANs.
- A `license boot level` typed in configuration mode prints `% use 'write' command to make license boot config take effect on next boot` and leaves the configuration's line, the level booted. A configuration loaded states the level the switch booted at.
- Each certificate prints as the NVRAM file IOS keeps it in: `certificate ca 01 nvram:CiscoLicensi#1CA.cer`. The name is the subject's common name without spaces cut to 12 characters, then `#` and its place, `CA` for a CA's.

| Platform | NVRAM |
|---|---|
| Catalyst 9200 and 9300 | 2048K |
| Catalyst 9400 | the 9300's |
| IE-3300 | 32768K |
| 2960 and IE 3000 | 64K |
| 2960-X and IE 3010 | 512K |

Sources are a C9300 capture at 17.15.02 and the lab 2960G.

## show interfaces status

The columns and rows follow the Catalyst 9400 captures (17.5), cabled as they ran.

### Status and VLAN

- A port is `disabled` when shut down, `inactive` when it is a supervisor uplink that is off, `notconnect` otherwise. An AppGigabitEthernet port or a cabled one is `connected`.
- A port BPDU guard err-disabled is `err-disabled`, and the far end of its cable `notconnect`.
- On IOS XE a lab file's third-party transceiver (`third_party = true`) is `err-disabled` with its type, reason `gbic-invalid`, until `service unsupported-transceiver` or `no errdisable detect cause gbic-invalid` (or `all`). The lab C9300 at 17.6.4 showed it with a third-party copper SFP.
- IOS XE shows an LACP member it has not bundled as `suspended`, duplex `auto`.
- The Vlan column holds the access VLAN, `trunk` while trunking, `routed` on a routed port. An access VLAN the database lacks shows as typed, `connected` or `notconnect`, never `inactive`.
- A port-channel no member bundles in prints `notconnect unassigned auto auto N/A` on IOS XE and on classic IOS before 15.2 (no Type there, its trailing space kept), whatever its mode. Classic IOS from 15.2 shows the access VLAN.
- The Name column holds the description cut to 18 characters.
- A module slot's ports show only where its module is fitted (an NM-8X's Te1/1/1-8).

### Speed, duplex and type

| Port | Duplex, speed, type |
|---|---|
| speed and duplex configured | the configured values, shown while down too |
| linked, transceiver | `full 10G SFP-10GBase-SR` |
| linked, copper | `a-full a-1000` |
| bundle | `a-full a-10G N/A`; its speed fixed where its members' is set (`a-full 1000`); members' speed `a-full a-<member speed>` |
| unplugged copper | `auto auto 10/100/1000BaseTX` |
| unplugged 1G SFP on a 9400 card | `auto a-1000 unknown` |
| unplugged SFP+ on a 9400 card | `auto a-10G unknown` |
| unplugged SFP on a fixed switch (C9300-24S), module port, supervisor and other ports | `auto auto unknown` |
| copper SFP (GLC-T, GLC-TE) | `a-full a-1000 10/100/1000BaseTX SFP` |
| far end on half duplex | `a-half` |
| C9200L 1G optic | `full 1000 1000BaseLX SFP` |
| AppGigabitEthernet up | `a-full a-1000 App-hosting port` (the 9400 captures print Type `unknown`) |

- A C9300's and C9300L's AppGigabitEthernet1/0/1 follows slot 1: shut, it prints `disabled` in VLAN 1 with `auto auto App-hosting port`, and `routed` in the Vlan column after `no switchport`.
- GLC-LH-SMD is `1000BaseLX SFP` and a 1000BASE-SX optic `1000BaseSX SFP`.
- A transceiver's Type name from its part number is inferred (SFP-10G-SR is `SFP-10GBase-SR`), and so are other unplugged ports (25G, 100G, mGig) as `auto auto unknown`.
- Port names on mixed-speed models are inferred from the installation guide and data sheet: C9300X-48HXN 1-40 FiveGigabitEthernet and 41-48 TenGigabitEthernet; the 9300L UXG, 9300LM UX and 9200L PXG mGig ports last.

### 9400 supervisor uplinks

- A redundant SUP-1XL pair with no QSFP port enabled runs ports 1-4; 5-8, 9 and 10 are `inactive`.
- A supervisor QSFP port runs once `enable` turns it on and turns its SFP ports off (SUP-1: 1-4 or 9, 5-8 or 10; SUP-2: 1-4 or 5). A pair never runs SUP-1 5-8 and 10 or SUP-2 7-8. Source: the Catalyst 9400 Supervisor Module Installation Note.

### Classic IOS

- A 2960-X has a 10-column Port, an empty SFP slot `auto auto Not Present`, a linked 1G optic `a-full a-1000`, no Type on a port-channel, and FastEthernet0 last as `routed` `10/100BaseTX`.
- A 2960 or 2960-Plus prints Fast Ethernet ports `10/100BaseTX` at 100, an empty dual-purpose uplink `Not Present`, and the header with a space after Type.
- An IE-3300 prints a transceiver's Type without ` SFP` (`1000BaseLX`).

## show vlan brief and show vlan

`show vlan brief` is the cores' table: a blank line, the header, VLAN 1, the VLANs defined to 1001, 1002-1005, then the extended VLANs.

- Each port not trunking is listed under its access VLAN, comma-separated and wrapped between ports where the next port and its comma would pass the console's width (80 when output is not a terminal).
- A bundled member's port-channel is listed in its place. A port-channel not trunking is listed under its access VLAN.
- A port-channel appears only once a member is bundled. The management port never appears.
- A port with a voice VLAN is listed under the voice VLAN too. A SPAN destination is listed under no VLAN.
- A VLAN under `state suspend` reads `suspended`.
- Classic IOS pads no line.

`show vlan` prints the `show vlan brief` table, then each VLAN's type, SAID and MTU, then the Remote SPAN VLANs and private VLAN tables, empty. Source: 39 show techs.

| VLANs | Type | STP | SAID | MTU |
|---|---|---|---|---|
| defined | `enet` | | 100000 plus the number | 1500 |
| 1002 | `fddi` | | 100000 plus the number | 1500 |
| 1003 | `tr` | | 100000 plus the number | 1500 |
| 1004 | `fdnet` | `ieee` | 100000 plus the number | 1500 |
| 1005 | `trnet` | `ibm` | 100000 plus the number | 1500 |

## show interfaces switchport

A block per switchport as the show techs of 39 switches print it, a blank line between. `show interfaces <port> switchport` prints one port's block alone with no blank line after it.

- The block holds administrative and operational mode, encapsulation, negotiation, access, native and voice VLANs with their names, and the allowed and pruning lists.
- Access ports print up and down; `Operational Trunking Encapsulation: native` appears only while up.
- A module port with no module fitted gives Name, Switchport and Administrative Mode then a blank line. A routed port is not listed.
- IOS XE Catalysts add `Vepa Enabled` and `App Interface`. Classic IOS and the IE-3300 do not.
- The IE-3300 tags no native VLAN by default and prints neither native-VLAN tagging line.

### Modes and trunks

- A trunk prints `Administrative Mode: trunk`, negotiation On, the native VLAN and the allowed list; operational `trunk` and encapsulation `dot1q` once it links.
- A port with no mode is `dynamic auto`. An access VLAN no block defines prints `(Inactive)`.
- AppGigabitEthernet is `static access` with encapsulation `native`.
- A bundled member's Operational Mode names its bundle (`trunk (member of bundle Po1)`).
- IOS XE prints an LACP member it has not bundled, linked or not, as `down (suspended member of bundle Po1)` with no operational encapsulation, and a PAgP or `on` member with no link the same.
- A port-channel prints `Access Mode VLAN: unassigned` and `Trunking Native Mode VLAN: 0 (Inactive)` while no member bundles, whatever its mode.
- A voice VLAN of `dot1p`, `untagged` or `none` prints as typed.
- The pruning list prints `NONE` when empty, and its `add` and `remove` lines apply in turn within 2-1001.

## show etherchannel summary

- IOS XE prints the legend in groups with a blank line between, and a blank line after the table.
- Each port-channel shows its protocol from `channel-group ... mode` (` -` for none), Layer 2 `S` or `no switchport` `R`, in use `U` while a member is bundled, else down `D`: `(SU)` or `(RU)` with bundled members `(P)`, `(SD)` or `(RD)` with no link and each member `(D)`.
- A member with a link that does not bundle is `(s)` on IOS XE, where standalone-disable is the default, and `(I)` under `no port-channel standalone-disable` and on classic IOS.
- LACP members cabled to each other in one channel do not bundle: the bundle `(SD)`, each member flagged as a member with a link that does not bundle, `(s)` on IOS XE.
- IOS XE pads each member to 16, two a line, the rest under the Ports column at 35, at any terminal width, trailing spaces kept.
- Classic IOS wraps the members under the Ports column before column 79 and keeps a memberless group's padding.
- Classic IOS 15.2 adds `N` and `m` to the legend and spaces its groups; a down LACP bundle is `(SD)` with a down member `(D)`.
- IOS 15.0 has IOS XE's legend without `A - formed by Auto LAG`.

## show interfaces trunk

- Each port trunking is listed (a port-channel in place of its members), with its mode, `802.1q` and its native VLAN. Nothing prints while no port trunks.
- Mode is `on` for a trunk-mode port and for a port-channel that inherited `switchport mode trunk`, `desirable` or `auto` for a dynamic port.
- The VLANs allowed, those the VLAN database holds, and those spanning tree forwards the port in print as lists; a port blocked in every VLAN prints `none`. Two VLANs in a row print as a range (`460-461`) where the running configuration writes `460,461`.
- Under MST every active VLAN is listed as forwarding.
- The Port column is 12 wide on classic IOS and 15 on IOS XE, which pads the header's `Native vlan` with two spaces.

## show power inline

- The layout follows the 9300 command reference's example (a C9300-48UN, 610 W): a row per PoE member with its default power supply's watts, those static ports reserve and those left.
- Each PoE port is `auto`, `static` or `off` (`power inline never`), oper `off` with no powered device. An idle port prints `auto off 0.0 n/a n/a <max>`.
- A static port reserves its maximum in Power and Used with no device. The maximum is the port's kind or its `max`: PoE+ 30.0, UPOE 60.0, UPOE+ 90.0.
- A stack lists every PoE member's row, then every port.
- A Catalyst 9200 or 9300 without PoE prints nothing.
- Classic IOS prints the same layout with no line padded, a PoE+ module row at its budget (370.0) and idle ports `auto off 0.0 n/a n/a 30.0`.
- A 2960-Plus without PoE prints `Available:0.0(w)  Used:0.0(w)  Remaining:0.0(w)` and the empty table. An IE-3300 with no PoE prints its own empty table (`Available: n/a(w)`).

## show vtp status

Versions 1 and 2 print the MD5 digest in two rows of eight `0xNN ` bytes. It is MD5 over:

| Part | Bytes |
|---|---|
| secret | 16 zero bytes, or with a VTP password MD5 over the password repeated to 100,032 bytes (1563 blocks of 64) |
| summary advertisement | version (1, or 2 at version 2), code 1, followers 0, domain length, domain padded to 32, revision (4 bytes), updater IPv4, 12 zero timestamp bytes, 16 zero digest bytes |
| each VLAN 1-1005 in order | record length, status (1 suspended), type (1 Ethernet, 2 FDDI, 3 Token Ring, 4 FDDI-net, 5 Token Ring-net), name length, VLAN ID, MTU 1500, SAID 100000 + ID, the name padded to 4 bytes, then its TLVs |
| secret | the same 16 bytes |

- TLVs are type, length in 2-byte words, a 2-byte value: 1002 and 1003 carry (1, 0) and (4, 0); 1004 (2, 0) and (3, 1); 1005 (2, 0) and (3, 2).
- Measured on the lab C9300 (17.6.4) in nine VLAN databases, the lab C2960G (15.0(2)SE11) and two Catalyst 9400 cores at revision 263 with 50 named VLANs.
- The secret was measured with six passwords on the lab C2960G; repeats of one password (`a`, `aa`, `aaaa`) give one digest.

- Version, domain, pruning and mode come from the VTP lines vlan.dat keeps (version 1, Server and pruning off by default). Traps come from `snmp-server enable traps vtp`.
- The VLANs count with 1 and 1002-1005; versions 1 and 2 leave out the extended VLANs.
- Device ID is the switch's base MAC address, as `show version` names it on every capture that prints both.
- A new database shows `0.0.0.0 at 0-0-00 00:00:00` at revision 0.
- A version 1 or 2 server counts one revision per VLAN database change typed at the console, once the VLAN submode is left, and stamps the updater and the switch's clock (`10-16-26 18:30:17`: month and day unpadded). A replayed configuration counts nothing.
- A new domain, transparent or off mode and version 3 start the revision at 0.
- Both digest lines end with a space on every platform.
- The updater is the lowest-numbered VLAN interface with an address. It skips a VLAN interface with no address (DHCP with no lease); with none, the first layer 3 interface with an address prints (`on interface Lo0 (first layer3 interface found)`). A transparent switch and VTP mode off name no updater.
- From IOS XE 17.6 the pruning line adds its operational state (`Enabled (Operationally Enabled)`); the 9400 at 17.5 prints `Enabled` alone.
- A 2960-Plus supports 255 VLANs locally and an IE-3300 259.

Version 3 prints no last change or updater, then a Feature VLAN block: the mode, the VLANs, the extended VLANs and 4096 VLANs supported. A server or client adds the revision, `Primary ID` `0000.0000.0000`, an empty `Primary Description` and an empty `MD5 digest`, as vswitch models no primary server; its revision stays 0. Feature MST and Feature UNKNOWN blocks follow, each Transparent, and a blank line ends the output.

## show vtp counters and show vtp password

- `show vtp counters` prints every advertisement and error count at 0, then the pruning table with a row per trunking port (`Gi1/1/1`, `Po10`), its three counts at 0.
- `show vtp password` prints `The VTP password is not configured.`, or `VTP Password: <password>` as typed.
- `vtp password <password>` prints `Setting device VTP password to <password>`, `no vtp password` prints `Clearing device VTP password.`, and `hidden` below version 3 prints `Device is operating in VTP version 1. hidden option is allowed from VTP version 3 only.` and keeps the password set before (the lab C2960G).

## show spanning-tree

PVST, Rapid PVST and MST in their converged state, across every switch of a lab, from the lab C9300-24T at 17.6.4 and the lab 2960G at 15.0(2)SE11. Listening and learning, BPDU counters, topology changes and ages are not modelled.

### The tree

- Each switch's bridge ID is its priority with the VLAN or MST instance added (`32769`) and its base MAC address: a lab file's `mac`, the switch list's, else one derived from the hardware.
- A VLAN has an instance where a port of the switch is up in it: an access port in its VLAN, a trunk in its allowed VLANs the database holds. A C9300's AppGigabitEthernet port runs in VLAN 1 with no cable.
- The root is the lowest bridge ID a switch reaches; each other switch's root port is the one with the lowest path cost, then designated bridge ID, designated port ID and its own port ID.
- On each link the end with the better path cost, bridge ID and port ID is designated. The other blocks: `Back` where both ends are one switch's (a cable between two of its ports), `Altn` otherwise.
- A port-channel is one port, its members left out, costed by the speed its members bundle.
- Cost is `spanning-tree [vlan <list>] cost`, else the speed's: long (802.1t, 1G 20000) by default on IOS XE, short (802.1D, 1G 4) on classic IOS, or `spanning-tree pathcost method`. The speed is the port's `speed`, else the speed a `no speed` cleared, else the link's: the lab C9300 kept costing a port at its old forced speed after `no speed`.
- A cable to a device the lab does not model carries no BPDUs unless the lab file's `[[device]]` gives the BPDUs it sends.
- `spanning-tree bpdufilter enable` sends and takes no BPDUs: both ends are designated forwarding.
- A port with BPDU guard that hears a BPDU is err-disabled: it leaves the tables, and its cable's far end loses its link. `show interfaces status` names its transceiver; `show interfaces status err-disabled` lists it with reason `bpduguard`.

### MST

- A switch in MST mode runs the CIST (MST0) and an MSTI for each `instance <n> vlan <list>` of `spanning-tree mst configuration`. A VLAN no instance lists runs in the CIST.
- The CIST's bridge priority is `spanning-tree mst 0 priority`, an MSTI's `spanning-tree mst <n> priority`, 32768 by default, with the instance added.
- Two MST switches are one region where their name, revision and VLAN-to-instance digest (HMAC-MD5 with the 802.1s key) match. In a region the CIST and every MSTI run across the link; between regions and against PVST only the CIST does.
- The CIST's root is the lowest bridge ID across the lab; the regional root is the region's bridge with the best path to it. The external cost counts links outside the region, the internal cost links inside it.
- An MSTI's port on a region's boundary takes the CIST's role, `Mstr` where the CIST's is `Root`.
- MST costs ports by the long method whatever is configured, and the summary prints `short (Operational value is long)` under the short method.
- Hops left are `spanning-tree mst max-hops` (20) less the root ports crossed inside the region.
- Against PVST or Rapid PVST, the CIST is VLAN 1 on a trunk and the access VLAN on an access port.
- PVST simulation: an MST boundary port facing a PVST-family trunk is held `BKN*` with `*PVST_Inc` in every instance where it is the CIST's root port and another VLAN's root path at the far port is worse than VLAN 1's, or where it is designated and the far port is designated in another VLAN (inferred).
- An MSTI port the far switch's CIST blocks gets no BPDUs and prints `Desg BLK`, a dispute.

### Guards

- Root guard (`spanning-tree guard root`) ignores the port's BPDUs; where they are better than its own the port prints `Desg BKN*` and `*ROOT_Inc`.
- Loop guard (`spanning-tree guard loop`, or `spanning-tree loopguard default` on every port, inferred) holds a designated port `BKN*` with `*LOOP_Inc` where the far end runs the instance behind its BPDU filter and the far end's root path is better: the port would be root or alternate if BPDUs arrived.
- An inconsistent port prints `BKN*` with its cost after the asterisk, no space between.

### Type

| Text | When |
|---|---|
| `P2p`, `Shr` | `Shr` on half duplex or under `spanning-tree link-type shared` |
| `Edge` | PortFast that hears no BPDU |
| `Peer(STP)` | a Rapid PVST port facing PVST, also behind the far end's BPDU filter; facing an MST port that is not the CIST's alternate or backup, or is held inconsistent; facing a device that sends STP BPDUs |
| `Bound(PVST)` | an MST port facing a PVST or Rapid PVST trunk |
| `Bound(RSTP)`, `Bound(STP)` | an MST port facing a Rapid PVST or PVST access port (inferred), another region's designated port (`RSTP`), or a device by its BPDUs |
| `*ROOT_Inc`, `*LOOP_Inc`, `*PVST_Inc` | the guard or simulation holding the port |

### Port numbers

| Port | Number |
|---|---|
| IOS XE switch port | its place in its member: front ports, slot 1's listing, the application port (Ap1/0/1 is 41 on a C9300-24T); 168 more per member after the first |
| IOS XE port-channel | 3048 plus its number (Po10 is 3058) |
| classic two-part port | its place (Gi0/1 is 1) |
| classic two-part port-channel | 63 plus its number (Po1 is 64) |
| classic stack member's port | its place in its member, 56 more per member after the first |

A Catalyst 9400's ports and a classic stack's port-channels have no known number: the forms that print one print the no-output-model line.

### Forms

| Form | Output |
|---|---|
| `show spanning-tree`, `active`, `vlan <list>` | a block per VLAN or MST instance: protocol (`rstp`, `ieee`, `mstp`), the root, the bridge, the ports by number; three blank lines between |
| `summary`, `summary totals` | the mode, the settings, then each instance's blocking and forwarding counts |
| `root`, `bridge`, `vlan <list> bridge` | a row per VLAN or instance; the last for the instances the list maps to (inferred) |
| `blockedports` | each instance's blocked ports and their count |
| `inconsistentports` | each instance's held ports, the reason (`Root Inconsistent`, `Loop Inconsistent`, `PVST Sim. Inconsistent`) and their count |
| `pathcost method` | the method in use |
| `interface <port>`, `interface <port> portfast` | the port's row per instance; `enabled` or `disabled` per instance |
| `mst`, `mst <n>` | MST mode: each instance's or one instance's bridge, root, regional root, hops and ports |
| `mst configuration` | the name, revision and VLANs per instance, in any mode |

- A bridge that is not the root adds `Cost` and `Port <n> (<name>)` under the root's address. Under MST the cost is the CIST's external one, the internal one where the CIST's root is in the region (inferred), and an MSTI's internal one.
- IOS XE pads the summary's labels to two widths and lists EtherChannel misconfig guard first. Classic IOS pads them to one width and lists it after Loopguard Default.
- The summary counts `1 vlan` and `1 mst` in the singular.
- UplinkFast and BackboneFast set outside PVST print `enabled but inactive in <mode> mode`.
- A port in no tree prints `no spanning tree info available for <port> `; a VLAN list with no instance prints `Spanning tree instance(s) for vlan <list> does not exist.`
- `detail`, `interface <port> detail`, `mst interface <port>`, `mst configuration digest`, MST forms outside MST mode and a switch with no instance print the no-output-model line. No BPDU counter, topology change or age is invented.
- Aging Time is `mac address-table aging-time` for the VLAN or the switch, else 300; MST blocks print none.

## show ip interface brief

The layout and column widths follow C9200L captures at 17.6 and 17.15, with no blank line before the header.

- Rows: the VLAN interfaces, the ports with each subinterface after its port, Bluetooth0/4 once made, the port-channels, loopbacks and tunnels.
- A provisioned member's ports print `unassigned YES unset down down`.
- A tunnel is up; its protocol is up while its source runs and a connected subnet or a static route's next hop reaches its destination.
- Where `ip pim rp-address` names an RP and multicast routing runs, the tunnels PIM makes follow the configured ones, method `unset`: the encapsulation tunnel addressed from the first sparse-mode interface, and on the RP the decapsulation tunnel with the RP's address.
- Vlan1 is `administratively down` until configured.
- Up: a loopback, an AppGigabitEthernet port, a cabled port, a port-channel with a bundled member, and a VLAN interface whose VLAN a running port carries. A VLAN interface whose VLAN exists with no running port carrying it is up, its protocol down.
- Method is `NVRAM` where a loaded configuration gives an address or `no ip address`, `manual` for an address typed at the console and for `no ip address` typed after it, and `DHCP` for `ip address dhcp` in a loaded configuration.
- `ip address dhcp` typed at the console reads `manual` while the interface is down and `DHCP` once it is up, with or without a lease.
- Method is `unset` on a front-panel switch port however configured, on a loopback with no address, and for the factory default's `no ip address`.
- IOS XE prints a name wider than 22 short (Te1/1/1, Fo1/1/1, Ap1/0/1). Classic IOS prints a name of 22 characters or more whole, followed by one space.
- Protocol is padded to 8. The IE-3300 pads Protocol to its header's width.

## show ip access-lists and show access-lists

`show ip access-lists [<name>]` prints each IPv4 list, or the one named: classic IOS as created, IOS XE the standard lists then the extended, each by name. A Catalyst 9300 at 17.6 adds the lists IOS XE makes itself (`IP-Adm-V4-Int-ACL-global`, `implicit_deny`, `implicit_permit`, `meraki-fqdn-dns`, `preauth_v4`); `IP-Adm-V4-Int-ACL-global` holds `permit tcp any any eq www` while `ip http server` runs and `eq 443` while `ip http secure-server` runs. `show access-lists [<name>]` prints the same, then on IOS XE the IPv6 lists and the MAC lists. Sources are the lab C9300-24T (17.6.4) and the lab 2960G (15.0(2)SE11).

- The heading is `Extended IP access list <name>` or `Standard IP access list <name>`.
- Each entry is four spaces in with its number. Remarks are left out.
- A standard wildcard prints as `<address>, wildcard bits <wildcard>`.
- A standard list's actions pad to `permit`'s width (`50 deny   any`). Classic IOS lists its host entries first.
- An IPv6 list prints `IPv6 access list <name>`, then each entry four spaces in with `sequence <n>` after it; `show ipv6 access-list [<name>]` prints the IPv6 lists alone, by name. The 9300 at 17.6 adds `implicit_deny_v6`, `implicit_permit_v6` and `preauth_v6`.
- A MAC list prints `Extended MAC access list <name> ` with a space after it, then each entry four spaces in.
- Nothing prints when no list exists.

## Output filters

`|` takes `include`, `exclude`, `begin`, `section` or `count` after any EXEC command, each keyword by any unique start. `append`, `format`, `redirect` and `tee` are refused at the keyword, as the switch keeps no files.

| Modifier | Output |
|---|---|
| `include`, `exclude` | the lines that match, or do not |
| `begin` | from the first matching line to the end |
| `section` | each matching line with the lines indented under it, printed once; a matching indented line prints without its parent |
| `section include` | the same as `section` |
| `section exclude` | the sections no line of which matches |
| `count` | `Number of lines which match regexp = N` |

- Expressions are IOS regular expressions. `_` matches a comma, brace, parenthesis, space or either end of the line, inside a bracket expression too.
- An expression ending in a lone backslash is refused, as is any malformed one.
- The expression is the rest of the line as typed. A space at the end is part of it (`| include Gi1/0/2 ` skips Gi1/0/20).
- A missing expression answers `% Incomplete command.`.

## show version and show license summary

A single C9300 at IOS XE 17.6 prints them as the lab C9300-24T at 17.6.4 did. Other platforms, releases and stacks answer the no-output-model line.

- `show version`: the 17.6.4 image's banner and the GPL and export notices, the uptime, the licence table, the hardware lines, the interface counts, the identity fields and the ports and model row.
- The uptime runs from the boot on the switch's clock: years, weeks, days and hours where not zero, then minutes always (`1 year, 47 weeks, 4 days, 3 hours, 0 minutes`). The control processor's is 90 seconds longer.
- The licence table and `show license summary` name the level booted: `license boot level`, network-advantage with dna-advantage without one.
- The interface counts are the VLAN interfaces as Virtual Ethernet, then the ports by speed, the management port left out.
- The base MAC address prints colon-separated from the switch's own. The serial numbers are derived from it: three letters, a year and week, four letters or digits.
- `show license summary` lists the base level and its addon, each `1 IN USE`, with an entitlement tag of the model's first eight characters and the level's title, cut to 22 characters and `...` past 25.

## Measured show messages

A privileged line a form of the session's level takes prints the `%` lines the switch printed for it, where the form holds them (Messages in `captures.md`). `src/messages.rs` holds the messages and their rules. A notice prints before the command's output. A fixed message is all the line prints. A state message is all the line prints where the running configuration lacks what it names; otherwise the line prints what it prints without it. It reads the context it names: each line under a block with the submode lines it stands in (an address family, its VRF, a process), and an interface's own lines apart from a submode's. A value a message names is the line's word as typed, an interface by its full name. A line printing messages alone is refused, as a message the configuration gives is, and those messages (Messages the configuration gives) print first. A line the shell warns of before it runs prints that warning and its output: the switch printed `% Switch is not in mst mode` before the MST region's configuration and digest.

| Kind | Message |
|---|---|
| notice | `% Command accepted but obsolete, unreleased or unsupported; see documentation.` |
| notice | `% Command accepted but obsolete, see 'show (ip\|ipv6) cef switching statistics [feature]'` |
| notice | `% NOTE: This command is deprecated. Please use 'show bgp ipv6 unicast'` |
| notice | `% Command "show ipv6 nd suppress <...>" is obsolete.`, `% Please use "show flooding-suppression <...>" instead.` |
| notice | `% Warning: Use of this command may result in packet drops` |
| fixed | `% Error: Unsupported command`, `% Error: ngwc not supported`, `% Error: Command not supported`, `% Error: Command is not supported on this platform` |
| fixed | `% Error: Show version installed user-interface is not supported`, `%Error: lowmem gd is not supported in this platform` |
| fixed | `% Missing filename in URL.` (a file system with no file name) |
| fixed | a line's own arguments, refused alike by the lab C9300-24T and C9200L-48T-4G (17.6.4) on their configurations: `% Error: Invalid timestamp. Input must be in the correct format with leading and trailing zeroes`, `% Error: Interval too large for reverse. Supported maximum interval is 600 seconds!`, `% Error: Invalid uuid. Requires prefix [0x]`, `% Error: Failed to add options for trace filter command, invalid input`, `% Error: Unknown Protocol`, `% Error: Unknown protocol`, `% Error: Unknown interface <word>`, `% Failed : No such seat`, `%error opening <file system>`, `%Error processing <file system>: Unable to allocate memory` |
| fixed | `% Linecard information not found` (a fixed-configuration switch has no linecards) |

| State message | Prints where the configuration has no |
|---|---|
| `% LISP is not running.` | `router lisp` |
| `%No active Subscriber Sessions`, `%No active services` | subscriber session: always, as vswitch holds none |
| `%SNMP agent not enabled` | `snmp-server` line other than `location` and `contact` |
| `%%SNMP manager not enabled` | `snmp-server manager` |
| `% MPLS Fault-OAM is not initialized.` | `mpls traffic-eng fault-oam` or `mpls tp` |
| `%AS <n> specified not configured`, `%AS(<n>) specified not configured` | EIGRP AS `<n>` in the line's family and VRF, where an EIGRP process runs: `router eigrp <n>` (`ip eigrp`, `eigrp address-family ipv4`), `ipv6 router eigrp <n>` (`ipv6 eigrp`, `eigrp address-family ipv6`), or a named process's `address-family` (`service-family` for an `eigrp service-family` line) of that family and VRF with `autonomous-system <n>`; on a BGP line, `router bgp <n>` where some `router bgp` runs |
| `% IP routing table topology <name> does not exist` | `topology <name>` in an IPv4 context, multicast for a multicast line and unicast otherwise, in the line's VRF or none; `base` always exists |
| `%Topology <name> specified does not exist.` | `topology <name>` under a named `router eigrp`'s `address-family` of the line's family, VRF and autonomous system where it names them, where an EIGRP process runs; `base` always exists |
| `%OSPF: Specified topology not found for any OSPF process` | `topology <name>` under any `router ospf`, in its `address-family ipv4 multicast` for a multicast line and outside it otherwise, where one runs; the unicast `base` always exists |
| `%OSPF: OSPF not enabled on <interface>` | OSPF on the interface, where a `router ospf` runs: `ip ospf <n> area` among its own lines, or a `network` of a process in its VRF covering one of its addresses; an address vswitch cannot read (DHCP, negotiated, pool, unnumbered) prints nothing where such a `network` exists |
| `% No VRF named <name>`, `% vrf <name> does not exist`, `% VRF <name> does not exist`, `%Error, VRF <name> does not exist`, `% Unknown VRF` | `vrf definition <name>` or `ip vrf <name>`; `Mgmt-vrf` exists where the stack has a management port |
| `% Config Logger disabled.` | `logging enable` under `archive`'s `log config` |
| `%XMCP server is not enabled` | `service-routing xmcp` line |
| `% Switch is not in mst mode` | `spanning-tree mode mst` |
| `% No matching EVPN ethernet segment` | `evpn ethernet-segment` among the named interface's own lines |
| `% dampening not enabled for base` | `bgp dampening` directly under `router bgp` or in its `address-family ipv4 [unicast]`, a VRF's not counting, where a `router bgp` runs |
| `% No such peer-group` | `neighbor <name> peer-group` for the group named, in the VRF's address family where the line names a VRF and outside every VRF's otherwise, where a `router bgp` runs |
| `%No Tlv-list exists. `, `%Tlv-list name does not exists.` | `cdp tlv-list` (for `*`), `cdp tlv-list <name>` (for a name) |

## Padding and unmodelled commands

- Every platform keeps IOS's trailing spaces (`act/unsup `, the VTP digest, a padded last column), as every family's show tech prints them. A terminal log loses them.
- Other `show` commands print `% vswitch has no output model for '...' yet`, `show inventory` among them.
- `show interface ...`, as the books spell it, is taken as `show interfaces ...`.

## Commands known to crash a switch

A line a lab switch crashed on prints a warning when Enter is pressed on it, as a line of its own after the switch's whole answer, which is unchanged: the caret stays under the typed line, and the warning follows the `%` message and its blank line, before the next prompt. `?` and Tab print none. The line is read as IOS reads it: each word a start of its keyword in any case, a whole shorter keyword read as itself (`show ip` is not `show ipv6`), what follows `|` ignored, and in configuration mode only after `do` or `do-exec`. An interface in a command is read by its type's abbreviation and its number, in one word or two (`tu0`, `tu 0`, `gi0/0`), and the line is a hazard on that interface alone. The table is `src/hazards.rs`.

| Command | Model | Release | Effect |
|---|---|---|---|
| `show ipv6 pim gir` | Catalyst 9300 | IOS XE 17.6.4 | IOSd segfault and reload |
| `show platform software tdl-database content bp`, with any words after it | Catalyst 9300 | IOS XE 17.6.4 | IOSd segfault and reload |
| `show interfaces Tunnel0 transceiver max-power` | Catalyst 9200L, 9200, 9200 B | IOS XE 17.15.6 | IOSd segfault and reload |
| `show interfaces Tunnel0 transceiver max-power violation` | Catalyst 9200L, 9200, 9200 B | IOS XE 17.15.6 | IOSd segfault and reload |
| `show interfaces GigabitEthernet0/0 transceiver max-power` | Catalyst 9200L, 9200, 9200 B | IOS XE 17.15.6 | IOSd segfault and reload |
| `show interfaces GigabitEthernet0/0 transceiver max-power violation` | Catalyst 9200L, 9200, 9200 B | IOS XE 17.15.6 | IOSd segfault and reload |

| Switch | Warning |
|---|---|
| a model listed, on the train of the command's release | `% vswitch: this command crashes the switch (IOSd segfault and reload on a Catalyst 9300 at IOS XE 17.6.4)` |
| a model listed, on another release | `% vswitch: this command crashed a Catalyst 9300 at IOS XE 17.6.4 (IOSd segfault and reload)` |
| another model | none |

The `transceiver max-power` warnings name a Catalyst 9200L at IOS XE 17.15.6.

Each Catalyst 9300 command was typed alone on one SSH session of a C9300-24T at network-advantage on 2026-10-17. IOSd segfaulted (`linux_iosd_image has failed (rc 139)`) and the switch reloaded with `Critical software exception`, answering SSH again about 4 minutes later. Each crash wrote a crashinfo file and a 58 MB system report to `crashinfo:`. `show ipv6 pim gir` faulted in the process running it, `SSH Process` over SSH; every `bp` form tried (`config local`, `config nodes`, `config nodes 0`, `oper local`, `oper nodes`) faulted in `Sched`. No published Cisco bug names either command.

These answered normally on the C9300-24T: `show ip pim gir`, `show ip igmp gir`, `show ipv6 mld gir`, and the other `show platform software tdl-database content` forms (`cfgmgr`, `fpmd`, `ftm`, `ios`, `ompd`, `vdaemon`).

Each `transceiver max-power` line crashed a C9200L-48T-4G at network-advantage on 2026-10-21, typed in a show sweep over SSH, and was the last command of its crashinfo file. IOSd segfaulted (`linux_iosd_image has failed (rc 139)`) and the switch reloaded with `Critical software exception`, answering SSH again about 5 minutes later. The same forms with no interface or on the sweep's other interfaces (GigabitEthernet1/x, Port-channel1, Vlan10) did not crash it. At 17.6.4 the Tunnel0 forms closed the session without a reload.

`run` prints the warning in the session; `probe` refuses the line, and `explain` names it ([commands.md](commands.md)).
