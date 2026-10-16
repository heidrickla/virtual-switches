//! The hardware a stack is made of: each member's model and network module, and the interfaces
//! they give. IOS refuses an interface the stack does not have; `Stack::problems` names hardware
//! Cisco does not build or stack.

use crate::release::Release;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Ports as interface type, first and last port.
pub type Ports = &'static [(&'static str, u32, u32)];

/// A model's family decides its stack partners; the Catalyst 9400 is a chassis whose cards count as
/// members by slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    C9300X,
    C9300,
    C9300L,
    C9300LM,
    /// the high-scale B models, which stack only with each other (data sheets)
    C9300B,
    C9400,
    C9200,
    C9200L,
    C9200B,
    /// classic IOS 15.2: uplinks numbered on slot 0 after the front ports, FastEthernet0 for management
    C2960X,
    /// classic IOS 12.2 and 15.2, one switch, no management port, ports named `<type>0/<port>`: the
    /// 2960 and 2960G, the 2960-Plus and the 2960-C
    C2960,
    C2960Plus,
    C2960C,
    /// IOS XE on an industrial switch: one switch, no management port, ports named
    /// `<type>1/<port>` (the IE-3300's base unit is slot 1)
    IE3300,
    /// classic IOS on industrial switches, one switch, no management port: the IE 3000 names its
    /// ports `<type>1/<port>`, the IE 3010 `<type>0/<port>`
    IE3000,
    IE3010,
}

impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Family::C9300X => "Catalyst 9300X",
            Family::C9300 => "Catalyst 9300",
            Family::C9300L => "Catalyst 9300L",
            Family::C9300LM => "Catalyst 9300LM",
            Family::C9300B => "Catalyst 9300 B",
            Family::C9400 => "Catalyst 9400",
            Family::C9200 => "Catalyst 9200",
            Family::C9200L => "Catalyst 9200L",
            Family::C9200B => "Catalyst 9200 B",
            Family::C2960X => "Catalyst 2960-X",
            Family::C2960 => "Catalyst 2960",
            Family::C2960Plus => "Catalyst 2960-Plus",
            Family::C2960C => "Catalyst 2960-C",
            Family::IE3300 => "Catalyst IE3300",
            Family::IE3000 => "Cisco IE 3000",
            Family::IE3010 => "Cisco IE 3010",
        }
    }

    /// How many members a stack takes (a 9400 chassis: its slots; a 2960 does not stack).
    pub fn max_members(self) -> usize {
        match self {
            Family::C9400 => 10,
            f if f.two_part() => 1,
            _ => 8,
        }
    }

    /// Whether the family names its ports `<type><slot>/<port>`, with no member number.
    pub fn two_part(self) -> bool {
        matches!(
            self,
            Family::C2960
                | Family::C2960Plus
                | Family::C2960C
                | Family::IE3300
                | Family::IE3000
                | Family::IE3010
        )
    }

    /// The slot a two-part name gives the front panel: a 2960's 0, an IE-3300's 1.
    pub fn front_slot(self) -> u32 {
        match self {
            Family::IE3300 | Family::IE3000 => 1,
            _ => 0,
        }
    }

    /// The families that run classic IOS, whose show output is laid out apart from IOS XE's.
    pub fn classic_ios(self) -> bool {
        matches!(
            self,
            Family::C2960X
                | Family::C2960
                | Family::C2960Plus
                | Family::C2960C
                | Family::IE3000
                | Family::IE3010
        )
    }

    /// The 2960, 2960-Plus and 2960-C: classic IOS with two-part names.
    pub fn classic_two_part(self) -> bool {
        matches!(self, Family::C2960 | Family::C2960Plus | Family::C2960C)
    }

    /// Slot-1 interfaces IOS lists whatever is fitted; empty where only the uplinks' own are known.
    pub fn slot1(self) -> Ports {
        match self {
            Family::C9300 | Family::C9300B => C9300_SLOT1,
            _ => &[],
        }
    }

    /// Catalyst 9300 data sheet, supported stacking options: the 9300X stacks with the 9300 at
    /// StackWise-480, the 9300L with the 9300LM at StackWise-320. Every other family stacks only
    /// with itself (Catalyst 9200 data sheet), the B models among them ("These SKUs cannot be
    /// stacked with C9200 SKUs with 4VNs").
    pub fn stacks_with(self, other: Family) -> bool {
        use Family::*;
        self == other
            || matches!(
                (self, other),
                (C9300X, C9300) | (C9300, C9300X) | (C9300L, C9300LM) | (C9300LM, C9300L)
            )
    }
}

/// What a port takes (`Stack::media`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Media {
    /// RJ-45, 10/100/1000
    Copper,
    /// a 1G SFP slot
    Sfp,
    /// a 10G SFP+ slot on a 9400 line card
    SfpPlus,
    /// a 9400 supervisor's uplink
    Uplink,
    /// a port no capture shows unplugged yet
    Other,
}

/// A transceiver: its part number, the speed it runs (Mb/s) and how `show interfaces status` names
/// it in the Type column. The names are as the cores printed them (17.5); a part number maps to its
/// name by the part's own words (SFP-10G-SR is 10GBASE-SR).
#[derive(Debug, PartialEq)]
pub struct Optic {
    pub part: &'static str,
    pub mbps: u32,
    pub shows: &'static str,
    /// a copper SFP, which negotiates its link as an RJ-45 port does (`a-full a-1000`)
    pub negotiates: bool,
}

pub const OPTICS: &[Optic] = &[
    Optic {
        part: "SFP-10G-SR",
        mbps: 10_000,
        shows: "SFP-10GBase-SR",
        negotiates: false,
    },
    Optic {
        part: "SFP-10G-SR-S",
        mbps: 10_000,
        shows: "SFP-10GBase-SR",
        negotiates: false,
    },
    Optic {
        part: "SFP-10G-LR",
        mbps: 10_000,
        shows: "SFP-10GBase-LR",
        negotiates: false,
    },
    Optic {
        part: "SFP-10G-LR-S",
        mbps: 10_000,
        shows: "SFP-10GBase-LR",
        negotiates: false,
    },
    Optic {
        part: "GLC-SX-MMD",
        mbps: 1000,
        shows: "1000BaseSX SFP",
        negotiates: false,
    },
    Optic {
        part: "GLC-LH-SMD",
        mbps: 1000,
        shows: "1000BaseLX SFP",
        negotiates: false,
    },
    Optic {
        part: "GLC-FE-100FX",
        mbps: 100,
        shows: "100BaseFX SFP",
        negotiates: false,
    },
    // copper SFPs (C9200Ls and C9300s at 17.15 show `10/100/1000BaseTX SFP`, `a-full a-1000`)
    Optic {
        part: "GLC-T",
        mbps: 1000,
        shows: "10/100/1000BaseTX SFP",
        negotiates: true,
    },
    Optic {
        part: "GLC-TE",
        mbps: 1000,
        shows: "10/100/1000BaseTX SFP",
        negotiates: true,
    },
];

pub fn optic(part: &str) -> Option<&'static Optic> {
    OPTICS.iter().find(|o| o.part.eq_ignore_ascii_case(part))
}

/// A speed as IOS's Speed column prints it: `10G` from 10 Gb/s, Mb/s below.
pub fn speed_text(mbps: u32) -> String {
    if mbps >= 10_000 && mbps.is_multiple_of(1000) {
        format!("{}G", mbps / 1000)
    } else {
        mbps.to_string()
    }
}

/// What a model's slot-1 uplinks are: a network module from a list, ports built in, or none.
#[derive(Clone, Copy, Debug)]
pub enum Uplinks {
    Modules(&'static [&'static str]),
    Fixed(Ports),
    /// built-in uplinks numbered on slot 0 after the front ports (a 2960-X's `Gi1/0/25-28`, `Te1/0/1-2`)
    Slot0(Ports),
    None,
}

#[derive(Clone, Copy, Debug)]
pub struct Model {
    pub name: &'static str,
    pub family: Family,
    /// the IOS XE release Cisco introduced it in (release notes); None where vswitch has no date
    pub since: Option<&'static str>,
    /// the last IOS XE release that runs it (release notes); None while the newest does
    pub until: Option<&'static str>,
    pub ports: Ports,
    pub uplinks: Uplinks,
    /// a fact about the model the documents do not settle, shown where the model is chosen
    pub note: Option<&'static str>,
    /// PoE on every front-panel port, with the budget its default power supply gives
    pub poe: Option<Poe>,
}

/// The PoE a model's front-panel ports give.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Poe {
    pub kind: PoeKind,
    /// watts available to the ports with the model's default power supply (hardware installation
    /// guide and data sheet); none for a 9400 card, whose chassis power supplies set the budget
    pub watts: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PoeKind {
    PoePlus,
    Upoe,
    UpoePlus,
}

