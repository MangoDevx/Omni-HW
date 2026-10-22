//! Power management: power plans and their settings, Fast Startup, wake sources, power
//! requests and per-device power saving. The data model and parsers are platform-agnostic;
//! reads and writes live in `win::power`, which returns empty data on Linux so far.
//!
//! Gathering is split in two: [`PowerInfo::gather_settings`] (power plans and Fast Startup,
//! cheap registry reads) and [`PowerInfo::gather`] (also devices, wake history and power
//! requests). Both run on the background sampler thread, never in the UI.

use crate::win::PlanKind;

/// A power plan (scheme).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scheme {
    /// Lower-case hyphenated GUID.
    pub guid: String,
    pub name: String,
    pub kind: PlanKind,
}

/// The built-in Balanced plan.
pub const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";

/// How a setting's value is shown and edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Timeout in seconds; 0 means "Never".
    Seconds,
    Percent,
    /// Fixed choices: (value, label).
    Choice(&'static [(u32, &'static str)]),
}

const BUTTON_ACTIONS: &[(u32, &str)] = &[
    (0, "Do nothing"),
    (1, "Sleep"),
    (2, "Hibernate"),
    (3, "Shut down"),
    (4, "Turn off the display"),
];
const LID_ACTIONS: &[(u32, &str)] = &[
    (0, "Do nothing"),
    (1, "Sleep"),
    (2, "Hibernate"),
    (3, "Shut down"),
];
const ON_OFF: &[(u32, &str)] = &[(0, "Disabled"), (1, "Enabled")];
const ASPM: &[(u32, &str)] = &[
    (0, "Off"),
    (1, "Moderate power savings"),
    (2, "Maximum power savings"),
];
const WAKE_TIMERS: &[(u32, &str)] = &[
    (0, "Disabled"),
    (1, "Enabled"),
    (2, "Important wake timers only"),
];
const WIRELESS: &[(u32, &str)] = &[
    (0, "Maximum performance"),
    (1, "Low power saving"),
    (2, "Medium power saving"),
    (3, "Maximum power saving"),
];

/// A power plan setting this app reads and can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Setting {
    DisplayOff,
    Sleep,
    Hibernate,
    DiskOff,
    LidAction,
    PowerButton,
    SleepButton,
    ProcessorMin,
    ProcessorMax,
    UsbSelectiveSuspend,
    PcieAspm,
    WakeTimers,
    WirelessPowerSaving,
    BatterySaverThreshold,
}

impl Setting {
    pub const ALL: [Setting; 14] = [
        Setting::DisplayOff,
        Setting::Sleep,
        Setting::Hibernate,
        Setting::DiskOff,
        Setting::LidAction,
        Setting::PowerButton,
        Setting::SleepButton,
        Setting::ProcessorMin,
        Setting::ProcessorMax,
        Setting::UsbSelectiveSuspend,
        Setting::PcieAspm,
        Setting::WakeTimers,
        Setting::WirelessPowerSaving,
        Setting::BatterySaverThreshold,
    ];

