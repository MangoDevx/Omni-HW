//! Power checks: settings that disrupt audio, networking, downloads or sleep. They read the
//! [`PowerInfo`](crate::power::PowerInfo) gathered on the sampler thread; most offer a one-click fix.

use super::helpers::{finding, plural};
use super::{Category, Check, Context, Finding, Level, Outcome};
use crate::power::{Action, AdapterKind, Setting, Source, WakeSource, format_timeout};

pub(super) const CHECKS: &[(&str, Category, Check)] = &[
    (
        "USB selective suspend",
        Category::Power,
        usb_selective_suspend,
    ),
    (
        "Network adapter power saving",
        Category::Power,
        network_power_saving,
    ),
    (
        "Bluetooth power saving",
        Category::Power,
        bluetooth_power_saving,
    ),
    ("PCI Express power saving", Category::Power, pcie_aspm),
    ("Sleep timer", Category::Power, sleep_timer),
    ("Waking from sleep", Category::Power, unexpected_wakes),
    ("Fast Startup", Category::Power, fast_startup),
    ("Hard disk turn-off timer", Category::Power, disk_timer),
    ("Processor state limits", Category::Power, processor_state),
    ("Apps keeping the PC awake", Category::Power, sleep_blockers),
];

/// The outcome of every power check until the power settings have been read.
fn not_read() -> Outcome {
    Outcome::Skipped("Power settings are still being read".into())
}

const ADVANCED: &str =
    "Control Panel > Power Options > Change plan settings > Change advanced power settings";

fn usb_selective_suspend(ctx: &Context) -> Outcome {
    let Some(p) = ctx.power else {
        return not_read();
    };
    let v = p.get(Setting::UsbSelectiveSuspend);
    let Some(ac) = v.ac else {
        return Outcome::Skipped("This power plan has no USB selective suspend setting".into());
    };
    if ac == 0 {
        return Outcome::Pass("Off while plugged in".into());
    }
    if p.sensitive_usb.is_empty() {
        return Outcome::Pass(
            "On, but no USB audio interfaces, capture cards, VR headsets or controllers are connected"
                .into(),
        );
    }
    let devices: Vec<String> = p
        .sensitive_usb
        .iter()
        .map(|(n, k)| format!("{n} ({})", k.name()))
        .collect();
    let source = if p.is_portable() {
        Source::Ac
    } else {
        Source::Both
    };
    Outcome::Findings(vec![
        finding(
            Level::Warning,
            Category::Power,
            "USB selective suspend can make USB devices drop out",
            format!(
                "USB selective suspend is on and these devices are connected: {}. Windows powers down \
                 USB ports it thinks are idle, which makes audio interfaces crackle or cut out and \
                 capture cards, VR headsets and controllers disconnect.",
                devices.join(", ")
            ),
            &[
                "Click Fix to turn USB selective suspend off for the current power plan (plugged in).",
                &format!(
                    "Or by hand: {ADVANCED} > USB settings > USB selective suspend setting > Disabled."
                ),
            ],
        )
        .with_fix(Action::SetValue {
            setting: Setting::UsbSelectiveSuspend,
            source,
            value: 0,
        }),
    ])
}