impl PoeKind {
    /// The most a port gives, in milliwatts: PoE+ 30 W, UPOE 60 W, UPOE+ 90 W (the 9300 command
    /// reference's `show power inline` prints 60.0 in the Max column of a UPOE port).
    pub fn max_mw(self) -> u32 {
        match self {
            PoeKind::PoePlus => 30_000,
            PoeKind::Upoe => 60_000,
            PoeKind::UpoePlus => 90_000,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Module {
    pub name: &'static str,
    pub family: Family,
    /// the IOS XE release Cisco introduced it in (release notes)
    pub since: Option<&'static str>,
    pub ports: Ports,
    /// its 40G ports take a 4x10G breakout cable onto the slot's 10G interfaces (hardware
    /// installation guides)
    pub breakout: bool,
    /// its ports are RJ-45 Multigigabit, not transceiver slots (9300 data sheet: the Multigigabit
    /// network modules; Multigigabit ports take RJ-45 connectors)
    pub copper: bool,
}

/// Slot-1 interfaces IOS lists on a Catalyst 9300 member whatever network module is fitted, in its
/// order (9300 command reference and configuration guide example output). The hardware installation
/// guide marks those a module does not use "Disregard"; IOS takes their configuration.
const C9300_SLOT1: Ports = &[
    ("GigabitEthernet", 1, 4),
    ("TenGigabitEthernet", 1, 8),
    ("FortyGigabitEthernet", 1, 2),
    ("TwentyFiveGigE", 1, 2),
];

// Catalyst 9300 17.15 release notes, supported network modules: C9300X-NM modules only on C9300X
// SKUs, C9300-NM and C3850-NM modules only on C9300 SKUs. Data sheet: C9300X-NM-4C "compatible only
// with the C9300X-48HX, C9300X-48TX, and C9300X-24Y models".
const C9300X_MODULES: &[&str] = &[
    "C9300X-NM-4C",
    "C9300X-NM-2C",
    "C9300X-NM-8Y",
    "C9300X-NM-8M",
];
const C9300X_NO_4C_MODULES: &[&str] = &["C9300X-NM-2C", "C9300X-NM-8Y", "C9300X-NM-8M"];
const C9300_MODULES: &[&str] = &[
    "C9300-NM-4G",
    "C9300-NM-4M",
    "C9300-NM-8X",
    "C9300-NM-2Q",
    "C9300-NM-2Y",
    "C3850-NM-4-1G",
    "C3850-NM-2-10G",
    "C3850-NM-4-10G",
    "C3850-NM-8-10G",
    "C3850-NM-2-40G",
];
/// Network module ports a model disables (release notes, Catalyst 9300X-HXN, 17.8.1 and later).
const DISABLED_MODULE_PORTS: &[(&str, &[&str], u32, u32)] =
    &[("C9300X-48HXN", &["C9300X-NM-8Y", "C9300X-NM-8M"], 7, 8)];
// Cisco Catalyst 9200 data sheet, network modules: C9200-NM-2Y and -2Q "Supported only on
// C9200-24PXG, C9200-48PXG"; C9200-NM-4G "Not supported on C9200-24PXG, C9200-48PXG".
const C9200_MODULES: &[&str] = &["C9200-NM-4G", "C9200-NM-4X"];
const C9200_PXG_MODULES: &[&str] = &["C9200-NM-4X", "C9200-NM-2Y", "C9200-NM-2Q"];
/// Catalyst 2960-X data sheet, configurations table: the LAN Lite models and the 24PSQ-L have no
/// FlexStack-Plus.
const NO_FLEXSTACK: &[&str] = &[
    "WS-C2960X-48TS-LL",
    "WS-C2960X-24TS-LL",
    "WS-C2960X-24PSQ-L",
];
/// A Catalyst 9400 chassis: the slots its supervisors and line cards go in (supervisor data sheet).
pub struct Chassis {
    pub name: &'static str,
    pub supervisors: &'static [u32],
    pub cards: &'static [u32],
    pub slots_text: &'static str,
}

pub const CHASSIS: &[Chassis] = &[
    Chassis {
        name: "C9404R",
        supervisors: &[2, 3],
        cards: &[1, 4],
        slots_text: "1 and 4",
    },
    Chassis {
        name: "C9407R",
        supervisors: &[3, 4],
        cards: &[1, 2, 5, 6, 7],
        slots_text: "1, 2 and 5 to 7",
    },
    Chassis {
        name: "C9410R",
        supervisors: &[5, 6],
        cards: &[1, 2, 3, 4, 7, 8, 9, 10],
        slots_text: "1 to 4 and 7 to 10",
    },
];

/// Whether a model is a Catalyst 9400 supervisor.
pub fn is_supervisor(name: &str) -> bool {
    name.to_ascii_uppercase().contains("-SUP-")
}

// Catalyst 9400 port names the documents do not settle: Cisco's books never tie these cards to
// interface names, so each takes the name IOS gives its port speed.
const SUP_25G_NOTE: &str = "ports 1-8 taken as TwentyFiveGigE, ports 9-10 FortyGigabitEthernet (supervisor installation note numbers them; check show interfaces status on a real switch)";
const SUP_100G_NOTE: &str = "ports 1-4 taken as TwentyFiveGigE, ports 5-8 HundredGigE (supervisor installation note numbers them; check show interfaces status on a real switch)";
const LC_24XY_NOTE: &str = "the 20 SFP28 ports taken as TwentyFiveGigE 1-20 and the 4 SFP+ as TenGigabitEthernet 21-24; check show interfaces status on a real switch";
const LC_12QC_NOTE: &str =
    "taken as 12 FortyGigabitEthernet ports; its 100G mode (enable mode 100G) is not modelled";
const LC_5G_NOTE: &str =
    "its 5G mGig ports taken as FiveGigabitEthernet; check show interfaces status on a real switch";
const HXN_NOTE: &str = "its 40 5G mGig ports taken as FiveGigabitEthernet 1-40 and the 8 10G as TenGigabitEthernet 41-48 \
    (the installation guide gives the counts, not the numbers); check show interfaces status on a real switch";
const MGIG_LAST_NOTE: &str = "the mGig port numbers are not in Cisco's documents: taken as the last ports, \
    TenGigabitEthernet; check show interfaces status on a real switch";
const PXG_NOTE: &str = "the mGig port numbers are not in Cisco's documents: taken as the last eight ports, \
    TenGigabitEthernet; check show interfaces status on a real switch";

/// Every model, in the order a selector offers them. Catalyst 9300X, 9300, 9300L and 9300LM: the 9300
/// data sheet and hardware installation guide (ports, PoE budgets; the C9300-48UXM's front-panel
/// figure numbers its 2.5G and 10G ports). Catalyst 9400: NetBox device types for the line cards, the
/// supervisor's ports from two C9407R cores' running configurations (2026-10-05), PoE kinds from the
/// 9400 data sheet. Catalyst 9200 and 9200L: the 9200 data sheet and hardware installation guide (24 or
/// 48 ports `GigabitEthernet<member>/0/<port>`, uplinks in slot 1, PoE budgets). Releases: the 26.1
/// release notes' tables.
pub const MODELS: &[Model] = &[
    Model {
        name: "C9300X-24Y",
        family: Family::C9300X,
        since: Some("17.5"),
        until: None,
        ports: &[("TwentyFiveGigE", 1, 24)],
        uplinks: Uplinks::Modules(C9300X_MODULES),
        note: None,
        poe: None,
    },
    Model {
        name: "C9300X-12Y",
        family: Family::C9300X,
        since: Some("17.5"),
        until: None,
        ports: &[("TwentyFiveGigE", 1, 12)],
        uplinks: Uplinks::Modules(C9300X_NO_4C_MODULES),
        note: None,
        poe: None,
    },
    Model {
        name: "C9300X-24HX",
        family: Family::C9300X,
        since: Some("17.9"),
        until: None,
        ports: &[("TenGigabitEthernet", 1, 24)],
        uplinks: Uplinks::Modules(C9300X_NO_4C_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::UpoePlus,
            watts: Some(735),
        }),
    },
    Model {
        name: "C9300X-48HX",
        family: Family::C9300X,
        since: Some("17.5"),
        until: None,
        ports: &[("TenGigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9300X_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::UpoePlus,
            watts: Some(590),
        }),
    },
    Model {
        name: "C9300X-48HXN",
        family: Family::C9300X,
        since: Some("17.9"),
        until: None,
        ports: &[
            ("FiveGigabitEthernet", 1, 40),
            ("TenGigabitEthernet", 41, 48),
        ],
        uplinks: Uplinks::Modules(C9300X_NO_4C_MODULES),
        note: Some(HXN_NOTE),
        poe: Some(Poe {
            kind: PoeKind::UpoePlus,
            watts: Some(690),
        }),
    },
    Model {
        name: "C9300X-48TX",
        family: Family::C9300X,
        since: Some("17.5"),
        until: None,
        ports: &[("TenGigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9300X_MODULES),
        note: None,
        poe: None,
    },
    Model {
        name: "C9300-24T",
        family: Family::C9300,
        since: Some("16.5"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: None,
    },
    Model {
        name: "C9300-48T",
        family: Family::C9300,
        since: Some("16.5"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: None,
    },
    Model {
        name: "C9300-24S",
        family: Family::C9300,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: None,
    },
    Model {
        name: "C9300-48S",
        family: Family::C9300,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: None,
    },
    Model {
        name: "C9300-24P",
        family: Family::C9300,
        since: Some("16.5"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(445),
        }),
    },
    Model {
        name: "C9300-48P",
        family: Family::C9300,
        since: Some("16.5"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(437),
        }),
    },
    Model {
        name: "C9300-24U",
        family: Family::C9300,
        since: Some("16.5"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(830),
        }),
    },
    Model {
        name: "C9300-48U",
        family: Family::C9300,
        since: Some("16.5"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(822),
        }),
    },
    Model {
        name: "C9300-24H",
        family: Family::C9300,
        since: Some("17.2"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::UpoePlus,
            watts: Some(830),
        }),
    },
    Model {
        name: "C9300-48H",
        family: Family::C9300,
        since: Some("17.2"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::UpoePlus,
            watts: Some(822),
        }),
    },
    Model {
        name: "C9300-24UX",
        family: Family::C9300,
        since: Some("16.6"),
        until: None,
        ports: &[("TenGigabitEthernet", 1, 24)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(560),
        }),
    },
    Model {
        name: "C9300-48UXM",
        family: Family::C9300,
        since: Some("16.6"),
        until: None,
        ports: &[
            ("TwoGigabitEthernet", 1, 36),
            ("TenGigabitEthernet", 37, 48),
        ],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(490),
        }),
    },
    Model {
        name: "C9300-48UN",
        family: Family::C9300,
        since: Some("16.8"),
        until: None,
        ports: &[("FiveGigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(610),
        }),
    },
    Model {
        name: "C9300-24UB",
        family: Family::C9300B,
        since: Some("16.12"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(830),
        }),
    },
    Model {
        name: "C9300-48UB",
        family: Family::C9300B,
        since: Some("16.12"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(822),
        }),
    },
    Model {
        name: "C9300-24UXB",
        family: Family::C9300B,
        since: Some("16.12"),
        until: None,
        ports: &[("TenGigabitEthernet", 1, 24)],
        uplinks: Uplinks::Modules(C9300_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(560),
        }),
    },
    Model {
        name: "C9300L-24T-4G",
        family: Family::C9300L,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Fixed(&[("GigabitEthernet", 1, 4)]),
        note: None,
        poe: None,
    },
    Model {
        name: "C9300L-48T-4G",
        family: Family::C9300L,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("GigabitEthernet", 1, 4)]),
        note: None,
        poe: None,
    },
    Model {
        name: "C9300L-24T-4X",
        family: Family::C9300L,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: None,
        poe: None,
    },
    Model {
        name: "C9300L-48T-4X",
        family: Family::C9300L,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: None,
        poe: None,
    },
    Model {
        name: "C9300L-24P-4G",
        family: Family::C9300L,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Fixed(&[("GigabitEthernet", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(505),
        }),
    },
    Model {
        name: "C9300L-48P-4G",
        family: Family::C9300L,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("GigabitEthernet", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(505),
        }),
    },
    Model {
        name: "C9300L-24P-4X",
        family: Family::C9300L,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(505),
        }),
    },
    Model {
        name: "C9300L-48P-4X",
        family: Family::C9300L,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(505),
        }),
    },
    Model {
        name: "C9300L-48PF-4G",
        family: Family::C9300L,
        since: Some("16.12"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("GigabitEthernet", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(890),
        }),
    },
    Model {
        name: "C9300L-48PF-4X",
        family: Family::C9300L,
        since: Some("16.12"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(890),
        }),
    },
    Model {
        name: "C9300L-24UXG-4X",
        family: Family::C9300L,
        since: Some("16.12"),
        until: None,
        ports: &[("GigabitEthernet", 1, 16), ("TenGigabitEthernet", 17, 24)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: Some(MGIG_LAST_NOTE),
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(880),
        }),
    },
    Model {
        name: "C9300L-24UXG-2Q",
        family: Family::C9300L,
        since: Some("16.12"),
        until: None,
        ports: &[("GigabitEthernet", 1, 16), ("TenGigabitEthernet", 17, 24)],
        uplinks: Uplinks::Fixed(&[("FortyGigabitEthernet", 1, 2)]),
        note: Some(MGIG_LAST_NOTE),
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(722),
        }),
    },
    Model {
        name: "C9300L-48UXG-4X",
        family: Family::C9300L,
        since: Some("16.12"),
        until: None,
        ports: &[("GigabitEthernet", 1, 36), ("TenGigabitEthernet", 37, 48)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: Some(MGIG_LAST_NOTE),
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(675),
        }),
    },
    Model {
        name: "C9300L-48UXG-2Q",
        family: Family::C9300L,
        since: Some("16.12"),
        until: None,
        ports: &[("GigabitEthernet", 1, 36), ("TenGigabitEthernet", 37, 48)],
        uplinks: Uplinks::Fixed(&[("FortyGigabitEthernet", 1, 2)]),
        note: Some(MGIG_LAST_NOTE),
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(675),
        }),
    },
    Model {
        name: "C9300LM-48T-4Y",
        family: Family::C9300LM,
        since: Some("17.9"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("TwentyFiveGigE", 1, 4)]),
        note: None,
        poe: None,
    },
    Model {
        name: "C9300LM-24U-4Y",
        family: Family::C9300LM,
        since: Some("17.9"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Fixed(&[("TwentyFiveGigE", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(420),
        }),
    },
    Model {
        name: "C9300LM-48U-4Y",
        family: Family::C9300LM,
        since: Some("17.9"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("TwentyFiveGigE", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(790),
        }),
    },
    Model {
        name: "C9300LM-48UX-4Y",
        family: Family::C9300LM,
        since: Some("17.9"),
        until: None,
        ports: &[("GigabitEthernet", 1, 40), ("TenGigabitEthernet", 41, 48)],
        uplinks: Uplinks::Fixed(&[("TwentyFiveGigE", 1, 4)]),
        note: Some(MGIG_LAST_NOTE),
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: Some(790),
        }),
    },
    Model {
        name: "C9200-24T",
        family: Family::C9200,
        since: Some("16.10"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Modules(C9200_MODULES),
        note: None,
        poe: None,
    },
    Model {
        name: "C9200-48T",
        family: Family::C9200,
        since: Some("16.10"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9200_MODULES),
        note: None,
        poe: None,
    },
    Model {
        name: "C9200-24PXG",
        family: Family::C9200,
        since: Some("17.1"),
        until: None,
        ports: &[("GigabitEthernet", 1, 16), ("TenGigabitEthernet", 17, 24)],
        uplinks: Uplinks::Modules(C9200_PXG_MODULES),
        note: Some(PXG_NOTE),
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "C9200-48PXG",
        family: Family::C9200,
        since: Some("17.1"),
        until: None,
        ports: &[("GigabitEthernet", 1, 40), ("TenGigabitEthernet", 41, 48)],
        uplinks: Uplinks::Modules(C9200_PXG_MODULES),
        note: Some(PXG_NOTE),
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(740),
        }),
    },
    Model {
        name: "C9200-24P",
        family: Family::C9200,
        since: Some("16.10"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Modules(C9200_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "C9200-48P",
        family: Family::C9200,
        since: Some("16.10"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9200_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(740),
        }),
    },
    Model {
        name: "C9200-48PL",
        family: Family::C9200,
        since: Some("17.3"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9200_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "C9200-24PB",
        family: Family::C9200B,
        since: Some("17.2"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Modules(C9200_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "C9200-48PB",
        family: Family::C9200B,
        since: Some("17.2"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Modules(C9200_MODULES),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(740),
        }),
    },
    Model {
        name: "C9200L-24T-4G",
        family: Family::C9200L,
        since: Some("16.9"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Fixed(&[("GigabitEthernet", 1, 4)]),
        note: None,
        poe: None,
    },
    Model {
        name: "C9200L-48T-4G",
        family: Family::C9200L,
        since: Some("16.9"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("GigabitEthernet", 1, 4)]),
        note: None,
        poe: None,
    },
    Model {
        name: "C9200L-24T-4X",
        family: Family::C9200L,
        since: Some("16.9"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: None,
        poe: None,
    },
    Model {
        name: "C9200L-48T-4X",
        family: Family::C9200L,
        since: Some("16.9"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: None,
        poe: None,
    },
    Model {
        name: "C9200L-24P-4G",
        family: Family::C9200L,
        since: Some("16.9"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Fixed(&[("GigabitEthernet", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "C9200L-48P-4G",
        family: Family::C9200L,
        since: Some("16.9"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("GigabitEthernet", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(740),
        }),
    },
    Model {
        name: "C9200L-24P-4X",
        family: Family::C9200L,
        since: Some("16.9"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "C9200L-48P-4X",
        family: Family::C9200L,
        since: Some("16.9"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(740),
        }),
    },
    Model {
        name: "C9200L-48PL-4G",
        family: Family::C9200L,
        since: Some("17.3"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("GigabitEthernet", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "C9200L-48PL-4X",
        family: Family::C9200L,
        since: Some("17.3"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "C9200L-24PXG-4X",
        family: Family::C9200L,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 16), ("TenGigabitEthernet", 17, 24)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: Some(PXG_NOTE),
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "C9200L-48PXG-4X",
        family: Family::C9200L,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 36), ("TenGigabitEthernet", 37, 48)],
        uplinks: Uplinks::Fixed(&[("TenGigabitEthernet", 1, 4)]),
        note: Some(MGIG_LAST_NOTE),
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(740),
        }),
    },
    Model {
        name: "C9200L-24PXG-2Y",
        family: Family::C9200L,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 16), ("TenGigabitEthernet", 17, 24)],
        uplinks: Uplinks::Fixed(&[("TwentyFiveGigE", 1, 2)]),
        note: Some(PXG_NOTE),
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "C9200L-48PXG-2Y",
        family: Family::C9200L,
        since: Some("16.11"),
        until: None,
        ports: &[("GigabitEthernet", 1, 40), ("TenGigabitEthernet", 41, 48)],
        uplinks: Uplinks::Fixed(&[("TwentyFiveGigE", 1, 2)]),
        note: Some(PXG_NOTE),
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(740),
        }),
    },
    Model {
        name: "C9400-SUP-1",
        family: Family::C9400,
        since: Some("16.6"),
        until: Some("17.18"),
        ports: &[
            ("TenGigabitEthernet", 1, 8),
            ("FortyGigabitEthernet", 9, 10),
            ("AppGigabitEthernet", 1, 1),
        ],
        uplinks: Uplinks::None,
        note: None,
        poe: None,
    },
    Model {
        name: "C9400-SUP-1XL",
        family: Family::C9400,
        since: Some("16.6"),
        until: Some("17.18"),
        ports: &[
            ("TenGigabitEthernet", 1, 8),
            ("FortyGigabitEthernet", 9, 10),
            ("AppGigabitEthernet", 1, 1),
        ],
        uplinks: Uplinks::None,
        note: None,
        poe: None,
    },
    Model {
        name: "C9400-SUP-1XL-Y",
        family: Family::C9400,
        since: Some("16.9"),
        until: Some("17.18"),
        ports: &[
            ("TwentyFiveGigE", 1, 8),
            ("FortyGigabitEthernet", 9, 10),
            ("AppGigabitEthernet", 1, 1),
        ],
        uplinks: Uplinks::None,
        note: Some(SUP_25G_NOTE),
        poe: None,
    },
    Model {
        name: "C9400X-SUP-2",
        family: Family::C9400,
        since: Some("17.7"),
        until: None,
        ports: &[
            ("TwentyFiveGigE", 1, 4),
            ("HundredGigE", 5, 8),
            ("AppGigabitEthernet", 1, 1),
        ],
        uplinks: Uplinks::None,
        note: Some(SUP_100G_NOTE),
        poe: None,
    },
    Model {
        name: "C9400X-SUP-2XL",
        family: Family::C9400,
        since: Some("17.7"),
        until: None,
        ports: &[
            ("TwentyFiveGigE", 1, 4),
            ("HundredGigE", 5, 8),
            ("AppGigabitEthernet", 1, 1),
        ],
        uplinks: Uplinks::None,
        note: Some(SUP_100G_NOTE),
        poe: None,
    },
    Model {
        name: "C9400-LC-24S",
        family: Family::C9400,
        since: Some("16.8"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::None,
        note: None,
        poe: None,
    },
    Model {
        name: "C9400-LC-48S",
        family: Family::C9400,
        since: Some("16.8"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::None,
        note: None,
        poe: None,
    },
    Model {
        name: "C9400-LC-24XS",
        family: Family::C9400,
        since: Some("16.6"),
        until: None,
        ports: &[("TenGigabitEthernet", 1, 24)],
        uplinks: Uplinks::None,
        note: None,
        poe: None,
    },
    Model {
        name: "C9400-LC-48XS",
        family: Family::C9400,
        since: Some("17.8"),
        until: None,
        ports: &[("TenGigabitEthernet", 1, 48)],
        uplinks: Uplinks::None,
        note: None,
        poe: None,
    },
    Model {
        name: "C9400-LC-24XY",
        family: Family::C9400,
        since: Some("17.12"),
        until: None,
        ports: &[("TwentyFiveGigE", 1, 20), ("TenGigabitEthernet", 21, 24)],
        uplinks: Uplinks::None,
        note: Some(LC_24XY_NOTE),
        poe: None,
    },
    Model {
        name: "C9400-LC-12QC",
        family: Family::C9400,
        since: Some("17.12"),
        until: None,
        ports: &[("FortyGigabitEthernet", 1, 12)],
        uplinks: Uplinks::None,
        note: Some(LC_12QC_NOTE),
        poe: None,
    },
    Model {
        name: "C9400-LC-48T",
        family: Family::C9400,
        since: Some("16.6"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::None,
        note: None,
        poe: None,
    },
    Model {
        name: "C9400-LC-48TX",
        family: Family::C9400,
        since: Some("17.13"),
        until: None,
        ports: &[("TenGigabitEthernet", 1, 48)],
        uplinks: Uplinks::None,
        note: None,
        poe: None,
    },
    Model {
        name: "C9400-LC-48U",
        family: Family::C9400,
        since: Some("16.6"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::None,
        note: None,
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: None,
        }),
    },
    Model {
        name: "C9400-LC-48P",
        family: Family::C9400,
        since: Some("16.8"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::None,
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: None,
        }),
    },
    Model {
        name: "C9400-LC-48H",
        family: Family::C9400,
        since: Some("16.12"),
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::None,
        note: None,
        poe: Some(Poe {
            kind: PoeKind::UpoePlus,
            watts: None,
        }),
    },
    Model {
        name: "C9400-LC-48UX",
        family: Family::C9400,
        since: Some("16.6"),
        until: None,
        ports: &[("GigabitEthernet", 1, 24), ("TenGigabitEthernet", 25, 48)],
        uplinks: Uplinks::None,
        note: None,
        poe: Some(Poe {
            kind: PoeKind::Upoe,
            watts: None,
        }),
    },
    Model {
        name: "C9400-LC-48HN",
        family: Family::C9400,
        since: Some("17.5"),
        until: None,
        ports: &[("FiveGigabitEthernet", 1, 48)],
        uplinks: Uplinks::None,
        note: Some(LC_5G_NOTE),
        poe: Some(Poe {
            kind: PoeKind::UpoePlus,
            watts: None,
        }),
    },
    Model {
        name: "C9400-LC-48HX",
        family: Family::C9400,
        since: Some("17.8"),
        until: None,
        ports: &[("TenGigabitEthernet", 1, 48)],
        uplinks: Uplinks::None,
        note: None,
        poe: Some(Poe {
            kind: PoeKind::UpoePlus,
            watts: None,
        }),
    },
    // Catalyst 2960-X data sheet (models, uplinks, PoE+ budgets); a 24PS-L's running configuration
    // (2026-10-06) names its ports Gi1/0/1-28 and its management port FastEthernet0
    Model {
        name: "WS-C2960X-48FPD-L",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Slot0(&[("TenGigabitEthernet", 1, 2)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(740),
        }),
    },
    Model {
        name: "WS-C2960X-48LPD-L",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Slot0(&[("TenGigabitEthernet", 1, 2)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "WS-C2960X-24PD-L",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Slot0(&[("TenGigabitEthernet", 1, 2)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "WS-C2960X-48TD-L",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Slot0(&[("TenGigabitEthernet", 1, 2)]),
        note: None,
        poe: None,
    },
    Model {
        name: "WS-C2960X-24TD-L",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Slot0(&[("TenGigabitEthernet", 1, 2)]),
        note: None,
        poe: None,
    },
    Model {
        name: "WS-C2960X-48FPS-L",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 49, 52)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(740),
        }),
    },
    Model {
        name: "WS-C2960X-48LPS-L",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 49, 52)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "WS-C2960X-24PS-L",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 25, 28)]),
        note: None,
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(370),
        }),
    },
    Model {
        name: "WS-C2960X-24PSQ-L",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 25, 28)]),
        note: Some(
            "uplinks are 2 SFP and 2 10/100/1000 ports (data sheet); vswitch takes all four as SFP",
        ),
        poe: Some(Poe {
            kind: PoeKind::PoePlus,
            watts: Some(110),
        }),
    },
    Model {
        name: "WS-C2960X-48TS-L",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 49, 52)]),
        note: None,
        poe: None,
    },
    Model {
        name: "WS-C2960X-24TS-L",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 25, 28)]),
        note: None,
        poe: None,
    },
    Model {
        name: "WS-C2960X-48TS-LL",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 48)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 49, 50)]),
        note: None,
        poe: None,
    },
    Model {
        name: "WS-C2960X-24TS-LL",
        family: Family::C2960X,
        since: None,
        until: None,
        ports: &[("GigabitEthernet", 1, 24)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 25, 26)]),
        note: None,
        poe: None,
    },
    // Catalyst 2960, 2960-Plus and 2960-C (data sheets): the dual-purpose uplinks (RJ-45 or SFP, one
    // active) on slot 0 after the front ports, the models the plant runs. The plain 2960 and 2960G
    // stop at 15.0(2)SE (15.0(2)SE11 is their last image); the 2960-Plus and 2960-C run 15.2(7)E
    Model {
        name: "WS-C2960-24TC-L",
        family: Family::C2960,
        since: None,
        until: Some("15.0(2)SE"),
        ports: &[("FastEthernet", 1, 24)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 1, 2)]),
        note: None,
        poe: None,
    },
    Model {
        name: "WS-C2960G-24TC-L",
        family: Family::C2960,
        since: None,
        until: Some("15.0(2)SE"),
        ports: &[("GigabitEthernet", 1, 20)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 21, 24)]),
        note: None,
        poe: None,
    },
    Model {
        name: "WS-C2960G-8TC-L",
        family: Family::C2960,
        since: None,
        until: Some("15.0(2)SE"),
        ports: &[("GigabitEthernet", 1, 7)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 8, 8)]),
        note: None,
        poe: None,
    },
    Model {
        name: "WS-C2960+24TC-L",
        family: Family::C2960Plus,
        since: None,
        until: None,
        ports: &[("FastEthernet", 1, 24)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 1, 2)]),
        note: None,
        poe: None,
    },
    Model {
        name: "WS-C2960C-8TC-L",
        family: Family::C2960C,
        since: None,
        until: None,
        ports: &[("FastEthernet", 1, 8)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 1, 2)]),
        note: None,
        poe: None,
    },
    // Cisco IE 3000 and IE 3010 (data sheets, captures): the IE-3000-8TC's eight copper Fast
    // Ethernet ports and two dual-purpose uplinks on slot 1; the IE-3010-16S-8PC's 16 SFP and 8 PoE
    // copper Fast Ethernet ports and two dual-purpose uplinks on slot 0
    Model {
        name: "IE-3000-8TC",
        family: Family::IE3000,
        since: None,
        until: None,
        ports: &[("FastEthernet", 1, 8)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 1, 2)]),
        note: None,
        poe: None,
    },
    Model {
        name: "IE-3010-16S-8PC",
        family: Family::IE3010,
        since: None,
        until: None,
        ports: &[("FastEthernet", 1, 24)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 1, 2)]),
        note: Some("PoE on FastEthernet0/17-24 is not modelled"),
        poe: None,
    },
    // Catalyst IE3300 (data sheet, captures): SFP ports Gi1/1-2 before the copper ports Gi1/3-10
    Model {
        name: "IE-3300-8T2S",
        family: Family::IE3300,
        since: Some("16.10"),
        until: None,
        ports: &[("GigabitEthernet", 3, 10)],
        uplinks: Uplinks::Slot0(&[("GigabitEthernet", 1, 2)]),
        note: None,
        poe: None,
    },
];