    /// Name as Power Options shows it.
    pub fn label(self) -> &'static str {
        match self {
            Setting::DisplayOff => "Turn off the display after",
            Setting::Sleep => "Sleep after",
            Setting::Hibernate => "Hibernate after",
            Setting::DiskOff => "Turn off hard disk after",
            Setting::LidAction => "Closing the lid",
            Setting::PowerButton => "Pressing the power button",
            Setting::SleepButton => "Pressing the sleep button",
            Setting::ProcessorMin => "Minimum processor state",
            Setting::ProcessorMax => "Maximum processor state",
            Setting::UsbSelectiveSuspend => "USB selective suspend",
            Setting::PcieAspm => "PCI Express Link State Power Management",
            Setting::WakeTimers => "Allow wake timers",
            Setting::WirelessPowerSaving => "Wireless adapter power saving",
            Setting::BatterySaverThreshold => "Battery saver turns on at",
        }
    }

    /// Stable key for the elevated-helper command line.
    pub fn key(self) -> &'static str {
        match self {
            Setting::DisplayOff => "display",
            Setting::Sleep => "sleep",
            Setting::Hibernate => "hibernate",
            Setting::DiskOff => "disk",
            Setting::LidAction => "lid",
            Setting::PowerButton => "pbutton",
            Setting::SleepButton => "sbutton",
            Setting::ProcessorMin => "procmin",
            Setting::ProcessorMax => "procmax",
            Setting::UsbSelectiveSuspend => "usbss",
            Setting::PcieAspm => "aspm",
            Setting::WakeTimers => "waketimers",
            Setting::WirelessPowerSaving => "wireless",
            Setting::BatterySaverThreshold => "batterysaver",
        }
    }

    /// Inverse of [`Setting::key`].
    pub fn from_key(k: &str) -> Option<Setting> {
        Setting::ALL.into_iter().find(|s| s.key() == k)
    }

    /// How the value is shown and edited.
    pub fn kind(self) -> Kind {
        match self {
            Setting::DisplayOff | Setting::Sleep | Setting::Hibernate | Setting::DiskOff => {
                Kind::Seconds
            }
            Setting::ProcessorMin | Setting::ProcessorMax | Setting::BatterySaverThreshold => {
                Kind::Percent
            }
            Setting::LidAction => Kind::Choice(LID_ACTIONS),
            Setting::PowerButton | Setting::SleepButton => Kind::Choice(BUTTON_ACTIONS),
            Setting::UsbSelectiveSuspend => Kind::Choice(ON_OFF),
            Setting::PcieAspm => Kind::Choice(ASPM),
            Setting::WakeTimers => Kind::Choice(WAKE_TIMERS),
            Setting::WirelessPowerSaving => Kind::Choice(WIRELESS),
        }
    }

    /// Power setting subgroup GUID.
    pub fn subgroup(self) -> u128 {
        match self {
            Setting::DisplayOff => 0x7516b95f_f776_4464_8c53_06167f40cc99,
            Setting::Sleep | Setting::Hibernate | Setting::WakeTimers => {
                0x238c9fa8_0aad_41ed_83f4_97be242c8f20
            }
            Setting::DiskOff => 0x0012ee47_9041_4b5d_9b77_535fba8b1442,
            Setting::LidAction | Setting::PowerButton | Setting::SleepButton => {
                0x4f971e89_eebd_4455_a8de_9e59040e7347
            }
            Setting::ProcessorMin | Setting::ProcessorMax => 0x54533251_82be_4824_96c1_47b60b740d00,
            Setting::UsbSelectiveSuspend => 0x2a737441_1930_4402_8d77_b2bebba308a3,
            Setting::PcieAspm => 0x501a4d13_42af_4429_9fd1_a8218c268e20,
            Setting::WirelessPowerSaving => 0x19cbb8fa_5279_450e_9fac_8a3d5fedd0c1,
            Setting::BatterySaverThreshold => 0xde830923_a562_41af_a086_e3a2c6bad2da,
        }
    }

    /// Power setting GUID.
    pub fn guid(self) -> u128 {
        match self {
            Setting::DisplayOff => 0x3c0bc021_c8a8_4e07_a973_6b14cbcb2b7e,
            Setting::Sleep => 0x29f6c1db_86da_48c5_9fdb_f2b67b1f44da,
            Setting::Hibernate => 0x9d7815a6_7ee4_497e_8888_515a05f02364,
            Setting::DiskOff => 0x6738e2c4_e8a5_4a42_b16a_e040e769756e,
            Setting::LidAction => 0x5ca83367_6e45_459f_a27b_476b1d01c936,
            Setting::PowerButton => 0x7648efa3_dd9c_4e3e_b566_50f929386280,
            Setting::SleepButton => 0x96996bc0_ad50_47ec_923b_6f41874dd9eb,
            Setting::ProcessorMin => 0x893dee8e_2bef_41e0_89c6_b55d0929964c,
            Setting::ProcessorMax => 0xbc5038f7_23e0_4960_96da_33abaf5935ec,
            Setting::UsbSelectiveSuspend => 0x48e6b7a6_50f5_4782_a5d4_53bb8f07e226,
            Setting::PcieAspm => 0xee12f906_d277_404b_b6da_e5fa1a576df5,
            Setting::WakeTimers => 0xbd3b718a_0680_4d9d_8ab2_e1d2b4ac806d,
            Setting::WirelessPowerSaving => 0x12bbebe6_58d6_4636_95bb_3217ef867c1a,
            Setting::BatterySaverThreshold => 0xe69653ca_cf7f_4f05_aa73_cb833fa90ad4,
        }
    }

    /// Display text for a value of this setting.
    pub fn format(self, v: u32) -> String {
        match self.kind() {
            Kind::Seconds => format_timeout(v),
            Kind::Percent => format!("{v} %"),
            Kind::Choice(c) => c
                .iter()
                .find(|(x, _)| *x == v)
                .map(|(_, l)| l.to_string())
                .unwrap_or_else(|| format!("Value {v}")),
        }
    }
}

/// "Never", "45 s", "10 min", "1 h 30 min".
pub fn format_timeout(secs: u32) -> String {
    match secs {
        0 => "Never".into(),
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => {
            if s % 60 == 0 {
                format!("{} min", s / 60)
            } else {
                format!("{} min {} s", s / 60, s % 60)
            }
        }
        s => {
            let (h, m) = (s / 3600, s / 60 % 60);
            if m == 0 {
                format!("{h} h")
            } else {
                format!("{h} h {m} min")
            }
        }
    }
}

/// Timeout choices offered in the UI (seconds; 0 = Never).
pub const TIMEOUT_PRESETS: &[u32] = &[
    60, 120, 180, 300, 600, 900, 1200, 1500, 1800, 2700, 3600, 7200, 10800, 18000, 0,
];
/// Percentage choices offered for processor state and battery saver.
pub const PERCENT_PRESETS: &[u32] = &[0, 5, 10, 20, 25, 30, 50, 70, 75, 80, 90, 99, 100];

/// Plugged-in (AC) and on-battery (DC) values of one setting; `None` if not readable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AcDc {
    pub ac: Option<u32>,
    pub dc: Option<u32>,
}

/// Which power source a setting change applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Ac,
    Dc,
    Both,
}