fn network_power_saving(ctx: &Context) -> Outcome {
    let Some(p) = ctx.power else {
        return not_read();
    };
    let nics: Vec<_> = p
        .adapters
        .iter()
        .filter(|a| a.kind != AdapterKind::Bluetooth)
        .collect();
    if nics.is_empty() {
        return Outcome::Skipped("No network adapters found".into());
    }
    let mut out = Vec::new();
    for a in &nics {
        if a.allow_turn_off == Some(true) {
            out.push(
                finding(
                    Level::Info,
                    Category::Power,
                    format!("Windows may turn off {} to save power", a.name),
                    format!(
                        "\"Allow the computer to turn off this device to save power\" is on for this {} \
                         adapter. On some PCs the adapter then drops the connection mid-game or mid-call, \
                         or doesn't come back after sleep.",
                        a.kind.name()
                    ),
                    &[
                        "Click Fix to turn this off.",
                        "Or by hand: Device Manager > Network adapters > the adapter > Properties > Power Management > untick \"Allow the computer to turn off this device to save power\".",
                    ],
                )
                .with_fix(Action::AllowTurnOff {
                    instance_id: a.instance_id.clone(),
                    allow: false,
                }),
            );
        }
        if let (Some(e), Some(key)) = (&a.eee, &a.class_key)
            && e.enabled
        {
            out.push(
                finding(
                    Level::Info,
                    Category::Power,
                    format!("Energy-Efficient Ethernet is on for {}", a.name),
                    "Energy-Efficient (Green) Ethernet lets the link nap between packets. With some \
                     switches and routers this causes lag spikes, drops or a link that renegotiates."
                        .to_string(),
                    &[
                        "Click Fix to turn it off (the adapter restarts, so the connection drops for a few seconds).",
                        "Or by hand: Device Manager > Network adapters > the adapter > Properties > Advanced > Energy-Efficient Ethernet > Disabled.",
                    ],
                )
                .with_fix(Action::NicProperty {
                    class_key: key.clone(),
                    value: e.value.clone(),
                    data: "0".into(),
                }),
            );
        }
    }
    let has_wifi = nics.iter().any(|a| a.kind == AdapterKind::WiFi);
    if has_wifi && let Some(v) = p.get(Setting::WirelessPowerSaving).ac.filter(|&v| v >= 2) {
        out.push(
            finding(
                Level::Info,
                Category::Power,
                "Wi-Fi power saving is high while plugged in",
                format!(
                    "The power plan sets wireless adapter power saving to \"{}\" when plugged in, which \
                     adds latency and can make Wi-Fi drop out.",
                    Setting::WirelessPowerSaving.format(v)
                ),
                &[
                    "Click Fix to set it to Maximum performance when plugged in.",
                    &format!(
                        "Or by hand: {ADVANCED} > Wireless Adapter Settings > Power Saving Mode."
                    ),
                ],
            )
            .with_fix(Action::SetValue {
                setting: Setting::WirelessPowerSaving,
                source: Source::Ac,
                value: 0,
            }),
        );
    }
    if out.is_empty() {
        return Outcome::Pass(format!(
            "Power saving off on {}",
            plural(nics.len(), "adapter", "adapters")
        ));
    }
    Outcome::Findings(out)
}

fn bluetooth_power_saving(ctx: &Context) -> Outcome {
    let Some(p) = ctx.power else {
        return not_read();
    };
    let bt: Vec<_> = p
        .adapters
        .iter()
        .filter(|a| a.kind == AdapterKind::Bluetooth)
        .collect();
    if bt.is_empty() {
        return Outcome::Skipped("No Bluetooth adapter found".into());
    }
    let out: Vec<Finding> = bt
        .iter()
        .filter(|a| a.allow_turn_off == Some(true))
        .map(|a| {
            finding(
                Level::Info,
                Category::Power,
                format!("Windows may turn off {} to save power", a.name),
                "\"Allow the computer to turn off this device to save power\" is on for the Bluetooth \
                 adapter. This is a common cause of controllers, headsets and mice disconnecting or \
                 stuttering."
                    .to_string(),
                &[
                    "Click Fix to turn this off.",
                    "Or by hand: Device Manager > Bluetooth > the adapter > Properties > Power Management > untick \"Allow the computer to turn off this device to save power\".",
                ],
            )
            .with_fix(Action::AllowTurnOff {
                instance_id: a.instance_id.clone(),
                allow: false,
            })
        })
        .collect();
    if out.is_empty() {
        if bt.iter().all(|a| a.allow_turn_off.is_none()) {
            return Outcome::Skipped("The Bluetooth adapter has no power-saving option".into());
        }
        return Outcome::Pass("Power saving off".into());
    }
    Outcome::Findings(out)
}

fn pcie_aspm(ctx: &Context) -> Outcome {
    let Some(p) = ctx.power else {
        return not_read();
    };
    if p.is_portable() {
        return Outcome::Skipped(
            "Only checked on desktops (laptops use it to save battery)".into(),
        );
    }
    let Some(v) = p.get(Setting::PcieAspm).ac else {
        return Outcome::Skipped("This power plan has no PCI Express setting".into());
    };
    if v < 2 {
        return Outcome::Pass(Setting::PcieAspm.format(v));
    }
    Outcome::Findings(vec![
        finding(
            Level::Warning,
            Category::Power,
            "PCI Express power saving is at maximum on a desktop",
            "Link State Power Management is set to Maximum power savings. On desktops this saves \
             very little but can cause GPU or NVMe stutter, black screens or devices not waking up.",
            &[
                "Click Fix to turn it off for the current power plan.",
                &format!(
                    "Or by hand: {ADVANCED} > PCI Express > Link State Power Management > Off."
                ),
            ],
        )
        .with_fix(Action::SetValue {
            setting: Setting::PcieAspm,
            source: Source::Ac,
            value: 0,
        }),
    ])
}

