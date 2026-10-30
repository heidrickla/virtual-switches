//! Where each output rule comes from: what a real switch printed, a Cisco example, or vswitch's
//! inference. What a switch printed overrides the rest; `vswitch verify
//! --outputs` lists the rules no switch has shown yet, as the list to check on hardware.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    /// printed by a real switch
    Live,
    /// a Cisco document's example
    Example,
    /// worked out by vswitch where neither shows it
    Inferred,
}

pub struct Rule {
    pub output: &'static str,
    pub rule: &'static str,
    pub source: Source,
    pub evidence: &'static str,
}

const CORES: &str = "core-a and core-b, Catalyst 9400, 17.5, 2026-10-05";
const CLASSIC_2960X: &str =
    "c2960x 15.2(7)E capture 8, Catalyst 2960-X, IOS 15.2(7)E0a, cabled as it ran, 2026-10-06";
const NEW: &str = "c2960x 15.2(7)E capture 7 (2960-X), c9200 17.6 capture 1 and c9200 17.15 capture 27 (C9200L, 17.6.4 and 17.15.2), cabled as they ran, 2026-10-06";
const C2960: &str = "11 switches of the 2960 family, each typed at its own release: WS-C2960G-24TC-L at 12.2(44)SE, WS-C2960-24TC-L and WS-C2960G-8TC-L at 12.2(55)SE, WS-C2960C-8TC-L at 15.2(2)E9, six WS-C2960+24TC-L at 15.2(6)E1; 2026-10-06";
const BATCH: &str = "76 switches captured, each cabled as it ran: 2960-X at 15.2(2)E, (6)E and (7)E; C9200 and C9200L at 17.6, 17.9, 17.12 and 17.15; C9300, C9300L and C9300-24S at 17.6 and 17.15; 2026-10-06";

const SHOW_TECH: &str = "the show techs of 39 switches: 2960-Plus at 15.2(6)E, C9200L at 17.15, C9300 at 17.15, IE-3300 at 17.6; 2026-10-07";
const C2960G: &str = "the lab C2960G (WS-C2960G-8TC-L, 15.0(2)SE11) over SSH";
const LAB: &str = "the lab C9300-24T with a C9300-NM-4G (17.6.4) on its console and the lab C2960G (15.0(2)SE11) over SSH, each case configured, read and reset, 2026-10-16";
const LAB_2960X: &str = "the lab WS-C2960X-24PS-L (15.2(2)E5 and 15.2(7)E0a) over SSH, each case configured, read and reset, and its 15.2(7)E0a show sweep replayed from its own running configuration, 222 modelled outputs compared whole, 2026-10-29";
const LAB_REPLAY: &str = "the lab C9300-24T with a C9300-NM-4G (17.6.4): its 745-line configuration, member 2 provisioned and absent, replayed and 39 outputs compared whole, and the follow-up measurements, 2026-10-17";