impl Source {
    fn key(self) -> &'static str {
        match self {
            Source::Ac => "ac",
            Source::Dc => "dc",
            Source::Both => "both",
        }
    }
    fn from_key(k: &str) -> Option<Source> {
        [Source::Ac, Source::Dc, Source::Both]
            .into_iter()
            .find(|s| s.key() == k)
    }
}

/// System power capabilities (`GetPwrCapabilities`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub lid: bool,
    pub sleep_button: bool,
    pub battery: bool,
    /// S3 sleep.
    pub s3: bool,
    /// Hibernation supported (S4).
    pub hibernate: bool,
    /// The hibernation file exists (hibernation turned on).
    pub hiberfile: bool,
    /// Modern Standby (S0 low power idle).
    pub modern_standby: bool,
}

/// What woke the PC, from the Power-Troubleshooter event log entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeSource {
    /// Not recorded (common after Fast Startup boots and on some firmware).
    Unknown,
    PowerButton,
    /// A device such as a mouse, keyboard or network adapter.
    Device(String),
    /// A wake timer (scheduled task, Windows Update...).
    Timer(String),
    Other(String),
}

impl WakeSource {
    /// Plain-language description, e.g. "Wake timer: ...".
    pub fn describe(&self) -> String {
        match self {
            WakeSource::Unknown => "Unknown".into(),
            WakeSource::PowerButton => "Power button".into(),
            WakeSource::Device(d) => format!("Device: {d}"),
            WakeSource::Timer(t) => format!("Wake timer: {t}"),
            WakeSource::Other(o) => o.clone(),
        }
    }

    /// Woke without a person pressing a button: timers and network adapters.
    pub fn is_unattended(&self) -> bool {
        match self {
            WakeSource::Timer(_) => true,
            WakeSource::Device(d) => is_network_name(d),
            _ => false,
        }
    }
}

/// A network adapter, judging by its name.
pub fn is_network_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let contains = [
        "ethernet", "network", "wi-fi", "wifi", "wireless", "802.11", "gbe",
    ]
    .iter()
    .any(|k| n.contains(k));
    let word = n
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| w == "lan" || w == "nic" || w == "wlan");
    contains || word
}

/// One resume from sleep, hibernation or a Fast Startup shutdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeEvent {
    /// UTC "YYYY-MM-DD HH:MM".
    pub time: String,
    pub source: WakeSource,
    /// Resumed from a Fast Startup shutdown (a boot), not from sleep.
    pub fast_startup: bool,
}

/// An active power request (`powercfg /requests`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerRequest {
    /// DISPLAY, SYSTEM, AWAYMODE, EXECUTION...
    pub kind: String,
    /// DRIVER, PROCESS or SERVICE.
    pub origin: String,
    pub name: String,
    pub reason: String,
}

impl PowerRequest {
    /// Keeps the PC (not just the screen) from sleeping.
    pub fn blocks_sleep(&self) -> bool {
        matches!(self.kind.as_str(), "SYSTEM" | "AWAYMODE" | "EXECUTION")
    }

    /// Short app/driver name: the file name of a process path.
    pub fn short_name(&self) -> &str {
        if self.origin == "PROCESS" {
            self.name.rsplit('\\').next().unwrap_or(&self.name)
        } else {
            &self.name
        }
    }
}

/// Network or Bluetooth adapter kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterKind {
    Ethernet,
    WiFi,
    Bluetooth,
}

impl AdapterKind {
    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            AdapterKind::Ethernet => "Ethernet",
            AdapterKind::WiFi => "Wi-Fi",
            AdapterKind::Bluetooth => "Bluetooth",
        }
    }

    /// From the NDIS `*PhysicalMediaType` value.
    pub fn from_media(m: u32) -> Option<AdapterKind> {
        match m {
            14 => Some(AdapterKind::Ethernet),
            9 => Some(AdapterKind::WiFi),
            10 => Some(AdapterKind::Bluetooth),
            _ => None,
        }
    }
}

/// An Energy-Efficient Ethernet style advanced property on a network adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EeeSetting {
    /// Registry value name (e.g. "*EEE").
    pub value: String,
    pub enabled: bool,
}

/// Power-saving state of a network or Bluetooth adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterPower {
    pub name: String,
    pub kind: AdapterKind,
    /// PnP device instance ID.
    pub instance_id: String,
    /// Network class subkey ("0003"), for network adapters.
    pub class_key: Option<String>,
    /// Device Manager's "Allow the computer to turn off this device to save power".
    pub allow_turn_off: Option<bool>,
    pub eee: Option<EeeSetting>,
}

/// Why a connected USB device is sensitive to USB selective suspend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSensitive {
    Audio,
    Capture,
    Vr,
    Controller,
}

impl UsbSensitive {
    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            UsbSensitive::Audio => "USB audio",
            UsbSensitive::Capture => "capture card",
            UsbSensitive::Vr => "VR headset",
            UsbSensitive::Controller => "game controller",
        }
    }
}