/// Sleep timers under this count as "very short" (seconds).
const SHORT_SLEEP: u32 = 600;

fn sleep_timer(ctx: &Context) -> Outcome {
    let Some(p) = ctx.power else {
        return not_read();
    };
    let Some(sleep) = p.get(Setting::Sleep).ac else {
        return Outcome::Skipped("Sleep timer not available".into());
    };
    if sleep == 0 {
        return Outcome::Pass("Never sleeps while plugged in".into());
    }
    let display = p.get(Setting::DisplayOff).ac.unwrap_or(0);
    let mut out = Vec::new();
    if sleep < SHORT_SLEEP {
        out.push(
            finding(
                Level::Warning,
                Category::Power,
                format!("PC goes to sleep after only {}", format_timeout(sleep)),
                "When plugged in, the PC sleeps after a short idle time. Downloads, renders, \
                 backups and game updates stop when it does.",
                &[
                    "Click Fix to set sleep to 30 minutes when plugged in.",
                    "Or by hand: Settings > System > Power > Screen, sleep & hibernate timeouts.",
                ],
            )
            .with_fix(Action::SetValue {
                setting: Setting::Sleep,
                source: Source::Ac,
                value: 1800,
            }),
        );
    } else if display != 0 && sleep < display {
        out.push(finding(
            Level::Info,
            Category::Power,
            "PC sleeps before the screen turns off",
            format!(
                "Sleep ({}) comes before the display timeout ({}), so the screen never gets to turn \
                 off first and the PC sleeps sooner than you may expect.",
                format_timeout(sleep),
                format_timeout(display)
            ),
            &["Set the display timeout shorter than the sleep timeout on the Power page or in Settings > System > Power."],
        ));
    }
    if out.is_empty() {
        return Outcome::Pass(format!(
            "Sleeps after {} when plugged in",
            format_timeout(sleep)
        ));
    }
    Outcome::Findings(out)
}

fn unexpected_wakes(ctx: &Context) -> Outcome {
    let Some(p) = ctx.power else {
        return not_read();
    };
    let wakes: Vec<_> = p.wakes.iter().filter(|w| !w.fast_startup).collect();
    let unattended: Vec<_> = wakes.iter().filter(|w| w.source.is_unattended()).collect();
    if unattended.is_empty() {
        return Outcome::Pass(match wakes.first() {
            Some(w) => format!("Last wake: {} ({} UTC)", w.source.describe(), w.time),
            None => "No wakes from sleep in the last 2 weeks".into(),
        });
    }
    let timers = unattended
        .iter()
        .any(|w| matches!(w.source, WakeSource::Timer(_)));
    let timers_allowed = p.get(Setting::WakeTimers).ac != Some(0);
    let listed: Vec<String> = unattended
        .iter()
        .take(3)
        .map(|w| format!("{} UTC: {}", w.time, w.source.describe()))
        .collect();
    let mut detail = format!(
        "The PC woke from sleep without anyone pressing a key or button. Recent: {}.",
        listed.join("; ")
    );
    if !p.wake_armed.is_empty() {
        detail += &format!(
            " Devices allowed to wake the PC: {}.",
            p.wake_armed.join(", ")
        );
    }
    let fix = if timers && timers_allowed {
        Some(Action::SetValue {
            setting: Setting::WakeTimers,
            source: Source::Both,
            value: 0,
        })
    } else {
        unattended.iter().find_map(|w| match &w.source {
            WakeSource::Device(d) => p
                .wake_armed
                .iter()
                .find(|a| a.starts_with(d.as_str()))
                .map(|a| Action::DisarmWake(a.clone())),
            _ => None,
        })
    };
    let f = finding(
        if unattended.len() >= 3 {
            Level::Warning
        } else {
            Level::Info
        },
        Category::Power,
        format!(
            "PC woke up by itself {} in the last 2 weeks",
            plural(unattended.len(), "time", "times")
        ),
        detail,
        &[
            "Wake timers (Windows Update restarts, maintenance tasks) wake the PC on a schedule. Turn them off on the Power page or in Power Options > Sleep > Allow wake timers.",
            "Network adapters can wake the PC on network traffic: on the Power page, under \"Devices that can wake the PC\", click Disarm, or untick \"Allow this device to wake the computer\" in Device Manager.",
            "The Power page shows the last wake source and the wake history.",
        ],
    );
    Outcome::Findings(vec![match fix {
        Some(a) => f.with_fix(a),
        None => f,
    }])
}