pub const RULES: &[Rule] = &[
    Rule {
        output: "every show command",
        rule: "every platform keeps IOS's trailing spaces (`act/unsup `, the VTP digest, a padded last column); a terminal log loses them, and those four sessions compare with line ends trimmed",
        source: Source::Live,
        evidence: BATCH,
    },
    Rule {
        output: "show interfaces switchport",
        rule: "a block per switchport, a blank line between; access ports up and down (`Operational Trunking Encapsulation: native` only while up), the VLAN's name beside its number; a module port with no module fitted gives Name, Switchport and Administrative Mode then a blank line; a routed port is not listed; the IOS XE Catalysts add `Vepa Enabled` and `App Interface`, classic IOS and the IE-3300 do not",
        source: Source::Live,
        evidence: SHOW_TECH,
    },
    Rule {
        output: "show interfaces switchport",
        rule: "a bundled member's Operational Mode names its bundle (`trunk (member of bundle Po1)`); IOS XE calls an LACP member it has not bundled, linked or not, and a PAgP or `on` member with no link, `down (suspended member of bundle Po1)` with no operational encapsulation, and classic IOS an LACP member with no link; a voice VLAN of `dot1p`, `untagged` or `none` as typed; the pruning lines read in turn as the allowed list reads, within 2-1001; one port's block alone with no blank line after it",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show interfaces switchport",
        rule: "a trunk: `Administrative Mode: trunk`, negotiation On, the native VLAN and the allowed list; operational `trunk` and encapsulation `dot1q` once it links; a port with no mode `dynamic auto`; an access VLAN no block defines `(Inactive)`; the voice VLAN; a port-channel with no capture lines, and while no member bundles `Access Mode VLAN: unassigned` and `Trunking Native Mode VLAN: 0 (Inactive)` whatever its mode; AppGigabitEthernet `static access`, encapsulation `native`",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show vlan",
        rule: "the `show vlan brief` table, then each VLAN's type (`enet`; 1002-1005 `fddi`, `tr`, `fdnet` with STP `ieee`, `trnet` with `ibm`), SAID 100000 plus its number, MTU 1500, then empty Remote SPAN VLANs and private VLAN tables",
        source: Source::Live,
        evidence: SHOW_TECH,
    },
    Rule {
        output: "show running-config",
        rule: "a 2960, 2960-Plus or 2960-C: two-part port names, no management port; IOS 12.2 writes `ip ssh` after the VLAN blocks, before the interfaces; `logging <address>` and numbered access lists after the interfaces, the lists before `snmp-server`; a line keeps the switch's own spacing",
        source: Source::Live,
        evidence: C2960,
    },
    Rule {
        output: "show interfaces status",
        rule: "a 2960 or 2960-Plus: Fast Ethernet ports `10/100BaseTX` at 100, an empty dual-purpose uplink `auto auto Not Present`, a space after Type in the header",
        source: Source::Live,
        evidence: C2960,
    },
    Rule {
        output: "show power inline, show vtp status",
        rule: "a 2960-Plus without PoE prints its zero totals and the empty table; it supports 255 VLANs locally",
        source: Source::Live,
        evidence: "c2960 15.2(6)E capture 5 and c2960 15.2(6)E capture 6 (WS-C2960+24TC-L, 15.2(6)E1), 2026-10-06",
    },
    Rule {
        output: "show running-config",
        rule: "version line, every interface listed, section order, `no` lines IOS keeps, `line con 0`, `exit-address-family` closing its section",
        source: Source::Live,
        evidence: CORES,
    },
    Rule {
        output: "show running-config",
        rule: "a line a real switch printed prints back as printed: its `no` form kept, a first word the books read as an abbreviation of another command taken as held (`service call-home`, not `service-template`), and a held line placed after the line typed before it",
        source: Source::Live,
        evidence: "every capture types and prints back line for line in order (live_capture_tests)",
    },
    Rule {
        output: "show running-config",
        rule: "section order: `logging buffered` and the other logging lines but host, origin-id, trap, source-interface and facility at the top; the VLAN blocks, then `cdp` lines, then class and policy maps; loopbacks first among the interfaces; `no ip ssh` and the access lists after the interfaces, the lists before `snmp-server`; `privilege` lines after `control-plane`; a banner typed in a submode taken globally",
        source: Source::Live,
        evidence: "c9200 17.15 capture 27 (17.15.2), c9200 17.6 capture 1 (17.6.4)",
    },
    Rule {
        output: "show running-config",
        rule: "classic IOS: `version 15.2`; a banner kept whole to its delimiter; `no ip domain-lookup`, `no ip igmp snooping`, `no cdp run`, `no spanning-tree vlan <n>` and a routed interface's `no ip route-cache` kept; every `snmp-server enable traps` line and port-security MAC; `ip default-gateway` after the interfaces",
        source: Source::Live,
        evidence: CLASSIC_2960X,
    },
    Rule {
        output: "show running-config",
        rule: "VLANs and VTP lines print only in VTP transparent or off mode; a VTP server keeps them in vlan.dat; the unnamed VLANs print on one line, their list compressed and ending with a space (`vlan 10,100-141 `), each named VLAN its own block; `vtp domain` before `vtp mode`; mode off prints no `vtp version` at versions 1 and 3; `Current configuration` counts each line through `end` with its newline, plus one",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show running-config",
        rule: "lines typed in any order print in the order the captures print their commands: each section's top-level lines as the release's captures and its family's order them, an interface's and a line's own lines as every capture does; a command no capture prints stays after the line typed before it; every class map before every policy map; `line con 0`, `line aux 0`, then the vty ranges by first line; a `version` line typed adds no second one",
        source: Source::Live,
        evidence: "the 102 captures, every one printed back in its own order (live_capture_tests)",
    },
    Rule {
        output: "show running-config",
        rule: "IOS XE 17.6: `vlan access-map` blocks among the global blocks in every VTP mode, then `vlan filter`, then the VLANs; `router` blocks after the last interface, before `ip forward-protocol`; `ip route` after the `ip http` lines; `ip access-list` blocks after `ip route`, before `control-plane`",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show running-config",
        rule: "other releases and classic IOS: the 17.6 places of `router`, `ip route` and `ip access-list`; `vlan filter` among the global lines before the access maps; `vtp version` and `vtp password` after `vtp mode`",
        source: Source::Inferred,
        evidence: "only the lab C9300 at 17.6.4 holds a routing process, a VLAN access map or a VLAN filter",
    },
    Rule {
        output: "show running-config",
        rule: "a `vlan configuration` with nothing under it is not kept; a blank line after `end`, the interface form's too; IOS XE keeps `no port-channel standalone-disable` under a port-channel; a member's `channel-group` typed makes its port-channel with the member's switch port lines",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show running-config",
        rule: "the interface form on the 2960G at 15.0(2)SE11, a physical port's and a port channel's block, its byte count one past its lines",
        source: Source::Live,
        evidence: C2960G,
    },
    Rule {
        output: "show running-config",
        rule: "`no switchport` kept on a routed port",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show running-config",
        rule: "`vtp version` prints whatever the VTP mode; a modular C9200's slot 1 lists every module's ports whatever is fitted; a section's lines are its own (each class of a policy map keeps its `priority level`); a line typed less indented than its section closes it",
        source: Source::Live,
        evidence: BATCH,
    },
    Rule {
        output: "show running-config",
        rule: "VTP mode off prints the domain, the mode and the VLANs as transparent mode does",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show running-config, show startup-config",
        rule: "a C9300 at 17.6 boots its factory default with no startup configuration, printed whole: every line and `!` line, the call-home comment and block, `switch 1 provision`, the two trustpoints and their certificate chains (TP-self-signed, a ten-digit number, then SLA-TrustPoint), the licence line, the 22 control-plane class maps with their descriptions' trailing spaces, the `system-cpp-policy` policy map and control-plane, the `! ` line before the first port, a `!` run of 8, 6, 1, 9 and 4 at network-essentials and 12, 9, 2, 10 and 6 at network-advantage; `show startup-config` prints `startup-config is not present`; a `license boot level` typed says `% use 'write' command to make license boot config take effect on next boot` and leaves the configuration's line; `license boot level` survives `write erase`",
        source: Source::Live,
        evidence: "the lab C9300-24T (17.6.4) after `write erase`, vlan.dat deleted and a reload, at network-essentials and at network-advantage, 2026-10-16",
    },
    Rule {
        output: "show running-config, show startup-config",
        rule: "a 2960-X at 15.2(2)E boots its factory default with no startup configuration, printed whole: `no service pad`, the timestamps, `no service password-encryption`, `hostname Switch`, the boot markers, `no aaa new-model`, `switch 1 provision`, `spanning-tree mode pvst` and `extend system-id`, `vlan internal allocation policy ascending`, FastEthernet0, every port and Vlan1 with nothing under them, `ip http server` and `ip http secure-server`, `line con 0` and `line vty 5 15` with nothing under them and no `line vty 0 4`, `!` runs of 10, 4, 11 and 5; so does a lab switch whose files hold no line; `show startup-config` prints `startup-config is not present`",
        source: Source::Live,
        evidence: "the lab WS-C2960X-24PS-L (15.2(2)E5) after `erase startup-config` and a reload with no cable, 2026-10-29: its running configuration compared whole",
    },
    Rule {
        output: "show running-config",
        rule: "the self-signed certificate (820 bytes, SHA-1, valid 3652 days from boot) and the licensing root's (805 bytes, SHA-256, its names and dates) in their measured DER layout, hex lines of 32 bytes in groups of four; the trustpoint number, keys, key identifiers and signatures are stand-ins derived from the base MAC address",
        source: Source::Live,
        evidence: "the lab C9300-24T (17.6.4): both certificates parsed from three running configurations, every byte outside the key, key identifiers, signature and trustpoint number matching",
    },
    Rule {
        output: "show running-config",
        rule: "the certificate's validity starts two seconds before the last-change line; the factory default of other platforms and releases (the 2960-X at 15.2(7)E among them): a switch with no configuration starts blank; a stack's `switch <n> provision` per member, on a 2960-X FlexStack too; the 2960-X default's size, one byte under the 1524 its header read, which a capture that trims trailing spaces cannot place; another model's low watermark; the license message on IOS XE releases other than 17.6",
        source: Source::Inferred,
        evidence: "only the lab C9300 at 17.6.4 and the lab 2960-X at 15.2(2)E5 were booted from the factory default, each alone; the C9300's two boots put two and three seconds between the certificate and the last change",
    },
    Rule {
        output: "show running-config",
        rule: "IOS XE 17.6 prints `! Last configuration change at HH:MM:SS UTC Day Mon D YYYY` between two `!` lines after the first, counted in the size: the switch's boot, then the last line typed; a switch's own printed configuration replayed keeps the `!` and comment lines, the indented `!` lines and the trailing spaces it printed wherever the same lines meet again, and a line added between two of them takes the replayed `!` lines before it (all of them less those computed after it, where those are fewer)",
        source: Source::Live,
        evidence: "the lab C9300-24T (17.6.4): two factory boots and a configuration replace with `vtp domain`, a VLAN access map and filter, a loopback, OSPF, a static route and an access list added, compared whole, 2026-10-16",
    },
    Rule {
        output: "show running-config",
        rule: "a switch's own printed configuration replayed prints its top-level lines in its order, its blank lines, the comment lines in its blocks, each line's spacing and case, the user its last-change line named until a line is typed, and the `no` lines it printed; an empty line counts nothing in the size; the interface form of an interface not made prints `end` alone",
        source: Source::Live,
        evidence: LAB_REPLAY,
    },
    Rule {
        output: "show running-config",
        rule: "classic IOS prints the last-change line between two `!` lines after the first, counted in the size, and counts an empty line's newline; `linenum` numbers each line but an empty one, right-aligned in five columns, six from 100 lines; `brief` leaves out the certificates' hex; `class-map` and `policy-map` print between `!` and `end`, one byte over their lines, nothing for one named that does not exist; `ip dhcp pool` each pool after a `!` with no heading; `vlan` unsized; with AAA off `aaa` a `!`, `no aaa new-model` and the users, its other sections a `!`; `identity` and `map-class` `end` unsized, `eap` a blank line, `flow` the heading alone; a class map's `match ip dscp ef ` with its trailing space",
        source: Source::Live,
        evidence: LAB_2960X,
    },
    Rule {
        output: "show running-config",
        rule: "a line added to a replayed configuration prints after every replayed line whose command IOS prints before its own (a new `spanning-tree mst configuration` after `spanning-tree extend system-id`); two replayed lines differing only in case each print as printed",
        source: Source::Live,
        evidence: "the lab C2960G (WS-C2960G-8TC-L, 15.0(2)SE11) with an MST region typed and reset, 2026-10-16; the captures whose ports carry descriptions differing only in case",
    },
    Rule {
        output: "show running-config",
        rule: "unnamed VLANs as runs split at each VLAN with lines (`vlan 100-119 `, `vlan 120` named, `vlan 121-139 `); `state suspend` under its VLAN, `state active` nothing; `monitor session` ports short; the tunnels after the port-channels, each subinterface after its port, Bluetooth0/4 before the VLAN interfaces; a member provisioned with no switch there prints its ports; 17.6 pads an access list's `deny`; an IPv6 list's entries as `sequence <n> <entry>`; IOS XE's standard access lists, then its extended, each by name; class-map DSCP and precedence values each followed by a space, a space apart, a CoS value with a space either side",
        source: Source::Live,
        evidence: LAB_REPLAY,
    },
    Rule {
        output: "show running-config",
        rule: "an interface section's lines and its `exit-` line two spaces in (`vrrp`); EIGRP named mode's families and their `topology base`; `bmp server` and BGP templates closed at their header's depth; a router's own lines before its families; `no neighbor <x> activate` kept under the IPv4 family and `no protocol <p>` in a device-tracking policy",
        source: Source::Live,
        evidence: LAB_REPLAY,
    },
    Rule {
        output: "show running-config, show ip interface brief, show access-lists",
        rule: "an IPv6 entry typed with no number takes the highest plus 10; more than one CoS value spaced as one; `no switch <n> provision` removing an absent member's interfaces; the tunnels PIM makes numbered past the lowest free; one HTTP list entry per server; IOS XE's own access lists absent on other platforms and releases; other IOS XE releases padding a standard entry in `show ip access-lists`",
        source: Source::Inferred,
        evidence: "the lab C9300-24T (17.6.4) printed numbered IPv6 entries, one CoS value per line, two PIM tunnels after one configured, its own lists at 17.6 alone",
    },
    Rule {
        output: "show vlan brief",
        rule: "a port is listed under its voice VLAN too; a SPAN destination, linked or not, is not listed; VLANs past 1005 after 1002-1005; `state suspend` reads `suspended`; a suspended LACP member under its access VLAN",
        source: Source::Live,
        evidence: LAB_REPLAY,
    },
    Rule {
        output: "show ip interface brief",
        rule: "a provisioned absent member's ports `unassigned YES unset down down`; Bluetooth0/4 once made, `administratively down` while shut; a tunnel up, its protocol up while its source runs and a connected subnet holds its destination; the tunnels PIM makes after the configured ones, `unset`, the encapsulation tunnel addressed from the first sparse-mode interface and on the RP the decapsulation tunnel with the RP's address; each subinterface after its port",
        source: Source::Live,
        evidence: LAB_REPLAY,
    },
    Rule {
        output: "show ip access-lists, show access-lists, show ipv6 access-list",
        rule: "IOS XE lists standard lists then extended, each by name, a standard entry's action padded; a 9300 at 17.6 adds the lists it makes itself (`IP-Adm-V4-Int-ACL-global`, `implicit_deny`, `implicit_permit`, `meraki-fqdn-dns`, `preauth_v4`, and the `_v6` lists), the first empty once `no ip http server` and `no ip http secure-server` are typed; `show access-lists` adds the IPv6 lists, each entry with `sequence <n>` after it, then the MAC lists, each title with a space after it",
        source: Source::Live,
        evidence: LAB_REPLAY,
    },
    Rule {
        output: "show ip access-lists, show access-lists, show ipv6 access-list",
        rule: "the 2960-X lists standard lists then extended, each by name, among them the lists it makes itself (`CISCO-CWA-URL-REDIRECT-ACL`, `preauth_ipv4_acl (per-user)`; `preauth_ipv6_acl (per-user)` among the IPv6 lists); `show access-lists` adds the IPv6 lists, then the MAC lists; `show ip access-lists interface <name> [in|out]` the lists applied there, each title naming its direction, nothing where none is; `ipc` and `hardware counters` print no output model",
        source: Source::Live,
        evidence: LAB_2960X,
    },
    Rule {
        output: "show running-config",
        rule: "other releases print no last-change line; a static route's, access list's and control-plane's `!` lines after the `ip http` lines on releases other than 17.6",
        source: Source::Inferred,
        evidence: "the last-change line and those gaps were measured on the lab C9300 at 17.6.4 only",
    },
    Rule {
        output: "show version, show license summary",
        rule: "a single C9300 at 17.6: the 17.6.4 image's banner, uptime in years, weeks, days and hours where not zero then minutes, the licence levels booted with the next reboot's the same, the interface counts by speed with the VLAN interfaces as Virtual Ethernet, the base MAC address colon-separated, the ports and model row; `show license summary` lists the booted level and its addon, each `IN USE`, the tag cut to 22 characters and `...` past 25; the serial numbers are stand-ins derived from the base MAC address",
        source: Source::Live,
        evidence: "the lab C9300-24T (17.6.4) at network-essentials and network-advantage, and the uptime lines of 110 captures, 47 of them a year or more, 2026-10-16",
    },
    Rule {
        output: "show version, show license summary",
        rule: "a 17.6 switch runs the 17.6.4 image; the control processor's uptime 90 seconds past the switch's; a year of 365 days; another model's memory, ROMMON, assembly and revision numbers as the C9300-24T's; a level with no addon its matching DNA tier; other platforms, releases and stacks answer the no-output-model line",
        source: Source::Inferred,
        evidence: "one switch, two readings: 1 and 3 minutes, 2 and 3 minutes",
    },
    Rule {
        output: "show interfaces status, show interfaces status err-disabled",
        rule: "a third-party transceiver on IOS XE is err-disabled `gbic-invalid`, its type printed, until `service unsupported-transceiver` or `no errdisable detect cause gbic-invalid`; `no errdisable detect cause` lines print in the running configuration",
        source: Source::Live,
        evidence: "the lab C9300-24T (17.6.4) with a third-party 1000BASE-T SFP in Gi1/1/1, before and after `write erase`, 2026-10-16",
    },
    Rule {
        output: "show interfaces status",
        rule: "`no errdisable detect cause all` allows a third-party transceiver; a port with an optic and no link prints the optic's type; classic IOS checks none",
        source: Source::Inferred,
        evidence: "the lab measured `service unsupported-transceiver` and the gbic-invalid cause only",
    },
    Rule {
        output: "show interfaces status",
        rule: "layout; disabled when shut, inactive on a redundant supervisor's quiet ports, notconnect otherwise; Vlan is the access VLAN when not trunking; a configured speed and duplex show while down",
        source: Source::Live,
        evidence: CORES,
    },
    Rule {
        output: "show interfaces status",
        rule: "an unplugged copper, 1G SFP, 9400 SFP+ card or 9400 supervisor port's duplex, speed and type",
        source: Source::Live,
        evidence: CORES,
    },
    Rule {
        output: "show interfaces status",
        rule: "an unplugged SFP port on a fixed switch (C9300-24S) and a C9200 or C9300 module port as `auto auto unknown`; a module slot's ports only where its module is fitted (an NM-8X's Te1/1/1-8)",
        source: Source::Live,
        evidence: BATCH,
    },
    Rule {
        output: "show interfaces status",
        rule: "other unplugged ports (25G, 100G, mGig) as `auto auto unknown`",
        source: Source::Inferred,
        evidence: "no capture of those ports yet",
    },
    Rule {
        output: "show interfaces status",
        rule: "a port-channel no member bundles in: `notconnect unassigned auto auto N/A` on IOS XE and `notconnect unassigned auto auto ` on classic IOS (no Type there, its trailing space kept), whatever its mode; one that was up keeps its VLAN and the speed it ran, which a switch here never has",
        source: Source::Live,
        evidence: "the lab C9300 (17.6.4) and the lab C2960G (15.0(2)SE11), 2026-10-16; the lab WS-C2960X-24PS-L (15.2(7)E0a), 2026-10-29; a 2960-X capture at 15.2(7)E with a port-channel down at `full 1000`",
    },
    Rule {
        output: "show interfaces status",
        rule: "an access port whose VLAN the database lacks shows that VLAN, `connected` or `notconnect`, never `inactive`; IOS XE shows an LACP member it has not bundled `suspended`, duplex `auto`; AppGigabitEthernet up `a-full a-1000 App-hosting port` (the cores' 9400s print Type `unknown`)",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show interfaces status",
        rule: "a cabled port `connected`, Vlan `trunk` while it trunks; a transceiver's `full <speed> <type>`, copper `a-full a-<speed>`, a bundle `a-full a-<member speed> N/A`",
        source: Source::Live,
        evidence: "both cores cabled as they ran (`vswitch lab`): every row the configuration decides, 140 and 142",
    },
    Rule {
        output: "show interfaces status",
        rule: "a bundle whose members' speed is set prints that speed fixed (`a-full 1000`); a C9200L's 1G optic `full 1000 1000BaseLX SFP`",
        source: Source::Live,
        evidence: NEW,
    },
    Rule {
        output: "show interfaces status",
        rule: "a copper SFP (GLC-T, GLC-TE) negotiates (`a-full a-1000 10/100/1000BaseTX SFP`); a far end on half duplex `a-half`; a C9300's and C9300L's AppGigabitEthernet1/0/1 listed after slot 1, shut `disabled routed auto auto App-hosting port`",
        source: Source::Live,
        evidence: BATCH,
    },
    Rule {
        output: "show interfaces status",
        rule: "a transceiver's Type name from its part number (SFP-10G-SR is SFP-10GBase-SR)",
        source: Source::Inferred,
        evidence: "the names are the cores'; the part numbers are the parts' own words; `show inventory` beside the table settles it",
    },
    Rule {
        output: "show interfaces status",
        rule: "GLC-LH-SMD is 1000BaseLX SFP and a 1000BASE-SX optic 1000BaseSX SFP",
        source: Source::Live,
        evidence: "c2960x 15.2(7)E capture 8: `show inventory` beside `show interfaces status`, 2026-10-06",
    },
    Rule {
        output: "show interfaces status",
        rule: "classic IOS: a 10-column Port; an empty SFP slot `auto auto Not Present`; a linked 1G optic `a-full a-1000`; no Type on a port-channel; FastEthernet0 last, `routed`, `10/100BaseTX`",
        source: Source::Live,
        evidence: CLASSIC_2960X,
    },
    Rule {
        output: "show interfaces status",
        rule: "a redundant SUP-1XL pair with no QSFP port enabled: ports 1-4 run, 5-8, 9 and 10 inactive",
        source: Source::Live,
        evidence: CORES,
    },
    Rule {
        output: "show interfaces status",
        rule: "a supervisor QSFP port runs once `enable` turns it on and turns its SFP ports off (SUP-1: 1-4 or 9, 5-8 or 10; SUP-2: 1-4 or 5); a pair never runs SUP-1 5-8 and 10 or SUP-2 7-8",
        source: Source::Example,
        evidence: "Catalyst 9400 Supervisor Module Installation Note, uplink port configuration",
    },
    Rule {
        output: "show vlan brief",
        rule: "layout, names, 1002-1005; a port listed under its access VLAN until it trunks; the list wrapped between ports",
        source: Source::Live,
        evidence: "both cores cabled as they ran: both tables exactly, 2026-10-05",
    },
    Rule {
        output: "show vlan brief",
        rule: "a member bundled into an access port-channel is not listed; its port-channel is",
        source: Source::Live,
        evidence: "c2960x 15.2(7)E capture 7: Po3 listed under VLAN 410, its four members not",
    },
    Rule {
        output: "show vlan brief",
        rule: "the wrap limit is the console's width: four ports a line at `terminal width 80`, eight on one line at 132 (the cores wrapped at 156 or 157 on a wider console)",
        source: Source::Live,
        evidence: C2960G,
    },
    Rule {
        output: "show vlan brief",
        rule: "a port-channel with no member bundled is not listed; the management port never is",
        source: Source::Live,
        evidence: CLASSIC_2960X,
    },
    Rule {
        output: "show vlan brief",
        rule: "a port-channel not trunking is listed under its access VLAN",
        source: Source::Live,
        evidence: "c2960x 15.2(7)E capture 7: Po3 under VLAN 410",
    },
    Rule {
        output: "show etherchannel summary",
        rule: "layout, flags legend, column widths",
        source: Source::Live,
        evidence: "c9200 17.6 capture 1 and c9200 17.15 capture 27: the legend in groups with blank lines between, a blank line after the table (the 9300 guide's example runs the groups together)",
    },
    Rule {
        output: "show etherchannel summary",
        rule: "classic IOS from 15.2: the legend adds `N` and `m` and spaces its groups; a blank line after the table; a down LACP bundle `(SD)` with a down member `(D)`",
        source: Source::Live,
        evidence: CLASSIC_2960X,
    },
    Rule {
        output: "show etherchannel summary",
        rule: "IOS 15.0: IOS XE's legend without `A - formed by Auto LAG`",
        source: Source::Live,
        evidence: C2960G,
    },
    Rule {
        output: "show etherchannel summary",
        rule: "with no link: the bundle `(SD)` or `(RD)`, each member `(D)`; LACP members cabled to each other in one channel do not bundle (`(SD)`, members flagged as a member with a link that does not bundle: `(s)` on IOS XE); no protocol prints ` -`",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show etherchannel summary",
        rule: "a bundle in use `(SU)` or `(RU)`, its bundled members `(P)`",
        source: Source::Live,
        evidence: "c2960x 15.2(7)E capture 7: Po3(SU), four members (P)",
    },
    Rule {
        output: "show etherchannel summary",
        rule: "classic IOS pads each member to 12, its trailing spaces kept, and the members wrap under the Ports column before column 79",
        source: Source::Live,
        evidence: "c2960x 15.2(7)E capture 7: Po3's fourth member on its own line (its log trimmed); the lab C2960G (15.0(2)SE11) and the lab WS-C2960X-24PS-L (15.2(7)E0a, 2026-10-29): `Gi1/0/3(D)  Gi1/0/4(D)  `",
    },
    Rule {
        output: "show etherchannel summary",
        rule: "a member with a link that does not bundle: `(s)` on IOS XE, where standalone-disable is the default, `(I)` under `no port-channel standalone-disable` and on classic IOS; IOS XE pads each member to 16, two a line, the rest under the Ports column at 35, at any terminal width, trailing spaces kept; classic IOS keeps a memberless group's padding",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show interfaces trunk",
        rule: "layout; Mode `on` for a trunk-mode port; nothing printed while no port trunks",
        source: Source::Live,
        evidence: "c9200 17.6 capture 1 (Mode on, the Port column 15 wide on IOS XE); c9200 17.15 capture 27 printed nothing with no trunk",
    },
    Rule {
        output: "show interfaces trunk",
        rule: "layout; Mode `auto` for a dynamic-auto port facing a trunk; every active VLAN forwarding; two VLANs in a row print as a range (`460-461`), where the running configuration writes `460,461`",
        source: Source::Live,
        evidence: CLASSIC_2960X,
    },
    Rule {
        output: "show interfaces trunk",
        rule: "Mode `desirable` or `auto` for a dynamic port; a port-channel in place of its members, Mode `on` when it inherited `switchport mode trunk`; IOS XE pads the header's `Native vlan` with two spaces",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show interfaces trunk",
        rule: "the active VLANs as the allowed ones the VLAN database holds; the forwarding list keeps those spanning tree forwards the port in, `none` on a port blocked in all of them",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show interfaces trunk",
        rule: "under MST every active VLAN is listed as forwarding",
        source: Source::Inferred,
        evidence: "MST is not modelled yet",
    },
    Rule {
        output: "show spanning-tree",
        rule: "PVST and Rapid PVST converged: a block per VLAN with a port up in it, three blank lines between; `rstp` or `ieee`; the bridge priority with the VLAN added; rows by port number, each ending `P2p ` with a space; a cable between two ports of one switch blocks the higher port ID as `Back BLK`; a port-channel is one row (Po1 128.3049, its members absent), costed 20000 long with one 1G member; `spanning-tree port-priority 64` and `cost 100` (`64.4`, cost 100); a port hearing BPDUs is not edge whatever its PortFast; Ap1/0/1 is port 41 on a C9300-24T and keeps VLAN 1 alive; 1G costs 20000 (long, the default at 17.6.4); `active` and `vlan <n>` print the same blocks",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show spanning-tree",
        rule: "a bridge that is not the root adds `Cost` and `Port <n> (<name>)` under the root's address; 1G costs 4 (short, the default at 15.0(2)SE); the 2960G's ports are numbered by their place",
        source: Source::Live,
        evidence: C2960G,
    },
    Rule {
        output: "show spanning-tree",
        rule: "IOS XE's non-root block as the 2960G's; port numbers on other models and on stack members after the first (168 a member on IOS XE, 56 on classic IOS), costs at speeds other than 1G, `Shr` on half duplex or `spanning-tree link-type shared`, Aging Time from `mac address-table aging-time`, `Peer(STP)` on a Rapid PVST port facing PVST, `Edge` on PortFast hearing no BPDU; a Catalyst 9400's and a classic port-channel's port numbers are not known and those forms print no output model",
        source: Source::Inferred,
        evidence: "the 9300 command reference's example (Fif2/0/17 is 128.185), the 2960 15.2(2)E configuration guide's (Gi4/0/1 is 128.169, Po4 costs 3 short), 802.1t and 802.1D",
    },
    Rule {
        output: "show spanning-tree",
        rule: "a port forced to 10 Mb/s and half duplex costs 2000000 long, Type `Shr`; after `no speed` and `no duplex` it keeps the 2000000 cost with Type `P2p`",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show spanning-tree",
        rule: "the cost of the speed `no speed` cleared holds until the port is given a speed again, on every platform and release",
        source: Source::Inferred,
        evidence: "one measurement, 30 seconds after the change, on the lab C9300 at 17.6.4",
    },
    Rule {
        output: "show spanning-tree",
        rule: "BPDU guard: a port that hears a BPDU is err-disabled and leaves the tables, its cable's far end `notconnect` (`show interfaces status`); BPDU filter sends none and ignores what it hears, both ends designated forwarding",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show spanning-tree summary, root, bridge, blockedports, inconsistentports, pathcost method, interface",
        rule: "IOS XE's summary labels at two widths, EtherChannel misconfig guard first, `<n> vlans`; the root and bridge tables with a padded last column; the blocked list and its count; an empty inconsistency table; `Spanning tree default pathcost method used is long`; one port's rows by VLAN, and `enabled` under `portfast` while PortFast is set",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show spanning-tree summary",
        rule: "classic IOS pads its labels to one width and lists EtherChannel misconfig guard after Loopguard Default; `1 vlan` in the singular; `Root bridge for: none`",
        source: Source::Live,
        evidence: C2960G,
    },
    Rule {
        output: "show spanning-tree summary, mst configuration digest",
        rule: "from 15.2 classic IOS lists EtherChannel misconfig guard first; from 15.2(7)E it pads its labels to 39 and adds PVST Simulation Default (`enabled but inactive in rapid-pvst mode`) and Bridge Assurance before UplinkFast; the default region's digest `0xAC36177F50283CD4B83821D8AB26DE62` and pre-standard digest `0xBB3B6C15EF8D089BB55ED10D24DF44DE`, after `% Switch is not in mst mode` outside MST",
        source: Source::Live,
        evidence: LAB_2960X,
    },
    Rule {
        output: "show spanning-tree summary, blockedports",
        rule: "the root bridge list and a VLAN's blocked ports on one line however long; UplinkFast under Rapid PVST `enabled but inactive in rapid-pvst mode` as the reference prints BackboneFast; `summary totals` the header and the total line alone",
        source: Source::Inferred,
        evidence: "no capture roots more than three VLANs, blocks more than two ports in one or holds `summary totals`",
    },
    Rule {
        output: "show spanning-tree",
        rule: "`detail`, `interface <port> detail`, a VLAN with no instance, a port not in the tables and MST print no output model: no BPDU counters, topology changes or ages are invented",
        source: Source::Inferred,
        evidence: "the lab's counters and ages are its own history",
    },
    Rule {
        output: "show ip interface brief",
        rule: "layout and column widths",
        source: Source::Live,
        evidence: "c9200 17.6 capture 1 and c9200 17.15 capture 27: no blank line before the header",
    },
    Rule {
        output: "show ip interface brief",
        rule: "Vlan1 administratively down while no block configures it; up: loopbacks, AppGigabitEthernet, a port-channel with a bundled member; Method `manual` for an address typed at the console, and for `no ip address` typed after it; `ip address dhcp` typed reads `manual` while the interface is down and `DHCP` once it is up, with no lease; the factory default's `no ip address` `unset`; IOS XE prints a name wider than 22 short (Te1/1/1, Fo1/1/1, Ap1/0/1); a front-panel port's `no ip address` `unset`",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show ip interface brief",
        rule: "Method `DHCP` for `ip address dhcp` in a configuration loaded; classic IOS prints a name 22 characters or wider whole, followed by one space",
        source: Source::Inferred,
        evidence: "the reference's example; no capture has an address from DHCP or a classic name of 22 characters",
    },
    Rule {
        output: "show ip interface brief",
        rule: "Method NVRAM where the configuration gives an address or `no ip address`, unset on a switch port however configured and on a loopback with no address; up on a cabled port and on a VLAN interface its VLAN's running port carries; a VLAN interface whose VLAN exists and no running port carries is up, its protocol down; Protocol padded to 8",
        source: Source::Live,
        evidence: "c9200 17.6 capture 1 and c9200 17.15 capture 27 cabled as they ran; the six IE-3300s and the lab C2960G (15.0(2)SE11) over SSH",
    },
    Rule {
        output: "show power inline",
        rule: "layout; a module row with the default power supply's budget; an idle port `auto off 0.0 n/a n/a <max>`; UPOE's 60.0",
        source: Source::Example,
        evidence: "9300 17.15 command reference (a C9300-48UN, 610 W)",
    },
    Rule {
        output: "show power inline",
        rule: "a static port reserves its maximum in Power and Used with no device; `never` is Admin `off`; a stack lists every PoE member's row, then every port; PoE+ 30.0 and UPOE+ 90.0",
        source: Source::Inferred,
        evidence: "the reference's field descriptions; no capture of a static port or a stack",
    },
    Rule {
        output: "show power inline",
        rule: "nothing on a Catalyst 9200 or 9300 without PoE",
        source: Source::Live,
        evidence: "c9300 17.15 capture 5 (C9300-24S) and c9300 17.15 capture 1 (C9300-24T) at 17.15, and the C9200L-24T and -48T show techs at 17.15",
    },
    Rule {
        output: "show power inline",
        rule: "classic IOS: the 9300 layout; a PoE+ module row at its budget (370.0) and idle ports `auto off 0.0 n/a n/a 30.0`",
        source: Source::Live,
        evidence: CLASSIC_2960X,
    },
    Rule {
        output: "show vtp status",
        rule: "layout under VTP version 1 or 2; version, domain, pruning and mode from vlan.dat; traps from `snmp-server enable traps vtp`; the updater the lowest-numbered VLAN interface with an address; the VLANs counted with 1 and 1002-1005, an extended VLAN not counted; both digest lines end with a space on every platform, which a terminal log loses",
        source: Source::Live,
        evidence: "core-a, core-b (9400, 17.5), c2960x 15.2(7)E capture 8 (2960-X, 15.2(7)E0a), c2960 15.2(6)E capture 5 and c2960 15.2(6)E capture 6 (2960-Plus, 15.2(6)E1), the C2960G (15.0(2)SE11) and the lab C9300 (17.6.4)",
    },
    Rule {
        output: "show vtp status",
        rule: "versions 1 and 2 print the MD5 digest over 16 zero bytes, the summary advertisement (version, code 1, followers 0, the domain padded to 32 bytes, revision, updater, a zero timestamp and digest), each VLAN's subset record in VLAN order (1002 and 1003 with ring number and parent 0, 1004 and 1005 with bridge number 0 and STP ieee or ibm) and 16 zero bytes again; two rows of eight `0xNN ` bytes",
        source: Source::Live,
        evidence: "the lab C9300 (17.6.4) in nine VLAN databases, version 1, off and server, with and without a domain; the lab C2960G (15.0(2)SE11) at revision 4; core-a and core-b (9400, 17.5) at revision 263 with 50 named VLANs",
    },
    Rule {
        output: "show vtp status",
        rule: "version 2 puts 2 in the summary's version byte; a VLAN's `state suspend` its status 1 and its `mtu` the record's MTU; extended VLANs stay out; a VTP password's secret is unmodelled and that form answers the no-output-model line",
        source: Source::Inferred,
        evidence: "the advertisement's fields; no switch here ran version 2, a suspended VLAN, a VLAN MTU or a password",
    },
    Rule {
        output: "show vtp status",
        rule: "a server counts one revision per VLAN database change and stamps the updater and the time (`10-16-26 18:30:17`); a new domain, transparent or off mode and version 3 start the revision at 0; version 1 mode off prints the last change and no updater; version 3 prints Feature VLAN, MST and UNKNOWN blocks, MST and UNKNOWN transparent, the extended VLANs and 4096 VLANs, and on a server the revision, `Primary ID 0000.0000.0000` and an empty digest",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show vtp status",
        rule: "a VLAN submode's changes count once it is left, a range as one; a version 3 server counts nothing, being no primary server; a version 3 client prints a server's lines; a replayed configuration counts nothing",
        source: Source::Inferred,
        evidence: "the lab added one VLAN per change on a version 1 server and never made a primary server",
    },
    Rule {
        output: "show vtp counters, show vtp password",
        rule: "counters all 0 and a pruning row per trunking port; `The VTP password is not configured.`",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show vtp counters, show vtp password",
        rule: "no advertisement counted between VTP servers in a lab; a password set answers the no-output-model line",
        source: Source::Inferred,
        evidence: "the lab C9300 ran VTP off when its counters were read and set no password",
    },
    Rule {
        output: "show vtp status, show vtp interface, show vtp devices",
        rule: "on the 2960-X the `vtp interface` named is the updater (`(preferred interface)`) and a server adds `Preferred interface name is <name>  `; version 3 counts the normal range apart from the extended VLANs, keeps 1005 and prints no Feature UNKNOWN block; `show vtp interface` lists each front port and port-channel, `disabled` under `no vtp`; `show vtp devices` below version 3 says it needs version 3",
        source: Source::Live,
        evidence: "the lab WS-C2960X-24PS-L (15.2(7)E0a) over SSH, transparent at versions 1 to 3, off at 3 and a server at 2, 2026-10-29",
    },
    Rule {
        output: "show vtp status",
        rule: "from IOS XE 17.6 the pruning line adds its operational state (`Enabled (Operationally Enabled)`); a transparent switch names no updater",
        source: Source::Live,
        evidence: "c9200 17.6 capture 1 (17.6.4) and c9200 17.15 capture 27 (17.15.2); the cores at 17.5 print `Enabled` alone",
    },
    Rule {
        output: "show vtp status",
        rule: "the updater skips a VLAN interface with no address (DHCP with no lease); with none, the first layer 3 interface with an address (`on interface Lo0 (first layer3 interface found)`); VTP mode off names no updater",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show vtp status",
        rule: "Device ID is the base MAC address `show version` names",
        source: Source::Live,
        evidence: "13 captures that print both, 2960-Plus, 2960-X, C2960G, C9200L and IE-3300, 2026-10-16",
    },
    Rule {
        output: "show vtp status",
        rule: "a new VLAN database: `0.0.0.0 at 0-0-00 00:00:00`, revision 0",
        source: Source::Live,
        evidence: "the lab C9300 (17.6.4) after vlan.dat was deleted and the switch reloaded, 2026-10-16",
    },
    Rule {
        output: "show running-config",
        rule: "`spanning-tree vlan` lines in VLAN order; `root primary` stored as `priority 24576`",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "show running-config",
        rule: "`root secondary` stored as `priority 28672`, and `root primary` as 24576 whatever the other bridges' priorities",
        source: Source::Inferred,
        evidence: "the command reference lowers the priority below a better root's; the lab switch was alone",
    },
    Rule {
        output: "show interfaces status",
        rule: "port names on mixed-speed models: C9300X-48HXN 1-40 FiveGigabitEthernet and 41-48 TenGigabitEthernet; the 9300L UXG, 9300LM UX and 9200L PXG mGig ports last",
        source: Source::Inferred,
        evidence: "the installation guide and data sheet give the counts only (the C9300-48UXM's figure numbers its ports)",
    },
    Rule {
        output: "?",
        rule: "layout: two-space indent, padded column, `<cr>` last",
        source: Source::Live,
        evidence: "core-b `show vtp ?` (and the books' 141 examples)",
    },
    Rule {
        output: "?",
        rule: "each description: the command reference's Syntax Description text, not IOS's help string",
        source: Source::Inferred,
        evidence: "IOS's strings differ (`show vtp ?` prints `VTP statistics`); `vswitch capture` asks a switch the show prefixes, and its answers replace these",
    },
    Rule {
        output: "?",
        rule: "a line a real switch of the platform and release was asked prints that switch's answer whole",
        source: Source::Live,
        evidence: "core-b `show vtp ?` (9400 17.5); `vswitch capture` adds the show prefixes it asks",
    },
    Rule {
        output: "?",
        rule: "an error answer (`% Unrecognized command`) is that switch's configuration, not its release's help, and is not kept",
        source: Source::Inferred,
        evidence: "c9200 17.15 capture 27's `show ?` at `>`, from its privilege lines",
    },
    Rule {
        output: "% Invalid input",
        rule: "the caret stands past the longest start of the refused word some keyword there begins with (`show config` at `>`: under the `f`, as `controllers` begins `con`), from the switch's own `?` answer where one was captured",
        source: Source::Live,
        evidence: CLASSIC_2960X,
    },
    Rule {
        output: "% Invalid input",
        rule: "the caret stands past the `bri` of `show spanning-tree vlan 1 brief` (IOS XE, `bridge` begins it) and past the `Po1` of `Po10` on a 2960G, which takes port channels 1 to 6",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "% Invalid input",
        rule: "a VLAN, loopback or tunnel typed with its number, and a port channel on another platform, is refused under the digit that takes it past the top; the other classic 2960s take port channels 1 to 6",
        source: Source::Inferred,
        evidence: "the lab C2960G's `Po10`, and the caret IOS gives a number past its range",
    },
    Rule {
        output: "% Invalid input",
        rule: "a command `privilege exec level <n>` raises above 1 is refused at `>` and its `?` answers `% Unrecognized command`; where the caret falls rests on the commands left at level 1, which no capture lists for c9200 17.15 capture 27",
        source: Source::Live,
        evidence: "c9200 17.15 capture 27 refuses `terminal length 0` and `show version` at `>`",
    },
    Rule {
        output: "| include, exclude, begin, section",
        rule: "a space typed at the end of the line is part of the expression (`| include Gi1/0/2 ` skips Gi1/0/20)",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "| section",
        rule: "a matching line prints with the lines indented under it, once; a matching indented line prints without its parent",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "| count",
        rule: "`Number of lines which match regexp = N`",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "write memory, copy running-config startup-config",
        rule: "`Destination filename [startup-config]?`, `Building configuration...`, `[OK]`",
        source: Source::Live,
        evidence: LAB,
    },
    Rule {
        output: "terminal length, terminal width",
        rule: "taken at `>` and `#`; the width is the one `show vlan brief` wraps to, 0 for none",
        source: Source::Live,
        evidence: "every capture types `terminal length 0` (the general command reference's, in no platform book)",
    },
];