/// USB vendor IDs of VR headset makers (Oculus/Meta, Valve, HTC, Pimax, HP Reverb's Microsoft
/// Mixed Reality hub, Varjo).
const VR_VIDS: &[&str] = &[
    "vid_2833",
    "vid_28de",
    "vid_0bb4",
    "vid_0483&pid_0101",
    "vid_045e&pid_0659",
    "vid_1fb5",
];

/// Classify a present USB device by its setup class, name and hardware/instance ID.
pub fn classify_usb(class: &str, name: &str, id: &str) -> Option<UsbSensitive> {
    let (class, name, id) = (
        class.to_ascii_lowercase(),
        name.to_ascii_lowercase(),
        id.to_ascii_lowercase(),
    );
    if VR_VIDS.iter().any(|v| id.contains(v))
        || ["vr headset", "rift", "quest", "index hmd", "vive", "reverb"]
            .iter()
            .any(|k| name.contains(k))
    {
        return Some(UsbSensitive::Vr);
    }
    let has = |ks: &[&str]| ks.iter().any(|k| name.contains(k));
    if has(&[
        "capture",
        "cam link",
        "elgato",
        "avermedia",
        "hd60",
        "live gamer",
        "magewell",
    ]) {
        return Some(UsbSensitive::Capture);
    }
    if class == "xnacomposite"
        || class == "xboxcomposite"
        || has(&[
            "game controller",
            "gamepad",
            "xbox",
            "dualsense",
            "dualshock",
            "wireless controller",
            "joystick",
        ])
    {
        return Some(UsbSensitive::Controller);
    }
    if class == "media" || class.contains("audio") || has(&["audio", "headset", "dac", "scarlett"])
    {
        return Some(UsbSensitive::Audio);
    }
    None
}

/// Everything the Power page and power checks use.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PowerInfo {
    pub schemes: Vec<Scheme>,
    /// Active scheme GUID.
    pub active: Option<String>,
    /// Values of the active scheme.
    pub values: Vec<(Setting, AcDc)>,
    pub caps: Capabilities,
    /// Fast Startup (`HiberbootEnabled`); only effective while hibernation is on.
    pub fast_startup: Option<bool>,
    /// Devices allowed to wake the PC (`powercfg /devicequery wake_armed`).
    pub wake_armed: Vec<String>,
    /// Recent resumes (last 14 days), newest first.
    pub wakes: Vec<WakeEvent>,
    /// Active power requests; `None` until read as administrator.
    pub requests: Option<Vec<PowerRequest>>,
    /// Active wake timers (`powercfg /waketimers`); `None` until read as administrator.
    pub wake_timers: Option<Vec<String>>,
    pub adapters: Vec<AdapterPower>,
    /// Connected USB devices that dislike selective suspend: (name, kind).
    pub sensitive_usb: Vec<(String, UsbSensitive)>,
}

impl PowerInfo {
    /// Everything, including devices, wake history and (when elevated) power requests.
    pub fn gather() -> PowerInfo {
        let mut p = PowerInfo::gather_settings();
        crate::win::power::gather_devices(&mut p);
        p
    }

    /// Only power plans, their settings, capabilities and Fast Startup (cheap).
    pub fn gather_settings() -> PowerInfo {
        let mut p = PowerInfo::default();
        crate::win::power::gather_settings(&mut p);
        p
    }

    /// Replace the plan-related fields with fresher ones from `s` (a `gather_settings` result).
    pub fn merge_settings(&mut self, s: PowerInfo) {
        self.schemes = s.schemes;
        self.active = s.active;
        self.values = s.values;
        self.caps = s.caps;
        self.fast_startup = s.fast_startup;
    }

    /// Values of `s` in the active plan (both `None` if unknown).
    pub fn get(&self, s: Setting) -> AcDc {
        self.values
            .iter()
            .find(|(k, _)| *k == s)
            .map(|(_, v)| *v)
            .unwrap_or_default()
    }

    /// The active power plan, if known.
    pub fn active_scheme(&self) -> Option<&Scheme> {
        let a = self.active.as_deref()?;
        self.schemes.iter().find(|s| s.guid == a)
    }

    /// Laptop or tablet (has a battery).
    pub fn is_portable(&self) -> bool {
        self.caps.battery
    }

    /// Fast Startup is turned on and can actually be used (hibernation is enabled).
    pub fn fast_startup_effective(&self) -> Option<bool> {
        self.fast_startup
            .map(|f| f && self.caps.hibernate && self.caps.hiberfile)
    }

    /// The most recent wake from sleep or hibernation (not Fast Startup boots).
    pub fn last_wake(&self) -> Option<&WakeEvent> {
        self.wakes.iter().find(|w| !w.fast_startup)
    }
}

/// A change the user (or a Health Check "Fix") can apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Switch the active power plan.
    SetScheme(String),
    /// Change a setting of the active plan.
    SetValue {
        setting: Setting,
        source: Source,
        value: u32,
    },
    FastStartup(bool),
    /// "Allow the computer to turn off this device to save power" (by device instance ID).
    AllowTurnOff {
        instance_id: String,
        allow: bool,
    },
    /// Set an advanced network adapter property (e.g. "*EEE") in its class key.
    NicProperty {
        class_key: String,
        value: String,
        data: String,
    },
    /// Stop a device from waking the PC (by the name `wake_armed` lists).
    DisarmWake(String),
}