fn fast_startup(ctx: &Context) -> Outcome {
    let Some(p) = ctx.power else {
        return not_read();
    };
    match p.fast_startup_effective() {
        None => Outcome::Skipped("Fast Startup setting not readable".into()),
        Some(false) => Outcome::Pass(if p.fast_startup == Some(true) {
            "On, but hibernation is off so it isn't used".into()
        } else {
            "Off".into()
        }),
        Some(true) => {
            let days = ctx.uptime_secs.unwrap_or(0) / 86400;
            if days < 7 {
                return Outcome::Pass("On".into());
            }
            Outcome::Findings(vec![
                finding(
                    Level::Info,
                    Category::Power,
                    "Fast Startup keeps Windows from starting fresh",
                    format!(
                        "Fast Startup is on, so \"Shut down\" saves the running Windows core and drivers \
                         and resumes them at the next start. Windows has been running for {days} days \
                         across shutdowns, so driver glitches and slowdowns carry over. Fast Startup also \
                         leaves the Windows drive locked, which causes trouble when dual-booting Linux or \
                         another Windows."
                    ),
                    &[
                        "Use Restart (not Shut down) when you want a clean start: it always starts fresh.",
                        "Click Fix to turn Fast Startup off (needs administrator). Starts take a few seconds longer.",
                        "Or by hand: Control Panel > Power Options > Choose what the power buttons do > Change settings that are currently unavailable > untick \"Turn on fast startup\".",
                    ],
                )
                .with_fix(Action::FastStartup(false)),
            ])
        }
    }
}

/// Hard disk timers under this count as short (seconds).
const SHORT_DISK: u32 = 600;

fn disk_timer(ctx: &Context) -> Outcome {
    let Some(p) = ctx.power else {
        return not_read();
    };
    let hdds = ctx
        .info
        .drives
        .iter()
        .filter(|d| d.ssd == Some(false) && !d.removable)
        .count();
    if hdds == 0 {
        return Outcome::Skipped("No hard disk drives (HDDs)".into());
    }
    let Some(v) = p.get(Setting::DiskOff).ac else {
        return Outcome::Skipped("Hard disk timer not available".into());
    };
    if v == 0 || v >= SHORT_DISK {
        return Outcome::Pass(format!("Turns off after: {}", format_timeout(v)));
    }
    Outcome::Findings(vec![
        finding(
            Level::Info,
            Category::Power,
            format!("Hard disks turn off after only {}", format_timeout(v)),
            format!(
                "{} spin down after a short idle time. Opening a file on them then freezes \
                 programs for several seconds while they spin up, and frequent spin-ups add wear.",
                plural(
                    hdds,
                    "hard disk drive (HDD) will",
                    "hard disk drives (HDDs) will"
                )
            ),
            &[
                "Click Fix to stop turning hard disks off while plugged in.",
                &format!("Or by hand: {ADVANCED} > Hard disk > Turn off hard disk after."),
            ],
        )
        .with_fix(Action::SetValue {
            setting: Setting::DiskOff,
            source: Source::Ac,
            value: 0,
        }),
    ])
}