/// Every network module; its ports are in slot 1 of the member it sits in. Port names: the 9300 and
/// 3850 hardware installation guides' network module port configuration tables (the interfaces
/// to configure).
pub const MODULES: &[Module] = &[
    Module {
        name: "C9300X-NM-4C",
        family: Family::C9300X,
        since: Some("17.6"),
        ports: &[("HundredGigE", 1, 4)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C9300X-NM-2C",
        family: Family::C9300X,
        since: Some("17.5"),
        ports: &[("HundredGigE", 1, 2)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C9300X-NM-8Y",
        family: Family::C9300X,
        since: Some("17.5"),
        ports: &[("TwentyFiveGigE", 1, 8)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C9300X-NM-8M",
        family: Family::C9300X,
        since: Some("17.5"),
        ports: &[("TenGigabitEthernet", 1, 8)],
        breakout: false,
        copper: true,
    },
    Module {
        name: "C9300-NM-4G",
        family: Family::C9300,
        since: Some("16.5"),
        ports: &[("GigabitEthernet", 1, 4)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C9300-NM-4M",
        family: Family::C9300,
        since: Some("16.8"),
        ports: &[("TenGigabitEthernet", 1, 4)],
        breakout: false,
        copper: true,
    },
    Module {
        name: "C9300-NM-8X",
        family: Family::C9300,
        since: Some("16.5"),
        ports: &[("TenGigabitEthernet", 1, 8)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C9300-NM-2Q",
        family: Family::C9300,
        since: Some("16.5"),
        ports: &[("FortyGigabitEthernet", 1, 2)],
        breakout: true,
        copper: false,
    },
    Module {
        name: "C9300-NM-2Y",
        family: Family::C9300,
        since: Some("16.8"),
        ports: &[("TwentyFiveGigE", 1, 2)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C3850-NM-4-1G",
        family: Family::C9300,
        since: Some("16.5"),
        ports: &[("GigabitEthernet", 1, 4)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C3850-NM-2-10G",
        family: Family::C9300,
        since: Some("16.5"),
        ports: &[("GigabitEthernet", 1, 2), ("TenGigabitEthernet", 3, 4)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C3850-NM-4-10G",
        family: Family::C9300,
        since: Some("16.5"),
        ports: &[("TenGigabitEthernet", 1, 4)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C3850-NM-8-10G",
        family: Family::C9300,
        since: Some("16.5"),
        ports: &[("TenGigabitEthernet", 1, 8)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C3850-NM-2-40G",
        family: Family::C9300,
        since: Some("16.5"),
        ports: &[("FortyGigabitEthernet", 1, 2)],
        breakout: true,
        copper: false,
    },
    Module {
        name: "C9200-NM-4G",
        family: Family::C9200,
        since: Some("16.10"),
        ports: &[("GigabitEthernet", 1, 4)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C9200-NM-4X",
        family: Family::C9200,
        since: Some("16.10"),
        ports: &[("TenGigabitEthernet", 1, 4)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C9200-NM-2Y",
        family: Family::C9200,
        since: Some("17.1"),
        ports: &[("TwentyFiveGigE", 1, 2)],
        breakout: false,
        copper: false,
    },
    Module {
        name: "C9200-NM-2Q",
        family: Family::C9200,
        since: Some("17.1"),
        ports: &[("FortyGigabitEthernet", 1, 2)],
        breakout: false,
        copper: false,
    },
];

pub fn model(name: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.name.eq_ignore_ascii_case(name))
}

pub fn module(name: &str) -> Option<&'static Module> {
    MODULES.iter().find(|m| m.name.eq_ignore_ascii_case(name))
}

/// An interface name's type and the member, slot and port it names: `GigabitEthernet2/0/3` is member 2,
/// slot 0, port 3; a 2960's `FastEthernet0/3` member 1, slot 0, port 3 (a 9400's member is its card's slot).
pub fn locate(name: &str) -> Option<(&str, u32, u32, u32)> {
    let (ty, num) = name.split_at(name.find(|c: char| c.is_ascii_digit())?);
    let parts: Vec<u32> = num.split('/').filter_map(|p| p.parse().ok()).collect();
    match parts[..] {
        [member, slot, port] => Some((ty, member, slot, port)),
        // a two-part name's front panel is slot 0 here: the 2960's `0/<port>` and the IE-3300's
        // `1/<port>`
        [0 | 1, port] => Some((ty, 1, 0, port)),
        _ => None,
    }
}

/// Front-panel ports per model.
pub fn model_ports(name: &str) -> Option<Ports> {
    model(name).map(|m| m.ports)
}

/// Uplink ports per network module, in slot 1.
pub fn module_ports(name: &str) -> Option<Ports> {
    module(name).map(|m| m.ports)
}

/// The order `infer_stack` takes families in where a configuration's interfaces fit several: a
/// 9200 for copper 1G ports, the 9300X and 9300 for faster ones or a module only they take, the
/// fixed-uplink and B models after, the 9400 where its supervisors' ports say so, and the 2960-X
/// where a classic release or its FastEthernet0 does.
const INFER_ORDER: &[Family] = &[
    Family::C9200,
    Family::C9300X,
    Family::C9300,
    Family::C9200L,
    Family::C9300L,
    Family::C9300LM,
    Family::C9200B,
    Family::C9300B,
    Family::C9400,
    Family::C2960X,
];

/// The model `infer_stack` takes among those the configuration cannot tell apart: a 9400's copper
/// card as a C9400-LC-48U, a Supervisor 1 as a C9400-SUP-1XL.
const INFERRED: &[&str] = &["C9400-LC-48U", "C9400-SUP-1XL"];

/// The network module `infer_stack` takes among those holding as many of slot 1's interfaces: the
/// fastest ports first, then the smaller module.
const MODULE_ORDER: &[&str] = &[
    "C9300X-NM-2C",
    "C9300X-NM-4C",
    "C9300X-NM-8Y",
    "C9300X-NM-8M",
    "C9300-NM-2Q",
    "C9300-NM-2Y",
    "C9300-NM-8X",
    "C9300-NM-4G",
    "C9200-NM-2Q",
    "C9200-NM-2Y",
    "C9200-NM-4X",
    "C9200-NM-4G",
];

/// Hardware `infer_stack` may take for a member: a model and the network module in it.
type Pick = (&'static Model, Option<&'static Module>);

/// Whether a model runs the release a configuration's `version` line names: IOS (12.x, 15.x) on the
/// families that run it and IOS XE on the rest, from its `since` up to its `until`.
fn runs_version(m: &Model, version: Option<Release>) -> bool {
    let Some(v) = version else {
        return true;
    };
    m.family.classic_ios() == (v.major() < 16)
        && m.since.is_none_or(|s| v.runs(s))
        && m.until
            .and_then(Release::parse)
            .is_none_or(|u| u.includes(v))
}

/// A stack's interfaces but its management port: what a configuration lists for its members.
fn member_ports(s: &Stack) -> BTreeSet<String> {
    let management = s.management();
    s.physical()
        .into_iter()
        .filter(|p| Some(p.as_str()) != management)
        .collect()
}

/// The members a configuration implies, written as `--stack` takes them; None when it names no
/// hardware here. A `switch N provision <model>` line names a member's model. Otherwise each member
/// takes a model whose interfaces hold every one the configuration names on it, which runs the
/// release its `version` line names and stacks with the others (`Stack::problems`). Among those: a
/// model whose whole port list the configuration names (a running configuration lists every port);
/// one whose front panel has no interface type the configuration leaves out; the families in
/// `INFER_ORDER`; the model in `INFERRED`; the fewest front ports; PoE only where the configuration
/// sets it. An AppGigabitEthernet port is no 9200's: none of the 9200 captures lists one. The
/// network module is the one holding the most of slot 1's interfaces named, none where a 9300's
/// whole slot is listed, which IOS lists whatever is fitted. Where nothing fits the release or
/// Cisco's stacking rules, the hardware the interfaces fit, for those rules to name.
pub fn infer_stack(config: &str) -> Option<String> {
    use std::collections::BTreeMap;
    let version = crate::release::version_line(config).and_then(Release::parse);
    let mut provisioned: BTreeMap<u32, &'static Model> = BTreeMap::new();
    // three-part names by member; the others whole (a two-part switch's ports, a management port)
    let mut members: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    let mut others: BTreeSet<String> = BTreeSet::new();
    for line in config.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if let ["switch", n, "provision", ty] = words[..]
            && let (Ok(n), Some(m)) = (n.parse::<u32>(), model(ty))
        {
            provisioned.insert(n, m);
            members.entry(n).or_default();
            continue;
        }
        let ["interface", first, ref rest @ ..] = words[..] else {
            continue;
        };
        let Some((ty, num, _)) = crate::cli::interface_name(first, rest.first().copied()) else {
            continue;
        };
        let Some(parts) = num
            .split('/')
            .map(|p| p.parse::<u32>().ok())
            .collect::<Option<Vec<u32>>>()
        else {
            continue;
        };
        let name = format!("{ty}{num}");
        match parts[..] {
            [n, _, _] => {
                members.entry(n).or_default().insert(name);
            }
            [_, _] => {
                others.insert(name);
            }
            // a 2960-X's management port
            [0] if ty == "FastEthernet" => {
                others.insert(name);
            }
            _ => {}
        }
    }
    if members.is_empty() {
        return [true, false]
            .into_iter()
            .find_map(|strict| fixed_model(&others, config, version, strict));
    }
    let poe = config
        .lines()
        .any(|l| l.trim_start().starts_with("power inline"));
    let alone = members.len() == 1;
    [true, false].into_iter().find_map(|strict| {
        let ranked: Vec<(u32, Vec<Pick>)> = members
            .iter()
            .map(|(&n, names)| {
                let given = provisioned.get(&n).copied();
                (n, candidates(n, names, given, version, poe, alone, strict))
            })
            .collect();
        // each family group member 1 can take, best first, with every member's best in it
        let mut groups: Vec<Family> = vec![];
        for (m, _) in &ranked.first()?.1 {
            if !groups.iter().any(|g| g.stacks_with(m.family)) {
                groups.push(m.family);
            }
        }
        groups.into_iter().find_map(|g| {
            // a 9400's supervisors are the members in one chassis's supervisor slots: of the
            // chassis that place every member, the one giving the members their best picks
            let chassis: Vec<Option<&Chassis>> = match g {
                Family::C9400 => CHASSIS.iter().map(Some).chain([None]).collect(),
                _ => vec![None],
            };
            let (_, s) = chassis
                .into_iter()
                .filter_map(|c| {
                    let mut ranks = vec![];
                    let members = ranked
                        .iter()
                        .map(|(n, picks)| {
                            let (rank, &(m, md)) =
                                picks.iter().enumerate().find(|(_, (m, _))| {
                                    m.family.stacks_with(g)
                                        && c.is_none_or(|c| {
                                            is_supervisor(m.name) == c.supervisors.contains(n)
                                        })
                                })?;
                            ranks.push(rank);
                            Some(Member {
                                number: *n,
                                model: m.name.to_string(),
                                module: md.map(|x| x.name.to_string()),
                            })
                        })
                        .collect::<Option<Vec<Member>>>()?;
                    let s = Stack { members };
                    let sound = s
                        .problems(version.unwrap_or(crate::release::DEFAULT))
                        .is_empty()
                        && others.iter().all(|o| s.has(o));
                    let placed = g != Family::C9400 || s.chassis_problems().is_empty();
                    (sound || !strict).then_some(((!sound, !placed, ranks), s))
                })
                .min_by(|a, b| a.0.cmp(&b.0))?;
            Some(s.spec())
        })
    })
}

/// A member's possible hardware for `infer_stack`, best first: each model, with the network module
/// that fits slot 1's names, whose interfaces hold the member's names; a provisioned model alone.
/// A model without FlexStack-Plus only where the member is `alone`; `strict` keeps only hardware
/// that runs the configuration's release.
fn candidates(
    n: u32,
    names: &BTreeSet<String>,
    provisioned: Option<&'static Model>,
    version: Option<Release>,
    poe: bool,
    alone: bool,
    strict: bool,
) -> Vec<Pick> {
    let app = names.iter().any(|x| x.starts_with("AppGigabitEthernet"));
    let names: BTreeSet<&String> = names
        .iter()
        .filter(|x| !x.starts_with("AppGigabitEthernet"))
        .collect();
    let mut front: BTreeSet<&str> = BTreeSet::new();
    let mut slot1: Vec<(&str, u32)> = vec![];
    for x in &names {
        match locate(x) {
            Some((t, _, 0, _)) => {
                front.insert(t);
            }
            Some((t, _, 1, p)) => slot1.push((t, p)),
            _ => {}
        }
    }
    let within = |ports: Ports, (t, p): (&str, u32)| {
        ports
            .iter()
            .any(|&(u, lo, hi)| u == t && (lo..=hi).contains(&p))
    };
    let mut out = vec![];
    for (index, m) in MODELS.iter().enumerate() {
        let given = provisioned.is_some_and(|p| p.name == m.name);
        if m.family.two_part() || provisioned.is_some() && !given {
            continue;
        }
        let nine200 = matches!(m.family, Family::C9200 | Family::C9200L | Family::C9200B);
        let stacks = alone || !NO_FLEXSTACK.contains(&m.name);
        if !given && (app && nine200 || !stacks || strict && !runs_version(m, version)) {
            continue;
        }
        let listed = m.family.slot1();
        let whole = !listed.is_empty()
            && listed
                .iter()
                .all(|&(t, lo, hi)| (lo..=hi).all(|p| slot1.contains(&(t, p))));
        let modules: Vec<Option<&'static Module>> = match m.uplinks {
            Uplinks::Modules(fits) if !slot1.is_empty() && !whole => fits
                .iter()
                .map(|f| module(f))
                .filter(Option::is_some)
                .collect(),
            _ => vec![None],
        };
        let mut types: BTreeSet<&str> = m.ports.iter().map(|p| p.0).collect();
        if let Uplinks::Slot0(up) = m.uplinks {
            types.extend(up.iter().map(|p| p.0));
        }
        types.remove("AppGigabitEthernet");
        let extra = types.iter().filter(|t| !front.contains(*t)).count();
        let family = INFER_ORDER
            .iter()
            .position(|f| *f == m.family)
            .unwrap_or(INFER_ORDER.len());
        let size: u32 = m.ports.iter().map(|&(_, lo, hi)| hi + 1 - lo).sum();
        let power = match (poe, m.poe.map(|p| p.kind)) {
            (false, None) | (true, Some(PoeKind::PoePlus)) => 0,
            (false, Some(PoeKind::PoePlus)) | (true, Some(PoeKind::Upoe)) => 1,
            (false, Some(PoeKind::Upoe)) | (true, Some(PoeKind::UpoePlus)) => 2,
            (false, Some(PoeKind::UpoePlus)) | (true, None) => 3,
        };
        for md in modules {
            if !given
                && strict
                && md
                    .and_then(|x| x.since)
                    .is_some_and(|s| version.is_some_and(|v| !v.runs(s)))
            {
                continue;
            }
            let one = Stack {
                members: vec![Member {
                    number: n,
                    model: m.name.to_string(),
                    module: md.map(|x| x.name.to_string()),
                }],
            };
            let ports: BTreeSet<String> = member_ports(&one)
                .into_iter()
                .filter(|x| !x.starts_with("AppGigabitEthernet"))
                .collect();
            let fits = names.iter().all(|x| ports.contains(*x));
            if !fits && !given {
                continue;
            }
            let exact = ports.len() == names.len() && fits;
            let held = md.map_or(0, |x| slot1.iter().filter(|&&s| within(x.ports, s)).count());
            let order = md.map_or(0, |x| {
                MODULE_ORDER
                    .iter()
                    .position(|o| *o == x.name)
                    .unwrap_or(MODULE_ORDER.len())
            });
            let key = (
                !fits,
                !exact,
                extra,
                family,
                !INFERRED.contains(&m.name),
                size,
                power,
                slot1.len() - held,
                order,
                index,
            );
            out.push((key, m, md));
        }
    }
    out.sort_by_key(|c| c.0);
    out.into_iter().map(|(_, m, md)| (m, md)).collect()
}

/// The one switch a configuration's two-part interface names give (a 2960, 2960-Plus, 2960-C, IE
/// 3000, IE 3010 or IE3300): a model whose ports hold them all, its whole port list named first.
/// An `alarm profile` or `alarm facility` line, which the IE 3000 and IE 3010 captures carry and the
/// 2960s' do not, marks the industrial switch; then the front panel's own interface types, a 2960
/// at 12.2 and a 2960-Plus otherwise, the fewest ports. `strict` keeps the models that run the
/// configuration's release.
fn fixed_model(
    names: &BTreeSet<String>,
    config: &str,
    version: Option<Release>,
    strict: bool,
) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    let alarms = config
        .lines()
        .any(|l| l.starts_with("alarm profile ") || l.starts_with("alarm facility "));
    let old = version.is_some_and(|v| v.major() == 12);
    let types: BTreeSet<&str> = names
        .iter()
        .map(|x| &x[..x.find(|c: char| c.is_ascii_digit()).unwrap_or(x.len())])
        .collect();
    MODELS
        .iter()
        .enumerate()
        .filter(|(_, m)| m.family.two_part() && (!strict || runs_version(m, version)))
        .filter_map(|(index, m)| {
            let one = Stack {
                members: vec![Member {
                    number: 1,
                    model: m.name.to_string(),
                    module: None,
                }],
            };
            let ports = member_ports(&one);
            if !names.iter().all(|x| ports.contains(x)) {
                return None;
            }
            let industrial = matches!(m.family, Family::IE3000 | Family::IE3010);
            let mut own: BTreeSet<&str> = m.ports.iter().map(|p| p.0).collect();
            if let Uplinks::Slot0(up) = m.uplinks {
                own.extend(up.iter().map(|p| p.0));
            }
            let extra = own.iter().filter(|t| !types.contains(*t)).count();
            let later = match m.family {
                Family::C2960 => !old,
                Family::C2960Plus => old,
                _ => false,
            };
            let key = (
                ports != *names,
                industrial != alarms,
                extra,
                later,
                ports.len(),
                index,
            );
            Some((key, m.name))
        })
        .min()
        .map(|(_, name)| name.to_string())
}

/// A stack from `--stack` notation: models joined by `,` (member 1 first), each with an optional
/// `+<network module>` and an optional `<number>=` prefix (a Catalyst 9400 card's slot).
pub fn parse_stack(spec: &str) -> Result<Stack, String> {
    let mut members = Vec::new();
    for (i, m) in spec.split(',').enumerate() {
        let (number, m) = match m.split_once('=') {
            Some((n, rest)) => match n.trim().parse::<u32>() {
                // `checked_stack` and the builder read number 0 as the whole stack
                Ok(0) => return Err("bad member number 0: members are numbered from 1".into()),
                Ok(v) => (v, rest),
                Err(_) => return Err(format!("bad member number {n}")),
            },
            None => (i as u32 + 1, m),
        };
        // a model's own name may hold a `+` (WS-C2960+24TC-L): the model is the longest part before a
        // `+` that names one, the rest its network module
        let cut = std::iter::once(m.len())
            .chain(m.rmatch_indices('+').map(|(i, _)| i))
            .find(|&i| model_ports(m[..i].trim()).is_some())
            .or_else(|| m.find('+'))
            .unwrap_or(m.len());
        let (model, module) = (
            m[..cut].trim(),
            m.get(cut + 1..).map(|r| r.trim().to_string()),
        );
        if model_ports(model).is_none() {
            return Err(format!("unknown model {model}"));
        }
        if let Some(md) = &module
            && module_ports(md).is_none()
        {
            return Err(format!("unknown network module {md}"));
        }
        members.push(Member {
            number,
            model: model.to_string(),
            module,
        });
    }
    Ok(Stack { members })
}

/// A stack that parses and that Cisco builds; its problems, each naming the member, otherwise.
pub fn checked_stack(spec: &str, release: Release) -> Result<Stack, String> {
    let s = parse_stack(spec)?;
    let problems = s.problems(release);
    if problems.is_empty() {
        return Ok(s);
    }
    Err(problems
        .into_iter()
        .map(|(n, why)| {
            if n == 0 {
                why
            } else {
                format!("member {n}: {why}")
            }
        })
        .collect::<Vec<_>>()
        .join("; "))
}

/// Hardware newer than the release, named with the release Cisco introduced it in.
fn too_new(name: &str, since: &str, release: Release) -> String {
    let train = Release::parse(since).map_or(since.to_string(), |r| r.to_string());
    let named = Release::parse(since).map_or(train, |r| r.named());
    format!(
        "the {name} runs {named} and later (release notes); this switch runs {}",
        release.named()
    )
}

impl Stack {
    /// `--stack` notation for this stack: numbers written where they differ from the position.
    pub fn spec(&self) -> String {
        self.members
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let mut s = if m.number != i as u32 + 1 || self.family() == Some(Family::C9400) {
                    format!("{}={}", m.number, m.model)
                } else {
                    m.model.clone()
                };
                if let Some(md) = &m.module {
                    s.push('+');
                    s.push_str(md);
                }
                s
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Member {
    pub number: u32,
    pub model: String,
    #[serde(default)]
    pub module: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Stack {
    pub members: Vec<Member>,
}

impl Stack {
    #[cfg(test)]
    pub fn single(model: &str, module: Option<&str>) -> Self {
        Stack {
            members: vec![Member {
                number: 1,
                model: model.into(),
                module: module.map(Into::into),
            }],
        }
    }

    /// Every front-panel interface in IOS order: member, then slot, then port.
    /// What a member's running configuration lists for its module slot whatever is fitted, where it
    /// lists more than the module's own ports: the family's listing (`Family::slot1`), or a modular
    /// 9200's every module's ports (C9200-24Ts at 17.9.4a, 17.9.5 and 17.12.4 list the NM-4G's and
    /// the NM-4X's).
    fn slot_listing(model_name: &str) -> Option<Vec<(&'static str, u32, u32)>> {
        let m = model(model_name)?;
        let listed = m.family.slot1();
        if !listed.is_empty() {
            return Some(listed.to_vec());
        }
        match (m.family, m.uplinks) {
            (Family::C9200, Uplinks::Modules(modules)) => {
                let mut out: Vec<(&'static str, u32, u32)> = vec![];
                for &(ty, lo, hi) in modules.iter().filter_map(|n| module_ports(n)).flatten() {
                    match out.iter_mut().find(|p| p.0 == ty) {
                        Some(p) => {
                            p.1 = p.1.min(lo);
                            p.2 = p.2.max(hi);
                        }
                        None => out.push((ty, lo, hi)),
                    }
                }
                Some(out)
            }
            _ => None,
        }
    }

    pub fn physical(&self) -> Vec<String> {
        let mut out: Vec<String> = self.management().map(str::to_string).into_iter().collect();
        for m in &self.members {
            // a 2960, 2960-Plus or 2960-C names a port `<type>0/<port>`, an IE-3300 `<type>1/<port>`
            let two_part = model(&m.model)
                .filter(|x| x.family.two_part())
                .map(|x| x.family.front_slot());
            let front = |ty: &str, p: u32| match two_part {
                Some(slot) => format!("{ty}{slot}/{p}"),
                None => format!("{ty}{}/0/{p}", m.number),
            };
            // the front ports, then built-in uplinks numbered after them or, on an IE-3300, before
            // them (its SFP ports are 1 and 2): by type as first listed, then number
            let mut own: Vec<(&str, u32)> = vec![];
            for &(ty, lo, hi) in model_ports(&m.model).unwrap_or_default() {
                own.extend((lo..=hi).map(|p| (ty, p)));
            }
            if let Some(Uplinks::Slot0(up)) = model(&m.model).map(|x| x.uplinks) {
                for &(ty, lo, hi) in up {
                    own.extend((lo..=hi).map(|p| (ty, p)));
                }
            }
            let types: Vec<&str> = own.iter().fold(vec![], |mut v, (t, _)| {
                if !v.contains(t) {
                    v.push(*t);
                }
                v
            });
            own.sort_by_key(|(t, p)| (types.iter().position(|x| x == t), *p));
            out.extend(own.into_iter().map(|(ty, p)| front(ty, p)));
            // slot 1: what IOS lists whatever is fitted, else built-in uplinks or the module's ports
            let family = model(&m.model).map(|x| x.family);
            let uplinks: Vec<(&str, u32, u32)> = match (
                Self::slot_listing(&m.model),
                model(&m.model).map(|x| x.uplinks),
            ) {
                (Some(listed), _) => listed,
                (_, Some(Uplinks::Fixed(p))) => p.to_vec(),
                _ => m
                    .module
                    .as_deref()
                    .and_then(module_ports)
                    .unwrap_or_default()
                    .to_vec(),
            };
            for (ty, lo, hi) in uplinks {
                out.extend((lo..=hi).map(|p| format!("{ty}{}/1/{p}", m.number)));
            }
            // the application hosting port, listed after slot 1 (9300 command reference example; two
            // C9300L-24T-4Gs at 17.6.5)
            if matches!(
                family,
                Some(Family::C9300 | Family::C9300B | Family::C9300L)
            ) {
                out.push(format!("AppGigabitEthernet{}/0/1", m.number));
            }
        }
        out
    }

    /// The management port: a 2960-X's FastEthernet0, none on a 2960, 2960-Plus or 2960-C, else
    /// GigabitEthernet0/0.
    pub fn management(&self) -> Option<&'static str> {
        match self.family() {
            Some(Family::C2960X) => Some("FastEthernet0"),
            Some(f) if f.two_part() => None,
            _ => Some("GigabitEthernet0/0"),
        }
    }

    /// A port's type, member, slot and number; none for the management port, which `locate` reads
    /// as member 1's port 0 (`GigabitEthernet0/0`) and which is copper on every model.
    fn port_of<'s, 'n>(&'s self, name: &'n str) -> Option<(&'n str, &'s Member, u32, u32)> {
        if self.management() == Some(name) {
            return None;
        }
        let (ty, member, slot, port) = locate(name)?;
        let m = self.members.iter().find(|m| m.number == member)?;
        Some((ty, m, slot, port))
    }

    /// The PoE a front-panel port gives: its member's model's, on the model's own ports.
    pub fn poe(&self, name: &str) -> Option<Poe> {
        let Some((ty, member, 0, port)) = self.port_of(name) else {
            return None;
        };
        let m = model(&member.model)?;
        m.ports
            .iter()
            .any(|&(t, lo, hi)| t == ty && (lo..=hi).contains(&port))
            .then_some(m.poe)
            .flatten()
    }

    /// Whether a port is there to run: a module slot's port is only where its module is fitted, the
    /// slot listing every module's ports (`Family::slot1`) whatever is in it.
    pub fn fitted(&self, name: &str) -> bool {
        let Some((ty, member, 1, port)) = locate(name) else {
            return true;
        };
        let Some(m) = self.members.iter().find(|m| m.number == member) else {
            return true;
        };
        if Self::slot_listing(&m.model).is_none() {
            return true;
        }
        m.module
            .as_deref()
            .and_then(module_ports)
            .unwrap_or_default()
            .iter()
            .any(|&(t, lo, hi)| t == ty && (lo..=hi).contains(&port))
    }

    /// The family the stack's first member names; every member stacks with it.
    pub fn family(&self) -> Option<Family> {
        self.members
            .first()
            .and_then(|m| model(&m.model))
            .map(|m| m.family)
    }

    /// A family that runs classic IOS, whose show output is laid out apart from IOS XE's.
    pub fn classic(&self) -> bool {
        self.family().is_some_and(Family::classic_ios)
    }

    /// Hardware Cisco does not build or stack, as (member number, why): a network module the model
    /// does not take, one in a model with built-in uplinks, families that do not stack together
    /// (`Family::stacks_with`), a 2960-X model without FlexStack-Plus in a stack, too many members,
    /// a member number used twice or outside the stack's 1 to 8 (a one-switch family's 1).
    pub fn problems(&self, release: Release) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        let family = self.family();
        let mut numbers = std::collections::BTreeSet::new();
        for m in &self.members {
            let Some(md) = model(&m.model) else {
                out.push((
                    m.number,
                    format!("{} is not a model vswitch knows", m.model),
                ));
                continue;
            };
            if !numbers.insert(m.number) {
                out.push((m.number, format!("member {} is numbered twice", m.number)));
            }
            // a 9400's members are its slots, which `chassis_problems` places; one switch of a
            // family that does not stack is told so once, below
            let most = md.family.max_members() as u32;
            if md.family != Family::C9400
                && !(1..=most).contains(&m.number)
                && (most > 1 || self.members.len() == 1)
            {
                out.push((
                    m.number,
                    if most == 1 {
                        format!("a {} is one switch, numbered 1", md.family.name())
                    } else {
                        format!(
                            "a {} stack numbers its members 1 to {most}",
                            md.family.name()
                        )
                    },
                ));
            }
            if self.members.len() > 1
                && NO_FLEXSTACK.iter().any(|n| n.eq_ignore_ascii_case(md.name))
            {
                out.push((
                    m.number,
                    format!(
                        "the {} has no FlexStack-Plus and does not stack (Cisco data sheet)",
                        md.name
                    ),
                ));
            }
            if let Some(since) = md.since
                && !release.runs(since)
            {
                out.push((m.number, too_new(md.name, since, release)));
            }
            if let Some(until) = md.until
                && Release::parse(until).is_some_and(|u| !u.includes(release))
            {
                out.push((
                    m.number,
                    format!(
                        "the {} runs up to {} (release notes); this switch runs {}",
                        md.name,
                        Release::parse(until).map_or(until.to_string(), |u| u.named()),
                        release.named()
                    ),
                ));
            }
            if let Some(nm) = m.module.as_deref().and_then(module)
                && let Some(since) = nm.since
                && !release.runs(since)
            {
                out.push((m.number, too_new(nm.name, since, release)));
            }
            if let Some(f) = family
                && !md.family.stacks_with(f)
            {
                out.push((
                    m.number,
                    format!(
                        "a {} does not stack with a {} (Cisco data sheet)",
                        md.family.name(),
                        f.name()
                    ),
                ));
            }
            let Some(nm) = m.module.as_deref() else {
                continue;
            };
            let Some(module) = module(nm) else {
                out.push((
                    m.number,
                    format!("{nm} is not a network module vswitch knows"),
                ));
                continue;
            };
            match md.uplinks {
                Uplinks::Modules(fits)
                    if fits.iter().any(|f| f.eq_ignore_ascii_case(module.name)) => {}
                Uplinks::Modules(fits) if module.family != md.family => out.push((
                    m.number,
                    format!(
                        "{} is a {} module; the {} takes {}",
                        module.name,
                        module.family.name(),
                        md.name,
                        fits.join(", ")
                    ),
                )),
                Uplinks::Modules(fits) => out.push((
                    m.number,
                    format!(
                        "{} does not fit the {}, which takes {} (Cisco data sheet)",
                        module.name,
                        md.name,
                        fits.join(", ")
                    ),
                )),
                Uplinks::Fixed(_) | Uplinks::Slot0(_) => out.push((
                    m.number,
                    format!(
                        "the {} has built-in uplinks and takes no network module",
                        md.name
                    ),
                )),
                Uplinks::None => {
                    out.push((m.number, format!("the {} takes no network module", md.name)))
                }
            }
        }
        if family == Some(Family::C9400) {
            out.extend(self.chassis_problems());
        }
        if let Some(f) = family
            && self.members.len() > f.max_members()
        {
            out.push((
                0,
                match f.max_members() {
                    1 => format!("a {} is one switch: it does not stack", f.name()),
                    most => format!("a {} stack takes at most {most} members", f.name()),
                },
            ));
        }
        out
    }

    /// A Catalyst 9400 chassis as its supervisors' slots place it (supervisor data sheet): one or two
    /// supervisors of one model, in slots 2 or 3 of a C9404R, 3 or 4 of a C9407R, 5 or 6 of a C9410R,
    /// and line cards in that chassis's other slots.
    fn chassis_problems(&self) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        let sups: Vec<&Member> = self
            .members
            .iter()
            .filter(|m| is_supervisor(&m.model))
            .collect();
        if sups.is_empty() {
            out.push((0, "a Catalyst 9400 needs a supervisor".to_string()));
            return out;
        }
        if sups.len() > 2 {
            out.push((
                0,
                "a Catalyst 9400 takes at most two supervisors".to_string(),
            ));
        }
        if sups
            .iter()
            .any(|m| !m.model.eq_ignore_ascii_case(&sups[0].model))
        {
            out.push((
                sups[1].number,
                format!(
                    "redundant supervisors are one model; slot {} holds a {}",
                    sups[0].number, sups[0].model
                ),
            ));
        }
        let fits: Vec<&Chassis> = CHASSIS
            .iter()
            .filter(|c| sups.iter().all(|m| c.supervisors.contains(&m.number)))
            .collect();
        if fits.is_empty() {
            out.push((
                sups[0].number,
                "a supervisor goes in slot 2 or 3 (C9404R), 3 or 4 (C9407R), 5 or 6 (C9410R) (supervisor data sheet)".to_string(),
            ));
            return out;
        }
        let cards: Vec<&Member> = self
            .members
            .iter()
            .filter(|m| !is_supervisor(&m.model))
            .collect();
        let mut misplaced = false;
        for m in &cards {
            if !fits.iter().any(|c| c.cards.contains(&m.number)) {
                misplaced = true;
                let names: Vec<String> = fits
                    .iter()
                    .map(|c| format!("{} line card slots {}", c.name, c.slots_text))
                    .collect();
                out.push((
                    m.number,
                    format!(
                        "slot {} is not a line card slot of a {} (supervisor data sheet)",
                        m.number,
                        names.join(", or a ")
                    ),
                ));
            }
        }
        // each card can sit in some chassis the supervisors fit while no one chassis holds them all
        if !misplaced
            && !fits
                .iter()
                .any(|c| cards.iter().all(|m| c.cards.contains(&m.number)))
        {
            let say = |v: &[&Member], one: &str, many: &str| {
                let mut n: Vec<u32> = v.iter().map(|m| m.number).collect();
                n.sort_unstable();
                let t: Vec<String> = n.iter().map(u32::to_string).collect();
                match t.split_last() {
                    Some((last, [])) => format!("a {one} in slot {last}"),
                    Some((last, rest)) => format!("{many} in slots {} and {last}", rest.join(", ")),
                    None => String::new(),
                }
            };
            out.push((
                0,
                format!(
                    "no one chassis has {} and {} (supervisor data sheet)",
                    say(&sups, "supervisor", "supervisors"),
                    say(&cards, "line card", "line cards")
                ),
            ));
        }
        out
    }

    /// What a front-panel port takes, which sets how `show interfaces status` shows it with nothing
    /// plugged in. Copper, 1G SFP, line-card SFP+ and supervisor ports are as the cores showed them
    /// (C9400-LC-48U, -48S, -24XS, -SUP-1XL, 17.5), a fixed switch's SFP ports as c9300 17.15 capture 5's
    /// C9300-24S (17.15). The management port is copper.
    pub fn media(&self, name: &str) -> Media {
        if self.management() == Some(name) {
            return Media::Copper;
        }
        let Some((ty, m, slot, port)) = self.port_of(name) else {
            return Media::Other;
        };
        let model = m.model.to_ascii_uppercase();
        if is_supervisor(&m.model) {
            return Media::Uplink;
        }
        if slot != 0 {
            return Media::Other;
        }
        // a 2960-X's uplinks share slot 0 with its front ports
        if let Some(Uplinks::Slot0(up)) = crate::platform::model(&model).map(|x| x.uplinks)
            && up
                .iter()
                .any(|&(t, lo, hi)| t == ty && (lo..=hi).contains(&port))
        {
            return if ty == "TenGigabitEthernet" {
                Media::SfpPlus
            } else {
                Media::Sfp
            };
        }
        match ty {
            "GigabitEthernet" if model.ends_with("-24S") || model.ends_with("-48S") => Media::Sfp,
            "FastEthernet" if model == "IE-3010-16S-8PC" && port <= 16 => Media::Sfp,
            "GigabitEthernet" | "FastEthernet" => Media::Copper,
            // the SFP+ line cards, and the 4 SFP+ ports of the C9400-LC-24XY (LC_24XY_NOTE)
            "TenGigabitEthernet"
                if matches!(
                    model.as_str(),
                    "C9400-LC-24XS" | "C9400-LC-48XS" | "C9400-LC-24XY"
                ) =>
            {
                Media::SfpPlus
            }
            _ => Media::Other,
        }
    }

    /// The speeds (Mb/s) a transceiver slot runs, or `None` for an RJ-45 port. A QSFP port takes
    /// none of `OPTICS`. 9400 SFP and SFP+ cards and supervisor SFP+ ports are as the cores ran them
    /// (100BASE-FX and 1000BASE-LX in LC-48S ports, 1000BASE-LX in an LC-24XS port), a 2960-X's SFP
    /// uplinks as the 2960-Xs cabled to them (100BASE-FX); the rest are the data sheets' port
    /// descriptions (SFP28 25G/10G/1G, network module SFP+ 10G/1G, SFP 1G, the IE-3010's 100 Mb/s
    /// SFP). A Multigigabit network module's ports and the management port are RJ-45.
    pub fn slot_speeds(&self, name: &str) -> Option<Vec<u32>> {
        let (ty, m, slot, port) = self.port_of(name)?;
        let model = m.model.to_ascii_uppercase();
        let copper_module = m.module.as_deref().and_then(module).is_some_and(|x| {
            x.copper
                && x.ports
                    .iter()
                    .any(|&(t, lo, hi)| t == ty && (lo..=hi).contains(&port))
        });
        if slot != 0 && copper_module {
            return None;
        }
        let uplink = slot != 0 || is_supervisor(&m.model);
        let media = self.media(name);
        match ty {
            "TwentyFiveGigE" => Some(vec![25_000, 10_000, 1000]),
            "FortyGigabitEthernet" | "HundredGigE" | "TwoHundredGigE" | "FourHundredGigE" => {
                Some(vec![])
            }
            "TenGigabitEthernet" if uplink || media == Media::SfpPlus => Some(vec![10_000, 1000]),
            // nine 2960-X uplinks run 100BASE-FX to the cores' LC-48S ports (the cores' status, 17.5)
            "GigabitEthernet"
                if media == Media::Sfp
                    && (model.starts_with("C9400") || self.family() == Some(Family::C2960X)) =>
            {
                Some(vec![1000, 100])
            }
            "GigabitEthernet" if uplink || media == Media::Sfp => Some(vec![1000]),
            "FastEthernet" if media == Media::Sfp => Some(vec![100]),
            _ => None,
        }
    }

    /// Whether a port is a dual-purpose uplink, RJ-45 or SFP with one active, which `media-type`
    /// fixes to one: the 2960's, 2960-Plus's, 2960-C's, IE 3000's and IE 3010's (data sheets).
    pub fn dual_purpose(&self, name: &str) -> bool {
        let Some((ty, m, 0, port)) = self.port_of(name) else {
            return false;
        };
        model(&m.model).is_some_and(|x| {
            matches!(
                x.family,
                Family::C2960
                    | Family::C2960Plus
                    | Family::C2960C
                    | Family::IE3000
                    | Family::IE3010
            ) && matches!(x.uplinks, Uplinks::Slot0(up)
                if up.iter().any(|&(t, lo, hi)| t == ty && (lo..=hi).contains(&port)))
        })
    }

    /// The supervisor and port a 9400 supervisor uplink names (`FortyGigabitEthernet3/0/9` is slot 3,
    /// port 9), and whether it is a Supervisor 2.
    fn uplink(&self, name: &str) -> Option<(u32, u32, bool)> {
        let (ty, num) = name.split_at(name.find(|c: char| c.is_ascii_digit())?);
        let parts: Vec<u32> = num.split('/').filter_map(|p| p.parse().ok()).collect();
        let [slot, 0, port] = parts[..] else {
            return None;
        };
        let m = self
            .members
            .iter()
            .find(|m| m.number == slot && is_supervisor(&m.model))?;
        (ty != "AppGigabitEthernet").then(|| {
            let sup2 = m.model.to_ascii_uppercase().starts_with("C9400X-SUP-2");
            (slot, port, sup2)
        })
    }

    /// The slot and port of a supervisor QSFP port that runs only once `enable` turns it on (port 9
    /// and 10 on a Supervisor 1, port 5 on a Supervisor 2).
    pub fn needs_enable(&self, name: &str) -> Option<(u32, u32)> {
        let (slot, port, sup2) = self.uplink(name)?;
        let qsfp = if sup2 {
            port == 5
        } else {
            port == 9 || port == 10
        };
        qsfp.then_some((slot, port))
    }

    /// Why a supervisor uplink is off, given the QSFP ports `enable` has turned on (slot, port). A
    /// Supervisor 1's SFP ports 1-4 and QSFP port 9 run one or the other, and so do 5-8 and 10; a
    /// Supervisor 2's 1-4 and 5. The QSFP port runs only when enabled; with two supervisors a
    /// Supervisor 1's 5-8 and 10, and a Supervisor 2's 7 and 8, never run (Catalyst 9400 Supervisor
    /// Module Installation Note, uplink port configuration; the cores show Te3/0/5-8 and Fo3/0/9-10
    /// inactive with ports 1-4 in use).
    pub fn uplink_off(&self, name: &str, enabled: &BTreeSet<(u32, u32)>) -> Option<String> {
        let (slot, port, sup2) = self.uplink(name)?;
        let pair = self
            .members
            .iter()
            .filter(|m| is_supervisor(&m.model))
            .count()
            >= 2;
        let on = |p: u32| enabled.contains(&(slot, p));
        let note = "(Supervisor Module Installation Note)";
        let why = if sup2 {
            match port {
                1..=4 if on(5) => format!("is off while port 5 is enabled {note}"),
                5 if !on(5) => {
                    format!(
                        "is off until `enable` turns it on, which turns off ports 1 to 4 {note}"
                    )
                }
                7 | 8 if pair => format!("is inactive with two supervisors {note}"),
                _ => return None,
            }
        } else {
            match port {
                1..=4 if on(9) => format!("is off while port 9 is enabled {note}"),
                9 if !on(9) => {
                    format!(
                        "is off until `enable` turns it on, which turns off ports 1 to 4 {note}"
                    )
                }
                5..=8 | 10 if pair => format!("is inactive with two supervisors {note}"),
                5..=8 if on(10) => format!("is off while port 10 is enabled {note}"),
                10 if !on(10) => {
                    format!(
                        "is off until `enable` turns it on, which turns off ports 5 to 8 {note}"
                    )
                }
                _ => return None,
            }
        };
        Some(why)
    }

    /// Why an interface IOS lists carries no traffic, if it does not: a supervisor uplink off as
    /// `uplink_off` says with no QSFP port enabled; a network module interface Cisco's port tables
    /// mark "Disregard"; a module port the model disables. IOS still takes their configuration, so
    /// this is a design check, not a refusal.
    #[cfg(test)]
    pub fn inactive(&self, name: &str) -> Option<String> {
        self.inactive_with(name, &BTreeSet::new())
    }

    /// `inactive`, with the supervisor QSFP ports `enable` has turned on.
    pub fn inactive_with(&self, name: &str, enabled: &BTreeSet<(u32, u32)>) -> Option<String> {
        if self.uplink(name).is_some() {
            return self.uplink_off(name, enabled);
        }
        let Some((ty, member, 1, port)) = locate(name) else {
            return None;
        };
        let m = self.members.iter().find(|m| m.number == member)?;
        let within = |ps: Ports| {
            ps.iter()
                .any(|&(t, lo, hi)| t == ty && (lo..=hi).contains(&port))
        };
        let listed = model(&m.model).map_or(&[][..], |x| x.family.slot1());
        let Some(md) = m.module.as_deref().and_then(module) else {
            return within(listed)
                .then(|| format!("is listed by IOS; member {member} has no network module"));
        };
        if within(listed) && !within(md.ports) {
            return Some(if md.breakout && ty == "TenGigabitEthernet" {
                format!(
                    "is used by the {} only with a 4x10G breakout cable (hardware installation guide)",
                    md.name
                )
            } else {
                format!(
                    "is listed by IOS but not used by the {} (hardware installation guide)",
                    md.name
                )
            });
        }
        DISABLED_MODULE_PORTS
            .iter()
            .find(|(model, modules, lo, hi)| {
                m.model.eq_ignore_ascii_case(model)
                    && modules.iter().any(|x| x.eq_ignore_ascii_case(md.name))
                    && within(md.ports)
                    && (*lo..=*hi).contains(&port)
            })
            .map(|(model, _, lo, _)| {
                format!(
                    "is disabled: a {model} uses ports 1-{} of a {} (release notes)",
                    lo - 1,
                    md.name
                )
            })
    }

    /// Whether IOS would accept `name` (full type and number) on this stack.
    pub fn has(&self, name: &str) -> bool {
        let split = name
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(name.len());
        let (ty, num) = name.split_at(split);
        let in_range = |lo: u64, hi: u64| num.parse::<u64>().is_ok_and(|v| v >= lo && v <= hi);
        match ty {
            // channel-group numbers 1-128 (command reference, channel-group)
            "Port-channel" => in_range(1, 128),
            "Vlan" => in_range(1, 4094),
            "Loopback" => in_range(0, 2_147_483_647),
            "Tunnel" => in_range(0, 2_147_483_647),
            // one null interface (the lab C9300, 17.6.4: `ip route ... Null0`)
            "Null" => num == "0",
            _ => self.physical().iter().any(|p| p == name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster() -> Stack {
        Stack {
            members: (1..=2)
                .map(|n| Member {
                    number: n,
                    model: "C9300X-24Y".into(),
                    module: Some("C9300X-NM-4C".into()),
                })
                .collect(),
        }
    }

    #[test]
    fn a_two_member_24y_stack_with_nm_4c_modules() {
        let s = cluster();
        let p = s.physical();
        assert_eq!(p.len(), 1 + 2 * (24 + 4));
        assert_eq!(p[1], "TwentyFiveGigE1/0/1");
        assert!(s.has("TwentyFiveGigE2/0/24"));
        assert!(s.has("HundredGigE2/1/4"));
        assert!(!s.has("TwentyFiveGigE1/0/25"));
        assert!(!s.has("HundredGigE1/1/5"));
        assert!(!s.has("TwentyFiveGigE3/0/1"));
        assert!(!s.has("TenGigabitEthernet1/0/1"));
        assert!(s.has("GigabitEthernet0/0"));
        assert!(s.has("Port-channel128") && !s.has("Port-channel129"));
        assert!(s.has("Vlan450") && !s.has("Vlan4095"));
    }

    #[test]
    fn a_c9407r_with_two_supervisors_has_their_ports_and_leaves_half_inactive() {
        let card = |number: u32, model: &str| Member {
            number,
            model: model.into(),
            module: None,
        };
        let one = Stack {
            members: vec![card(2, "C9400-LC-24XS"), card(3, "C9400-SUP-1XL")],
        };
        assert!(one.has("TenGigabitEthernet3/0/8") && one.has("FortyGigabitEthernet3/0/10"));
        assert!(one.has("AppGigabitEthernet3/0/1") && !one.has("TenGigabitEthernet3/0/9"));
        assert!(
            one.inactive("TenGigabitEthernet3/0/5").is_none(),
            "one supervisor uses all its ports"
        );
        let two = Stack {
            members: vec![
                card(2, "C9400-LC-24XS"),
                card(3, "C9400-SUP-1XL"),
                card(4, "C9400-SUP-1XL"),
            ],
        };
        // port 9 too, as the cores show it (the data sheet keeps it)
        for p in [
            "TenGigabitEthernet3/0/5",
            "TenGigabitEthernet4/0/8",
            "FortyGigabitEthernet3/0/9",
            "FortyGigabitEthernet4/0/10",
        ] {
            assert!(two.inactive(p).is_some(), "{p}");
        }
        for p in ["TenGigabitEthernet3/0/4", "TenGigabitEthernet2/0/5"] {
            assert!(two.inactive(p).is_none(), "{p}");
        }
    }

    fn names(spec: &str) -> Vec<String> {
        checked_stack(spec, crate::release::DEFAULT)
            .unwrap()
            .physical()
    }

    #[test]
    fn every_9300_model_and_module_lead_to_their_ports() {
        let of = |f: Family| -> Vec<&str> {
            MODELS
                .iter()
                .filter(|m| m.family == f)
                .map(|m| m.name)
                .collect()
        };
        assert_eq!(
            of(Family::C9300),
            [
                "C9300-24T",
                "C9300-48T",
                "C9300-24S",
                "C9300-48S",
                "C9300-24P",
                "C9300-48P",
                "C9300-24U",
                "C9300-48U",
                "C9300-24H",
                "C9300-48H",
                "C9300-24UX",
                "C9300-48UXM",
                "C9300-48UN"
            ]
        );
        assert_eq!(
            of(Family::C9300B),
            ["C9300-24UB", "C9300-48UB", "C9300-24UXB"]
        );
        assert_eq!(
            of(Family::C9300L),
            [
                "C9300L-24T-4G",
                "C9300L-48T-4G",
                "C9300L-24T-4X",
                "C9300L-48T-4X",
                "C9300L-24P-4G",
                "C9300L-48P-4G",
                "C9300L-24P-4X",
                "C9300L-48P-4X",
                "C9300L-48PF-4G",
                "C9300L-48PF-4X",
                "C9300L-24UXG-4X",
                "C9300L-24UXG-2Q",
                "C9300L-48UXG-4X",
                "C9300L-48UXG-2Q"
            ]
        );
        assert_eq!(
            of(Family::C9300LM),
            [
                "C9300LM-48T-4Y",
                "C9300LM-24U-4Y",
                "C9300LM-48U-4Y",
                "C9300LM-48UX-4Y"
            ]
        );
        // mixed-speed fronts: the 48UXM's ports 1-36 are 2.5G and 37-48 10G (installation guide
        // figure), the 48UN's all 5G, the 48HXN's 40 5G and 8 10G
        let m = names("C9300-48UXM,C9300-48UN,C9300X-48HXN+C9300X-NM-8M");
        for n in [
            "TwoGigabitEthernet1/0/36",
            "TenGigabitEthernet1/0/37",
            "FiveGigabitEthernet2/0/48",
            "FiveGigabitEthernet3/0/40",
            "TenGigabitEthernet3/0/41",
            "AppGigabitEthernet1/0/1",
        ] {
            assert!(m.contains(&n.to_string()), "{n}");
        }
        for n in [
            "TwoGigabitEthernet1/0/37",
            "GigabitEthernet2/0/1",
            "FiveGigabitEthernet3/0/41",
        ] {
            assert!(!m.contains(&n.to_string()), "{n}");
        }
        // a B model stacks only with B models, and takes the 9300's modules and slot
        let b = names("C9300-24UB+C9300-NM-8X,C9300-48UB");
        assert!(b.contains(&"TenGigabitEthernet1/1/8".to_string()));
        assert!(b.contains(&"FortyGigabitEthernet1/1/2".to_string()));
        assert!(b.contains(&"AppGigabitEthernet2/0/1".to_string()));
        assert!(checked_stack("C9300-24UB,C9300-24U", crate::release::DEFAULT).is_err());
        // a model newer than the release is refused (C9300-24H, 17.2.1)
        assert!(checked_stack("C9300-24H", Release::parse("17.1").unwrap()).is_err());
        assert!(checked_stack("C9300-24H", Release::parse("17.2").unwrap()).is_ok());
        assert!(checked_stack("C9300L-48PF-4G", Release::parse("16.11").unwrap()).is_err());
        // a 9300 lists the whole slot whatever is fitted, in IOS's order, then the app port
        let p = names("C9300-48T+C9300-NM-8X");
        let at = |n: &str| {
            p.iter()
                .position(|x| x == n)
                .unwrap_or_else(|| panic!("{n}"))
        };
        assert!(at("GigabitEthernet1/0/48") < at("GigabitEthernet1/1/1"));
        assert!(at("GigabitEthernet1/1/4") < at("TenGigabitEthernet1/1/1"));
        assert!(at("TenGigabitEthernet1/1/8") < at("FortyGigabitEthernet1/1/1"));
        assert!(at("FortyGigabitEthernet1/1/2") < at("TwentyFiveGigE1/1/1"));
        assert!(at("TwentyFiveGigE1/1/2") < at("AppGigabitEthernet1/0/1"));
        assert_eq!(p.len(), 1 + 48 + 4 + 8 + 2 + 2 + 1);
        assert_eq!(
            p,
            names("C9300-48T"),
            "the slot does not change with the module"
        );
        assert_eq!(names("C9300-24S").len(), 1 + 24 + 16 + 1);
        let l = names("C9300L-48T-4X,C9300L-24T-4G,C9300LM-48T-4Y");
        for n in [
            "TenGigabitEthernet1/1/4",
            "GigabitEthernet2/0/24",
            "GigabitEthernet2/1/4",
            "GigabitEthernet3/0/48",
            "TwentyFiveGigE3/1/4",
            // a 9300L's application hosting port (two C9300L-24T-4Gs at 17.6.5)
            "AppGigabitEthernet1/0/1",
            "AppGigabitEthernet2/0/1",
        ] {
            assert!(l.contains(&n.to_string()), "{n}");
        }
        for n in [
            "TenGigabitEthernet1/1/5",
            "TenGigabitEthernet2/1/1",
            "TwentyFiveGigE3/1/5",
            "AppGigabitEthernet3/0/1",
        ] {
            assert!(!l.contains(&n.to_string()), "{n}");
        }
        // a 9300X lists its module's ports only
        let x = names("C9300X-24Y+C9300X-NM-4C");
        assert!(x.contains(&"HundredGigE1/1/4".to_string()));
        assert!(!x.contains(&"GigabitEthernet1/1/1".to_string()));
    }

    #[test]
    fn interfaces_ios_lists_that_carry_no_traffic_are_named() {
        let s = checked_stack(
            "C9300-48T+C9300-NM-4G,C9300-48T+C9300-NM-2Q,C9300-48T+C3850-NM-2-10G,C9300-48T",
            crate::release::DEFAULT,
        )
        .unwrap();
        for used in [
            "GigabitEthernet1/1/4",
            "FortyGigabitEthernet2/1/2",
            "GigabitEthernet3/1/2",
            "TenGigabitEthernet3/1/3",
            "GigabitEthernet4/0/48",
            "AppGigabitEthernet4/0/1",
        ] {
            assert_eq!(s.inactive(used), None, "{used}");
        }
        for (unused, why) in [
            ("TenGigabitEthernet1/1/1", "not used by the C9300-NM-4G"),
            ("TwentyFiveGigE1/1/2", "not used by the C9300-NM-4G"),
            (
                "TenGigabitEthernet2/1/8",
                "only with a 4x10G breakout cable",
            ),
            ("GigabitEthernet2/1/1", "not used by the C9300-NM-2Q"),
            ("TenGigabitEthernet3/1/1", "not used by the C3850-NM-2-10G"),
            ("GigabitEthernet3/1/3", "not used by the C3850-NM-2-10G"),
            ("TenGigabitEthernet4/1/1", "member 4 has no network module"),
        ] {
            let got = s.inactive(unused).unwrap_or_default();
            assert!(got.contains(why), "{unused}: {got}");
        }
        let hxn = checked_stack(
            "C9300X-48HXN+C9300X-NM-8Y,C9300X-48HX+C9300X-NM-8Y",
            crate::release::DEFAULT,
        )
        .unwrap();
        assert!(
            hxn.inactive("TwentyFiveGigE1/1/7")
                .is_some_and(|w| w.contains("uses ports 1-6 of a C9300X-NM-8Y"))
        );
        for used in [
            "TwentyFiveGigE1/1/6",
            "TwentyFiveGigE2/1/7",
            "TwentyFiveGigE2/1/8",
        ] {
            assert_eq!(hxn.inactive(used), None, "{used}");
        }
    }

    #[test]
    fn a_provision_line_names_the_model_and_slot_1_the_module() {
        assert_eq!(
            infer_stack(
                "switch 1 provision c9300l-48t-4x\nswitch 2 provision c9300lm-48t-4y\n\
                 interface GigabitEthernet1/0/1\n"
            )
            .as_deref(),
            Some("C9300L-48T-4X,C9300LM-48T-4Y")
        );
        // a running configuration lists the whole slot, so the module stays unnamed
        let mut run = String::from("switch 1 provision c9300-48t\n");
        for p in names("C9300-48T") {
            run.push_str(&format!("interface {p}\n"));
        }
        assert_eq!(infer_stack(&run).as_deref(), Some("C9300-48T"));
        // a generated configuration names the ports it sets
        for (config, stack) in [
            (
                "interface GigabitEthernet1/0/2\ninterface TenGigabitEthernet1/1/8\n",
                "C9300-24T+C9300-NM-8X",
            ),
            (
                "interface GigabitEthernet1/0/30\ninterface FortyGigabitEthernet1/1/1\n",
                "C9300-48T+C9300-NM-2Q",
            ),
            (
                "switch 1 provision c9300-24s\ninterface TenGigabitEthernet1/1/2\n",
                "C9300-24S+C9300-NM-8X",
            ),
            (
                "interface GigabitEthernet1/0/2\ninterface TenGigabitEthernet1/1/1\n",
                "C9200-24T+C9200-NM-4X",
            ),
            // 2.5G and 5G ports name their models; PoE lines a PoE model
            ("interface TwoGigabitEthernet1/0/3\n", "C9300-48UXM"),
            ("interface FiveGigabitEthernet1/0/3\n", "C9300-48UN"),
            (
                "interface FiveGigabitEthernet1/0/3\ninterface TenGigabitEthernet1/0/44\n",
                "C9300X-48HXN",
            ),
            (
                "interface GigabitEthernet1/0/2\n power inline never\ninterface TenGigabitEthernet1/1/8\n",
                "C9300-24P+C9300-NM-8X",
            ),
            (
                "interface GigabitEthernet1/0/30\n power inline static\ninterface TenGigabitEthernet1/1/1\n",
                "C9200-48P+C9200-NM-4X",
            ),
        ] {
            assert_eq!(infer_stack(config).as_deref(), Some(stack), "{config}");
        }
    }

    #[test]
    fn hardware_runs_from_the_release_cisco_introduced_it_in() {
        let r = |t| Release::parse(t).unwrap();
        for (spec, release, why) in [
            (
                "C9300X-24Y+C9300X-NM-2C",
                "17.4",
                "the C9300X-24Y runs IOS XE 17.5 and later (release notes); this switch runs IOS XE 17.4",
            ),
            (
                "C9300X-48TX+C9300X-NM-4C",
                "17.5",
                "the C9300X-NM-4C runs IOS XE 17.6",
            ),
            (
                "C9300X-24HX+C9300X-NM-8M",
                "17.8",
                "the C9300X-24HX runs IOS XE 17.9",
            ),
            (
                "C9300L-48T-4X",
                "16.10",
                "the C9300L-48T-4X runs IOS XE 16.11",
            ),
            (
                "C9300-48T+C9300-NM-2Y",
                "16.6",
                "the C9300-NM-2Y runs IOS XE 16.8",
            ),
            (
                "C9200-48T+C9200-NM-4X",
                "16.9",
                "the C9200-48T runs IOS XE 16.10",
            ),
            (
                "C9200-24PXG+C9200-NM-2Y",
                "16.12",
                "the C9200-24PXG runs IOS XE 17.1",
            ),
        ] {
            let e = checked_stack(spec, r(release)).unwrap_err();
            assert!(e.contains(why), "{spec} at {release}: {e}");
        }
        for (spec, release) in [
            ("C9300X-24Y+C9300X-NM-2C", "17.5"),
            ("C9300X-48TX+C9300X-NM-4C", "17.6"),
            ("C9300-48T+C3850-NM-2-40G", "16.5"),
            ("C9200L-48T-4X", "16.9"),
            ("C9200-24PXG+C9200-NM-2Y", "26.x"),
            ("2=C9400-LC-24XS,3=C9400-SUP-1XL", "16.6"),
        ] {
            assert!(
                checked_stack(spec, r(release)).is_ok(),
                "{spec} at {release}"
            );
        }
    }

    #[test]
    fn a_9400_chassis_follows_its_supervisors_slots() {
        let r = Release::parse("17.15").unwrap();
        // the cores: supervisors in 3 and 4 make a C9407R, line cards in 1, 2 and 5
        assert!(
            checked_stack(
                "1=C9400-LC-48S,2=C9400-LC-24XS,3=C9400-SUP-1XL,4=C9400-SUP-1XL,5=C9400-LC-48U",
                r
            )
            .is_ok()
        );
        // one supervisor in slot 3 fits a C9404R or a C9407R: slot 5 only the C9407R
        assert!(checked_stack("1=C9400-LC-48T,3=C9400-SUP-1XL,5=C9400-LC-48T", r).is_ok());
        for (spec, why) in [
            ("1=C9400-LC-48T", "a Catalyst 9400 needs a supervisor"),
            (
                "4=C9400-SUP-1XL,5=C9400-SUP-1XL",
                "a supervisor goes in slot 2 or 3 (C9404R)",
            ),
            (
                "3=C9400-SUP-1XL,4=C9400-SUP-1",
                "redundant supervisors are one model",
            ),
            (
                "2=C9400-SUP-1XL,7=C9400-LC-48T",
                "slot 7 is not a line card slot of a C9404R line card slots 1 and 4",
            ),
            (
                "5=C9400-SUP-1XL,6=C9400-SUP-1XL,7=C9400-SUP-1XL",
                "at most two supervisors",
            ),
            // slot 2 is a C9407R card slot and slot 4 a C9404R one; no chassis has both with 3
            (
                "2=C9400-LC-48T,3=C9400-SUP-1XL,4=C9400-LC-48T",
                "no one chassis has a supervisor in slot 3 and line cards in slots 2 and 4",
            ),
        ] {
            let e = checked_stack(spec, r).unwrap_err();
            assert!(e.contains(why), "{spec}: {e}");
        }
    }

    #[test]
    fn a_plain_2960_stops_at_15_0_2_se_where_a_2960_plus_runs_15_2() {
        let r = |t| Release::parse(t).unwrap();
        for model in ["WS-C2960-24TC-L", "WS-C2960G-24TC-L", "WS-C2960G-8TC-L"] {
            assert!(checked_stack(model, r("15.0(2)SE")).is_ok(), "{model}");
            let e = checked_stack(model, r("15.2(7)E")).unwrap_err();
            assert!(
                e.contains(&format!("the {model} runs up to IOS 15.0(2)SE")),
                "{model}: {e}"
            );
        }
        assert!(checked_stack("WS-C2960+24TC-L", r("15.2(7)E")).is_ok());
    }

    #[test]
    fn a_9400_supervisor_and_card_run_only_their_releases() {
        let r = |t| Release::parse(t).unwrap();
        let e = checked_stack("1=C9400-LC-48T,3=C9400-SUP-1XL", r("26.x")).unwrap_err();
        assert!(
            e.contains("the C9400-SUP-1XL runs up to IOS XE 17.18"),
            "{e}"
        );
        assert!(checked_stack("1=C9400-LC-48T,3=C9400-SUP-1XL", r("17.18")).is_ok());
        assert!(checked_stack("1=C9400-LC-48T,3=C9400X-SUP-2", r("26.x")).is_ok());
        let e = checked_stack("1=C9400-LC-48HN,3=C9400X-SUP-2", r("17.4")).unwrap_err();
        assert!(e.contains("the C9400-LC-48HN runs IOS XE 17.5"), "{e}");
        let s = checked_stack(
            "1=C9400-LC-48HN,3=C9400X-SUP-2XL,4=C9400X-SUP-2XL",
            r("17.15"),
        )
        .unwrap();
        assert!(s.has("FiveGigabitEthernet1/0/48") && !s.has("GigabitEthernet1/0/1"));
        assert!(
            s.has("TwentyFiveGigE3/0/4") && s.has("HundredGigE4/0/8") && !s.has("HundredGigE4/0/9")
        );
        let ux = checked_stack("1=C9400-LC-48UX,3=C9400-SUP-1", r("17.15")).unwrap();
        assert!(ux.has("GigabitEthernet1/0/24") && ux.has("TenGigabitEthernet1/0/25"));
    }

    /// The installation note's uplink tables, port by port.
    fn running(s: &Stack, enabled: &[(u32, u32)], names: &[&str]) -> Vec<bool> {
        let enabled: BTreeSet<(u32, u32)> = enabled.iter().copied().collect();
        names
            .iter()
            .map(|n| s.inactive_with(n, &enabled).is_none())
            .collect()
    }

    #[test]
    fn a_2960x_numbers_its_uplinks_on_slot_0_and_manages_on_fastethernet0() {
        let r = Release::parse("15.2(7)E").unwrap();
        // a 24PS-L's running configuration: Gi1/0/1-28, FastEthernet0
        let ps = checked_stack("WS-C2960X-24PS-L", r).unwrap();
        let names = ps.physical();
        assert_eq!(names.first().map(String::as_str), Some("FastEthernet0"));
        assert_eq!(names.len(), 1 + 28);
        assert_eq!(
            names.last().map(String::as_str),
            Some("GigabitEthernet1/0/28")
        );
        assert_eq!(ps.management(), Some("FastEthernet0"));
        assert_eq!(ps.media("GigabitEthernet1/0/1"), Media::Copper);
        assert_eq!(ps.media("GigabitEthernet1/0/25"), Media::Sfp);
        // 100BASE-FX as well: nine 2960-X uplinks run it to the cores
        assert_eq!(
            ps.slot_speeds("GigabitEthernet1/0/25"),
            Some(vec![1000, 100])
        );
        // PoE+ on the front ports only, at the data sheet's 370 W
        assert_eq!(
            ps.poe("GigabitEthernet1/0/1"),
            Some(Poe {
                kind: PoeKind::PoePlus,
                watts: Some(370)
            })
        );
        assert_eq!(ps.poe("GigabitEthernet1/0/25"), None);
        // a TD model's SFP+ uplinks
        let td = checked_stack("WS-C2960X-48TD-L", r).unwrap();
        assert!(td.has("TenGigabitEthernet1/0/2") && !td.has("GigabitEthernet1/0/49"));
        assert_eq!(td.media("TenGigabitEthernet1/0/1"), Media::SfpPlus);
        assert_eq!(
            td.slot_speeds("TenGigabitEthernet1/0/1"),
            Some(vec![10_000, 1000])
        );
        assert_eq!(td.poe("GigabitEthernet1/0/1"), None);
        // FlexStack-Plus: 2960-X members only; no network module
        assert!(checked_stack("WS-C2960X-24PS-L,WS-C2960X-48TD-L", r).is_ok());
        assert!(checked_stack("WS-C2960X-24PS-L,C9200-24T", r).is_err());
        assert!(checked_stack("WS-C2960X-24PS-L+C9200-NM-4X", r).is_err());
        // a 9300 keeps GigabitEthernet0/0
        assert_eq!(
            checked_stack("C9300-48P", crate::release::DEFAULT)
                .unwrap()
                .management(),
            Some("GigabitEthernet0/0")
        );
    }

    #[test]
    fn a_9400_supervisor_s_uplinks_follow_the_installation_note() {
        let r = Release::parse("17.15").unwrap();
        // a Supervisor 1: ports 1-4 or QSFP port 9, ports 5-8 or QSFP port 10; a QSFP port runs once
        // `enable` turns it on
        let one = checked_stack("1=C9400-LC-48T,3=C9400-SUP-1XL", r).unwrap();
        let sup1 = [
            "TenGigabitEthernet3/0/1",
            "TenGigabitEthernet3/0/5",
            "FortyGigabitEthernet3/0/9",
            "FortyGigabitEthernet3/0/10",
        ];
        assert_eq!(
            running(&one, &[], &sup1),
            [true, true, false, false],
            "8 + 0"
        );
        assert_eq!(
            running(&one, &[(3, 9), (3, 10)], &sup1),
            [false, false, true, true],
            "2 + 0"
        );
        assert_eq!(
            running(&one, &[(3, 10)], &sup1),
            [true, false, false, true],
            "5 + 0: ports 1-4 and port 10"
        );
        assert!(
            one.inactive("FortyGigabitEthernet3/0/9")
                .unwrap()
                .contains("`enable`")
        );
        // two: 5-8 and 10 never run; each supervisor runs 1-4 or port 9
        let pair = checked_stack("1=C9400-LC-48T,3=C9400-SUP-1XL,4=C9400-SUP-1XL", r).unwrap();
        let both = [
            "TenGigabitEthernet3/0/1",
            "TenGigabitEthernet4/0/1",
            "FortyGigabitEthernet3/0/9",
            "FortyGigabitEthernet4/0/9",
            "FortyGigabitEthernet3/0/10",
            "TenGigabitEthernet4/0/5",
        ];
        assert_eq!(
            running(&pair, &[], &both),
            [true, true, false, false, false, false],
            "4 + 4"
        );
        assert_eq!(
            running(&pair, &[(3, 9)], &both),
            [false, true, true, false, false, false],
            "1 + 4"
        );
        // a Supervisor 2: ports 1-4 or port 5; two never run 7 and 8
        let two = checked_stack("1=C9400-LC-48T,3=C9400X-SUP-2", r).unwrap();
        let sup2 = [
            "TwentyFiveGigE3/0/1",
            "HundredGigE3/0/5",
            "HundredGigE3/0/6",
            "HundredGigE3/0/8",
        ];
        assert_eq!(
            running(&two, &[], &sup2),
            [true, false, true, true],
            "7 + 0"
        );
        assert_eq!(
            running(&two, &[(3, 5)], &sup2),
            [false, true, true, true],
            "4 + 0"
        );
        let pair2 = checked_stack("1=C9400-LC-48T,3=C9400X-SUP-2,4=C9400X-SUP-2", r).unwrap();
        assert_eq!(
            running(&pair2, &[], &sup2),
            [true, false, true, false],
            "5 + 5: ports 1-4 and 6"
        );
        assert_eq!(
            running(&pair2, &[(3, 5)], &sup2),
            [false, true, true, false],
            "2 + 2: ports 5 and 6"
        );
        assert!(pair2.inactive("AppGigabitEthernet3/0/1").is_none());
        assert_eq!(pair2.needs_enable("HundredGigE4/0/5"), Some((4, 5)));
        assert_eq!(pair2.needs_enable("HundredGigE4/0/6"), None);
    }

    #[test]
    fn an_ie_3300_names_its_ports_on_slot_1_sfp_first() {
        let r = Release::parse("17.6").unwrap();
        // ie3300 17.6 capture 1: SFP ports Gi1/1-2, copper Gi1/3-10, no management port, one switch
        let ie = checked_stack("IE-3300-8T2S", r).unwrap();
        let names = ie.physical();
        assert_eq!(names.len(), 10);
        assert_eq!(names[0], "GigabitEthernet1/1");
        assert_eq!(names[9], "GigabitEthernet1/10");
        assert_eq!(ie.management(), None);
        assert!(!ie.classic());
        assert_eq!(ie.media("GigabitEthernet1/2"), Media::Sfp);
        assert_eq!(ie.media("GigabitEthernet1/3"), Media::Copper);
        assert_eq!(
            locate("GigabitEthernet1/10"),
            Some(("GigabitEthernet", 1, 0, 10))
        );
        assert!(checked_stack("IE-3300-8T2S,IE-3300-8T2S", r).is_err());
        let config = "version 17.6\ninterface GigabitEthernet1/1\ninterface GigabitEthernet1/10\n";
        assert_eq!(infer_stack(config).as_deref(), Some("IE-3300-8T2S"));
    }

    #[test]
    fn a_2960_names_its_ports_in_two_parts_and_does_not_stack() {
        let r = Release::parse("15.2(6)E").unwrap();
        // a model whose name holds a `+`; 24 Fast Ethernet ports, 2 dual-purpose uplinks, no
        // management port (c2960 15.2(6)E capture 5)
        let plus = checked_stack("WS-C2960+24TC-L", r).unwrap();
        assert_eq!(plus.members[0].model, "WS-C2960+24TC-L");
        assert_eq!(plus.members[0].module, None);
        let names = plus.physical();
        assert_eq!(names.len(), 26);
        assert_eq!(names.first().map(String::as_str), Some("FastEthernet0/1"));
        assert_eq!(names.last().map(String::as_str), Some("GigabitEthernet0/2"));
        assert_eq!(plus.management(), None);
        assert!(plus.classic());
        assert_eq!(plus.media("FastEthernet0/3"), Media::Copper);
        assert_eq!(plus.media("GigabitEthernet0/2"), Media::Sfp);
        assert_eq!(locate("FastEthernet0/3"), Some(("FastEthernet", 1, 0, 3)));
        assert_eq!(
            locate("GigabitEthernet2/0/3"),
            Some(("GigabitEthernet", 2, 0, 3))
        );
        assert!(checked_stack("WS-C2960+24TC-L,WS-C2960+24TC-L", r).is_err());
        // a configuration's two-part names give the model; 12.2 is a 2960, 15.2 a 2960-Plus
        let fa = "interface FastEthernet0/24\ninterface GigabitEthernet0/2\n";
        assert_eq!(
            infer_stack(&format!("version 12.2\n{fa}")).as_deref(),
            Some("WS-C2960-24TC-L")
        );
        assert_eq!(
            infer_stack(&format!("version 15.2\n{fa}")).as_deref(),
            Some("WS-C2960+24TC-L")
        );
        assert_eq!(
            infer_stack("interface FastEthernet0/8\ninterface GigabitEthernet0/2\n").as_deref(),
            Some("WS-C2960C-8TC-L")
        );
        assert_eq!(
            infer_stack("interface GigabitEthernet0/24\n").as_deref(),
            Some("WS-C2960G-24TC-L")
        );
        // an IOS XE switch's management port names no 2960
        assert_eq!(
            infer_stack("interface GigabitEthernet0/0\ninterface GigabitEthernet1/0/24\n")
                .as_deref(),
            Some("C9200-24T")
        );
    }

    /// A configuration listing `s`'s interfaces, as its running configuration does.
    fn listing(s: &Stack, head: &str) -> String {
        let mut config = head.to_string();
        for p in s.physical() {
            config.push_str(&format!("interface {p}\n"));
        }
        config
    }

    #[test]
    fn every_model_s_own_interfaces_infer_hardware_with_those_ports() {
        // a B model lists its sibling's ports; a 9400 card sits beside a supervisor; an IE 3000 or
        // IE 3010 configuration carries its alarm profile, as the captures do
        let same = |f: Option<Family>| match f {
            Some(Family::C9300B) => Some(Family::C9300),
            Some(Family::C9200B) => Some(Family::C9200),
            f => f,
        };
        for m in MODELS {
            let release = match m.family {
                Family::C2960 | Family::IE3010 => "12.2",
                Family::IE3300 => "17.6",
                f if f.classic_ios() => "15.2",
                _ => "17.15",
            };
            let spec = match m.family {
                Family::C9400 if is_supervisor(m.name) => format!("1=C9400-LC-48U,3={}", m.name),
                Family::C9400 => format!("1={},3=C9400-SUP-1XL", m.name),
                _ => m.name.to_string(),
            };
            let r = Release::parse(release).unwrap();
            let want = checked_stack(&spec, r).unwrap();
            let alarm = if matches!(m.family, Family::IE3000 | Family::IE3010) {
                "alarm profile defaultPort\n"
            } else {
                ""
            };
            let config = listing(&want, &format!("version {release}\n{alarm}"));
            let got = infer_stack(&config).unwrap_or_else(|| panic!("{spec}: none inferred"));
            let s = checked_stack(&got, r).unwrap_or_else(|e| panic!("{spec}: {got}: {e}"));
            assert_eq!(s.physical(), want.physical(), "{spec}: {got}");
            assert_eq!(same(s.family()), same(want.family()), "{spec}: {got}");
        }
    }

    #[cfg(all(test, captures))]
    include!(concat!(env!("VSWITCH_SITE_TESTS"), "/platform-tests.rs"));

    #[test]
    fn a_9400_is_inferred_from_either_supervisor_s_ports_and_its_cards_from_theirs() {
        let ports = |ty: &str, slot: u32, lo: u32, hi: u32| -> String {
            (lo..=hi)
                .map(|p| format!("interface {ty}{slot}/0/{p}\n"))
                .collect()
        };
        let sup1 = ports("TenGigabitEthernet", 3, 1, 8) + &ports("FortyGigabitEthernet", 3, 9, 10);
        let sup2 = ports("TwentyFiveGigE", 3, 1, 4) + &ports("HundredGigE", 3, 5, 8);
        for (config, stack) in [
            (
                ports("GigabitEthernet", 1, 1, 48) + &sup2,
                "1=C9400-LC-48U,3=C9400X-SUP-2",
            ),
            (
                ports("FiveGigabitEthernet", 1, 1, 48) + &sup1,
                "1=C9400-LC-48HN,3=C9400-SUP-1XL",
            ),
            (
                ports("TenGigabitEthernet", 1, 1, 48) + &sup1,
                "1=C9400-LC-48XS,3=C9400-SUP-1XL",
            ),
            (
                ports("GigabitEthernet", 1, 1, 24)
                    + &ports("TenGigabitEthernet", 1, 25, 48)
                    + &sup1,
                "1=C9400-LC-48UX,3=C9400-SUP-1XL",
            ),
            (
                ports("FortyGigabitEthernet", 1, 1, 12) + &sup1,
                "1=C9400-LC-12QC,3=C9400-SUP-1XL",
            ),
            (
                ports("TwentyFiveGigE", 1, 1, 20) + &ports("TenGigabitEthernet", 1, 21, 24) + &sup1,
                "1=C9400-LC-24XY,3=C9400-SUP-1XL",
            ),
            // a generated configuration names a port of each: the card keeps its own slot, the
            // supervisor its QSFP port
            (
                "interface TenGigabitEthernet2/0/3\ninterface FortyGigabitEthernet3/0/9\n".into(),
                "2=C9400-LC-24XS,3=C9400-SUP-1XL",
            ),
        ] {
            assert_eq!(infer_stack(&config).as_deref(), Some(stack), "{config}");
        }
    }

    #[test]
    fn inferred_hardware_takes_its_module_and_runs_the_configuration_s_release() {
        // the smallest front panel is passed over where it takes no such module or runs no such
        // release
        for (config, release, stack) in [
            (
                "interface TwentyFiveGigE1/0/5\ninterface HundredGigE1/1/4\n",
                "17.15",
                "C9300X-24Y+C9300X-NM-4C",
            ),
            (
                "interface TenGigabitEthernet1/0/5\ninterface HundredGigE1/1/3\n",
                "17.15",
                "C9300X-48TX+C9300X-NM-4C",
            ),
            (
                "version 17.6\ninterface TenGigabitEthernet1/0/5\n",
                "17.6",
                "C9300X-48TX",
            ),
            // a classic release's three-part names are a 2960-X's
            (
                "version 15.2\ninterface GigabitEthernet1/0/3\n",
                "15.2(7)E",
                "WS-C2960X-24TS-L",
            ),
        ] {
            assert_eq!(infer_stack(config).as_deref(), Some(stack), "{config}");
            assert!(checked_stack(stack, Release::parse(release).unwrap()).is_ok());
        }
        // a stack of 2960-Xs takes models with FlexStack-Plus, not the LAN Lite 50-port one
        let two: String = (1..=2)
            .flat_map(|n| (1..=50).map(move |p| format!("interface GigabitEthernet{n}/0/{p}\n")))
            .collect();
        assert_eq!(
            infer_stack(&format!("version 15.2\ninterface FastEthernet0\n{two}")).as_deref(),
            Some("WS-C2960X-48TS-L,WS-C2960X-48TS-L")
        );
        // where no hardware runs the release, the hardware the interfaces fit, which the release
        // check then names
        let config = "version 17.5\n".to_string()
            + &(1..=48)
                .map(|p| format!("interface TenGigabitEthernet1/0/{p}\n"))
                .collect::<String>()
            + "interface FortyGigabitEthernet3/0/9\n";
        assert_eq!(
            infer_stack(&config).as_deref(),
            Some("1=C9400-LC-48XS,3=C9400-SUP-1XL")
        );
    }

    #[test]
    fn transceiver_slots_rj45_ports_and_dual_purpose_uplinks_follow_the_data_sheets() {
        let r = crate::release::DEFAULT;
        let classic = |t| Release::parse(t).unwrap();
        // the IE-3010's 16 SFP ports take 100 Mb/s optics; the other 8 are copper
        let ie = checked_stack("IE-3010-16S-8PC", classic("12.2(53)EZ")).unwrap();
        assert_eq!(ie.slot_speeds("FastEthernet0/16"), Some(vec![100]));
        assert_eq!(ie.slot_speeds("FastEthernet0/17"), None);
        // the C9400-LC-24XY's TenGigabitEthernet 21-24 are SFP+
        let xy = checked_stack("1=C9400-LC-24XY,3=C9400-SUP-1XL", r).unwrap();
        assert_eq!(xy.media("TenGigabitEthernet1/0/21"), Media::SfpPlus);
        assert_eq!(
            xy.slot_speeds("TenGigabitEthernet1/0/24"),
            Some(vec![10_000, 1000])
        );
        // a Multigigabit module's ports are RJ-45, an SFP+ module's transceiver slots
        let nm = checked_stack(
            "C9300-48T+C9300-NM-4M,C9300-48T+C9300-NM-8X,C9300X-24Y+C9300X-NM-8M",
            r,
        )
        .unwrap();
        assert_eq!(nm.slot_speeds("TenGigabitEthernet1/1/4"), None);
        assert_eq!(
            nm.slot_speeds("TenGigabitEthernet2/1/4"),
            Some(vec![10_000, 1000])
        );
        assert_eq!(nm.slot_speeds("TenGigabitEthernet3/1/8"), None);
        // the management port is copper whatever member 1's front panel is
        for spec in ["C9300-24S", "1=C9400-LC-48S,3=C9400-SUP-1XL"] {
            let s = checked_stack(spec, r).unwrap();
            assert_eq!(s.media("GigabitEthernet0/0"), Media::Copper, "{spec}");
            assert_eq!(s.slot_speeds("GigabitEthernet0/0"), None, "{spec}");
            assert!(s.slot_speeds("GigabitEthernet1/0/1").is_some(), "{spec}");
        }
        // a 2960's and an IE 3010's uplinks are dual-purpose; a 2960-X's and an IE-3300's are SFP
        let g = checked_stack("WS-C2960G-24TC-L", classic("15.0(2)SE")).unwrap();
        assert!(g.dual_purpose("GigabitEthernet0/21") && !g.dual_purpose("GigabitEthernet0/20"));
        assert!(ie.dual_purpose("GigabitEthernet0/2"));
        let x = checked_stack("WS-C2960X-24PS-L", classic("15.2(7)E")).unwrap();
        assert!(!x.dual_purpose("GigabitEthernet1/0/25"));
        let i = checked_stack("IE-3300-8T2S", classic("17.6")).unwrap();
        assert!(!i.dual_purpose("GigabitEthernet1/1"));
    }

    #[test]
    fn members_are_numbered_within_their_stack_and_only_stacking_models_stack() {
        let r = crate::release::DEFAULT;
        let classic = Release::parse("15.2(7)E").unwrap();
        // 0 is no member number: problems name the whole stack as 0
        let e = parse_stack("0=C9300-48P").unwrap_err();
        assert!(e.contains("members are numbered from 1"), "{e}");
        for (spec, why) in [
            (
                "9=C9300-48P",
                "member 9: a Catalyst 9300 stack numbers its members 1 to 8",
            ),
            (
                "1=C9300-48P,12=C9300-48P",
                "member 12: a Catalyst 9300 stack numbers its members 1 to 8",
            ),
        ] {
            let e = checked_stack(spec, r).unwrap_err();
            assert!(e.contains(why), "{spec}: {e}");
        }
        // a switch renumbered within its stack keeps its number; a 9400's members are its slots
        for spec in [
            "2=C9200L-24T-4G",
            "1=C9300-48P,8=C9300-48P",
            "1=C9400-LC-48T,3=C9400-SUP-1XL",
        ] {
            assert!(checked_stack(spec, r).is_ok(), "{spec}");
        }
        // a family that does not stack: its one switch is 1, and a second is refused once
        let e = checked_stack("2=WS-C2960+24TC-L", classic).unwrap_err();
        assert!(
            e.contains("member 2: a Catalyst 2960-Plus is one switch, numbered 1"),
            "{e}"
        );
        assert_eq!(
            checked_stack("WS-C2960+24TC-L,WS-C2960+24TC-L", classic).unwrap_err(),
            "a Catalyst 2960-Plus is one switch: it does not stack"
        );
        // no FlexStack-Plus on the 2960-X LAN Lite models and the 24PSQ-L (data sheet)
        for spec in [
            "WS-C2960X-48TS-LL,WS-C2960X-24TS-LL",
            "WS-C2960X-24PS-L,WS-C2960X-24PSQ-L",
        ] {
            let e = checked_stack(spec, classic).unwrap_err();
            assert!(e.contains("has no FlexStack-Plus"), "{spec}: {e}");
        }
        assert!(checked_stack("WS-C2960X-48TS-LL", classic).is_ok());
    }
}