impl Action {
    /// Short description, e.g. for a button or status line.
    pub fn label(&self) -> String {
        match self {
            Action::SetScheme(_) => "Switch power plan".into(),
            Action::SetValue {
                setting,
                source,
                value,
            } => {
                let src = match source {
                    Source::Ac => " (plugged in)",
                    Source::Dc => " (on battery)",
                    Source::Both => "",
                };
                format!(
                    "Set \"{}\" to {}{src}",
                    setting.label(),
                    setting.format(*value)
                )
            }
            Action::FastStartup(on) => {
                format!("Turn Fast Startup {}", if *on { "on" } else { "off" })
            }
            Action::AllowTurnOff { allow, .. } => {
                if *allow {
                    "Allow Windows to turn off this device".into()
                } else {
                    "Stop Windows turning off this device to save power".into()
                }
            }
            Action::NicProperty { value, data, .. } => {
                if is_eee_value(value) && data == "0" {
                    "Turn off Energy-Efficient Ethernet".into()
                } else {
                    format!("Set {value} to {data}")
                }
            }
            Action::DisarmWake(d) => format!("Stop \"{d}\" from waking the PC"),
        }
    }

    /// Changes machine-wide settings that need administrator rights.
    pub fn needs_admin(&self) -> bool {
        !matches!(self, Action::SetScheme(_) | Action::SetValue { .. })
    }

    /// Ask before applying: changes that briefly disconnect a device or alter shutdown.
    pub fn confirm_text(&self) -> Option<&'static str> {
        match self {
            Action::FastStartup(_) => Some(
                "This changes how Windows shuts down and starts. It takes effect from the next shutdown.",
            ),
            Action::AllowTurnOff { .. } => Some(
                "The device may need to be unplugged and reconnected, or the PC restarted, for this to take effect.",
            ),
            Action::NicProperty { .. } => Some(
                "The network adapter will restart: the connection drops for a few seconds. Unsaved downloads or calls may be interrupted.",
            ),
            _ => None,
        }
    }

    /// Encode for the elevated helper's command line (hex, so no quoting issues).
    pub fn to_arg(&self) -> String {
        let s = match self {
            Action::SetScheme(g) => format!("scheme\t{g}"),
            Action::SetValue {
                setting,
                source,
                value,
            } => format!("value\t{}\t{}\t{value}", setting.key(), source.key()),
            Action::FastStartup(on) => format!("faststartup\t{}", u8::from(*on)),
            Action::AllowTurnOff { instance_id, allow } => {
                format!("turnoff\t{instance_id}\t{}", u8::from(*allow))
            }
            Action::NicProperty {
                class_key,
                value,
                data,
            } => format!("nic\t{class_key}\t{value}\t{data}"),
            Action::DisarmWake(d) => format!("disarm\t{d}"),
        };
        s.bytes().map(|b| format!("{b:02x}")).collect()
    }

    /// Decode [`Action::to_arg`]; `None` for anything malformed or not allowed.
    pub fn from_arg(arg: &str) -> Option<Action> {
        if arg.len() % 2 != 0 {
            return None;
        }
        let bytes: Option<Vec<u8>> = (0..arg.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(arg.get(i..i + 2)?, 16).ok())
            .collect();
        let s = String::from_utf8(bytes?).ok()?;
        let f: Vec<&str> = s.split('\t').collect();
        let flag = |v: &str| match v {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        };
        Some(match f.as_slice() {
            ["scheme", g] => Action::SetScheme(g.to_string()),
            ["value", k, src, v] => Action::SetValue {
                setting: Setting::from_key(k)?,
                source: Source::from_key(src)?,
                value: v.parse().ok()?,
            },
            ["faststartup", on] => Action::FastStartup(flag(on)?),
            ["turnoff", id, allow] => Action::AllowTurnOff {
                instance_id: id.to_string(),
                allow: flag(allow)?,
            },
            ["nic", k, v, d] => {
                // Only 4-digit class subkeys and known power-saving properties.
                if k.len() != 4 || !k.chars().all(|c| c.is_ascii_digit()) || !is_eee_value(v) {
                    return None;
                }
                Action::NicProperty {
                    class_key: k.to_string(),
                    value: v.to_string(),
                    data: d.to_string(),
                }
            }
            ["disarm", d] => Action::DisarmWake(d.to_string()),
            _ => return None,
        })
    }
}

/// Advanced network adapter properties for Energy-Efficient / Green Ethernet.
pub const EEE_VALUES: &[&str] = &[
    "*EEE",
    "EEE",
    "EEELinkAdvertisement",
    "AdvancedEEE",
    "EnableGreenEthernet",
    "GreenEthernet",
    "*GreenEthernet",
];

/// One of the [`EEE_VALUES`] properties.
pub fn is_eee_value(v: &str) -> bool {
    EEE_VALUES.contains(&v)
}

/// Why applying an [`Action`] failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// Needs administrator rights.
    AccessDenied,
    Failed(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::AccessDenied => f.write_str("administrator rights are required"),
            ApplyError::Failed(e) => f.write_str(e),
        }
    }
}