fn processor_state(ctx: &Context) -> Outcome {
    let Some(p) = ctx.power else {
        return not_read();
    };
    let (min, max) = (p.get(Setting::ProcessorMin), p.get(Setting::ProcessorMax));
    let Some(max_ac) = max.ac else {
        return Outcome::Skipped("Processor power settings not available".into());
    };
    let mut out = Vec::new();
    if max_ac < 100 {
        out.push(
            finding(
                Level::Warning,
                Category::Power,
                format!("CPU is capped at {max_ac} % when plugged in"),
                format!(
                    "Maximum processor state is {max_ac} % while plugged in, which keeps the CPU from \
                     reaching its full speed (99 % also turns off boost clocks)."
                ),
                &[
                    "Click Fix to set it back to 100 %.",
                    "If you lowered it on purpose to keep a laptop cool or quiet, you can ignore this.",
                    &format!(
                        "Or by hand: {ADVANCED} > Processor power management > Maximum processor state."
                    ),
                ],
            )
            .with_fix(Action::SetValue {
                setting: Setting::ProcessorMax,
                source: Source::Ac,
                value: 100,
            }),
        );
    }
    if p.is_portable() && min.dc == Some(100) {
        out.push(
            finding(
                Level::Warning,
                Category::Power,
                "CPU never slows down on battery",
                "Minimum processor state is 100 % on battery, so the CPU runs at full speed even \
                 when idle. Battery life and heat suffer for no performance gain.",
                &[
                    "Click Fix to set it to 5 % on battery.",
                    &format!(
                        "Or by hand: {ADVANCED} > Processor power management > Minimum processor state."
                    ),
                ],
            )
            .with_fix(Action::SetValue {
                setting: Setting::ProcessorMin,
                source: Source::Dc,
                value: 5,
            }),
        );
    }
    if out.is_empty() {
        return Outcome::Pass(format!(
            "Plugged in: minimum {}, maximum {max_ac} %",
            min.ac.map_or("-".into(), |v| format!("{v} %"))
        ));
    }
    Outcome::Findings(out)
}