/// The rules no switch has shown, as `verify --outputs` prints them.
pub fn unverified() -> Vec<String> {
    RULES
        .iter()
        .filter(|r| r.source != Source::Live)
        .map(|r| {
            let from = match r.source {
                Source::Example => "Cisco example",
                _ => "inferred",
            };
            format!("{}: {} ({from}: {})", r.output, r.rule, r.evidence)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_lists_only_what_no_switch_has_shown() {
        let list = unverified();
        assert_eq!(
            list.len(),
            RULES.iter().filter(|r| r.source != Source::Live).count()
        );
        assert!(list.iter().any(|l| l.starts_with("show interfaces trunk:")));
        assert!(list.iter().any(|l| l.starts_with("show spanning-tree:")));
        assert!(!list.iter().any(|l| l.contains("Device ID is the base MAC")));
        assert!(
            !list
                .iter()
                .any(|l| l.contains("a redundant SUP-1XL pair with no QSFP port enabled"))
        );
        // what a capture shows is not asked of hardware again: a non-PoE switch's empty table, the
        // C2960G's wrap at 80 and 132, the Method column
        for shown in [
            "nothing without PoE",
            "the wrap limit is the console's width",
            "Method NVRAM",
        ] {
            assert!(!list.iter().any(|l| l.contains(shown)), "{shown}");
        }
    }
}