/// Apply a change. `SetValue` writes the active plan and re-applies it.
pub fn apply(action: &Action) -> Result<(), ApplyError> {
    crate::win::power::apply(action)
}

/// Raw `powercfg /requests` and `/waketimers` output; `None` unless running as administrator.
/// Parse with [`parse_requests`] and [`parse_wake_timers`].
pub fn query_admin_raw() -> Option<(String, String)> {
    crate::win::power::query_admin_raw()
}

// ---- Parsers --------------------------------------------------------------------------------

/// Parse `powercfg /requests` output. Section headers ("SYSTEM:") and the "[PROCESS]" style
/// tags are not localized; everything else is kept as text.
pub fn parse_requests(text: &str) -> Vec<PowerRequest> {
    let mut out: Vec<PowerRequest> = Vec::new();
    let mut kind = String::new();
    let mut open = false;
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            open = false;
            continue;
        }
        if let Some(h) = line.strip_suffix(':')
            && !h.is_empty()
            && h.chars().all(|c| c.is_ascii_uppercase())
        {
            kind = h.to_string();
            open = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix('[')
            && let Some((tag, name)) = rest.split_once(']')
        {
            out.push(PowerRequest {
                kind: kind.clone(),
                origin: tag.trim().to_string(),
                name: name.trim().to_string(),
                reason: String::new(),
            });
            open = true;
            continue;
        }
        if open && let Some(r) = out.last_mut() {
            if !r.reason.is_empty() {
                r.reason.push(' ');
            }
            r.reason.push_str(line);
        }
    }
    out
}

/// Parse `powercfg /waketimers`: one entry per timer ("Timer set by [SERVICE] ..." plus its
/// indented reason). Output without a "[...]" tag means no timers.
pub fn parse_wake_timers(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let indented = line.starts_with([' ', '\t']);
        if !indented && t.contains('[') {
            out.push(t.to_string());
        } else if indented && let Some(last) = out.last_mut() {
            last.push_str(" — ");
            last.push_str(t);
        }
    }
    out
}

pub use crate::checkup::event_data;