fn sleep_blockers(ctx: &Context) -> Outcome {
    let Some(p) = ctx.power else {
        return not_read();
    };
    let Some(r) = &p.requests else {
        return Outcome::Skipped(
            "Needs administrator rights: use \"Check now\" on the Power page".into(),
        );
    };
    let blockers: Vec<_> = r.iter().filter(|r| r.blocks_sleep()).collect();
    if blockers.is_empty() {
        return Outcome::Pass("Nothing is keeping the PC awake".into());
    }
    let list: Vec<String> = blockers
        .iter()
        .map(|b| {
            if b.reason.is_empty() {
                b.short_name().to_string()
            } else {
                format!("{} ({})", b.short_name(), b.reason)
            }
        })
        .collect();
    Outcome::Findings(vec![finding(
        Level::Info,
        Category::Power,
        format!(
            "{} keeping the PC awake",
            plural(blockers.len(), "app or driver is", "apps or drivers are")
        ),
        format!(
            "Windows won't sleep on its own while these power requests are active: {}.",
            list.join("; ")
        ),
        &[
            "If the PC never goes to sleep, close or pause the app listed (a browser tab playing media, a game launcher, a download).",
            "\"An audio stream is currently in use\" means some app keeps the sound device open: close apps one by one to find it.",
            "To make Windows ignore one, an administrator can run: powercfg /requestsoverride PROCESS app.exe SYSTEM",
        ],
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SystemInfo;
    use crate::power::{AcDc, AdapterPower, PowerInfo, PowerRequest, UsbSensitive, WakeEvent};
    use crate::sensors::SensorTable;

    fn ctx<'a>(info: &'a SystemInfo, s: &'a SensorTable, p: &'a PowerInfo) -> Context<'a> {
        Context {
            info,
            sensors: s,
            today: (2026, 10, 12),
            uptime_secs: Some(3600),
            power: Some(p),
            network: None,
            whea_live: &[],
        }
    }

    fn with(values: &[(Setting, u32, u32)]) -> PowerInfo {
        PowerInfo {
            values: values
                .iter()
                .map(|&(s, ac, dc)| {
                    (
                        s,
                        AcDc {
                            ac: Some(ac),
                            dc: Some(dc),
                        },
                    )
                })
                .collect(),
            ..Default::default()
        }
    }

    fn fixes(o: &Outcome) -> Vec<Option<Action>> {
        match o {
            Outcome::Findings(f) => f.iter().map(|f| f.fix.clone()).collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn skipped_without_power_info() {
        let (i, s) = (SystemInfo::default(), SensorTable::default());
        let c = Context {
            info: &i,
            sensors: &s,
            today: (2026, 10, 12),
            uptime_secs: None,
            power: None,
            network: None,
            whea_live: &[],
        };
        for (_, _, check) in CHECKS {
            assert!(matches!(check(&c), Outcome::Skipped(_)));
        }
    }

    #[test]
    fn usb_suspend_only_with_sensitive_devices() {
        let (i, s) = (SystemInfo::default(), SensorTable::default());
        let mut p = with(&[(Setting::UsbSelectiveSuspend, 1, 1)]);
        assert!(matches!(
            usb_selective_suspend(&ctx(&i, &s, &p)),
            Outcome::Pass(_)
        ));
        p.sensitive_usb = vec![("Scarlett Solo USB".into(), UsbSensitive::Audio)];
        let o = usb_selective_suspend(&ctx(&i, &s, &p));
        assert_eq!(
            fixes(&o),
            vec![Some(Action::SetValue {
                setting: Setting::UsbSelectiveSuspend,
                source: Source::Both,
                value: 0
            })]
        );
        // Laptops keep it on battery.
        p.caps.battery = true;
        let o = usb_selective_suspend(&ctx(&i, &s, &p));
        assert!(matches!(
            &fixes(&o)[0],
            Some(Action::SetValue {
                source: Source::Ac,
                ..
            })
        ));
        let p = with(&[(Setting::UsbSelectiveSuspend, 0, 1)]);
        assert!(matches!(
            usb_selective_suspend(&ctx(&i, &s, &p)),
            Outcome::Pass(_)
        ));
    }

    #[test]
    fn aspm_desktop_only() {
        let (i, s) = (SystemInfo::default(), SensorTable::default());
        let mut p = with(&[(Setting::PcieAspm, 2, 2)]);
        assert_eq!(fixes(&pcie_aspm(&ctx(&i, &s, &p))).len(), 1);
        p.caps.battery = true;
        assert!(matches!(pcie_aspm(&ctx(&i, &s, &p)), Outcome::Skipped(_)));
        let p = with(&[(Setting::PcieAspm, 1, 2)]);
        assert!(matches!(pcie_aspm(&ctx(&i, &s, &p)), Outcome::Pass(_)));
    }

    #[test]
    fn sleep_timers() {
        let (i, s) = (SystemInfo::default(), SensorTable::default());
        let p = with(&[(Setting::Sleep, 300, 300), (Setting::DisplayOff, 120, 60)]);
        assert_eq!(fixes(&sleep_timer(&ctx(&i, &s, &p))).len(), 1);
        let p = with(&[(Setting::Sleep, 900, 300), (Setting::DisplayOff, 1800, 60)]);
        assert_eq!(fixes(&sleep_timer(&ctx(&i, &s, &p))), vec![None]);
        let p = with(&[(Setting::Sleep, 0, 300), (Setting::DisplayOff, 600, 60)]);
        assert!(matches!(sleep_timer(&ctx(&i, &s, &p)), Outcome::Pass(_)));
        let p = with(&[(Setting::Sleep, 1800, 300), (Setting::DisplayOff, 600, 60)]);
        assert!(matches!(sleep_timer(&ctx(&i, &s, &p)), Outcome::Pass(_)));
    }

    #[test]
    fn processor_caps() {
        let (i, s) = (SystemInfo::default(), SensorTable::default());
        let p = with(&[
            (Setting::ProcessorMin, 5, 100),
            (Setting::ProcessorMax, 99, 100),
        ]);
        // Desktop: only the AC cap counts.
        assert_eq!(fixes(&processor_state(&ctx(&i, &s, &p))).len(), 1);
        let mut p = p;
        p.caps.battery = true;
        assert_eq!(fixes(&processor_state(&ctx(&i, &s, &p))).len(), 2);
        let p = with(&[
            (Setting::ProcessorMin, 100, 5),
            (Setting::ProcessorMax, 100, 100),
        ]);
        assert!(matches!(
            processor_state(&ctx(&i, &s, &p)),
            Outcome::Pass(_)
        ));
    }

    fn wake(source: WakeSource, fast_startup: bool) -> WakeEvent {
        WakeEvent {
            time: "2026-10-10 03:00".into(),
            source,
            fast_startup,
        }
    }

    #[test]
    fn wakes() {
        let (i, s) = (SystemInfo::default(), SensorTable::default());
        let mut p = with(&[(Setting::WakeTimers, 1, 1)]);
        p.wakes = vec![
            wake(WakeSource::Unknown, true),
            wake(WakeSource::PowerButton, false),
        ];
        assert!(matches!(
            unexpected_wakes(&ctx(&i, &s, &p)),
            Outcome::Pass(_)
        ));
        p.wakes
            .push(wake(WakeSource::Timer("Update".into()), false));
        assert!(matches!(
            &fixes(&unexpected_wakes(&ctx(&i, &s, &p)))[0],
            Some(Action::SetValue {
                setting: Setting::WakeTimers,
                ..
            })
        ));
        // Wake timers already off: offer to disarm the network adapter instead.
        let mut p = with(&[(Setting::WakeTimers, 0, 0)]);
        p.wake_armed = vec!["Intel(R) Ethernet Controller (3) I225-V #2".into()];
        p.wakes = vec![wake(
            WakeSource::Device("Intel(R) Ethernet Controller (3) I225-V".into()),
            false,
        )];
        assert_eq!(
            fixes(&unexpected_wakes(&ctx(&i, &s, &p))),
            vec![Some(Action::DisarmWake(
                "Intel(R) Ethernet Controller (3) I225-V #2".into()
            ))]
        );
    }

    #[test]
    fn fast_startup_with_long_uptime() {
        let (i, s) = (SystemInfo::default(), SensorTable::default());
        let mut p = PowerInfo {
            fast_startup: Some(true),
            ..Default::default()
        };
        p.caps.hibernate = true;
        p.caps.hiberfile = true;
        assert!(matches!(fast_startup(&ctx(&i, &s, &p)), Outcome::Pass(_)));
        let mut c = ctx(&i, &s, &p);
        c.uptime_secs = Some(9 * 86400);
        assert_eq!(
            fixes(&fast_startup(&c)),
            vec![Some(Action::FastStartup(false))]
        );
    }

    #[test]
    fn adapters() {
        let (i, s) = (SystemInfo::default(), SensorTable::default());
        let mut p = PowerInfo::default();
        assert!(matches!(
            network_power_saving(&ctx(&i, &s, &p)),
            Outcome::Skipped(_)
        ));
        p.adapters = vec![
            AdapterPower {
                name: "Ethernet".into(),
                kind: AdapterKind::Ethernet,
                instance_id: "PCI\\X".into(),
                class_key: Some("0003".into()),
                allow_turn_off: Some(false),
                eee: Some(crate::power::EeeSetting {
                    value: "*EEE".into(),
                    enabled: true,
                }),
            },
            AdapterPower {
                name: "Bluetooth".into(),
                kind: AdapterKind::Bluetooth,
                instance_id: "USB\\Y".into(),
                class_key: None,
                allow_turn_off: Some(true),
                eee: None,
            },
        ];
        assert!(matches!(
            &fixes(&network_power_saving(&ctx(&i, &s, &p)))[..],
            [Some(Action::NicProperty { .. })]
        ));
        assert!(matches!(
            &fixes(&bluetooth_power_saving(&ctx(&i, &s, &p)))[..],
            [Some(Action::AllowTurnOff { allow: false, .. })]
        ));
    }

    #[test]
    fn blockers() {
        let (i, s) = (SystemInfo::default(), SensorTable::default());
        let mut p = PowerInfo::default();
        assert!(matches!(
            sleep_blockers(&ctx(&i, &s, &p)),
            Outcome::Skipped(_)
        ));
        p.requests = Some(vec![PowerRequest {
            kind: "DISPLAY".into(),
            origin: "PROCESS".into(),
            name: r"\Device\x\video.exe".into(),
            reason: String::new(),
        }]);
        assert!(matches!(sleep_blockers(&ctx(&i, &s, &p)), Outcome::Pass(_)));
        p.requests.as_mut().unwrap().push(PowerRequest {
            kind: "SYSTEM".into(),
            origin: "DRIVER".into(),
            name: "Realtek Audio".into(),
            reason: "An audio stream is currently in use.".into(),
        });
        assert!(matches!(
            sleep_blockers(&ctx(&i, &s, &p)),
            Outcome::Findings(_)
        ));
    }
}
