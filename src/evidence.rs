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
        rule: "a bundled member's Operational Mode names its bundle (`trunk (member of bundle Po1)`); IOS XE calls an LACP member it has not bundled, linked or not, `down (suspended member of bundle Po1)` with no operational encapsulation; a voice VLAN of `dot1p`, `untagged` or `none` as typed; the pruning lines read in turn as the allowed list reads, within 2-1001; one port's block alone with no blank line after it",
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
        rule: "VLANs and VTP lines print only in VTP transparent mode; a VTP server keeps them in vlan.dat; an unnamed VLAN's line ends with a space",
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
        rule: "`router` blocks after the last interface, before `ip forward-protocol` and `ip route`; `vlan access-map` blocks among the global blocks in every VTP mode",
        source: Source::Inferred,
        evidence: "no capture holds a routing process or a VLAN access map",
    },
    Rule {
        output: "show running-config",
        rule: "a `vlan configuration` with nothing under it is not kept; a blank line after `end`, the interface form's too; IOS XE keeps `no port-channel standalone-disable` under a port-channel; a member's `channel-group` typed makes its port-channel with the member's switch port lines",
        source: Source::Live,
        evidence: LAB,
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
        rule: "VTP mode off (version 3) prints them as transparent mode does",
        source: Source::Inferred,
        evidence: "no document or capture of mode off",
    },
    Rule {
        output: "show running-config",
        rule: "default lines of a new switch (`vrf definition Mgmt-vrf`, GigabitEthernet0/0's, `service timestamps`)",
        source: Source::Inferred,
        evidence: "not printed until configured; a new switch's capture settles them",
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
        rule: "a port-channel no member bundles in: `notconnect unassigned auto auto N/A` on IOS XE and on classic IOS before 15.2 (no Type there, its trailing space kept), whatever its mode; classic IOS from 15.2 shows the access VLAN",
        source: Source::Live,
        evidence: "the lab C9300 (17.6.4) and the lab C2960G (15.0(2)SE11), 2026-10-16; a 2960-X capture at 15.2",
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
        rule: "with no link: the bundle `(SD)` or `(RD)`, each member `(D)`; LACP members cabled to each other in one channel do not bundle (`(SD)`, members `(I)`); no protocol prints ` -`",
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
        rule: "the members wrap under the Ports column before column 79",
        source: Source::Live,
        evidence: "c2960x 15.2(7)E capture 7: Po3's fourth member on its own line",
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
        rule: "the active VLANs as the allowed ones the VLAN database holds, all forwarding (no spanning-tree blocking modelled)",
        source: Source::Inferred,
        evidence: "the lab C9300 shows `none` forwarding on a port spanning tree blocks; vswitch runs no spanning tree",
    },
    Rule {
        output: "show ip interface brief",
        rule: "layout and column widths",
        source: Source::Live,
        evidence: "c9200 17.6 capture 1 and c9200 17.15 capture 27: no blank line before the header",
    },
    Rule {
        output: "show ip interface brief",
        rule: "Vlan1 administratively down while no block configures it; up: loopbacks, AppGigabitEthernet, a port-channel with a bundled member; Method `manual` for an address typed at the console, DHCP's with no lease too; IOS XE prints a name wider than 22 short (Te1/1/1, Fo1/1/1, Ap1/0/1); a front-panel port's `no ip address` `unset`",
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
        rule: "layout under VTP version 1 or 2; version, domain, pruning and mode from vlan.dat; traps from `snmp-server enable traps vtp`; the updater the lowest-numbered VLAN interface with an address; the VLANs counted with 1 and 1002-1005; both digest lines end with a space on every platform, which a terminal log loses; version 3 has no output model",
        source: Source::Live,
        evidence: "core-a, core-b (9400, 17.5), c2960x 15.2(7)E capture 8 (2960-X, 15.2(7)E0a), c2960 15.2(6)E capture 5 and c2960 15.2(6)E capture 6 (2960-Plus, 15.2(6)E1) and the C2960G (15.0(2)SE11); every capture runs version 1",
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
        rule: "Device ID from the hostname, the database never modified (`0.0.0.0 at 0-0-00 00:00:00`), revision 0, and a digest of the domain and VLANs: a virtual switch has no burned-in address, change history or vlan.dat",
        source: Source::Inferred,
        evidence: "what a switch holds outside its configuration",
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