/// A Power-Troubleshooter event 1 ("The system has returned from a low power state").
pub fn parse_wake_event(xml: &str) -> Option<WakeEvent> {
    let wake = event_data(xml, "WakeTime")?;
    // "2026-10-13T13:01:39.6488795Z" -> "2026-10-13 13:01"
    let time = wake.get(..16).unwrap_or(&wake).replacen('T', " ", 1);
    let text = event_data(xml, "WakeSourceText").unwrap_or_default();
    let text = text.trim();
    let timer_owner = event_data(xml, "WakeTimerOwner").unwrap_or_default();
    let timer_ctx = event_data(xml, "WakeTimerContext").unwrap_or_default();
    // SYSTEM_POWER_STATE: 6 = shutdown (a Fast Startup boot).
    let fast_startup = event_data(xml, "TargetState").as_deref() == Some("6");
    let source = if !timer_owner.trim().is_empty() || text.starts_with("Timer") {
        let what = [text, timer_ctx.trim(), timer_owner.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("");
        WakeSource::Timer(what.strip_prefix("Timer - ").unwrap_or(what).to_string())
    } else if let Some(d) = text.strip_prefix("Device") {
        WakeSource::Device(d.trim_start_matches([' ', '-']).trim().to_string())
    } else if text.eq_ignore_ascii_case("Power Button") {
        WakeSource::PowerButton
    } else if text.is_empty() || text.eq_ignore_ascii_case("Unknown") {
        WakeSource::Unknown
    } else {
        WakeSource::Other(text.to_string())
    };
    Some(WakeEvent {
        time,
        source,
        fast_startup,
    })
}

/// Report section for the exported system report.
pub fn report_section(p: &PowerInfo) -> crate::report::Section {
    let mut rows = Vec::new();
    let mut row = |k: &str, v: String| {
        if !v.is_empty() {
            rows.push((k.to_string(), v));
        }
    };
    if let Some(s) = p.active_scheme() {
        row("Power plan", s.name.clone());
    }
    for s in [
        Setting::DisplayOff,
        Setting::Sleep,
        Setting::Hibernate,
        Setting::DiskOff,
        Setting::ProcessorMin,
        Setting::ProcessorMax,
        Setting::UsbSelectiveSuspend,
        Setting::PcieAspm,
        Setting::WakeTimers,
    ] {
        let v = p.get(s);
        let fmt = |x: Option<u32>| x.map(|x| s.format(x));
        let text = match (fmt(v.ac), fmt(v.dc), p.is_portable()) {
            (Some(a), Some(d), true) => format!("{a} plugged in, {d} on battery"),
            (Some(a), _, _) => a,
            _ => continue,
        };
        row(s.label(), text);
    }
    if let Some(f) = p.fast_startup_effective() {
        row("Fast Startup", if f { "On" } else { "Off" }.into());
    }
    if p.caps.modern_standby {
        row("Standby", "Modern Standby (S0)".into());
    }
    if let Some(w) = p.last_wake() {
        row(
            "Last wake",
            format!("{} ({} UTC)", w.source.describe(), w.time),
        );
    }
    if !p.wake_armed.is_empty() {
        row("Devices that can wake the PC", p.wake_armed.join(", "));
    }
    for a in &p.adapters {
        let mut parts = Vec::new();
        if let Some(t) = a.allow_turn_off {
            parts.push(format!(
                "power saving {}",
                if t { "allowed" } else { "off" }
            ));
        }
        if let Some(e) = &a.eee {
            parts.push(format!(
                "Energy-Efficient Ethernet {}",
                if e.enabled { "on" } else { "off" }
            ));
        }
        if !parts.is_empty() {
            row(&a.name, parts.join(", "));
        }
    }
    crate::report::Section {
        title: "Power".into(),
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeouts() {
        assert_eq!(format_timeout(0), "Never");
        assert_eq!(format_timeout(45), "45 s");
        assert_eq!(format_timeout(600), "10 min");
        assert_eq!(format_timeout(90), "1 min 30 s");
        assert_eq!(format_timeout(3600), "1 h");
        assert_eq!(format_timeout(5400), "1 h 30 min");
    }

    #[test]
    fn setting_format_and_keys() {
        assert_eq!(Setting::PcieAspm.format(2), "Maximum power savings");
        assert_eq!(Setting::ProcessorMax.format(99), "99 %");
        assert_eq!(Setting::LidAction.format(9), "Value 9");
        for s in Setting::ALL {
            assert_eq!(Setting::from_key(s.key()), Some(s));
        }
    }

    #[test]
    fn action_round_trip() {
        let actions = [
            Action::SetScheme(BALANCED.into()),
            Action::SetValue {
                setting: Setting::UsbSelectiveSuspend,
                source: Source::Both,
                value: 0,
            },
            Action::FastStartup(false),
            Action::AllowTurnOff {
                instance_id: r"PCI\VEN_8086&DEV_15F3\04 7C".into(),
                allow: false,
            },
            Action::NicProperty {
                class_key: "0003".into(),
                value: "*EEE".into(),
                data: "0".into(),
            },
            Action::DisarmWake("HID Keyboard Device (001)".into()),
        ];
        for a in actions {
            let arg = a.to_arg();
            assert!(arg.chars().all(|c| c.is_ascii_hexdigit()));
            assert_eq!(Action::from_arg(&arg), Some(a));
        }
        assert_eq!(Action::from_arg("zz"), None);
        assert_eq!(Action::from_arg("abc"), None);
        // Arbitrary registry writes are refused.
        let bad = Action::NicProperty {
            class_key: r"..\..\Foo".into(),
            value: "*EEE".into(),
            data: "0".into(),
        };
        assert_eq!(Action::from_arg(&bad.to_arg()), None);
        let bad = Action::NicProperty {
            class_key: "0001".into(),
            value: "NetworkAddress".into(),
            data: "0".into(),
        };
        assert_eq!(Action::from_arg(&bad.to_arg()), None);
    }

    #[test]
    fn admin_and_confirm() {
        assert!(!Action::SetScheme(BALANCED.into()).needs_admin());
        assert!(Action::FastStartup(true).needs_admin());
        assert!(Action::FastStartup(true).confirm_text().is_some());
        assert!(Action::DisarmWake("x".into()).confirm_text().is_none());
    }

    #[test]
    fn requests() {
        let text = "DISPLAY:\r\nNone.\r\n\r\nSYSTEM:\r\n[DRIVER] Realtek High Definition Audio (HDAUDIO\\FUNC_01&VEN_10EC)\r\nAn audio stream is currently in use.\r\n\r\nAWAYMODE:\r\nNone.\r\n\r\nEXECUTION:\r\n[PROCESS] \\Device\\HarddiskVolume3\\Program Files\\App\\app.exe\r\nPlaying audio\r\n[SERVICE] \\Device\\HarddiskVolume3\\Windows\\System32\\svchost.exe (wuauserv)\r\n\r\nPERFBOOST:\r\nNone.\r\n";
        let r = parse_requests(text);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].kind, "SYSTEM");
        assert_eq!(r[0].origin, "DRIVER");
        assert_eq!(r[0].reason, "An audio stream is currently in use.");
        assert_eq!(r[1].kind, "EXECUTION");
        assert_eq!(r[1].short_name(), "app.exe");
        assert_eq!(r[1].reason, "Playing audio");
        assert_eq!(r[2].reason, "");
        assert!(r.iter().all(|r| r.blocks_sleep()));
        assert!(parse_requests("DISPLAY:\nNone.\n\nSYSTEM:\nNone.\n").is_empty());
    }

    #[test]
    fn wake_timers() {
        let text = "Timer set by [SERVICE] \\Device\\HarddiskVolume3\\Windows\\System32\\svchost.exe (SystemEventsBroker) expires at 3:01:09 AM on 10/15/2026.\r\n  Reason: Windows will execute 'NT TASK\\Microsoft\\Windows\\UpdateOrchestrator\\Reboot' scheduled task that requested waking the computer.\r\n\r\n";
        let t = parse_wake_timers(text);
        assert_eq!(t.len(), 1);
        assert!(t[0].starts_with("Timer set by [SERVICE]"));
        assert!(t[0].contains("UpdateOrchestrator"));
        assert!(parse_wake_timers("There are no active wake timers in the system.\r\n").is_empty());
    }

    const FAST_BOOT: &str = "<Event><EventData><Data Name='SleepTime'>2026-10-13T02:09:51.9354381Z</Data><Data Name='WakeTime'>2026-10-13T13:01:39.6488795Z</Data><Data Name='TargetState'>6</Data><Data Name='EffectiveState'>5</Data><Data Name='WakeSourceType'>0</Data><Data Name='WakeSourceText'></Data><Data Name='WakeTimerOwner'></Data><Data Name='WakeTimerContext'></Data></EventData></Event>";

    #[test]
    fn wake_events() {
        let e = parse_wake_event(FAST_BOOT).unwrap();
        assert_eq!(e.time, "2026-10-13 13:01");
        assert_eq!(e.source, WakeSource::Unknown);
        assert!(e.fast_startup);

        let dev = FAST_BOOT
            .replace("<Data Name='TargetState'>6", "<Data Name='TargetState'>4")
            .replace(
                "<Data Name='WakeSourceText'></Data>",
                "<Data Name='WakeSourceText'>Device -Intel(R) Ethernet Controller (3) I225-V</Data>",
            );
        let e = parse_wake_event(&dev).unwrap();
        assert!(!e.fast_startup);
        assert_eq!(
            e.source,
            WakeSource::Device("Intel(R) Ethernet Controller (3) I225-V".into())
        );
        assert!(e.source.is_unattended());

        let timer = FAST_BOOT.replace(
            "<Data Name='WakeTimerOwner'></Data>",
            "<Data Name='WakeTimerOwner'>\\Device\\HarddiskVolume3\\Windows\\System32\\svchost.exe (SystemEventsBroker)</Data>",
        ).replace(
            "<Data Name='WakeTimerContext'></Data>",
            "<Data Name='WakeTimerContext'>Windows will execute &apos;NT TASK\\Microsoft\\Windows\\UpdateOrchestrator\\Reboot&apos;</Data>",
        );
        let e = parse_wake_event(&timer).unwrap();
        match &e.source {
            WakeSource::Timer(t) => assert!(t.starts_with("Windows will execute 'NT TASK")),
            s => panic!("{s:?}"),
        }
        assert!(e.source.is_unattended());

        let button = FAST_BOOT.replace(
            "<Data Name='WakeSourceText'></Data>",
            "<Data Name='WakeSourceText'>Power Button</Data>",
        );
        assert_eq!(
            parse_wake_event(&button).unwrap().source,
            WakeSource::PowerButton
        );
        assert_eq!(parse_wake_event("<Event/>"), None);
    }

    #[test]
    fn network_names() {
        assert!(is_network_name("Intel(R) Ethernet Controller (3) I225-V"));
        assert!(is_network_name("Realtek PCIe GbE Family Controller"));
        assert!(is_network_name("Intel(R) Wi-Fi 6E AX210 160MHz"));
        assert!(!is_network_name("HID Keyboard Device"));
        assert!(!is_network_name("HID-compliant mouse"));
        assert!(!WakeSource::Device("HID-compliant mouse".into()).is_unattended());
    }

    #[test]
    fn usb_classification() {
        assert_eq!(
            classify_usb(
                "Focusrite Audio",
                "Scarlett Solo USB",
                r"USB\VID_1235&PID_8211\Y7X9"
            ),
            Some(UsbSensitive::Audio)
        );
        assert_eq!(
            classify_usb("MEDIA", "USB Audio Device", r"USB\VID_0D8C&PID_0014&MI_00"),
            Some(UsbSensitive::Audio)
        );
        assert_eq!(
            classify_usb("Camera", "Cam Link 4K", r"USB\VID_0FD9&PID_0066"),
            Some(UsbSensitive::Capture)
        );
        assert_eq!(
            classify_usb("USB", "USB Composite Device", r"USB\VID_2833&PID_0186"),
            Some(UsbSensitive::Vr)
        );
        assert_eq!(
            classify_usb(
                "XnaComposite",
                "Xbox 360 Controller for Windows",
                r"USB\VID_045E&PID_028E"
            ),
            Some(UsbSensitive::Controller)
        );
        assert_eq!(
            classify_usb(
                "HIDClass",
                "USB Input Device",
                r"USB\VID_3434&PID_D030&MI_00"
            ),
            None
        );
        assert_eq!(
            classify_usb("USB", "Generic USB Hub", r"USB\VID_05E3&PID_0610"),
            None
        );
    }

    #[test]
    fn adapter_media() {
        assert_eq!(AdapterKind::from_media(14), Some(AdapterKind::Ethernet));
        assert_eq!(AdapterKind::from_media(9), Some(AdapterKind::WiFi));
        assert_eq!(AdapterKind::from_media(0), None);
    }

    #[test]
    fn fast_startup_needs_hibernation() {
        let mut p = PowerInfo {
            fast_startup: Some(true),
            ..Default::default()
        };
        assert_eq!(p.fast_startup_effective(), Some(false));
        p.caps.hibernate = true;
        p.caps.hiberfile = true;
        assert_eq!(p.fast_startup_effective(), Some(true));
    }
}
